//! Ollama's `/api/generate` endpoint.

use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};

use super::{Backend, BackendError, GenerateOptions, GenerateRequest, LlmSettings};

/// How long Ollama keeps the model loaded after a request (its default is only 5 minutes).
const MODEL_KEEP_ALIVE: &str = "30m";

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    keep_alive: &'static str,
    options: OllamaOptions<'a>,
}

#[derive(Serialize)]
struct OllamaOptions<'a> {
    temperature: f32,
    num_predict: u32,
    repeat_penalty: f32,
    stop: &'a [String],
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagsModel>,
}

#[derive(Deserialize)]
struct TagsModel {
    name: String,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

/// Talks to an Ollama server over HTTP.
#[derive(Debug, Clone)]
pub struct OllamaBackend {
    settings: LlmSettings,
}

impl OllamaBackend {
    pub fn new(settings: LlmSettings) -> Self {
        Self { settings }
    }

    /// The JSON body Ollama receives; pinned by the golden request tests.
    fn request_body(&self, request: &GenerateRequest) -> String {
        let options = &request.options;
        let body = OllamaRequest {
            model: &self.settings.model,
            prompt: &request.prompt,
            stream: false,
            keep_alive: MODEL_KEEP_ALIVE,
            options: OllamaOptions {
                temperature: options.temperature,
                num_predict: options.num_predict,
                repeat_penalty: options.repeat_penalty,
                stop: &options.stop,
            },
        };
        serde_json::to_string(&body).expect("request body always serializes")
    }

    /// Map a failed send onto a `BackendError` naming `url`.
    fn transport_error(&self, url: &str, timeout_secs: u64, e: &reqwest::Error) -> BackendError {
        let (url, model) = (url.to_string(), self.settings.model.clone());
        if e.is_timeout() {
            BackendError::Timeout { url, timeout_secs }
        } else if e.is_connect() {
            BackendError::Connect { url, model, detail: e.to_string() }
        } else {
            BackendError::Transport { url, model, detail: e.to_string() }
        }
    }

    /// Send `request` and return the response's `response` field.
    fn post(&self, request: &GenerateRequest, timeout_secs: u64) -> Result<String, BackendError> {
        let url = &self.settings.api_url;
        let model = &self.settings.model;
        let transport = |e: reqwest::Error| self.transport_error(url, timeout_secs, &e);

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(transport)?;
        let response = client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(self.request_body(request))
            .send()
            .map_err(transport)?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(BackendError::Http {
                url: url.clone(),
                model: model.clone(),
                status: status.as_u16(),
                body,
            });
        }

        response
            .json::<OllamaResponse>()
            .map(|res| res.response)
            .map_err(|e| BackendError::Malformed { url: url.clone(), detail: e.to_string() })
    }
}

impl Backend for OllamaBackend {
    fn generate(&self, request: &GenerateRequest) -> Result<String, BackendError> {
        self.post(request, self.settings.timeout_secs)
    }

    /// Load the model and prime Ollama's prompt cache with `prefix`, so the first real request
    /// skips the cold start. Loading can take long, so this waits at least a minute.
    fn warm_up(&self, prefix: &str) -> Result<(), BackendError> {
        let request =
            GenerateRequest { prompt: prefix.to_string(), options: GenerateOptions::for_warm_up() };
        self.post(&request, self.settings.timeout_secs.max(60)).map(drop)
    }

    /// `GET /api/tags` on the server `api_url` points at.
    fn installed_models(&self) -> Result<Option<Vec<String>>, BackendError> {
        let api_url = &self.settings.api_url;
        let timeout_secs = self.settings.timeout_secs;
        let model = &self.settings.model;
        let mut url = reqwest::Url::parse(api_url).map_err(|e| BackendError::Transport {
            url: api_url.clone(),
            model: model.clone(),
            detail: e.to_string(),
        })?;
        url.set_path("/api/tags");
        url.set_query(None);
        let tags_url = url.to_string();
        let transport = |e: reqwest::Error| self.transport_error(&tags_url, timeout_secs, &e);

        let response = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(transport)?
            .get(url)
            .send()
            .map_err(transport)?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(BackendError::Http {
                url: tags_url,
                model: model.clone(),
                status: status.as_u16(),
                body,
            });
        }

        response
            .json::<TagsResponse>()
            .map(|tags| Some(tags.models.into_iter().map(|m| m.name).collect()))
            .map_err(|e| BackendError::Malformed { url: tags_url, detail: e.to_string() })
    }

    fn model(&self) -> &str {
        &self.settings.model
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    fn backend(url: &str, timeout_secs: u64) -> OllamaBackend {
        OllamaBackend::new(LlmSettings {
            api_url: url.to_string(),
            model: "qwen2.5-coder:3b".to_string(),
            temperature: 0.0,
            timeout_secs,
        })
    }

    fn golden_body(options: GenerateOptions) -> String {
        let backend = backend("http://localhost:11434/api/generate", 15);
        backend.request_body(&GenerateRequest { prompt: "P".to_string(), options })
    }

    fn command_request() -> GenerateRequest {
        GenerateRequest { prompt: "P".into(), options: GenerateOptions::for_command(0.0) }
    }

    /// Read one HTTP request (headers plus a `Content-Length` body) so the client sees a
    /// well-behaved server.
    fn read_request(stream: &mut TcpStream) -> String {
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&data);
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().ok())?
                    })
                    .unwrap_or(0usize);
                if data.len() >= end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    /// Serve one canned HTTP response on a local port; returns the URL and the received request.
    fn serve_once(status: &str, body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/generate", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = thread::spawn(move || {
            // Give up if no client comes, so a broken backend fails the test instead of hanging.
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                    Err(_) => return String::new(),
                }
            };
            stream.set_nonblocking(false).unwrap();
            let request = read_request(&mut stream);
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        (url, handle)
    }

    /// A URL on which nothing is listening.
    ///
    /// Binds port 0 and drops the listener, so there is a tiny window in which another
    /// process could grab the port; acceptable for tests.
    fn closed_port_url() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/generate", listener.local_addr().unwrap());
        drop(listener);
        url
    }

    #[test]
    fn golden_request_body_command() {
        let fixture = include_str!("golden/request_command.json");
        assert_eq!(golden_body(GenerateOptions::for_command(0.0)), fixture.trim_end_matches('\n'));
    }

    #[test]
    fn golden_request_body_warm_up() {
        let fixture = include_str!("golden/request_warm_up.json");
        assert_eq!(golden_body(GenerateOptions::for_warm_up()), fixture.trim_end_matches('\n'));
    }

    #[test]
    fn request_keeps_the_model_loaded() {
        let json: serde_json::Value =
            serde_json::from_str(&golden_body(GenerateOptions::for_warm_up())).unwrap();
        assert_eq!(json["keep_alive"], "30m");
        assert_eq!(json["options"]["num_predict"], 1);
    }

    #[test]
    fn request_options_cap_length_and_repetition() {
        let json: serde_json::Value =
            serde_json::from_str(&golden_body(GenerateOptions::for_command(0.0))).unwrap();
        assert_eq!(json["options"]["num_predict"], 128);
        assert!((json["options"]["repeat_penalty"].as_f64().unwrap() - 1.1).abs() < 1e-6);
        assert_eq!(json["options"]["stop"], serde_json::json!(["\n\n"]));
        assert_eq!(json["options"]["temperature"], 0.0);
    }

    #[test]
    fn ollama_200_returns_the_response_field_and_sends_the_golden_body() {
        let (url, server) = serve_once("200 OK", r#"{"response":"ls -la","done":true}"#);
        let backend = backend(&url, 5);
        assert_eq!(backend.generate(&command_request()).unwrap(), "ls -la");
        let sent = server.join().unwrap();
        assert!(sent.starts_with("POST /api/generate "), "{sent}");
        assert!(sent.to_ascii_lowercase().contains("content-type: application/json"), "{sent}");
        assert!(sent.ends_with(&backend.request_body(&command_request())), "{sent}");
    }

    #[test]
    fn ollama_200_with_garbage_body_is_malformed_error() {
        let (url, server) = serve_once("200 OK", "this is not json");
        let err = backend(&url, 5).generate(&command_request()).unwrap_err();
        server.join().unwrap();
        assert!(matches!(err, BackendError::Malformed { .. }), "{err:?}");
        let msg = err.to_string();
        assert!(msg.starts_with(&format!("Unexpected response from {url}: ")), "{msg}");
    }

    #[test]
    fn ollama_404_gives_pull_hint() {
        let (url, server) =
            serve_once("404 Not Found", r#"{"error":"model 'qwen2.5-coder:3b' not found"}"#);
        let err = backend(&url, 5).generate(&command_request()).unwrap_err();
        server.join().unwrap();
        assert!(matches!(err, BackendError::Http { status: 404, .. }), "{err:?}");
        assert_eq!(
            err.to_string(),
            "The model \"qwen2.5-coder:3b\" isn't installed. Run `ollama pull qwen2.5-coder:3b`."
        );
    }

    #[test]
    fn ollama_500_reports_status_and_body() {
        let (url, server) = serve_once("500 Internal Server Error", "out of memory");
        let err = backend(&url, 5).generate(&command_request()).unwrap_err();
        server.join().unwrap();
        assert_eq!(
            err.to_string(),
            format!("The AI server at {url} returned HTTP 500: out of memory")
        );
    }

    #[test]
    fn ollama_connection_refused_says_start_ollama() {
        let url = closed_port_url();
        let err = backend(&url, 5).generate(&command_request()).unwrap_err();
        assert!(matches!(err, BackendError::Connect { .. }), "{err:?}");
        assert_eq!(
            err.to_string(),
            format!(
                "Can't reach the AI server at {url}. Is Ollama running? Start it with `ollama serve`."
            )
        );
    }

    #[test]
    fn ollama_timeout_message_names_seconds() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/generate", listener.local_addr().unwrap());
        // Accept and read, but never answer; the read ends when the client gives up.
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            while matches!(stream.read(&mut buf), Ok(n) if n > 0) {}
        });

        let started = Instant::now();
        let err = backend(&url, 1).generate(&command_request()).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            err.to_string(),
            format!(
                "The AI server at {url} did not answer within 1s (the model may still be \
                 loading; try again)."
            )
        );
    }

    #[test]
    fn warm_up_sends_the_prefix_with_one_token() {
        let (url, server) = serve_once("200 OK", r#"{"response":"x","done":true}"#);
        backend(&url, 5).warm_up("PREFIX").unwrap();
        let sent = server.join().unwrap();
        let body = &sent[sent.find("\r\n\r\n").unwrap() + 4..];
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["prompt"], "PREFIX");
        assert_eq!(json["keep_alive"], "30m");
        assert_eq!(json["options"]["num_predict"], 1);
    }

    #[test]
    fn warm_up_failure_returns_error_without_panic() {
        let url = closed_port_url();
        let err = backend(&url, 5).warm_up("PREFIX").unwrap_err();
        assert!(matches!(err, BackendError::Connect { .. }), "{err:?}");
    }

    #[test]
    fn installed_models_parses_tags() {
        let (url, server) =
            serve_once("200 OK", r#"{"models":[{"name":"a:1"},{"name":"b:latest"}]}"#);
        let models = backend(&url, 5).installed_models().unwrap();
        server.join().unwrap();
        assert_eq!(models, Some(vec!["a:1".to_string(), "b:latest".to_string()]));
    }

    #[test]
    fn installed_models_garbage_body_is_malformed_error() {
        let (url, server) = serve_once("200 OK", "this is not json");
        let err = backend(&url, 5).installed_models().unwrap_err();
        server.join().unwrap();
        assert!(matches!(err, BackendError::Malformed { .. }), "{err:?}");
    }

    #[test]
    fn installed_models_uses_tags_path_on_the_configured_host() {
        let (url, server) = serve_once("200 OK", r#"{"models":[]}"#);
        backend(&url, 5).installed_models().unwrap();
        let sent = server.join().unwrap();
        assert!(sent.starts_with("GET /api/tags "), "{sent}");
    }

    #[test]
    fn installed_models_unreachable_is_connect_error() {
        let err = backend(&closed_port_url(), 5).installed_models().unwrap_err();
        assert!(matches!(err, BackendError::Connect { .. }), "{err:?}");
    }

    #[test]
    fn model_is_the_configured_model() {
        assert_eq!(backend("http://x", 1).model(), "qwen2.5-coder:3b");
    }
}

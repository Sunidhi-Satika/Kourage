//! The LLM behind a swappable `Backend` trait.

use std::fmt;

use crate::diagnostics::llm_failure_message;

pub mod mock;
pub mod ollama;

pub use mock::MockBackend;
pub use ollama::OllamaBackend;

/// Where and how to reach the LLM server.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmSettings {
    pub api_url: String,
    pub model: String,
    /// The configured default; the per-request `GenerateOptions.temperature` wins.
    pub temperature: f32,
    pub timeout_secs: u64,
}

/// Sampling options for one generation.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateOptions {
    pub temperature: f32,
    /// Hard cap on generated tokens; a command never needs more.
    pub num_predict: u32,
    /// Discourages the repetition loops small models fall into.
    pub repeat_penalty: f32,
    pub stop: Vec<String>,
}

impl GenerateOptions {
    /// Options for turning an instruction into a single command.
    pub fn for_command(temperature: f32) -> Self {
        Self { temperature, num_predict: 128, repeat_penalty: 1.1, stop: vec!["\n\n".into()] }
    }

    /// Options for the startup warm-up: process the prompt, generate (almost) nothing.
    pub fn for_warm_up() -> Self {
        Self { temperature: 0.0, num_predict: 1, repeat_penalty: 1.1, stop: vec!["\n\n".into()] }
    }
}

/// One prompt to complete.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateRequest {
    pub prompt: String,
    pub options: GenerateOptions,
}

/// Why a backend call failed. `Display` is the message shown to the user.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendError {
    /// No answer within the configured timeout.
    Timeout { url: String, timeout_secs: u64 },
    /// The server could not be reached at all.
    Connect { url: String, model: String, detail: String },
    /// Any other failure to send the request or receive the response.
    Transport { url: String, model: String, detail: String },
    /// The server answered with a non-success HTTP status.
    Http { url: String, model: String, status: u16, body: String },
    /// The server answered success, but the body could not be understood.
    Malformed { url: String, detail: String },
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { url, timeout_secs } => write!(
                f,
                "The AI server at {url} did not answer within {timeout_secs}s (the model may \
                 still be loading; try again)."
            ),
            Self::Connect { url, model, detail } => {
                f.write_str(&llm_failure_message(url, model, None, detail, true))
            },
            Self::Transport { url, model, detail } => {
                f.write_str(&llm_failure_message(url, model, None, detail, false))
            },
            Self::Http { url, model, status, body } => {
                f.write_str(&llm_failure_message(url, model, Some(*status), body, false))
            },
            Self::Malformed { url, detail } => {
                write!(f, "Unexpected response from {url}: {detail}")
            },
        }
    }
}

impl std::error::Error for BackendError {}

/// Something that completes prompts.
pub trait Backend: Send + Sync {
    /// Complete the prompt, blocking until the reply arrives or the request fails.
    fn generate(&self, request: &GenerateRequest) -> Result<String, BackendError>;

    /// Prepare for requests starting with `prefix` (e.g. load the model). Default: nothing.
    fn warm_up(&self, _prefix: &str) -> Result<(), BackendError> {
        Ok(())
    }

    /// The names of the models installed on the server. `None` means this backend cannot
    /// list them. Default: `None`.
    fn installed_models(&self) -> Result<Option<Vec<String>>, BackendError> {
        Ok(None)
    }

    /// The model name, for status text.
    fn model(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://localhost:11434/api/generate";

    #[test]
    fn command_options_cap_length_and_repetition() {
        let options = GenerateOptions::for_command(0.0);
        assert_eq!(options.num_predict, 128);
        assert!((options.repeat_penalty - 1.1).abs() < 1e-6);
        assert_eq!(options.stop, vec!["\n\n".to_string()]);
        assert_eq!(options.temperature, 0.0);
    }

    #[test]
    fn warm_up_options_generate_one_token() {
        assert_eq!(GenerateOptions::for_warm_up().num_predict, 1);
    }

    #[test]
    fn timeout_error_text_is_unchanged() {
        let err = BackendError::Timeout { url: URL.into(), timeout_secs: 15 };
        assert_eq!(
            err.to_string(),
            "The AI server at http://localhost:11434/api/generate did not answer within 15s (the \
             model may still be loading; try again)."
        );
    }

    #[test]
    fn transport_error_reports_the_detail() {
        let err = BackendError::Transport {
            url: URL.into(),
            model: "m".into(),
            detail: "connection reset".into(),
        };
        assert_eq!(
            err.to_string(),
            format!("Request to the AI server at {URL} failed: connection reset")
        );
    }

    #[test]
    fn http_error_uses_diagnostics() {
        let err = BackendError::Http {
            url: URL.into(),
            model: "m".into(),
            status: 500,
            body: "boom".into(),
        };
        assert_eq!(err.to_string(), llm_failure_message(URL, "m", Some(500), "boom", false));
    }

    #[test]
    fn default_warm_up_is_a_no_op() {
        struct Silent;
        impl Backend for Silent {
            fn generate(&self, _: &GenerateRequest) -> Result<String, BackendError> {
                unreachable!()
            }

            fn model(&self) -> &str {
                "silent"
            }
        }
        assert_eq!(Silent.warm_up("prefix"), Ok(()));
    }
}

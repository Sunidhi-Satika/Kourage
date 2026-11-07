//! Actionable error messages for missing or unreachable AI dependencies.

/// Explain a failed request to the LLM server in terms the user can act on.
///
/// `status` is the HTTP status when the server answered, `connect_failed` is set when it could
/// not be reached at all.
pub fn llm_failure_message(
    url: &str,
    model: &str,
    status: Option<u16>,
    body: &str,
    connect_failed: bool,
) -> String {
    if connect_failed {
        return format!(
            "Can't reach the AI server at {url}. Is Ollama running? Start it with `ollama serve`."
        );
    }

    let detail: String = body.trim().chars().take(200).collect();
    if status == Some(404) || detail.contains("not found") {
        return format!("The model \"{model}\" isn't installed. Run `ollama pull {model}`.");
    }

    match status {
        Some(code) => format!("The AI server at {url} returned HTTP {code}: {detail}"),
        None => format!("Request to the AI server at {url} failed: {detail}"),
    }
}

/// Explain how to get the Whisper model the voice feature needs.
pub fn whisper_missing_message(path: &str) -> String {
    format!(
        "Whisper model not found at \"{path}\". Download one (e.g. ggml-base.en.bin from \
         https://huggingface.co/ggerganov/whisper.cpp/tree/main) and set \
         [ai] whisper_model_path in alacritty.toml."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://localhost:11434/api/generate";

    #[test]
    fn unreachable_server_says_to_start_ollama() {
        let msg = llm_failure_message(URL, "qwen2.5-coder:3b", None, "", true);
        assert!(msg.contains("ollama serve"), "{msg}");
        assert!(msg.contains(URL), "{msg}");
    }

    #[test]
    fn missing_model_says_to_pull_it() {
        let body = r#"{"error":"model 'qwen2.5-coder:3b' not found"}"#;
        let msg = llm_failure_message(URL, "qwen2.5-coder:3b", Some(404), body, false);
        assert!(msg.contains("ollama pull qwen2.5-coder:3b"), "{msg}");
    }

    #[test]
    fn other_http_errors_show_status_and_body() {
        let msg = llm_failure_message(URL, "m", Some(500), "boom", false);
        assert!(msg.contains("HTTP 500") && msg.contains("boom"), "{msg}");
    }

    #[test]
    fn long_error_bodies_are_truncated() {
        let msg = llm_failure_message(URL, "m", Some(500), &"x".repeat(5000), false);
        assert!(msg.len() < 600, "message too long: {}", msg.len());
    }

    #[test]
    fn whisper_message_names_path_config_key_and_download() {
        let msg = whisper_missing_message("/path/to/your/whisper/model.bin");
        assert!(msg.contains("/path/to/your/whisper/model.bin"), "{msg}");
        assert!(msg.contains("whisper_model_path"), "{msg}");
        assert!(msg.contains("ggml-base.en.bin"), "{msg}");
    }
}

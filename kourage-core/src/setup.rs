//! Checks that the AI dependencies (the LLM and its model, the Whisper model) are in place.

use std::path::Path;

use crate::backend::Backend;
use crate::diagnostics::{llm_failure_message, whisper_missing_message};

/// The `whisper_model_path` shipped in the example config; it means "not configured yet".
pub const WHISPER_PLACEHOLDER_PATH: &str = "/path/to/your/whisper/model.bin";

/// One thing that must be fixed before the AI features fully work.
#[derive(Debug, Clone, PartialEq)]
pub enum SetupIssue {
    BackendUnreachable { message: String },
    ModelMissing { model: String, message: String },
    WhisperNotConfigured { message: String },
    WhisperFileMissing { path: String, message: String },
}

impl SetupIssue {
    /// The user-facing explanation, with the fix.
    pub fn message(&self) -> &str {
        match self {
            Self::BackendUnreachable { message }
            | Self::ModelMissing { message, .. }
            | Self::WhisperNotConfigured { message }
            | Self::WhisperFileMissing { message, .. } => message,
        }
    }
}

/// Whether `wanted` is among the `installed` names. A name without a tag means `:latest`;
/// an explicit tag must match exactly.
pub fn model_is_installed(wanted: &str, installed: &[String]) -> bool {
    let wanted = if wanted.contains(':') { wanted.to_string() } else { format!("{wanted}:latest") };
    installed.contains(&wanted)
}

/// Everything wrong with the setup: backend and model issues first, then Whisper issues.
pub fn check_setup(backend: &dyn Backend, whisper_model_path: &str) -> Vec<SetupIssue> {
    let mut issues = Vec::new();

    match backend.installed_models() {
        Ok(Some(installed)) if !model_is_installed(backend.model(), &installed) => {
            let model = backend.model().to_string();
            let message = llm_failure_message("", &model, Some(404), "", false);
            issues.push(SetupIssue::ModelMissing { model, message });
        },
        Ok(_) => (),
        Err(error) => issues.push(SetupIssue::BackendUnreachable { message: error.to_string() }),
    }

    if whisper_model_path == WHISPER_PLACEHOLDER_PATH {
        issues.push(SetupIssue::WhisperNotConfigured {
            message: format!(
                "Voice input is not set up: [ai] whisper_model_path in alacritty.toml is still \
                 the placeholder. {}",
                whisper_missing_message(whisper_model_path)
            ),
        });
    } else if !Path::new(whisper_model_path).exists() {
        issues.push(SetupIssue::WhisperFileMissing {
            path: whisper_model_path.to_string(),
            message: whisper_missing_message(whisper_model_path),
        });
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{GenerateRequest, MockBackend};
    use crate::{BackendError, whisper_missing_message};

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// A temp file that exists for the test's duration.
    fn temp_file(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("kourage-setup-{}-{name}", std::process::id()));
        std::fs::write(&path, b"x").unwrap();
        path
    }

    #[test]
    fn model_is_installed_matches_latest_tag() {
        assert!(model_is_installed("llama3", &installed(&["llama3:latest"])));
        assert!(model_is_installed("qwen2.5-coder:3b", &installed(&["qwen2.5-coder:3b"])));
        assert!(!model_is_installed("qwen2.5-coder:3b", &installed(&["qwen2.5-coder:7b"])));
        assert!(!model_is_installed("llama3:8b", &installed(&["llama3:latest"])));
    }

    #[test]
    fn check_setup_reports_unreachable_backend() {
        let file = temp_file("unreachable");
        let issues = check_setup(&MockBackend::unreachable(), file.to_str().unwrap());
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(matches!(issues[0], SetupIssue::BackendUnreachable { .. }), "{issues:?}");
        assert!(!issues.iter().any(|i| matches!(i, SetupIssue::ModelMissing { .. })));
    }

    #[test]
    fn check_setup_reports_missing_model() {
        let file = temp_file("missing-model");
        let backend = MockBackend::with_models(&["other:1b"]);
        let issues = check_setup(&backend, file.to_str().unwrap());
        assert_eq!(issues.len(), 1, "{issues:?}");
        match &issues[0] {
            SetupIssue::ModelMissing { model, message } => {
                assert_eq!(model, "mock");
                assert!(message.contains("ollama pull mock"), "{message}");
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn check_setup_ok_when_model_present_and_whisper_exists() {
        let file = temp_file("ok");
        let backend = MockBackend::with_models(&["mock:latest"]);
        assert_eq!(check_setup(&backend, file.to_str().unwrap()), vec![]);
    }

    #[test]
    fn placeholder_whisper_path_is_not_configured() {
        let backend = MockBackend::with_models(&["mock:latest"]);
        let issues = check_setup(&backend, WHISPER_PLACEHOLDER_PATH);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(matches!(issues[0], SetupIssue::WhisperNotConfigured { .. }), "{issues:?}");
        assert!(issues[0].message().contains("whisper_model_path"));
    }

    #[test]
    fn missing_whisper_file_reports_download_instructions() {
        let backend = MockBackend::with_models(&["mock:latest"]);
        let path = "/definitely/not/here/model.bin";
        let issues = check_setup(&backend, path);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(matches!(issues[0], SetupIssue::WhisperFileMissing { .. }), "{issues:?}");
        assert_eq!(issues[0].message(), whisper_missing_message(path));
    }

    #[test]
    fn backend_that_cannot_list_models_is_not_an_issue() {
        struct Opaque;
        impl Backend for Opaque {
            fn generate(&self, _: &GenerateRequest) -> Result<String, BackendError> {
                unreachable!()
            }

            fn model(&self) -> &str {
                "opaque"
            }
        }
        let file = temp_file("opaque");
        assert_eq!(check_setup(&Opaque, file.to_str().unwrap()), vec![]);
    }
}

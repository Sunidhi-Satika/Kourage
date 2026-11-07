//! UI-free core of the Kourage AI terminal.

pub mod ask;
pub mod backend;
pub mod context;
pub mod diagnostics;
pub mod platform;
pub mod prompt;
pub mod reply;
pub mod safety;
pub mod setup;

pub use ask::{ask, AskError, Proposal};
pub use backend::{
    Backend, BackendError, GenerateOptions, GenerateRequest, LlmSettings, MockBackend,
    OllamaBackend,
};
pub use context::AiContext;
pub use diagnostics::{llm_failure_message, whisper_missing_message};
pub use platform::{Platform, SystemPlatform};
pub use prompt::PROMPT_EXAMPLES;
pub use reply::{clean_llm_reply, finalize_reply, ReplyError};
pub use safety::is_destructive_command;
pub use setup::{check_setup, model_is_installed, SetupIssue, WHISPER_PLACEHOLDER_PATH};

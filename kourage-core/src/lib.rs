//! UI-free core of the Kourage AI terminal.

pub mod diagnostics;
pub mod reply;
pub mod safety;

pub use diagnostics::{llm_failure_message, whisper_missing_message};
pub use reply::{clean_llm_reply, finalize_llm_reply, require_single_line};
pub use safety::is_destructive_command;

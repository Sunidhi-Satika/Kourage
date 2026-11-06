//! AI modules for Kourage terminal.

pub mod context;
pub mod diagnostics;
pub mod reply;
pub mod safety;

pub use context::AiContext;
pub use reply::{clean_llm_reply, require_single_line};
pub use safety::is_destructive_command;

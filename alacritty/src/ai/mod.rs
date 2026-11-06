//! AI modules for Kourage terminal.

pub mod context;
pub mod diagnostics;
pub mod reply;
pub mod status;
pub mod safety;

pub use context::AiContext;
pub use reply::{clean_llm_reply, finalize_llm_reply, require_single_line};
pub use status::{AiPhase, AiStatus};
pub use safety::is_destructive_command;

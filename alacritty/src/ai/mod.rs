//! AI modules for Kourage terminal.

pub mod context;
pub mod status;

pub use context::AiContext;
pub use kourage_core::diagnostics;
pub use kourage_core::{
    clean_llm_reply, finalize_llm_reply, is_destructive_command, require_single_line,
};
pub use status::{AiPhase, AiStatus};

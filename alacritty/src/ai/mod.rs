//! AI modules for Kourage terminal.

pub mod context;
pub mod reply;
pub mod safety;

pub use context::AiContext;
pub use reply::clean_llm_reply;
pub use safety::is_destructive_command;

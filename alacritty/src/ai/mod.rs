//! AI modules for Kourage terminal.

pub mod context;
pub mod safety;

pub use context::AiContext;
pub use safety::is_destructive_command;

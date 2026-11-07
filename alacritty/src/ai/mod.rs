//! AI modules for Kourage terminal.

pub mod context;
pub mod status;

pub use kourage_core::diagnostics;
pub use kourage_core::AiContext;
pub use status::{AiPhase, AiStatus};

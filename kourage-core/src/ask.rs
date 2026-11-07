//! The one entry point the terminal calls: instruction in, vetted command proposal out.

use std::fmt;

use crate::backend::{Backend, BackendError, GenerateOptions, GenerateRequest};
use crate::context::AiContext;
use crate::reply::{finalize_reply, ReplyError};
use crate::safety::is_destructive_command;

/// A single command ready to preview, with its safety classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub command: String,
    pub is_destructive: bool,
}

/// Why `ask` produced no proposal. `Display` is the message shown to the user.
#[derive(Debug, Clone, PartialEq)]
pub enum AskError {
    /// The reply was empty after cleaning.
    EmptyReply,
    /// The reply held this many lines instead of one command.
    MultiLine(usize),
    /// The backend call failed.
    Backend(BackendError),
}

impl fmt::Display for AskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyReply => ReplyError::Empty.fmt(f),
            Self::MultiLine(n) => ReplyError::MultiLine(*n).fmt(f),
            Self::Backend(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for AskError {}

impl From<BackendError> for AskError {
    fn from(e: BackendError) -> Self {
        Self::Backend(e)
    }
}

impl From<ReplyError> for AskError {
    fn from(e: ReplyError) -> Self {
        match e {
            ReplyError::Empty => Self::EmptyReply,
            ReplyError::MultiLine(n) => Self::MultiLine(n),
        }
    }
}

/// Turn a natural-language `instruction` into one vetted command via `backend`.
pub fn ask(
    backend: &dyn Backend,
    context: &AiContext,
    instruction: &str,
    temperature: f32,
) -> Result<Proposal, AskError> {
    let request = GenerateRequest {
        prompt: context.format_prompt(instruction),
        options: GenerateOptions::for_command(temperature),
    };
    let raw = backend.generate(&request)?;
    let command = finalize_reply(&raw)?;
    let is_destructive = is_destructive_command(&command);
    Ok(Proposal { command, is_destructive })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MockBackend;

    fn run(reply: &str) -> Result<Proposal, AskError> {
        ask(&MockBackend::replying(&[reply]), &AiContext::default(), "disk usage", 0.1)
    }

    #[test]
    fn ask_returns_clean_safe_command() {
        let p = run("`du -sh .`").unwrap();
        assert_eq!(p, Proposal { command: "du -sh .".into(), is_destructive: false });
    }

    #[test]
    fn ask_flags_destructive_reply() {
        let p = run("rm -rf ~").unwrap();
        assert_eq!(p.command, "rm -rf ~");
        assert!(p.is_destructive);
    }

    #[test]
    fn ask_rejects_multi_line_reply() {
        let err = run("ls\nrm -rf /").unwrap_err();
        assert_eq!(err, AskError::MultiLine(2));
    }

    #[test]
    fn ask_rejects_empty_whitespace_and_empty_fence_replies() {
        for raw in ["", "  \n\t ", "```\n```"] {
            assert_eq!(run(raw).unwrap_err(), AskError::EmptyReply, "{raw:?}");
        }
    }

    #[test]
    fn ask_sends_prompt_with_instruction_last() {
        let backend = MockBackend::replying(&["ls"]);
        let context = AiContext { cwd: Some("/tmp".into()), ..AiContext::default() };
        ask(&backend, &context, "list files", 0.1).unwrap();
        let requests = backend.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].prompt.ends_with("\n### User Instruction:\n\"list files\"\n"));
        assert_eq!(requests[0].prompt, context.format_prompt("list files"));
    }

    #[test]
    fn ask_propagates_backend_error_text() {
        let e = BackendError::Timeout { url: "http://h:1".into(), timeout_secs: 15 };
        let text = e.to_string();
        let err = ask(&MockBackend::failing(e), &AiContext::default(), "x", 0.1).unwrap_err();
        assert!(matches!(err, AskError::Backend(_)));
        assert_eq!(err.to_string(), text);
    }

    #[test]
    fn ask_uses_command_options() {
        let backend = MockBackend::replying(&["ls"]);
        ask(&backend, &AiContext::default(), "x", 0.3).unwrap();
        let options = backend.requests()[0].options.clone();
        assert_eq!(options, GenerateOptions::for_command(0.3));
        assert_eq!(options.num_predict, 128);
        assert_eq!(options.stop, vec!["\n\n".to_string()]);
    }

    #[test]
    fn ask_error_display_matches_existing_messages() {
        assert_eq!(
            AskError::EmptyReply.to_string(),
            "The model returned an empty command. Try rephrasing."
        );
        assert_eq!(
            AskError::MultiLine(3).to_string(),
            "The model returned 3 lines but a single command is required. Try rephrasing."
        );
        let e = BackendError::Timeout { url: "u".into(), timeout_secs: 1 };
        assert_eq!(AskError::from(e.clone()).to_string(), e.to_string());
    }
}

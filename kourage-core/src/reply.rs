//! Cleanup of raw LLM replies into a bare shell command.

/// Strip markdown code fences and a single wrapping backtick pair from an LLM reply.
pub fn clean_llm_reply(raw: &str) -> String {
    let text = raw.trim();

    if let Some(rest) = text.strip_prefix("```") {
        // Drop an optional language tag, which runs to the first newline.
        let body = match rest.find('\n') {
            Some(idx) => &rest[idx + 1..],
            None => rest,
        };
        let body = body.rfind("```").map_or(body, |idx| &body[..idx]);
        return body.trim().to_string();
    }

    if let Some(inner) = text.strip_prefix('`').and_then(|t| t.strip_suffix('`')) {
        if !inner.contains('`') {
            return inner.trim().to_string();
        }
    }

    text.to_string()
}

/// Require a reply to be exactly one line, so one confirm never runs several commands.
pub fn require_single_line(cmd: &str) -> Result<String, String> {
    let lines: Vec<&str> = cmd.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    match lines.as_slice() {
        [only] => Ok((*only).to_string()),
        [] => Ok(String::new()),
        many => Err(format!(
            "The model returned {} lines but a single command is required. Try rephrasing.",
            many.len()
        )),
    }
}

/// Why a raw model reply cannot be used as a command. `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyError {
    /// Nothing usable was left after cleaning.
    Empty,
    /// The reply held this many (non-blank) lines; a single command is required.
    MultiLine(usize),
}

impl std::fmt::Display for ReplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "The model returned an empty command. Try rephrasing."),
            Self::MultiLine(n) => write!(
                f,
                "The model returned {n} lines but a single command is required. Try rephrasing."
            ),
        }
    }
}

impl std::error::Error for ReplyError {}

/// Turn a raw model reply into the one command to preview, or say why it can't be used.
pub fn finalize_reply(raw: &str) -> Result<String, ReplyError> {
    let cleaned = clean_llm_reply(raw);
    let lines: Vec<&str> = cleaned.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    match lines.as_slice() {
        [only] => Ok((*only).to_string()),
        [] => Err(ReplyError::Empty),
        many => Err(ReplyError::MultiLine(many.len())),
    }
}

/// String-error form of [`finalize_reply`].
pub fn finalize_llm_reply(raw: &str) -> Result<String, String> {
    finalize_reply(raw).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_accepts_fenced_and_backticked_replies() {
        assert_eq!(finalize_llm_reply("```bash\nls -la\n```").unwrap(), "ls -la");
        assert_eq!(finalize_llm_reply("`du -sh .`").unwrap(), "du -sh .");
    }

    #[test]
    fn finalize_rejects_empty_replies() {
        for raw in ["", "   \n", "``", "```\n```"] {
            let err = finalize_llm_reply(raw).unwrap_err();
            assert!(err.contains("empty"), "{raw:?} -> {err}");
        }
    }

    #[test]
    fn finalize_rejects_multi_line_replies() {
        let err = finalize_llm_reply("cd /tmp\nls").unwrap_err();
        assert!(err.contains("2 lines"), "{err}");
    }

    #[test]
    fn require_single_line_accepts_one_line() {
        assert_eq!(require_single_line("ls -la").unwrap(), "ls -la");
    }

    #[test]
    fn require_single_line_rejects_several_lines() {
        let err = require_single_line("cd /tmp\nrm -rf *").unwrap_err();
        assert!(err.contains("2 lines"), "unexpected message: {err}");
    }

    #[test]
    fn require_single_line_ignores_blank_lines_around_one_command() {
        assert_eq!(require_single_line("\n\nls\n\n").unwrap(), "ls");
    }

    #[test]
    fn require_single_line_rejects_backslash_continuation() {
        assert!(require_single_line("ls \\\n  -la").is_err());
    }

    #[test]
    fn clean_llm_reply_table() {
        let cases = [
            ("`du -sh .`", "du -sh ."),
            ("```bash\nls -la\n```", "ls -la"),
            ("```\nls\n```", "ls"),
            ("```ls```", "ls"),
            ("  ls -la \n", "ls -la"),
            ("echo `date`", "echo `date`"),
            ("`a` && `b`", "`a` && `b`"),
            ("``", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(clean_llm_reply(input), expected, "input: {input:?}");
        }
    }
}

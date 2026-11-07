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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_error_display_is_the_user_message() {
        assert_eq!(
            ReplyError::Empty.to_string(),
            "The model returned an empty command. Try rephrasing."
        );
        assert_eq!(
            ReplyError::MultiLine(3).to_string(),
            "The model returned 3 lines but a single command is required. Try rephrasing."
        );
    }

    #[test]
    fn finalize_accepts_one_line_and_blank_padding() {
        assert_eq!(finalize_reply("ls -la").unwrap(), "ls -la");
        assert_eq!(finalize_reply("\n\nls\n\n").unwrap(), "ls");
    }

    #[test]
    fn finalize_accepts_fenced_and_backticked_replies() {
        assert_eq!(finalize_reply("```bash\nls -la\n```").unwrap(), "ls -la");
        assert_eq!(finalize_reply("`du -sh .`").unwrap(), "du -sh .");
    }

    #[test]
    fn finalize_rejects_empty_replies() {
        for raw in ["", "   \n", "``", "```\n```"] {
            assert_eq!(finalize_reply(raw), Err(ReplyError::Empty), "{raw:?}");
        }
    }

    #[test]
    fn finalize_rejects_multi_line_replies() {
        assert_eq!(finalize_reply("cd /tmp\nls"), Err(ReplyError::MultiLine(2)));
        assert_eq!(finalize_reply("cd /tmp\nrm -rf *"), Err(ReplyError::MultiLine(2)));
    }

    #[test]
    fn finalize_rejects_backslash_continuation() {
        assert_eq!(finalize_reply("ls \\\n  -la"), Err(ReplyError::MultiLine(2)));
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

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

#[cfg(test)]
mod tests {
    use super::*;

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

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

#[cfg(test)]
mod tests {
    use super::*;

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

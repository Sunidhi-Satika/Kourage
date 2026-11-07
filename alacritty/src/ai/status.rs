//! What the AI is doing right now, in words the user can always see.

use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiPhase {
    /// The microphone is open.
    Listening,
    /// Speech is being turned into text.
    Transcribing,
    /// The request was sent to the model and we're waiting for its reply.
    Thinking { model: String },
    /// Something failed; stays until the user presses a key.
    Error(String),
    /// Information the user should see (e.g. no speech heard); stays until a key press.
    Notice(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiStatus {
    pub phase: AiPhase,
    pub since: Instant,
}

impl AiStatus {
    pub fn new(phase: AiPhase) -> Self {
        Self { phase, since: Instant::now() }
    }

    /// Errors and notices stay on screen until the user presses a key.
    pub fn is_dismissible(&self) -> bool {
        matches!(self.phase, AiPhase::Error(_) | AiPhase::Notice(_))
    }
}

/// A `width`-cell bar for an input level in `[0, 1]` (speech is typically 0.02 to 0.3).
pub fn level_meter(level: f32, width: usize) -> String {
    // Perceived loudness is closer to a square root than to the raw RMS.
    let scaled = (level.max(0.0) / 0.1).sqrt().min(1.0);
    let filled = ((scaled * width as f32).round() as usize).min(width);
    format!("{}{}", "#".repeat(filled), "-".repeat(width - filled))
}

/// The single line shown for `status`, at most `columns` cells wide. ASCII only, so it
/// renders even where the font has no emoji.
pub fn format_status(status: &AiStatus, now: Instant, level: f32, columns: usize) -> String {
    let secs = now.saturating_duration_since(status.since).as_secs();
    let text = match &status.phase {
        AiPhase::Listening => {
            format!("LISTENING [{}] {secs}s  speak now - Esc: cancel", level_meter(level, 10))
        },
        AiPhase::Transcribing => format!("TRANSCRIBING speech... {secs}s"),
        AiPhase::Thinking { model } => {
            let hint = if secs >= 10 { " (the first request loads the model)" } else { "" };
            format!("SENT to {model} - waiting for reply... {secs}s{hint} (Esc to cancel)")
        },
        AiPhase::Error(message) => format!("ERROR: {message}  (press any key)"),
        AiPhase::Notice(message) => format!("{message}  (press any key)"),
    };
    truncate_with_ellipsis(&text, columns)
}

fn truncate_with_ellipsis(text: &str, columns: usize) -> String {
    if text.chars().count() <= columns {
        return text.to_string();
    }
    if columns <= 3 {
        return text.chars().take(columns).collect();
    }
    let mut short: String = text.chars().take(columns - 3).collect();
    short.push_str("...");
    short
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(phase: AiPhase, elapsed_secs: u64) -> (AiStatus, Instant) {
        let since = Instant::now();
        (AiStatus { phase, since }, since + Duration::from_secs(elapsed_secs))
    }

    #[test]
    fn listening_shows_state_meter_time_and_how_to_cancel() {
        let (status, now) = at(AiPhase::Listening, 3);
        let text = format_status(&status, now, 0.05, 80);
        assert!(text.contains("LISTENING"), "{text}");
        assert!(text.contains("3s"), "{text}");
        assert!(text.contains("Esc"), "{text}");
        assert!(text.contains('#'), "meter should show some level: {text}");
    }

    #[test]
    fn meter_is_empty_for_silence_and_full_for_loud_speech() {
        assert_eq!(level_meter(0.0, 10), "----------");
        assert_eq!(level_meter(0.5, 10), "##########");
        let mid = level_meter(0.01, 10);
        assert_eq!(mid.chars().count(), 10);
        assert!(mid.contains('#') && mid.contains('-'), "{mid}");
    }

    #[test]
    fn thinking_names_the_model_and_says_it_is_waiting() {
        let (status, now) = at(AiPhase::Thinking { model: "qwen2.5-coder:3b".into() }, 5);
        let text = format_status(&status, now, 0.0, 100);
        assert!(text.contains("SENT") && text.contains("qwen2.5-coder:3b"), "{text}");
        assert!(text.contains("waiting") && text.contains("5s"), "{text}");
        assert!(!text.contains("loads the model"), "too early for the hint: {text}");
        assert_eq!(text, "SENT to qwen2.5-coder:3b - waiting for reply... 5s (Esc to cancel)");
    }

    #[test]
    fn a_long_wait_explains_the_first_request_is_slow() {
        let (status, now) = at(AiPhase::Thinking { model: "m".into() }, 12);
        let text = format_status(&status, now, 0.0, 120);
        assert!(text.contains("loads the model"), "{text}");
        assert_eq!(
            text,
            "SENT to m - waiting for reply... 12s (the first request loads the model) (Esc to \
             cancel)"
        );
    }

    #[test]
    fn transcribing_shows_time() {
        let (status, now) = at(AiPhase::Transcribing, 2);
        let text = format_status(&status, now, 0.0, 80);
        assert!(text.contains("TRANSCRIBING") && text.contains("2s"), "{text}");
    }

    #[test]
    fn errors_and_notices_say_how_to_dismiss() {
        let (err, now) = at(AiPhase::Error("Can't reach Ollama".into()), 0);
        let text = format_status(&err, now, 0.0, 100);
        assert!(text.contains("ERROR") && text.contains("Can't reach Ollama"), "{text}");
        assert!(text.contains("any key"), "{text}");

        let (notice, now) = at(AiPhase::Notice("No speech heard".into()), 0);
        let text = format_status(&notice, now, 0.0, 100);
        assert!(text.contains("No speech heard") && text.contains("any key"), "{text}");
    }

    #[test]
    fn long_text_is_truncated_to_the_width_with_an_ellipsis() {
        let (status, now) = at(AiPhase::Error("x".repeat(200)), 0);
        let text = format_status(&status, now, 0.0, 30);
        assert_eq!(text.chars().count(), 30);
        assert!(text.ends_with("..."), "{text}");
        assert_eq!(format_status(&status, now, 0.0, 0), "");
    }

    #[test]
    fn only_errors_and_notices_are_dismissible() {
        assert!(AiStatus::new(AiPhase::Error("e".into())).is_dismissible());
        assert!(AiStatus::new(AiPhase::Notice("n".into())).is_dismissible());
        assert!(!AiStatus::new(AiPhase::Listening).is_dismissible());
        assert!(!AiStatus::new(AiPhase::Transcribing).is_dismissible());
        assert!(!AiStatus::new(AiPhase::Thinking { model: "m".into() }).is_dismissible());
    }

    #[test]
    fn status_lines_are_plain_ascii() {
        for phase in [
            AiPhase::Listening,
            AiPhase::Transcribing,
            AiPhase::Thinking { model: "m".into() },
            AiPhase::Error("e".into()),
            AiPhase::Notice("n".into()),
        ] {
            let (status, now) = at(phase, 1);
            let text = format_status(&status, now, 0.05, 100);
            assert!(text.is_ascii(), "non-ASCII in {text:?}");
        }
    }
}

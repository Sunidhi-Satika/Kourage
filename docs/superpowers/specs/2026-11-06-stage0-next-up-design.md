# Stage 0 "Next up" fixes: design (short)

Approved by the owner on 2026-11-06. Source: the three **Next up** items in `ROADMAP.md`, found in a live test.

## Intent
Make AI-proposed commands safe and reliable to run: stop a backtick-wrapped reply from executing as command substitution, stop `exit` from being treated as safe, and gather evidence for the voice "Bye." hallucination.

## Goals
- Single-backtick (and fenced) LLM replies are cleaned to a bare command.
- Session-ending commands are flagged by the preview guard.
- Voice audio can be dumped for listening before any threshold tuning.

## Non-goals
- Fixing the other safety-guard holes (`sudo`, `curl | sh`, `rm` substring false positives).
- Changing `SPEECH_RMS_THRESHOLD` or the timings before the audio has been heard.
- Multi-line enforcement, the `[ai]` config rename, or the other roadmap items.

## Anti-goals
- Stripping legitimate inner backticks (`echo \`date\``).
- Flagging harmless text that merely contains "exit" (`echo exit`, `exit_code.sh`).

## Approaches
Cleanup could stay inline in `call_llm` or move into a pure function. The pure function was chosen because it is testable without an HTTP server. A mock-LLM backend would be heavier and is Stage 1 work.

## Design
1. `clean_llm_reply(&str) -> String` in `alacritty/src/ai/`. It handles code fences (with a language tag) and one backtick pair wrapping the whole reply. `call_llm` uses it. Unit tests are written first.
2. `is_destructive_command` flags `exit`, `logout`, `kill $$`, `kill -9 $$` and `exec …`, matched on word boundaries, including after `;`, `&&` and `||`. Table-driven tests are written first.
3. With debug logging on, `voice.rs` writes the 16 kHz audio to `/tmp/kourage-voice-<pid>.wav`. The owner listens to it, then thresholds are tuned in a follow-up.

## Left to the builder
Exact module placement of `clean_llm_reply`, and the tokenisation approach for item 2. Keep both small and dependency-free.

# Stage 0 "Next up" Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Strip backtick-wrapped LLM replies, flag session-ending commands in the preview guard, and add a debug audio dump for the voice "Bye." hallucination.

**Architecture:** Two small pure functions (`clean_llm_reply`, extended `is_destructive_command`) with table-driven tests, plus a dependency-free WAV encoder in `voice.rs` used only when debug logging is on.

**Tech Stack:** Rust, `cargo test -p alacritty`, `log` crate. No new dependencies (`hound` was removed in the last commit; do not re-add it).

**Spec:** `docs/superpowers/specs/2026-11-06-stage0-next-up-design.md`

## Global Constraints

- No new dependencies.
- Do not change `SPEECH_RMS_THRESHOLD`, `TRAILING_SILENCE` or `NO_SPEECH_TIMEOUT` (spec non-goal).
- Do not fix other safety-guard holes (`sudo`, `curl | sh`, `rm` substring false positives).
- Inner backticks in a command must survive (`` echo `date` ``).
- Harmless text containing "exit" must stay safe (`echo exit`, `./exit_code.sh`).
- Run `cargo build` before any manual testing; `cargo test` does not rebuild the app binary.

## Review Focus

- `` `a` && `b` `` starts and ends with a backtick but is two substitutions: must NOT be unwrapped.
- A fenced reply on one line (```` ```ls``` ````) should clean to `ls`.
- A reply that is only backticks (`` `` ``) cleans to empty, and `call_llm` must still return the "empty command" error.
- `exit` after `;`, `&&`, `||`, `|` or a newline is flagged; `EXIT` in uppercase is too.
- `ls executable` and `grep exit log.txt` stay safe.

---

### Task 1: Clean LLM replies

**Files:**
- Create: `alacritty/src/ai/reply.rs`
- Modify: `alacritty/src/ai/mod.rs` (add `pub mod reply;` and `pub use reply::clean_llm_reply;`)
- Modify: `alacritty/src/voice.rs:267-309` (`call_llm` calls the new function in place of the inline fence-stripping block)

**Interfaces:**
- Produces: `pub fn clean_llm_reply(raw: &str) -> String` in `ai::reply`, re-exported from `ai`.

- [ ] **Step 1: Write the failing tests** in `ai/reply.rs` (`#[cfg(test)] mod tests`), each asserting `clean_llm_reply(input) == expected`:
  - `` "`du -sh .`" `` → `"du -sh ."`
  - ```` "```bash\nls -la\n```" ```` → `"ls -la"`
  - ```` "```\nls\n```" ```` → `"ls"`
  - ```` "```ls```" ```` → `"ls"`
  - `"  ls -la \n"` → `"ls -la"`
  - `` "echo `date`" `` → unchanged
  - `` "`a` && `b`" `` → unchanged
  - `` "``" `` → `""`
- [ ] **Step 2: Run** `cargo test -p alacritty clean_llm_reply` (name tests with that prefix). Expected: FAIL (function missing).
- [ ] **Step 3: Implement `clean_llm_reply`.** Trim; if it starts with ```` ``` ````, drop that opening fence (and a language tag up to the first newline, when one exists) and the trailing fence; else, if it starts and ends with a single backtick and the text between contains no backtick, unwrap it. Trim the result. Then replace the inline block in `call_llm` with a call to it, keeping the existing empty-reply error.
- [ ] **Step 4: Run** `cargo test -p alacritty clean_llm_reply`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Strip single-backtick and one-line fenced LLM replies"`

### Task 2: Flag session-ending commands

**Files:**
- Modify: `alacritty/src/ai/safety.rs` (new check in `is_destructive_command`, new tests in the existing `tests` module)

**Interfaces:**
- Consumes: `is_destructive_command(raw_cmd: &str) -> bool` (signature unchanged).

- [ ] **Step 1: Write the failing test** `test_session_ending_commands`:
  - flagged: `exit`, `exit 0`, `logout`, `kill $$`, `kill -9 $$`, `exec bash`, `ls; exit`, `true && exit`, `false || exit 1`, `echo hi\nexit`, `EXIT`
  - safe: `echo exit`, `./exit_code.sh`, `cat exit.txt`, `grep exit log.txt`, `ls executable`, `kill 1234`
- [ ] **Step 2: Run** `cargo test -p alacritty test_session_ending`. Expected: FAIL.
- [ ] **Step 3: Implement** a private `fn ends_session(cmd: &str) -> bool` and call it from `is_destructive_command`. Split the lowercased command on `;`, `&&`, `||`, `|`, `&` and newlines. A segment counts when its first whitespace-separated word is `exit`, `logout` or `exec`, or when its first word is `kill` and some word is exactly `$$`.
- [ ] **Step 4: Run** `cargo test -p alacritty` (whole crate, to confirm the existing safety tests still pass). Expected: PASS.
- [ ] **Step 5: Commit** `git commit -m "Flag session-ending commands in the safety guard"`

### Task 3: Debug dump of recorded voice audio

**Files:**
- Modify: `alacritty/src/voice.rs` (new `encode_wav_16k_mono`, dump call in `transcribe_audio`, tests in the existing `tests` module)
- Modify: `ROADMAP.md` (tick the three "Next up" items; for the voice item, say the dump exists and the threshold tuning is waiting on listening to it)

**Interfaces:**
- Produces: `fn encode_wav_16k_mono(samples: &[f32]) -> Vec<u8>` (RIFF/WAVE, PCM 16-bit, 1 channel, 16000 Hz, 44-byte header).

- [ ] **Step 1: Write the failing tests:**
  - `wav_header_is_valid`: for 3 samples the output length is 44 + 6; bytes 0..4 are `RIFF`, 8..12 `WAVE`; the sample rate field (bytes 24..28, little-endian) is 16000; the data-size field (bytes 40..44) is 6.
  - `wav_clamps_and_scales`: samples `[1.0, -1.0, 2.0]` encode to `i16` `32767, -32767, 32767` (clamp to ±1.0 first).
- [ ] **Step 2: Run** `cargo test -p alacritty wav_`. Expected: FAIL.
- [ ] **Step 3: Implement `encode_wav_16k_mono`** by hand (no dependency). In `transcribe_audio`, after `to_whisper_input`, write the encoded bytes to `/tmp/kourage-voice-<pid>.wav` only when `log::max_level() >= log::LevelFilter::Debug`. Log the path with `info!` and any write error with `warn!` (never fail transcription over it). Debug level means `-vv` (or `[debug] log_level = "Debug"`).
- [ ] **Step 4: Run** `cargo test -p alacritty` and `cargo build`. Expected: all PASS, build succeeds.
- [ ] **Step 5: Manual check (owner):** `./target/debug/alacritty -vv`, press Ctrl+Shift+S, say a command, then play `/tmp/kourage-voice-<pid>.wav` (`aplay`). Tune the thresholds only after listening (follow-up, not this plan).
- [ ] **Step 6: Commit** `git commit -m "Dump 16 kHz voice audio to /tmp at debug level; update roadmap"`

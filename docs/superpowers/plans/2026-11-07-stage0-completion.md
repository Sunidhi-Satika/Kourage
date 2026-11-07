# Stage 0 Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close Stage 0: a first-run setup check, cancelling an in-flight AI request with Esc, a config flag for the startup warm-up, a README audit, and an honest roadmap close-out.

**Architecture:** Pure logic (what is missing from the setup) lives in `kourage-core` behind the `Backend` trait and is tested with the mock; the terminal wires it to the status line. Request cancellation lives in `alacritty/src/voice.rs` (the caller stops waiting; no trait change).

**Tech Stack:** Rust 2024 workspace (`kourage-core`, `alacritty`), reqwest 0.11 blocking, existing `AiPhase`/`StatusReporter` status line.

**Spec:** the `ROADMAP.md` "Stage 0 gaps" and "Exit criteria for Stage 0" sections (the binding source), plus `docs/superpowers/specs/2026-11-07-kourage-core-design.md` for the crate boundary.

## Global Constraints

- Branch policy: work on a short-lived topic branch off `master`, merged and deleted at the end; `master` is the only long-lived branch. Do not push.
- `kourage-core` must not depend on `winit`, `alacritty_terminal`, `cpal`, `whisper-rs`, or the `alacritty` crate; no new dependencies in `kourage-core`.
- User-visible texts that exist today do not change (error messages, preview texts), except where a task says so.
- `cargo test --workspace` and `cargo clippy --all-targets` clean; no test lost (baseline 395 passed); `cargo build` shows no new warnings.
- Windows and macOS must keep compiling (CI runs `cargo test` and clippy on both); code that cannot compile there stays behind `cfg`.
- Commit messages end with: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` exactly.
- Never claim to have run or seen what needs an interactive window or a microphone; list such checks for the owner instead.

## Review Focus

1. Ollama answers `/api/tags` with HTTP 200 but non-JSON, or with an empty model list → reported as a clear setup problem, never a panic (Task 1).
2. A model name that differs only by tag, e.g. configured `qwen2.5-coder:3b` vs installed `qwen2.5-coder:3b-instruct-q4`, or configured `llama3` vs installed `llama3:latest` → `llama3` must match `llama3:latest`; a different tag must not (Task 1).
3. Esc pressed when nothing is in flight, twice quickly, or after the answer already arrived → no panic, no stale preview appears after "cancelled" (Task 3).
4. A cancelled request whose late answer arrives afterwards → the answer is dropped and never opens a preview (Task 3).
5. Setup check when the config still has the default placeholder Whisper path (`/path/to/your/whisper/model.bin`) → reported as "not configured", and never as an error for text-only use (Task 2).

---

### Task 1: Setup check in `kourage-core`

**Files:**
- Create: `kourage-core/src/setup.rs`
- Modify: `kourage-core/src/backend/mod.rs`, `kourage-core/src/backend/ollama.rs`, `kourage-core/src/backend/mock.rs`, `kourage-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `Backend::installed_models(&self) -> Result<Option<Vec<String>>, BackendError>` with default `Ok(None)` (None = this backend cannot list models).
  - `OllamaBackend::installed_models` calls `GET <base>/api/tags` where `<base>` is the scheme, host and port of `LlmSettings.api_url`; parses `{"models":[{"name": "..."}]}`; same timeout/error mapping as `generate`.
  - `MockBackend::with_models(&[&str])` and `MockBackend::unreachable()` (installed_models returns the scripted list / a `BackendError::Connect`).
  - `pub enum SetupIssue { BackendUnreachable { message: String }, ModelMissing { model: String, message: String }, WhisperNotConfigured { message: String }, WhisperFileMissing { path: String, message: String } }` with `fn message(&self) -> &str`.
  - `pub fn check_setup(backend: &dyn Backend, whisper_model_path: &str) -> Vec<SetupIssue>`; `pub const WHISPER_PLACEHOLDER_PATH: &str = "/path/to/your/whisper/model.bin"`.
  - `pub fn model_is_installed(wanted: &str, installed: &[String]) -> bool` (a name without a tag matches `name:latest`; an explicit tag must match exactly).
- Consumes: `diagnostics::llm_failure_message`, `diagnostics::whisper_missing_message`, `BackendError`.

- [ ] **Step 1:** Write failing tests in `setup.rs`: `model_is_installed_matches_latest_tag` (`llama3` vs `["llama3:latest"]` true; `qwen2.5-coder:3b` vs `["qwen2.5-coder:3b"]` true; `qwen2.5-coder:3b` vs `["qwen2.5-coder:7b"]` false; `llama3:8b` vs `["llama3:latest"]` false), `check_setup_reports_unreachable_backend` (mock unreachable → one `BackendUnreachable`, no `ModelMissing`), `check_setup_reports_missing_model` (mock with other models → `ModelMissing` whose message contains `ollama pull <model>`), `check_setup_ok_when_model_present_and_whisper_exists` (temp file as whisper path → empty vec), `placeholder_whisper_path_is_not_configured`, `missing_whisper_file_reports_download_instructions` (message equals `whisper_missing_message(path)`), `backend_that_cannot_list_models_is_not_an_issue` (`installed_models` returns `Ok(None)` → no model issue).
- [ ] **Step 2:** Add stub-server tests in `ollama.rs` (reuse the existing stub helper): `installed_models_parses_tags` (`{"models":[{"name":"a:1"},{"name":"b:latest"}]}` → `Some(vec!["a:1","b:latest"])`), `installed_models_garbage_body_is_malformed_error`, `installed_models_uses_tags_path_on_the_configured_host` (stub records `GET /api/tags`). Run all new tests; expected: FAIL.
- [ ] **Step 3:** Implement the trait method, the Ollama and mock implementations, `setup.rs` and the `lib.rs` exports. `check_setup` order: backend/model issues first, then whisper issues. Messages for backend and model issues reuse `llm_failure_message` / the existing "ollama pull" wording from `diagnostics.rs`; do not invent a new wording where one exists.
- [ ] **Step 4:** `cargo test -p kourage-core && cargo clippy --all-targets`. Expected: all pass, no new warnings.
- [ ] **Step 5:** Commit: `git add -A kourage-core && git commit -m "Add setup check to kourage-core (installed models, Whisper path)"`.

### Task 2: First-run notice in the terminal

**Files:**
- Modify: `alacritty/src/voice.rs`, `alacritty/src/event.rs` (or wherever the first window is created; find it), `alacritty/src/main.rs`

**Interfaces:**
- Consumes: `kourage_core::{check_setup, SetupIssue, OllamaBackend}`, `llm_settings(&Voice)`, `StatusReporter`, `AiPhase::Notice`.
- Produces: `pub fn announce_setup_issues(config: &UiConfig, proxy: &EventLoopProxy<Event>, window_id: WindowId)` in `voice.rs`; `fn setup_summary(issues: &[SetupIssue]) -> Option<String>` (pure, testable).

- [ ] **Step 1:** Write failing tests for `setup_summary`: no issues → `None`; one issue → its message; two issues → first message followed by ` (+1 more, see the log)`; `WhisperNotConfigured` (the placeholder path) is skipped by `setup_summary` and only logged, so text-only users are never nagged; `WhisperFileMissing` (a configured path that does not exist) is included. Signature is exactly `fn setup_summary(issues: &[SetupIssue]) -> Option<String>`.
- [ ] **Step 2:** Run them; expected FAIL.
- [ ] **Step 3:** Implement `setup_summary` and `announce_setup_issues`: spawn a thread, build an `OllamaBackend`, call `check_setup`, log every issue with `warn!`, and if `setup_summary` is `Some`, show it with `StatusReporter` as `AiPhase::Notice` for that window. Call it once, for the first window only (a `static AtomicBool` guard), from where the first window is created. It must not delay window creation or the warm-up, and must never panic.
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --all-targets`. Expected: pass.
- [ ] **Step 5:** Commit: `git commit -am "Tell the user once at startup what is missing (Ollama, model, Whisper)"`.

### Task 3: Esc cancels an in-flight request

**Files:**
- Modify: `alacritty/src/voice.rs`, `alacritty/src/input/keyboard.rs`

**Interfaces:**
- Produces:
  - `struct AiRequestSession` (like `VoiceSession`): `begin()`, `end()`, `request_cancel() -> bool` (true only while a request is in flight), `is_cancelled()`; static `AI_REQUEST_SESSION`.
  - `pub fn cancel_ai() -> bool`: cancels the recording if one is in progress, otherwise the in-flight request; returns whether anything was cancelled. `input/keyboard.rs:62` calls `cancel_ai()` instead of `cancel_recording()`.
  - `fn wait_or_cancel<T>(rx: &std::sync::mpsc::Receiver<T>, cancelled: &dyn Fn() -> bool) -> Option<T>`: polls `recv_timeout(50ms)`, returns `None` as soon as `cancelled()` is true.
- Consumes: `run_prompt` / `propose_command` from the kourage-core extraction.

- [ ] **Step 1:** Write failing tests: `wait_or_cancel_returns_value_when_it_arrives`; `wait_or_cancel_returns_none_promptly_after_cancel` (producer sleeps 2 s, cancel set after 100 ms, assert returns in under 500 ms); `late_result_after_cancel_is_dropped` (producer sends after cancel; the caller already returned `None` and nothing panics); `cancel_without_request_returns_false`; `cancel_after_end_returns_false`; `begin_clears_previous_cancel`.
- [ ] **Step 2:** Run them; expected FAIL.
- [ ] **Step 3:** In `propose_command`, run the backend call on a helper thread and wait with `wait_or_cancel`. On cancel: `reporter.show(AiPhase::Notice("Request cancelled.".into()))`, no preview, `AI_REQUEST_SESSION.end()`. The abandoned thread ends by itself within the configured timeout. Keep `Thinking` shown before the worker thread starts, and keep all existing texts.
- [ ] **Step 4:** Update the Esc handling and the status line hint for Thinking: `AiPhase::Thinking` text should end with `(Esc to cancel)`; update the status tests in `ai/status.rs` for the new exact text.
- [ ] **Step 5:** `cargo test --workspace && cargo clippy --all-targets`. Expected: pass.
- [ ] **Step 6:** Commit: `git commit -am "Esc cancels an in-flight AI request"`.

### Task 4: Config flag for the startup warm-up

**Files:**
- Modify: `alacritty/src/config/voice.rs`, `alacritty/src/main.rs`, `README.md` (config example)

**Interfaces:**
- Produces: `Voice.warm_up: bool`, default `true`, read from `[ai] warm_up`.

- [ ] **Step 1:** Write failing tests in `config/voice.rs`: `warm_up_defaults_to_true`; `warm_up_can_be_disabled` (parse `[ai]\nwarm_up = false`); `legacy_voice_section_accepts_warm_up`.
- [ ] **Step 2:** Run them; expected FAIL.
- [ ] **Step 3:** Add the field and default; in `main.rs` call `voice::warm_up_llm(&config.ai)` only when `config.ai.warm_up`; add one row to the README config example.
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --all-targets`. Expected: pass.
- [ ] **Step 5:** Commit: `git commit -am "Add [ai] warm_up flag"`.

### Task 5: README audit and roadmap close-out

**Files:**
- Modify: `README.md`, `ROADMAP.md`

**Interfaces:**
- Consumes: the behaviour from Tasks 1 to 4.

- [ ] **Step 1:** Audit `README.md` as a fresh user would: follow Quick Start literally. Check each claim against the code: install commands and build dependencies against `INSTALL.md`, config keys and defaults against `alacritty/src/config/voice.rs`, the keybinding table against the default bindings in `alacritty/src/config/bindings.rs`, the status-line section against `alacritty/src/ai/status.rs`, and each troubleshooting row against the message texts in `kourage-core/src/diagnostics.rs` and `reply.rs`. Record each mismatch in the report.
- [ ] **Step 2:** Fix every mismatch in the README; add: Esc cancels a request in flight, the first-run notice, and the `warm_up` flag. Keep the README's existing voice and structure.
- [ ] **Step 3:** Update `ROADMAP.md`: mark the Stage 0 gaps done by this plan (first-run check, request cancel and thinking hint, warm-up flag); correct stale text (the `[voice]` alias warning is already logged at startup; the multi-line reply rule: replies are rejected with a clear message, and showing them in the preview is a deliberate non-goal for now because rejecting is safer); tick each exit criterion that is met with the evidence (test name or log), and list for the owner as OPEN: voice on a stock laptop mic, listening to the recording dump to tune `SPEECH_RMS_THRESHOLD`, the Ollama-stopped and Esc smoke test, the status-line redraw check, and a clone-to-first-command run on a clean machine. Do not tick anything that needs a human at a keyboard.
- [ ] **Step 4:** `cargo test --workspace` (docs only; confirm nothing changed).
- [ ] **Step 5:** Commit: `git commit -am "README audit and Stage 0 roadmap close-out"`.

---

## Self-review (done)

- **Coverage:** every item the owner and I agreed on has a task (setup check 1-2, cancel 3, warm-up flag 4, README audit and roadmap 5). Deliberately excluded and recorded in Task 5: showing multi-line replies in the preview. Items needing the owner are listed in Task 5.
- **Types:** `installed_models`, `SetupIssue`, `check_setup`, `setup_summary`, `cancel_ai`, `wait_or_cancel` are each defined once and used by name later.
- **Review Focus:** each of the five items is pinned by a named test in Tasks 1 to 3 (item 5 by `placeholder_whisper_path_is_not_configured` and the `setup_summary` placeholder rule).

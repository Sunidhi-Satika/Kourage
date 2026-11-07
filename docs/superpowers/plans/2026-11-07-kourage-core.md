# `kourage-core` Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move Kourage's AI logic out of the `alacritty` crate into a UI-free `kourage-core` crate with a swappable, mockable LLM `Backend`, with no change in behaviour.

**Architecture:** New workspace member `kourage-core/` holds prompt building, safety, reply cleaning, error messages, plain-data `AiContext`, a `Platform` trait, a blocking `Backend` trait (`OllamaBackend`, `MockBackend`) and one entry point `ask()`. `alacritty` keeps terminal-side context gathering, status UI, voice/Whisper, and the `Voice` config, and calls `kourage_core::ask`.

**Tech Stack:** Rust 2024 (workspace), `reqwest` 0.11 blocking, `serde`/`serde_json`, `log`. No async runtime.

**Spec:** `docs/superpowers/specs/2026-11-07-kourage-core-design.md`

## Global Constraints

- Behaviour unchanged: prompt text and Ollama request body byte-identical (golden tests); same replies, same safety verdicts, same user-facing messages; config keys (`[ai]`, legacy `[voice]`) unchanged.
- `kourage-core` must not depend on `winit`, `alacritty_terminal`, `cpal`, `whisper-rs`, or the `alacritty` crate (`cargo tree -p kourage-core` shows none).
- Rust 2024 edition, workspace resolver 2, Apache-2.0; `reqwest` 0.11 blocking stays.
- `cargo test` (workspace) and `cargo clippy --all-targets` are clean; Windows and macOS keep compiling (CI runs both).
- No existing test is lost: all 170 current tests remain or move with their code.
- No daemon, no new features, no prompt or safety-rule changes, no cancellation.
- Commit messages end with: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

## Review Focus

1. Ollama answers HTTP 200 with a body that is not the expected JSON → a "Malformed response" error naming the URL, never a panic (Task 5).
2. Model reply that is empty, only whitespace, or only an empty code fence → `AskError::EmptyReply` with today's message (Task 6).
3. Instruction containing quotes, newlines, or non-ASCII text, and the empty instruction → the prompt stays well-formed and `ask` does not panic (Task 4).
4. Directory listing with non-UTF-8 names, more than 15 entries, or an unreadable directory → `gather` never panics and caps the list (Task 4).
5. Warm-up when Ollama is down or the model is missing → returns an error that is logged, never blocks the UI or panics (Task 5).

---

### Task 1: Finish and verify the latency work (step 1)

**Files:**
- Modify: `alacritty/src/voice.rs` (log request duration in `call_llm`), `ROADMAP.md`
- Existing uncommitted changes: `alacritty/src/ai/context.rs`, `alacritty/src/main.rs`, `alacritty/src/voice.rs`

**Interfaces:**
- Produces: a clean, committed baseline that Task 2's golden tests pin.

- [ ] **Step 1:** In `call_llm`, add `info!("LLM request took {:.1}s", elapsed)` using `Instant` around the HTTP call.
- [ ] **Step 2:** Run `cargo test -p alacritty --bins && cargo clippy --all-targets`. Expected: 170+ tests pass, no clippy output.
- [ ] **Step 3:** `cargo build`, then run `./target/debug/alacritty -v` with Ollama running. Make three prompts with different wording (one file-related, two not) and read the "LLM request took" lines and the "LLM warm-up finished" line from the log. Record the numbers.
- [ ] **Step 4:** Check the redraw question: compare request time while the status line animates against the same request with `[debug]`-level redraw unchanged but the status thread idle (or temporarily lower the status tick); note in `ROADMAP.md` whether it matters. If it does, lower the tick while waiting (new test in `ai/status.rs` for the tick interval).
- [ ] **Step 5:** Update `ROADMAP.md` "Next up" step 1: mark done items, add the in-app measurements and the 3-request direct-API table (11.4 s cold → 2.2 s → 0.7 s).
- [ ] **Step 6:** Commit.

```bash
git add ROADMAP.md alacritty/src
git commit -m "Latency: reorder prompt, trim context, warm up model, keep_alive"
```

### Task 2: Golden tests pin current behaviour

**Files:**
- Create: `alacritty/src/ai/golden/prompt_rust_files.txt`, `prompt_empty_context.txt`, `request_command.json`, `request_warm_up.json`
- Modify: `alacritty/src/ai/context.rs` (tests), `alacritty/src/voice.rs` (tests)

**Interfaces:**
- Produces: fixtures that move into `kourage-core` in Tasks 4–5 and must pass unchanged.

- [ ] **Step 1:** Write test `golden_prompt_rust_project` building a fixed `AiContext` (cwd `/home/u/proj`, os `Ubuntu 24.04 (Linux)`, shell `bash`, stack `Rust (Cargo), Git Repository`, 20 file names, 5 terminal lines) with instruction `"show the biggest files here"` and asserting `format_prompt(..) == include_str!("golden/prompt_rust_files.txt")`. Add `golden_prompt_empty_context` (default context, instruction `"what is my ip address"`).
- [ ] **Step 2:** Write test `golden_request_body_command` asserting `serde_json::to_string(&LlmRequest{ model: "qwen2.5-coder:3b", prompt: "P", stream: false, keep_alive: "30m", options: for_command(0.0) })` equals the fixture; and `golden_request_body_warm_up` likewise with `for_warm_up()`.
- [ ] **Step 3:** Run the four tests; they fail because the fixture files are missing/empty. Generate each fixture from the current code's output, read it once to confirm it matches the intended layout (rules, examples, environment, output, instruction), save it.
- [ ] **Step 4:** Run `cargo test -p alacritty --bins golden`. Expected: 4 pass.
- [ ] **Step 5:** Commit: `git add alacritty/src/ai && git commit -m "Pin prompt text and Ollama request body with golden tests"`.

### Task 3: Create `kourage-core`; move safety, reply, diagnostics

**Files:**
- Create: `kourage-core/Cargo.toml`, `kourage-core/src/lib.rs`, `kourage-core/src/{safety,reply,diagnostics}.rs`
- Modify: `Cargo.toml` (add `"kourage-core"` to `members`), `alacritty/Cargo.toml` (depend on `kourage-core = { path = "../kourage-core" }`), `alacritty/src/ai/mod.rs`
- Delete: `alacritty/src/ai/{safety,reply,diagnostics}.rs`

**Interfaces:**
- Produces (public, re-exported from `kourage_core`):
  `is_destructive_command(raw_cmd: &str) -> bool`; `clean_llm_reply(raw: &str) -> String`; `require_single_line(cmd: &str) -> Result<String, String>`; `finalize_llm_reply(raw: &str) -> Result<String, String>`; `llm_failure_message(url, model, status: Option<u16>, body, is_connect: bool) -> String` (keep the existing signature); `whisper_missing_message(path: &str) -> String`.
- `alacritty/src/ai/mod.rs` re-exports these so existing call sites keep compiling.

- [ ] **Step 1:** Create the crate (name `kourage-core`, edition/rust-version from workspace, Apache-2.0, `log` dependency only) and add it to the workspace.
- [ ] **Step 2:** Move the three files with `git mv` into `kourage-core/src/`, tests included; expose them in `lib.rs`.
- [ ] **Step 3:** In `alacritty/src/ai/mod.rs`, replace the module declarations with `pub use kourage_core::{...}` for the names above (keep `context` and `status` local for now).
- [ ] **Step 4:** Run `cargo test --workspace && cargo clippy --all-targets`. Expected: same test count as before (tests moved, not lost); `cargo tree -p kourage-core` lists no UI crates.
- [ ] **Step 5:** Commit: `git add -A && git commit -m "Create kourage-core; move safety, reply and diagnostics into it"`.

### Task 4: Move context, prompt and platform into core

**Files:**
- Create: `kourage-core/src/{context,platform,prompt}.rs`, `kourage-core/src/golden/` (move fixtures from Task 2)
- Modify: `alacritty/src/ai/context.rs` (shrinks to terminal-side gathering), `alacritty/src/event.rs:1233`, `alacritty/src/input/mod.rs:156`

**Interfaces:**
- Produces in core:
  - `pub struct AiContext { pub cwd: Option<String>, pub directory_contents: Vec<String>, pub detected_stack: Option<String>, pub shell: Option<String>, pub os: Option<String>, pub recent_terminal_lines: Option<String> }` (derive `Debug, Default, Clone, PartialEq, Eq`).
  - `pub trait Platform { fn os_description(&self) -> String; fn shell(&self) -> String; }` plus `pub struct SystemPlatform;` implementing it per OS with `cfg`. Strings identical to today (`"{PRETTY_NAME} (Linux)"`/`"Linux"`, `"macOS (Unix)"`, `"Windows"`, `"Unix"`; shell from `$SHELL` basename, default `"bash"`).
  - `impl AiContext { pub fn gather(cwd: Option<&Path>, platform: &dyn Platform, recent_terminal_lines: Option<String>) -> Self; pub fn format_prompt(&self, instruction: &str) -> String; }` (`gather` lists the directory, capped at 35 entries as today, and detects the stack).
  - `pub const PROMPT_EXAMPLES: [(&str, &str); 10]`.
- Consumes: `is_destructive_command`, `require_single_line` (Task 3) in the existing example tests.
- Terminal side keeps `extract_recent_lines(term, max_lines)` and the cwd lookup; `extract_ai_context` calls `AiContext::gather(cwd.as_deref(), &SystemPlatform, lines)`.

- [ ] **Step 1:** Move the golden fixtures and golden/prompt tests into core; run them in core: they fail to compile (types not moved yet).
- [ ] **Step 2:** Move `AiContext`, `format_prompt`, `PROMPT_EXAMPLES`, `mentions_files`, limits, tech-stack detection and the existing prompt tests to core; implement `Platform` and `SystemPlatform`.
- [ ] **Step 3:** Add tests, each failing first: `gather_never_panics_on_unreadable_directory` (nonexistent path → empty contents), `gather_caps_listing_at_35`, `gather_handles_non_utf8_names` (unix only, `OsStr::from_bytes`), `prompt_survives_hostile_instruction` (quotes, newlines, emoji, empty string: output contains the instruction section and does not panic), `mock_platform_gives_deterministic_context`.
- [ ] **Step 4:** Slim `alacritty/src/ai/context.rs` to the terminal-side function; update the two call sites.
- [ ] **Step 5:** `cargo test --workspace && cargo clippy --all-targets`. Expected: goldens pass unchanged, no tests lost.
- [ ] **Step 6:** Commit: `git commit -am "Move AiContext, prompt building and Platform into kourage-core"`.

### Task 5: Backend trait, Ollama and mock backends

**Files:**
- Create: `kourage-core/src/backend/{mod,ollama,mock}.rs`
- Modify: `kourage-core/Cargo.toml` (`reqwest` 0.11 blocking+json, `serde`, `serde_json`), `alacritty/src/voice.rs`, `alacritty/Cargo.toml`

**Interfaces:**
- Produces:
  - `pub struct LlmSettings { pub api_url: String, pub model: String, pub temperature: f32, pub timeout_secs: u64 }`
  - `pub struct GenerateOptions { pub temperature: f32, pub num_predict: u32, pub repeat_penalty: f32, pub stop: Vec<String> }` with `for_command(temperature: f32)` (128, 1.1, `["\n\n"]`) and `for_warm_up()` (1 token).
  - `pub struct GenerateRequest { pub prompt: String, pub options: GenerateOptions }`
  - `pub enum BackendError { Timeout, Connect, Http { status: u16, body: String }, Malformed }` carrying url, model and detail so `Display` equals today's `llm_failure_message` / timeout text exactly.
  - `pub trait Backend: Send + Sync { fn generate(&self, request: &GenerateRequest) -> Result<String, BackendError>; fn warm_up(&self, prefix: &str) -> Result<(), BackendError> { Ok(()) } fn model(&self) -> &str; }`
  - `pub struct OllamaBackend` (`OllamaBackend::new(settings: LlmSettings)`), sends `keep_alive: "30m"`.
  - `pub struct MockBackend` with `MockBackend::replying(&[&str])`, `MockBackend::failing(BackendError)`, and `requests(&self) -> Vec<GenerateRequest>` to assert what was sent.
- Consumes: `diagnostics::llm_failure_message` (Task 3); golden request fixtures (Task 2, moved to core).

- [ ] **Step 1:** Move the request-body golden tests to core, written against `OllamaBackend`'s body builder (a private `fn request_body(&self, &GenerateRequest) -> String`); they fail until the code exists.
- [ ] **Step 2:** Implement the trait, types, `OllamaBackend` (code moved from `voice.rs` `call_llm`/`warm_up_llm`; timeout from settings, warm-up uses `timeout_secs.max(60)` as today), and `MockBackend`.
- [ ] **Step 3:** Add tests against a local `std::net::TcpListener` serving canned HTTP: `ollama_200_with_garbage_body_is_malformed_error` (message names the URL), `ollama_404_gives_pull_hint`, `ollama_connection_refused_says_start_ollama`, `ollama_timeout_message_names_seconds`, `warm_up_failure_returns_error_without_panic` (nothing listening).
- [ ] **Step 4:** In `voice.rs`, replace `LlmRequest`/`LlmOptions`/`call_llm` internals with an `OllamaBackend` built from `Voice` via `fn llm_settings(&Voice) -> LlmSettings`; `warm_up_llm` calls `backend.warm_up(&AiContext::default().format_prompt(""))` on a thread.
- [ ] **Step 5:** `cargo test --workspace && cargo clippy --all-targets`. Expected: goldens unchanged and green.
- [ ] **Step 6:** Commit: `git add -A && git commit -m "Add Backend trait with Ollama and mock implementations"`.

### Task 6: `ask`, `Proposal`, `AskError`, end-to-end tests

**Files:**
- Create: `kourage-core/src/ask.rs`
- Modify: `kourage-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct Proposal { pub command: String, pub is_destructive: bool }`
  - `pub enum AskError { EmptyReply, MultiLine, Backend(BackendError) }`; `Display` of `EmptyReply`/`MultiLine` is exactly the message `finalize_llm_reply` returns today; `Backend` forwards the backend's `Display`.
  - `pub fn ask(backend: &dyn Backend, context: &AiContext, instruction: &str, temperature: f32) -> Result<Proposal, AskError>` (builds prompt, `GenerateOptions::for_command(temperature)`, calls backend, `finalize_llm_reply`, `is_destructive_command`).
- Consumes: Tasks 3–5.

- [ ] **Step 1:** Write failing tests with `MockBackend`: `ask_returns_clean_safe_command` (reply `` `du -sh .` `` → command `du -sh .`, not destructive); `ask_flags_destructive_reply` (`rm -rf ~` → `is_destructive`); `ask_rejects_multi_line_reply`; `ask_rejects_empty_whitespace_and_empty_fence_replies` (three cases → `EmptyReply`); `ask_sends_prompt_with_instruction_last` (recorded request's prompt ends with the instruction section); `ask_propagates_backend_error_text` (timeout error text unchanged); `ask_uses_command_options` (128 tokens, stop `\n\n`).
- [ ] **Step 2:** Run them; expected: FAIL (`ask` undefined).
- [ ] **Step 3:** Implement `ask` and the types.
- [ ] **Step 4:** `cargo test -p kourage-core`. Expected: all pass with no Ollama running.
- [ ] **Step 5:** Commit: `git add -A && git commit -m "Add kourage_core::ask with end-to-end tests on the mock backend"`.

### Task 7: Slim the terminal side and verify

**Files:**
- Modify: `alacritty/src/voice.rs`, `alacritty/src/ai/mod.rs`, `alacritty/Cargo.toml`, `ROADMAP.md`, `README.md` (only if it mentions file layout)

**Interfaces:**
- Consumes: `kourage_core::{ask, Proposal, AskError, OllamaBackend, LlmSettings, AiContext, SystemPlatform}`.

- [ ] **Step 1:** Write failing test `text_prompt_flow_reaches_preview_event_with_mock` in `voice.rs`: refactor `handle_text_prompt`'s worker body into `fn run_text_prompt(backend: &dyn Backend, ...) -> Result<Proposal, String>` and assert via `MockBackend` that a destructive reply yields `is_destructive: true` and a backend failure yields the status-line error text.
- [ ] **Step 2:** Make `call_llm` a thin wrapper over `ask`; `send_preview` takes a `Proposal` instead of re-running the safety check; delete dead code and the `reqwest`/`anyhow` imports in `alacritty` if now unused (keep what Whisper/voice still uses).
- [ ] **Step 3:** Full verification: `cargo test --workspace`, `cargo clippy --all-targets`, `cargo tree -p kourage-core | grep -E "winit|alacritty|cpal|whisper"` (expect no output), `cargo build`.
- [ ] **Step 4:** Manual smoke in the real app (`./target/debug/alacritty -v`, Ollama running): text prompt "show disk usage" → preview → cancel; a destructive request ("delete everything here") → flagged preview; stop Ollama → actionable error message in the status line. Compare messages with the pre-refactor wording.
- [ ] **Step 5:** Update `ROADMAP.md`: record "Stage 1, first half: `kourage-core` extracted" and tick the "no mock LLM backend" gap; add the crate to the project layout notes.
- [ ] **Step 6:** Commit: `git add -A && git commit -m "Terminal calls kourage-core; roadmap updated"`.

---

## Self-review (done)

- **Spec coverage:** goals 1–6 map to Tasks 3–7; golden tests and order of work map to Tasks 1–2; the seven post-check decisions appear as: baseline/golden (Tasks 1–2), CI (Global Constraints), no cancellation (Task 5 trait has none), structured errors (Task 6), plain-data context and `gather` (Task 4), other-OS strings (Task 4), layout (Task 3).
- **Type consistency:** `LlmSettings`, `GenerateRequest`, `Backend`, `Proposal`, `AskError`, `AiContext::gather` are defined once (Tasks 4–6) and used by name afterwards.
- **Proportion:** the plan is about three times the spec's length, mostly interface blocks and test names; no function bodies are written out.

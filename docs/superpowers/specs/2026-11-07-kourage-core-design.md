# `kourage-core`: extract the brain from the terminal (design)

Status: draft, awaiting owner approval. Owner direction (2026-11-07): choose the path that gives the best result, not the easiest or fastest.

## Intent and why
Today the AI logic (prompt building, safety analysis, reply cleaning, LLM client, error messages) lives inside the `alacritty` crate, mixed with winit, cpal and Whisper code. Consequences:

- The ask → command → safety verdict path cannot be tested without a real Ollama (the roadmap lists "no mock LLM backend").
- The next planned features (shell hook, error diagnosis) would be added to `voice.rs` and `event.rs`, and Stage 1 would then have to pull them out again.
- The AI patch surface in the Alacritty fork is large, which makes upstream rebases costly.
- Quality work (an eval set to tune prompts and models) needs a way to call the brain without a terminal.

This change moves the brain into a UI-free library crate, `kourage-core`, with a swappable LLM `Backend`, and leaves the terminal as a thin caller. It is the first half of Roadmap Stage 1. The daemon is the second half and comes later.

## Goals
1. New workspace crate `kourage-core` containing: prompt building, safety analysis, reply cleaning, LLM-error messages, the `AiContext` data type, and the LLM client behind a `Backend` trait with `OllamaBackend` and `MockBackend`.
2. `kourage-core` has no dependency on `winit`, `alacritty_terminal`, `cpal`, `whisper-rs`, or anything in the `alacritty` crate.
3. Behaviour is unchanged: the prompt text is byte-identical for the same inputs, replies and verdicts are identical, config keys are unchanged.
4. A single high-level entry point, `ask(&dyn Backend, &AiContext, instruction) -> Result<Proposal, AskError>`, where `Proposal` carries the cleaned command and the safety verdict. End-to-end tests drive it with the mock backend.
5. OS-specific lookups (OS description, default shell) sit behind a small `Platform` trait with Linux, macOS and Windows implementations, so the crate stays cross-platform (Roadmap "Cross-platform release").
6. The `alacritty` crate's AI code shrinks to: gathering terminal-side context, status/overlay UI, voice capture and Whisper, and calling `kourage_core::ask`.

## Non-goals
- No daemon, socket, or wire protocol (Stage 1's second half; its protocol should be shaped by the shell hook and diagnosis work).
- No new features, no prompt changes, no safety-rule changes.
- No move of voice capture/Whisper out of `alacritty`.
- No config-file changes (`[ai]` section and keys stay as they are).
- The eval harness is a separate design, but this one must make it possible.

## Anti-goals (failure even if it compiles)
- Any change in what the user sees or gets: same prompts, same replies, same verdicts, same messages.
- A `kourage-core` that quietly depends on terminal types, or a `Backend` trait so Ollama-shaped that a second backend would need a redesign.
- Tests that only exist for the mock. The Ollama backend's request shape stays tested.
- Losing any existing test: all 170 current tests either remain or move with their code.

## Constraints
- Rust 2024, workspace resolver 2, Apache-2.0, same as the rest of the repo.
- `reqwest` 0.11 blocking stays (no async runtime is introduced).
- Windows and macOS must still compile (the crate is built on all three in CI where that exists today).
- Must keep `cargo clippy --all-targets` clean.

## Approaches considered
- **A. Keep the structure, add features in place.** Fastest, but the hook and diagnosis would be built into UI-coupled files and redone in Stage 1. Rejected: rework, and it keeps the AI untestable end to end.
- **B. Extract `kourage-core` in place, then build on it (chosen).** Costs a stretch with no visible change, but every later step (eval set, shell hook, diagnosis, daemon) lands on a tested, UI-free base.
- **C. Build the full daemon now.** The protocol would be fixed before we know what the hook and diagnosis need to send. Rejected for now; B is a prerequisite for it anyway.

Sub-decision, where config lives: the `Voice` config struct uses `alacritty_config_derive::ConfigDeserialize`, which is tied to the terminal's config system. Chosen: it stays in `alacritty`, and the terminal converts it into a plain `LlmSettings` struct owned by core. Moving it into core was rejected because it would pull the terminal's config derive into the library.

## Design

### Crate layout (the builder may adjust file names)
```
kourage-core/
  src/lib.rs          public API re-exports
  src/context.rs      AiContext (plain data), tech-stack detection, directory listing
  src/platform.rs     Platform trait + Linux/macOS/Windows impls (os description, shell)
  src/prompt.rs       format_prompt, PROMPT_EXAMPLES, file/line limits
  src/safety.rs       moved from alacritty/src/ai/safety.rs
  src/reply.rs        moved: clean_llm_reply, finalize_llm_reply, require_single_line
  src/diagnostics.rs  moved: actionable LLM error messages
  src/backend/mod.rs  Backend trait, GenerateRequest, LlmSettings, BackendError
  src/backend/ollama.rs  OllamaBackend (request body, keep_alive, options, warm-up)
  src/backend/mock.rs    MockBackend (scripted replies/errors, records requests)
  src/ask.rs          ask(), Proposal, AskError
```

### What stays in `alacritty`
- Extracting recent terminal lines from a `Term` and finding the foreground process cwd (both need terminal/pty types). They fill an `AiContext` and pass it to core.
- `AiStatus`/`AiPhase` and the status line, the preview overlay, key handling.
- Voice capture, resampling, Whisper, transcript cleaning, `BusyGuard`, `VoiceSession`.
- The `Voice` config struct and its legacy `[voice]` handling.

### Backend trait
```
trait Backend: Send + Sync {
    fn generate(&self, request: &GenerateRequest) -> Result<String, BackendError>;
    fn warm_up(&self, prefix: &str) -> Result<(), BackendError>;   // default: no-op
    fn model(&self) -> &str;           // model name, for status text ("model: …")
}
```
`GenerateRequest` holds the prompt and generation options (temperature, max tokens, repeat penalty, stop sequences). `BackendError` distinguishes timeout, connection failure, HTTP status with body, and malformed response, so `diagnostics` can keep producing the same messages. A second implementation (llama.cpp in-process, remote) must fit without changing the trait; that is the test of the shape.

### `ask`
`ask` builds the prompt, calls the backend, cleans and single-line-checks the reply, runs the safety analysis, and returns `Proposal { command, is_destructive }` or an `AskError` carrying the same user-facing message as today.

### Keeping behaviour identical
- Before moving anything, add golden tests that pin `format_prompt` output for several fixed contexts and instructions, and the serialized Ollama request body. They must pass unchanged after the move.
- Existing tests move with the code they cover.
- Where `alacritty` still has a thin caller (`call_llm`, `handle_text_prompt`, `handle_voice_command`), a test with `MockBackend` covers the path up to the preview event.

### Order of work
1. Finish and commit the latency work already in the tree (prompt reorder, trimming, warm-up, `keep_alive`), re-measure in the real app, update the roadmap.
2. Add golden tests (prompt, request body).
3. Create the crate; move pure modules (`safety`, `reply`, `diagnostics`) with their tests.
4. Move `AiContext`, `Platform`, prompt building.
5. Introduce `Backend`, `OllamaBackend`, `MockBackend`; move the HTTP code out of `voice.rs`.
6. Add `ask` and end-to-end tests.
7. Slim the terminal side; run full tests and clippy; manual smoke test in the real app.

## Decisions made after the builder check (my calls, please check these)
1. **Baseline and golden reference.** Step 1 is "done" when the latency work is committed, the in-app latency is re-measured, and `ROADMAP.md` records the numbers. The golden tests (prompt text, Ollama request body) are written only after that commit and pin the post-step-1 behaviour. The test baseline is the 170 tests passing on that commit (verified green on the current tree on 2026-11-07).
2. **CI exists.** `.github/workflows/ci.yml` runs `cargo test` and clippy on Windows and macOS (not Linux; Linux is checked locally). The end-to-end mock tests run in the existing workspace `cargo test`, so no new CI job is needed. Keeping the new crate building on all three is part of done.
3. **No cancellation.** Today an in-flight request cannot be cancelled (the roadmap lists this as open) and the only limit is the timeout. So `Backend::generate` is blocking and takes its timeout from `LlmSettings`. Cancellation is a non-goal here; adding it later means a trait change, which is acceptable because it is a new feature.
4. **Structured errors.** `AskError` is an enum (empty reply, multi-line reply, backend failure, and so on). Its `Display` is exactly today's user-facing message, so the terminal shows the same text but can branch on the kind.
5. **Context is plain data.** `AiContext` stays a plain struct and `ask` takes only the context and instruction, so mock tests are deterministic. `Platform` is used by a separate gathering function (`AiContext::gather(cwd, &dyn Platform, ...)`), called by the terminal. `ask` does no filesystem or process probing.
6. **Other OSes.** The macOS and Windows `Platform` implementations must compile and return the same strings as today (`"macOS (Unix)"`, `"Windows"`). I can only test Linux here, so CI is the check for the others.
7. **Layout.** The crate is a top-level `kourage-core/` directory and a workspace member. The root `Cargo.toml` edit and a lockfile change are accepted; the patch to upstream files stays small.
8. **Accepted deviations in the built code.** `ReplyError`/`AskError` use a `MultiLine(usize)` tuple variant (the message embeds the line count). `BackendError` has a fifth variant, `Transport`, so non-connect transport errors keep today's text. `LlmSettings.temperature` is the configured default; the per-request `GenerateOptions.temperature` wins. The trait method is `model()`, not `name()`.

## Left to the builder
File and type names, error-type derive crate (`thiserror` or hand-written), exact `GenerateRequest` field shape, how `warm_up` is scheduled (thread stays in the terminal or moves to core), whether `AiContext` fields stay `pub`.

## Success looks like
All existing behaviour unchanged, the golden tests green, `cargo test` and `cargo clippy --all-targets` clean on the workspace, `kourage-core` building with no terminal/windowing dependencies (`cargo tree -p kourage-core` shows none), and an end-to-end ask → proposal test running in CI with no Ollama.

# Kourage Roadmap: from AI terminal to AI-native Linux OS

## Vision

Build an operating system where you state intent (typed or spoken) and the system acts on it. All inference runs locally and every action is inspectable and reversible.

Path: **AI in an app → AI as a system service → AI as the default experience in a Linux distro.**

## Guiding principles

1. **Local-first.** No cloud dependency. Cloud models are an optional plugin and never the default.
2. **The AI proposes and the user disposes.** Nothing destructive runs without a preview. Every agent action is logged and, where possible, undoable.
3. **One brain, many clients.** The terminal is the first client of the AI, not its home. Anything the OS will need (shell, launcher, file manager, settings) must be able to reuse the same service.
4. **Degrade gracefully.** The system must stay useful with no GPU, a small model, or no microphone.
5. **Stay shippable.** Each stage ends with something a real person can install and use.

## Where we are (Stage 0: mostly built, not finished)

Working in the Alacritty fork: prompt bar, voice via Whisper, Ollama client, context engine (cwd, shell, OS, tech stack, buffer), and a preview and destructive-command guard. The project builds and its tests pass, but the open gaps below block calling Stage 0 done.

The architectural problem (everything wired into `event.rs`, `display/mod.rs`, `voice.rs`) is Stage 1's job. The items below are Stage 0's own unfinished business.

### Next up (agreed 2026-11-06)

Order matters: each step makes the next one cheaper or more reliable.

1. **Prompt speedups (latency).** Measured on the author's CPU-only machine with the real 599-token prompt: reading the prompt costs 11.4 s cold, 3.1 s partly cached and 7.7 s when the context changes; writing the answer costs under 1 s. So the wait is prompt processing, not generation. In the app, a request takes 14 to 18 s today. Planned changes, with measured gains:
   - [x] Put the fixed text (rules, examples) first and the changing text (environment, terminal output, instruction) last, so Ollama can reuse its cache: about 8.6 s to 4.5 s.
   - [x] Trim the context: cap the file listing to about 15 names and the terminal output to the last 2 to 3 lines, and include the listing only when the request mentions files: about 4.5 s to 1.6 s.
   - [x] Warm the model with one tiny request at startup, which removes the roughly 11 s cold start. `keep_alive` (30m) is sent so it stays loaded.
   - [x] `LLM request took` is logged for each request (`run_prompt` in `voice.rs`), and `LLM warm-up finished` for the warm-up.
   - Direct-API measurement (new prompt, about 500 prompt tokens, CPU-only, `qwen2.5-coder:3b`), three different requests in a row: 11.4 s (model not yet loaded), then 2.2 s, then 0.7 s.
   - [ ] OPEN: re-measure in the real app flow (not yet done; needs an interactive session).
   - [ ] OPEN: check whether the status line's 4 redraws per second slow Ollama on this CPU-rendered (llvmpipe) window, and lower the rate while waiting if so.
   - [ ] OPEN, optional: 5 examples instead of 10 (saves about 125 tokens). Try `qwen2.5-coder:1.5b` only after the above, since its answers are worse.
   - Use `-vv` for the voice WAV dump when tuning Whisper (resampling filter, settings), which still mis-hears some speech.
2. **Shell hook.** Bash/zsh/fish snippets that record each command, its exit code and where its output starts and ends. Screen scraping cannot tell where the last command ended. This is the Stage 2 shell integration, pulled forward because step 3 needs it.
3. **Error diagnosis and fix (hotkey).** After a failed command, send the command and its error output to the model and show a one-line explanation plus a proposed fix in the existing preview (still confirm-to-run, still safety-checked). Expect weaker answers from a 3B model, and keep prompts short for latency.

Later, once there is real data: collect accepted and rejected commands (opt-in, local only), build a roughly 100-case eval set, then consider fine-tuning a small model. A tuned model could drop most of the long prompt, which is itself a latency win. Writing our own model from scratch is out of scope.

### Stage 0 gaps

- [ ] OPEN: warm-up fires unconditionally at startup; consider a config flag to disable it.

**Voice and hotkeys:**
- [x] ~~Float WAV written, read back as `i16`.~~ Fixed: audio now stays in memory, so there's no WAV round-trip.
- [x] ~~No resampling to 16 kHz; only 2-channel downmix.~~ Fixed: any channel count and any rate are converted (`to_whisper_input`, tested).
- [x] ~~Silence detection stops before the user speaks.~~ Fixed: silence only counts after speech is heard (1 s trailing), and recording gives up after 6 s with no speech (`SilenceDetector`, tested).
- [x] ~~Holding Ctrl+Shift+S starts many overlapping recordings ("Connection refused" from ALSA).~~ Fixed: only one voice session runs at a time (`BusyGuard`, tested).
- [x] ~~The prompt hotkey Ctrl+Shift+Space needs two presses under WSLg (the first Space never arrives), and it took vi mode's upstream binding.~~ Fixed: the prompt is now Ctrl+Shift+A, and Ctrl+Shift+Space toggles vi mode again.
- [x] ~~Backspace (and any other bound key) inside the prompt or preview goes to the shell.~~ Fixed: only the prompt toggle binding fires while the overlay is open.
- [x] ~~The input callback is hard-typed to `f32`, so stream creation fails on devices whose default format is `i16`/`u16`.~~ Fixed: the stream is built per sample format (`f32`, `i16`, `u16`, `i32`) and converted with `convert_to_f32` (tested).
- [ ] Resampling is linear with no anti-alias filter. Fine for speech, but revisit if accuracy suffers.
- [x] ~~There's no way to cancel a recording, and no push-to-talk.~~ Fixed: Esc cancels a recording (`VoiceSession`, tested). Push-to-talk is still open: it needs key-release events.
- [x] ~~The Whisper model is loaded from disk on every command (slow). The default `whisper_model_path` is a placeholder, so voice fails out of the box with no guidance.~~ Fixed: the model is cached after first load, and a missing path now gives download instructions.
- [x] ~~Pasting into the prompt doesn't work (Paste is blocked while the overlay is open; it should insert into the prompt).~~ Fixed: paste is allowed while the prompt is open and line breaks become spaces (tested).
- [x] ~~**Next up (found 2026-11-06 in a live test):** the model replied `` `du -sh .` `` with single backticks. Only triple-backtick blocks are stripped, so bash ran it as command substitution and tried to execute du's output. Strip single backticks too, with tests.~~ Fixed: `clean_llm_reply` (tested) strips fences and a wrapping backtick pair.
- [ ] **Next up (dump added, tuning pending):** voice transcribed "Bye." (a typical Whisper hallucination on near-silence) after only 2.5 s of recording, so the recording probably stopped before the user spoke. Add a debug dump of the 16 kHz audio to `/tmp` (when debug logging is on), listen to it, then tune `SPEECH_RMS_THRESHOLD` and the timings. The dump now exists: run with `-vv` and listen to `/tmp/kourage-voice-<pid>-<time>.wav (mode 0600)`. **Partly fixed (2026-11-06):** Whisper non-speech markers (`[BLANK_AUDIO]`, `(music)`) and the "Bye."/"Thank you." silence hallucinations are now dropped (`clean_transcript`, tested), so they no longer reach the LLM. Threshold tuning still waits on listening to the dump.
- [x] ~~**Next up:** the preview marked `exit` as safe, and running it closed the terminal. Flag commands that end the session (`exit`, `logout`, `kill $$`, `exec …`).~~ Fixed (tested).
- [x] ~~Voice has no default binding on macOS (it lives in `common_keybindings`, which excludes macOS).~~ Fixed: Ctrl+Shift+S is now in the shared default bindings (tested).

**LLM client robustness:**
- No reply-length cap. With temperature 0, `qwen2.5-coder:3b` can fall into a repetition loop: a bare prompt produced 3,700+ tokens of `$6, $7, …` before we killed it. Set `num_predict` (about 128) and a `repeat_penalty`. **Fixed:** requests now send `num_predict: 128`, `repeat_penalty: 1.1` and a `\n\n` stop (`LlmOptions::for_command`, tested).
- The model ignores "no code fences" and wraps replies in ```` ```bash ````. Fence stripping is required, and needs a test. **Fixed:** `clean_llm_reply` strips fences and backticks (tested).
- Answer quality at 3B is shaky: "list hidden files sorted by modification time" gave `ls -la | grep ^. | sort -k6`. The preview is essential; consider few-shot examples in the prompt. **Done:** 10 few-shot examples are now in the prompt (`PROMPT_EXAMPLES`, each tested to be a safe single line). Whether they improve answers is unmeasured: that needs the eval set from `docs/stage0-solutions.md`.
- If Ollama isn't running or the model isn't pulled, the user sees an opaque "failed to parse response" error. Add a health check and actionable messages ("run `ollama pull qwen2.5-coder:3b`"). **Fixed:** `ai/diagnostics.rs` maps connection, 404 and HTTP errors to actionable messages (tested).
- The single-line output isn't enforced. A multi-line reply gets written to the PTY with embedded newlines, so several commands run on one confirm. Reject or collapse multi-line output and show it explicitly in the preview. **Fixed:** `require_single_line` rejects multi-line replies with a clear message (tested). Showing multi-line replies in the preview is still open.
- An in-flight request can't be cancelled, and there's no "thinking" indicator beyond the message bar. The blocking 15 s timeout fits small models only.
- LLM settings live under `[voice]` even though the text prompt uses them too. Rename to `[ai]` (with a migration) before more config piles up. **Fixed:** the section is now `[ai]`, with `[voice]` still accepted as an alias (tested). No deprecation warning is logged yet.

**Safety guard holes:**
- It misses common dangers: `sudo …`, `curl … | sh`, `find … -delete`, `mv` onto system paths, `> file` truncation, `chmod -R 777 ~`, `kill -9 -1`, and `rm` behind `xargs` or `;`/`&&` chains. **Fixed:** a tokenizer-based analyzer now flags all of these (`ai/safety.rs`, table-tested). Known gaps: `$'…'` quoting, `x=rm; $x -rf ~`, heredocs, and quote-naive raw SQL patterns (`drop table` inside a grep or commit message is flagged).
- It has false positives: any `-r` substring in an `rm` line (e.g. `rm my-report.txt`). **Fixed:** `rm` is judged by parsed flags, so `rm my-report.txt` is safe.
- After `to_lowercase()`, the `-R` checks are dead code. **Fixed:** flag checks are case-sensitive now.
- Needs a table-driven test of dangerous vs safe commands (the roadmap already plans this for Stage 1; it should exist now). **Fixed:** `test_dangerous_command_table` covers about 70 commands.

**Testing and quality:**
- No tests for the voice pipeline (WAV round-trip would have caught the bug above), LLM response parsing (code-fence stripping, empty and multi-line replies), or the prompt/preview state machines. **Done:** added tests for audio conversion, WAV encoding, voice cancel, reply cleanup and finalising, LLM error messages, paste and the safety table. The prompt/preview state machines are still lightly tested.
- No mock LLM backend, so the end-to-end flow can't be tested in CI. **Fixed:** `kourage-core` has a `MockBackend`, and end-to-end tests run `ask` against it with no Ollama (`kourage-core/src/ask.rs`, plus `text_prompt_flow_reaches_preview_event_with_mock` in `voice.rs`).
- `cargo clippy --all-targets` reports 10 errors from earlier Kourage code (`ai/context.rs`, `ai/safety.rs`, `display/mod.rs`, `window_context.rs`, `event.rs` tests). CI will fail until they're fixed. **Done:** clippy is clean (CI already runs it).
- Testing tip: `cargo test` doesn't rebuild the app binary. Run `cargo build` before manual testing (a stale March binary cost us a debugging round).

**Docs and first-run:**
- The README describes the happy path, but there's no troubleshooting section (no mic, no model, Ollama down) and no list of keybindings for the preview (confirm/edit/cancel). **Fixed:** a troubleshooting table was added; the preview keys were already listed.
- On first run, the terminal should detect what's missing (Ollama, model, Whisper file) and say so once, clearly.

### Dev environment

WSL2 is fine for Stage 0 to 2 (terminal, daemon, shell integration) but not beyond:
- **Stage 3** needs real systemd user units, bubblewrap/Landlock sandboxing and btrfs/ZFS snapshots, all of which are partial or absent in WSL.
- **Stage 4** needs a real desktop session (global hotkeys, D-Bus, NetworkManager, GNOME/KDE extensions).
- **Stage 5** needs to boot an ISO on real hardware.
- **Throughout:** WSL audio goes through WSLg's RDP bridge and GPU access for models is limited, so voice and model performance there aren't representative.

Decision (2026-11-06): develop in WSL2 (Ubuntu) on Windows through the end of Stage 2. Before starting Stage 3, set up a dual-boot Ubuntu install (or Linux on an external SSD). Add a VM later for testing the Stage 5 ISO.

TODO for WSL setup (on any Windows machine):
- [ ] Clone the repo and run `cargo build` and `cargo test -p alacritty`. Install the build deps from `INSTALL.md`, plus `clang`/`libclang` for whisper-rs and `libasound2-dev` for cpal.
- [ ] Install and check audio tooling: `sudo apt install alsa-utils pulseaudio-utils libasound2-plugins`. Then `pactl list short sources` should show WSLg's `RDPSource`, and `arecord -d 3 -f cd /tmp/t.wav && aplay /tmp/t.wav` should play back your voice. If ALSA finds no device, add an `~/.asoundrc` routing `default` to `pulse`.
- [ ] Install Ollama **inside WSL**, not on Windows, because WSL can't reach Windows' `localhost` without mirrored networking. Run `ollama pull qwen2.5-coder:3b`, and check that `curl localhost:11434/api/tags` lists it.
- [ ] Download a Whisper model (e.g. `ggml-base.en.bin`) to `~/.config/alacritty/models/` and point `[voice] whisper_model_path` at it in `~/.config/alacritty/alacritty.toml`.
- [ ] Run `cargo build`, then smoke test with `./target/debug/alacritty -v`: Ctrl+Shift+A opens the prompt, and Ctrl+Shift+S records and transcribes voice. For key debugging, set `[debug] print_events = true` and `persistent_logging = true`, then read `/tmp/Alacritty-<pid>.log`.
- [ ] Ollama installed without root: download `ollama-linux-amd64.tar.zst` from ollama.com, extract it to `~/.local/ollama`, and run `~/.local/ollama/bin/ollama serve`.

**Exit criteria for Stage 0:** voice works end to end on a stock Ubuntu laptop mic; Ollama-down and model-missing produce helpful messages; multi-line LLM output can't slip through; the safety table test passes; a fresh user can go from clone to first working command using only the README.

---

## Stage 1: Decouple the brain (the key architectural step)

Goal: split AI logic from terminal UI so the terminal becomes a thin client.

- New workspace crate `kourage-core` (library): context, prompt building, safety classification, intent to command. It has no dependency on any UI or windowing code.
- New binary `kouraged` (user-level daemon) with a local API over a Unix socket (JSON-RPC or gRPC). Methods: `ask`, `transcribe`, `explain`, `diagnose`, `history`.
- Terminal changes: `alacritty/src/ai/` and `voice.rs` shrink to a client that calls `kouraged`. Fallback: if the daemon is absent, embed `kourage-core` directly so the terminal still works standalone.
- Provider abstraction: a `Backend` trait with Ollama, llama.cpp (in-process) and an optional remote implementation.
- Config moves to its own `kourage.toml`, separate from the terminal config.
- Tests: unit tests for the safety classifier (a table of dangerous vs safe commands) and for context extraction, plus a mock backend for integration tests.

Exit criteria: terminal behaviour is unchanged, and `kouraged` can be driven from a plain CLI (`kourage ask "..."`) with no terminal running.

Progress:
- [x] Stage 1, first half (2026-11-07): `kourage-core` extracted (UI-free crate with Backend trait, Ollama + mock backends, `ask()` entry point, golden tests). It lives in the top-level `kourage-core/` directory as a workspace member and holds prompt building, safety analysis, reply cleaning, LLM error messages, `AiContext` and the `Backend` trait. `cargo tree -p kourage-core` shows no `winit`, `alacritty_terminal`, `cpal` or `whisper-rs`. The terminal now calls `kourage_core::ask` from `voice.rs`; Whisper and audio capture stay in the terminal for now. Design: `docs/superpowers/specs/2026-11-07-kourage-core-design.md`.
- [ ] OPEN: manual smoke test of the extracted flow in the real app (`./target/debug/alacritty -v`, Ollama running; needs an interactive session): the text prompt "show disk usage" shows a preview that can be cancelled; "delete everything here" shows a flagged (destructive) preview; with Ollama stopped, the status line shows the actionable error. Compare the wording with the pre-refactor messages.
- [ ] Note: kourage-core modules are both `pub mod` and re-exported at the crate root; pick one path style before external users.
- [ ] Second half: the `kouraged` daemon, the CLI, more backends and `kourage.toml`.

## Stage 2: A genuinely useful terminal agent

Goal: move from "one sentence → one command" to an assistant that helps with real work.

- **Diagnose and fix:** hotkey that reads the last failed command and its output, explains the error, and proposes a fix.
- **Shell integration:** hook scripts for bash, zsh and fish that report command, exit code, cwd and duration. This is more reliable than scraping the screen buffer.
- **Workspace awareness:** read manifests and git state (Cargo, npm, Python, Makefile) so commands fit the project.
- **Multi-turn memory:** session history so "now do the same for the other folder" works.
- **Explain mode:** flag-by-flag breakdown of a command before running it.
- **Multi-step plans:** the model proposes a sequence, the user approves per step or all at once.
- **Safety v2:** replace the pattern list with a layered check: static rules, then parsing of the shell AST, then a model-based risk review. Add a dry-run option where the tool supports one.

Exit criteria: the terminal is the author's daily driver for two weeks, and a short list of failure cases is recorded and fixed.

## Stage 3: The system service and tools layer

Goal: turn `kouraged` into a platform other programs can use.

- **Tool registry:** typed tools the model can call (run command, read file, search files, list processes, open app, change setting). Each tool declares the permissions it needs.
- **Permission model:** per-tool capabilities (read-only, write in cwd, network, system) with user grants and an audit log. Enforce with real sandboxing (bubblewrap/Landlock), not just prompts.
- **Undo:** filesystem snapshots (btrfs/ZFS where available, otherwise a copy-on-write fallback for touched files) so an agent action can be rolled back.
- **MCP support:** speak the Model Context Protocol so existing tool servers plug in.
- **Model manager:** download, verify, switch and size models by hardware tier (CPU-only, 8 GB GPU, 16 GB+).
- **Voice as a service:** wake word or push-to-talk, STT and TTS handled by the daemon so any app can use them.
- **Packaging:** `.deb` for `kouraged` and the terminal, a systemd user unit, and a one-line installer.

Exit criteria: a second client (for example a GTK or Qt launcher, or a Nautilus-style file-manager extension) works against the same daemon with no changes to it.

## Stage 4: Desktop-level integration

Goal: the AI is available everywhere in the session, not only in the terminal.

- Global invoke hotkey and overlay (like Spotlight) that works over any window.
- Natural-language settings: "make the screen dimmer", "connect to the office wifi" via NetworkManager, D-Bus and portal APIs.
- File search by meaning (local embeddings index, opt-in).
- Notification and clipboard awareness (opt-in), such as "summarise what I just copied".
- Choose a desktop strategy (see decisions below): extensions on an existing desktop first, own shell later if justified.

Exit criteria: a non-technical tester can accomplish ten common tasks by voice or text without touching the terminal.

## Stage 5: Kourage OS (the distro)

Goal: a bootable, installable flavor with Kourage as the default experience.

- **Base:** build on an existing distro rather than from scratch. Leading candidates: Ubuntu or Debian (live-build, broad hardware support, easy remix) or Arch (archiso, newest packages). NixOS is attractive for reproducibility but has a steeper contributor curve.
- **Build pipeline:** reproducible ISO built in CI, signed releases, an apt repository for updates.
- **First-boot experience:** hardware detection, pick a model tier, download the model, short voice and permission onboarding.
- **Defaults:** Kourage terminal as the default terminal, `kouraged` enabled for the user session, the desktop invoke hotkey configured, sane sandbox policies shipped.
- **Offline operation:** the ISO should be usable without internet once a model is present (offer a "big" ISO with a bundled small model).
- **Telemetry:** none by default.
- **Project hygiene:** website, docs, bug tracker, security disclosure policy, and a clear upgrade and rollback story.

Exit criteria: someone other than the author installs it on real hardware from the ISO and completes the onboarding.

## Cross-platform release (Kourage app for Windows, macOS and Linux)

Once the app is in good shape, make a copy of it, separate from the Linux distro track, that runs on all major operating systems. Alacritty is already cross-platform, so this stays cheap as long as Kourage code doesn't lock itself to Linux. Rules from Stage 1 onward:

- `kourage-core` stays OS-agnostic. OS-specific bits (cwd lookup via `/proc`, `/etc/os-release`, shell detection) go behind a small platform trait with Linux, macOS and Windows implementations.
- Daemon transport: a Unix socket on Linux/macOS and a named pipe on Windows, behind one interface. When the daemon is absent, the embedded fallback (Stage 1) is the default on Windows and macOS.
- Shell integration (Stage 2) covers PowerShell as well as bash, zsh and fish.
- Safety rules include Windows-dangerous commands (`Remove-Item -Recurse`, `format`, `del /s`, `diskpart`).
- Linux-only features from Stages 3 to 5 (Landlock/bubblewrap, btrfs snapshots, NetworkManager, systemd) stay Linux-only, and the cross-platform app degrades gracefully without them.
- Packaging: `.msi` for Windows (Alacritty already has one), `.dmg` for macOS, `.deb`/AppImage for Linux. CI should build all three from Stage 1 on, so breakage shows up early.

Timing: after Stage 2, the app is a complete product. The cross-platform release can ship then, in parallel with Stage 3.

## Stage 6: Beyond (only after Stage 5 is stable)

Custom desktop shell, on-device fine-tuning or LoRA for personal workflows, multi-device sync, and an app ecosystem that exposes its actions to the agent.

---

## Branching

- **`master`** is the Kourage app and the only long-lived branch for now. Work lands here directly.
- When the distro work (Stage 3 onward) starts, create one **`os`** branch for it. From then on there are exactly two long-lived branches: the app and the OS.
- No archive or stage branches: history lives in git tags and commits, and old code is not kept on branches nobody will use. Short-lived topic branches are fine and get deleted after merging.

---

## Cross-cutting risks

| Risk | Mitigation |
| :--- | :--- |
| Fork drift: upstream Alacritty will not take AI features | Keep Kourage-specific code in separate modules and crates, rebase upstream on a schedule, keep the patch surface small |
| Hardware range: local models are heavy | Hardware tiers, small default model, CPU fallback, clear minimum requirements |
| Safety: agent runs something harmful | Preview by default, sandboxing, snapshots, audit log, no silent root |
| Build complexity: whisper and clang bindings | Prebuilt binaries in packages, feature flags, CI matrix |
| Model licences and redistribution | Only bundle models whose licences allow it, otherwise download at first boot |
| Branding: "Kourage", the Attic Computer theme and the logo are tied to a third-party IP | Keep the name as a homage if desired but use original artwork, and check trademark risk before a public distro release |
| Scope: an OS is a huge project | Do not start Stage 5 until Stage 3 is solid. Each stage must stand alone as a useful product |

## Decisions needed from the owner

1. Daemon first (Stage 1 as written) or keep extending the terminal for a while longer?
2. Distro base: Ubuntu/Debian, Arch, or NixOS?
3. Desktop: ship GNOME/KDE with extensions, or aim for a custom shell later?
4. Is an optional cloud-model plugin acceptable, or strictly local only?
5. ~~Dev environment~~ Decided: WSL through Stage 2, dual-boot before Stage 3.
6. ~~Cross-platform~~ Decided: yes, the app ships for Windows, macOS and Linux as well as the distro.

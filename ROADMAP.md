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

Working in the Alacritty fork: prompt bar, voice via Whisper, Ollama client, context engine (cwd, shell, OS, tech stack, buffer), and a preview and destructive-command guard. The project builds and its 91 tests pass, but the gaps below block calling Stage 0 done.

The architectural problem (everything wired into `event.rs`, `display/mod.rs`, `voice.rs`) is Stage 1's job. The items below are Stage 0's own unfinished business.

### Stage 0 gaps

**Voice is broken (must fix):**
- `record_audio` writes a 32-bit float WAV, but `transcribe_audio` reads it back as `i16`. hound 3.5.1 rejects that (`InvalidSampleFormat`, reproduced), so every voice command fails at transcription.
- Audio is never resampled. Whisper needs 16 kHz mono, but the recording uses the device rate (usually 44.1/48 kHz). Only exactly 2 channels are downmixed.
- The input callback is hard-typed to `f32`, so stream creation fails on devices whose default format is `i16`/`u16`.
- Silence detection can stop the recording before the user starts speaking (about 30 quiet callbacks). There's no way to cancel a recording, and no push-to-talk.
- The Whisper model is loaded from disk on every command (slow). The default `whisper_model_path` is a placeholder, so voice fails out of the box with no guidance.

**LLM client robustness:**
- If Ollama isn't running or the model isn't pulled, the user sees an opaque "failed to parse response" error. Add a health check and actionable messages ("run `ollama pull qwen2.5-coder:3b`").
- The single-line output isn't enforced. A multi-line reply gets written to the PTY with embedded newlines, so several commands run on one confirm. Reject or collapse multi-line output and show it explicitly in the preview.
- An in-flight request can't be cancelled, and there's no "thinking" indicator beyond the message bar. The blocking 15 s timeout fits small models only.
- LLM settings live under `[voice]` even though the text prompt uses them too. Rename to `[ai]` (with a migration) before more config piles up.

**Safety guard holes:**
- It misses common dangers: `sudo …`, `curl … | sh`, `find … -delete`, `mv` onto system paths, `> file` truncation, `chmod -R 777 ~`, `kill -9 -1`, and `rm` behind `xargs` or `;`/`&&` chains.
- It has false positives: any `-r` substring in an `rm` line (e.g. `rm my-report.txt`).
- After `to_lowercase()`, the `-R` checks are dead code.
- Needs a table-driven test of dangerous vs safe commands (the roadmap already plans this for Stage 1; it should exist now).

**Testing and quality:**
- No tests for the voice pipeline (WAV round-trip would have caught the bug above), LLM response parsing (code-fence stripping, empty and multi-line replies), or the prompt/preview state machines.
- No mock LLM backend, so the end-to-end flow can't be tested in CI.

**Docs and first-run:**
- The README describes the happy path, but there's no troubleshooting section (no mic, no model, Ollama down) and no list of keybindings for the preview (confirm/edit/cancel).
- On first run, the terminal should detect what's missing (Ollama, model, Whisper file) and say so once, clearly.

### Dev environment

WSL2 is fine for Stage 0 to 2 (terminal, daemon, shell integration) but not beyond:
- **Stage 3** needs real systemd user units, bubblewrap/Landlock sandboxing and btrfs/ZFS snapshots, all of which are partial or absent in WSL.
- **Stage 4** needs a real desktop session (global hotkeys, D-Bus, NetworkManager, GNOME/KDE extensions).
- **Stage 5** needs to boot an ISO on real hardware.
- **Throughout:** WSL audio goes through WSLg's RDP bridge and GPU access for models is limited, so voice and model performance there aren't representative.

Decision: move development to a native Linux machine now rather than at Stage 3.

TODO on the native machine:
- [ ] Clone the repo and run `cargo build` and `cargo test -p alacritty`. Install the build deps from `INSTALL.md`, plus `clang`/`libclang` for whisper-rs and `libasound2-dev` for cpal.
- [ ] Install and check audio tooling: `alsa-utils`, then `arecord -d 3 -f cd /tmp/t.wav && aplay /tmp/t.wav`.
- [ ] Install Ollama, run `ollama pull qwen2.5-coder:3b`, and check that `curl localhost:11434/api/tags` lists it.
- [ ] Download a Whisper model to `~/.config/alacritty/models/ggml-tiny.bin` and point `[voice] whisper_model_path` at it in `~/.config/alacritty/alacritty.toml`.
- [ ] Smoke test: the text prompt (Ctrl+Shift+Space) should work. Voice (Ctrl+Shift+S) is expected to fail at "Whisper transcription failed" until the WAV format bug above is fixed. Confirm this, then start on the Stage 0 gaps.

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

## Stage 6: Beyond (only after Stage 5 is stable)

Custom desktop shell, on-device fine-tuning or LoRA for personal workflows, multi-device sync, and an app ecosystem that exposes its actions to the agent.

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

# Alacritty AI & Voice Command Integration Tracker

This document tracks the roadmap, architecture, completed milestones, and upcoming tasks for transforming Alacritty from a standard terminal interface into an intelligent, context-aware AI agent terminal.

---

## 🎯 Project Objective

Transform Alacritty into an intelligent terminal that understands intent and context, and executes or automates tasks locally without cloud dependencies:
- **Natural Language & Voice Input**: Convert spoken voice (`whisper-rs`) or typed intent (`Ctrl+Shift+Space`) into valid shell commands.
- **Context-Aware Inference**: Provide the local LLM (e.g., Ollama / llama.cpp) with real-time environment context (Active CWD, Shell, OS, and Screen Buffer/Error context).
- **Safety & Interactive Guardrails**: Interactive command preview with execution (`Enter`), editing (`Tab`), cancellation (`Esc`), and destructive command warnings.
- **100% Local Execution**: Completely offline speech-to-text and LLM inference.

---

## 📊 Status Matrix

| Phase | Feature | Status | Module / Files |
| :--- | :--- | :--- | :--- |
| **Phase 1: Voice & STT** | Audio capture via `cpal` with RMS silence detection | ✅ Completed | `alacritty/src/voice.rs` |
| **Phase 1: Voice & STT** | Local Whisper speech-to-text inference | ✅ Completed | `alacritty/src/voice.rs` |
| **Phase 1: Voice & STT** | Keybinding `Ctrl+Shift+S` -> `Action::VoiceCommand` | ✅ Completed | `alacritty/src/config/bindings.rs` |
| **Phase 1: LLM Engine** | REST HTTP client to local Ollama (`/api/generate`) | ✅ Completed | `alacritty/src/voice.rs` |
| **Phase 1: PTY Execution** | In-process command injection via `TerminalEvent::PtyWrite` | ✅ Completed | `alacritty/src/voice.rs`, `event.rs` |
| **Phase 1: Toolchain** | Fix FFI/C-bindings (`whisper-rs-sys`, `libopenblas`, `clang-sys`) | ✅ Completed | `alacritty/Cargo.toml` |
| **Phase 2: UI & Feedback** | Message bar visual status (`Listening...`, `Transcribing...`, `Querying LLM...`) | 🔲 Planned (Next) | `alacritty/src/message_bar.rs`, `voice.rs` |
| **Phase 2: Config** | Configurable LLM model, prompt templates, and timeouts in `alacritty.toml` | 🔲 Planned (Next) | `alacritty/src/config/voice.rs` |
| **Phase 3: Context** | CWD extraction via `/proc/<pid>/cwd` and Shell metadata | 🔲 Planned | `alacritty/src/ai/context.rs` |
| **Phase 3: Safety Guard** | Interactive Preview Bar (`Enter` run, `Tab` edit, `Esc` cancel) | 🔲 Planned | `alacritty/src/display/`, `message_bar.rs` |
| **Phase 4: Multi-modal** | Text-based prompt bar overlay (`Ctrl+Shift+Space`) | 🔲 Planned | `alacritty/src/input/` |
| **Phase 4: Diagnostics** | Terminal buffer error diagnosis & fix (`Ctrl+Shift+E`) | 🔲 Planned | `alacritty/src/ai/` |

---

## 🛠️ What Has Been Done

1. **Audio Recording Engine (`alacritty/src/voice.rs`)**:
   - Captures microphone stream locally using `cpal`.
   - Uses RMS (Root Mean Square) energy thresholding for automatic silence detection.
   - Enforces a 10-second maximum recording timeout to prevent hanging.
   - Writes audio stream to temporary standard WAV format.

2. **Offline Speech Recognition (`alacritty/src/voice.rs`)**:
   - Integrated `whisper-rs` bindings.
   - Loads GGML Whisper models (e.g. `ggml-tiny.bin`, `ggml-base.bin`).
   - Handles stereo-to-mono normalization and greedy transcription.

3. **LLM Dispatcher (`alacritty/src/voice.rs`)**:
   - HTTP client using `reqwest` (blocking) querying local LLM daemon (Ollama endpoint).
   - System prompt to enforce clean single-line shell command generation.

4. **Terminal Event Loop & Action Mapping**:
   - Added `Action::VoiceCommand` in `alacritty/src/config/bindings.rs`.
   - Default hotkey `Ctrl+Shift+S` triggers the asynchronous worker thread.
   - Injects resulting shell command into the PTY stream via `EventLoopProxy` -> `TerminalEvent::PtyWrite`.

5. **Configuration Schema (`alacritty/src/config/voice.rs`)**:
   - Added `[voice]` section with `whisper_model_path` and `llm_api_url`.
   - Connected configuration parsing to `UiConfig`.

6. **Build System & Dependencies**:
   - Resolved all native system library bindings (`fontconfig`, `libasound2-dev`, `libclang-dev`, `libopenblas-dev`).
   - Clean compilation with zero compiler warnings.

---

## 📋 What Needs To Be Done Next

### 1. Visual Feedback in Message Bar (Immediate Priority)
- [ ] Connect `voice.rs` worker thread stages to Alacritty's `MessageBar`:
  - `🎙️ Recording audio... Speak your command`
  - `⚙️ Transcribing speech with Whisper...`
  - `🤖 Generating command via LLM...`
  - `❌ Error notification` if model/Ollama connection fails.

### 2. Configurable LLM Model & Settings
- [ ] Expand `alacritty/src/config/voice.rs` to include:
  - `model`: e.g. `"llama3"`, `"qwen2.5-coder:3b"`, `"mistral"` (default: `"qwen2.5-coder:3b"` or `"llama3"`).
  - `temperature`: LLM sampling temperature (default: `0.0` or `0.2` for deterministic commands).
  - `timeout_secs`: Timeout duration for LLM inference (default: `15`).

### 3. Context Awareness Engine
- [ ] **Current Working Directory (CWD)**:
  - Inspect `/proc/<child_pid>/cwd` (on Linux) to provide active directory path and folder contents to the LLM.
- [ ] **Environment Metadata**:
  - Detect active shell (`$SHELL`, `bash`, `zsh`, `fish`) and OS platform (`Ubuntu 24.04 Linux`).
- [ ] **Screen Buffer Context**:
  - Read the last 20–50 terminal lines for contextual operations (e.g. diagnosing a failed command).

### 4. Command Preview & Safety Confirmation Layer
- [ ] Render interactive confirmation overlay:
  - `[Enter]`: Execute command immediately.
  - `[Tab]`: Insert command into shell prompt without newline (for manual review and editing).
  - `[Esc]`: Discard command.
- [ ] Highlight destructive commands (`rm -rf`, `dd`, `mkfs`, `git reset --hard`) with a red warning badge.

### 5. Text-Based Natural Language Prompt Bar (`Ctrl+Shift+Space`)
- [ ] Add lightweight text input overlay bar to type natural language queries directly.
- [ ] Example: *"find all log files older than 7 days and gzip them"*.

### 6. Error Explanation & Auto-Fix (`Ctrl+Shift+E`)
- [ ] Add action to capture selected error output in terminal and ask the LLM for explanation and suggested fix command.

---

## 🧪 End-to-End Testing Checklist

1. **Verify Whisper Model**:
   - Download a GGML model:
     ```bash
     mkdir -p ~/.config/alacritty/models
     wget -O ~/.config/alacritty/models/ggml-tiny.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin
     ```
   - Update `alacritty.toml`:
     ```toml
     [voice]
     whisper_model_path = "~/.config/alacritty/models/ggml-tiny.bin"
     llm_api_url = "http://localhost:11434/api/generate"
     ```

2. **Verify Local LLM (Ollama)**:
   - Ensure Ollama is running:
     ```bash
     ollama run llama3 # or ollama run qwen2.5-coder:3b
     ```

3. **Run & Test**:
   - Start Alacritty: `cargo run --bin alacritty`
   - Press `Ctrl+Shift+S`.
   - Speak: *"Show all files including hidden ones"*.
   - Expected Output in PTY: `ls -a` (or `ls -la`).

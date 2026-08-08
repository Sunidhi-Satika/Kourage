# Alacritty Natural Language & LLM Integration Architecture Plan

## Executive Overview

This plan outlines the architecture for embedding natural language command generation and LLM assistance directly into the **Alacritty** terminal emulator. The goal is to allow users to express intent in natural language (via hotkeys, inline command bar, or voice) and automatically generate, preview, and execute appropriate shell commands (e.g., converting *"go to the Courage folder"* into `cd /path/to/courage`).

---

## Key Objectives

1. **Natural Language to Shell Translation**: Convert human prompt inputs (text or voice) directly into executable shell commands tailored to the user's OS and shell environment.
2. **Context-Aware Generation**: Supply the LLM with relevant context (current working directory, OS type, shell binary, and recent screen buffer context) for accurate results.
3. **Non-Blocking Architecture**: Run LLM inference and HTTP calls asynchronously to preserve Alacritty's high-performance 60+ FPS GPU rendering.
4. **Safety & Preview Guardrails**: Provide an interactive UI preview with confirmation before executing potentially destructive commands.
5. **Flexible Provider Backend**: Support local LLM daemons (Ollama, vLLM, llama.cpp), embedded in-process engines (`candle` / `llama-cpp-rs`), and cloud API fallbacks.

---

## System Architecture & Workflow

```
+-----------------------------------------------------------------------------------+
|                                  USER INTERFACE                                   |
|  - Keybinding Trigger (Ctrl+Shift+Space)                                          |
|  - Natural Language Overlay Bar (Alacritty Message Bar / Input Overlay)           |
|  - Voice Input Trigger (STT via Whisper)                                          |
+-----------------------------------------------------------------------------------+
                                         |
                                         v
+-----------------------------------------------------------------------------------+
|                             CONTEXT EXTRACTION ENGINE                             |
|  - Active CWD (Current Working Directory via PTY process / procfs)                |
|  - Active Shell ($SHELL / zsh / bash / fish / powershell)                         |
|  - Scrollback & Screen Buffer (Last N lines for error / status context)           |
+-----------------------------------------------------------------------------------+
                                         |
                                         v
+-----------------------------------------------------------------------------------+
|                                LLM AGENT PROVIDER                                 |
|  - Format System Prompt + User Query + Context Data                               |
|  - Send to Backend (Ollama / Local llama.cpp REST / Embedded Model)               |
|  - Parse Structured Output (JSON with command, explanation, risk level)           |
+-----------------------------------------------------------------------------------+
                                         |
                                         v
+-----------------------------------------------------------------------------------+
|                           PREVIEW & CONFIRMATION GUARD                            |
|  - Render Command Preview Overlay in Terminal UI                                  |
|  - User Actions: [Enter] Execute | [Esc] Cancel | [Tab/E] Edit in PTY             |
+-----------------------------------------------------------------------------------+
                                         |
                                         v
+-----------------------------------------------------------------------------------+
|                                PTY EXECUTION                                      |
|  - Write payload into PTY via EventLoopProxy -> TerminalEvent::PtyWrite           |
+-----------------------------------------------------------------------------------+
```

---

## Detailed Component Specifications

### 1. User Interface & Trigger Mechanisms
- **AI Command Overlay Bar**:
  - Activated via hotkey (default: `Ctrl+Shift+I` or `Ctrl+Shift+Space`).
  - Renders a lightweight, non-modal input bar at the top or bottom of the active window using Alacritty's existing text rendering engine.
  - Accepts natural language text input (e.g. *"find all rust files containing EventLoop"*).
- **Voice-to-Intent Pipeline**:
  - Leverages local audio capture (`cpal`) and local Speech-to-Text (`whisper-rs`).
  - Text output is fed directly into the LLM prompt builder.
- **Terminal Selection Context ("Explain / Fix")**:
  - Selecting an error message in the buffer and pressing `Ctrl+Shift+E` triggers LLM analysis explaining the error and suggesting a fix.

### 2. Context Extraction Engine (`alacritty/src/ai/context.rs`)
To produce valid commands like `cd folder`, the system extracts real-time environmental context:
- **Current Working Directory (CWD)**:
  - Retrieved via process table inspection (`/proc/<pid>/cwd` on Linux, `sysctl`/`libproc` on macOS) associated with the PTY child process.
- **Environment Metadata**:
  - Shell executable name (`bash`, `zsh`, `fish`).
  - Operating System & OS Architecture.
- **Screen Buffer Context**:
  - Extracts the last 20–50 lines of the terminal grid for error diagnosis or relevant context.

### 3. LLM Provider Layer (`alacritty/src/ai/provider.rs`)
Modular provider design supporting multiple backends:
```rust
pub trait LlmProvider: Send + Sync {
    fn generate_command(&self, prompt: &str, context: &TerminalContext) -> Result<LlmResponse, AiError>;
}
```

- **Supported Backends**:
  1. **Ollama / OpenAI API Compatible REST Server** (Default):
     - Connects to `http://localhost:11434` or custom endpoints.
     - Fast, out-of-process, zero binary overhead.
  2. **In-Process Embedded LLM Engine (`candle` / `llama-cpp-rs`)**:
     - Runs small quantized models (e.g. `Qwen2.5-Coder-1.5B-Instruct-GGUF`) directly inside Alacritty without needing separate background services.
  3. **Cloud Providers**: Support for API keys via environment variables for OpenAI, Anthropic, or Gemini.

- **System Prompt Design**:
  - Instructs model to output JSON:
    ```json
    {
      "command": "cd courage",
      "explanation": "Navigates to the courage directory",
      "is_destructive": false
    }
    ```

### 4. Safety & Command Confirmation Layer (`alacritty/src/ai/ui.rs`)
- **Interactive Preview Bar**:
  - Before writing to PTY, the proposed command is rendered in an interactive preview bar.
  - Controls:
    - `Enter`: Inject `command + \n` into PTY.
    - `Tab`: Inject `command` into PTY line buffer without trailing newline (allows user to edit before hitting Enter).
    - `Esc`: Dismiss preview.
- **Safety Heuristics**:
  - Flags destructive commands (`rm -rf`, `dd`, `mkfs`, `git reset --hard`) with a red warning badge.

### 5. Configuration & Keybindings (`alacritty_config`)
Add a new `[ai]` section to `alacritty.toml`:

```toml
[ai]
enabled = true
provider = "ollama" # "ollama" | "llama_cpp" | "openai"
api_url = "http://localhost:11434/api/generate"
model = "qwen2.5-coder:3b"
auto_execute = false # Require confirmation before running
whisper_model_path = "~/.config/alacritty/models/ggml-tiny.bin"

[keyboard]
bindings = [
  { key = "Space", mods = "Control|Shift", action = "ToggleAiPrompt" },
  { key = "S", mods = "Control|Shift", action = "ToggleAiVoice" },
  { key = "E", mods = "Control|Shift", action = "ExplainSelection" }
]
```

---

## Implementation Roadmap

### Phase 1: Core Modular Abstraction & Configuration
- [ ] Define `AiConfig` structures in `alacritty_config`.
- [ ] Create `alacritty/src/ai/` module containing `provider.rs`, `context.rs`, and `prompts.rs`.
- [ ] Add `LlmProvider` trait with initial `OllamaProvider` REST implementation.

### Phase 2: Natural Language Command Bar & Context Engine
- [ ] Implement CWD extraction for active PTY sessions.
- [ ] Build natural language text prompt overlay inside `alacritty/src/display/` and `message_bar.rs`.
- [ ] Bind key combo (`Ctrl+Shift+Space`) to launch prompt overlay.

### Phase 3: Safety Guardrails & PTY Injection
- [ ] Implement command preview overlay with `Enter` (run), `Tab` (edit), `Esc` (cancel).
- [ ] Hook PTY injection logic via `EventLoopProxy` (`TerminalEvent::PtyWrite`).
- [ ] Add safety check parser for high-risk shell commands.

### Phase 4: Voice Input & Selection Diagnosis Refinement
- [ ] Refine `whisper-rs` audio stream integration in `alacritty/src/voice.rs`.
- [ ] Connect selection capture (`Ctrl+Shift+E`) to terminal scrollback for AI error explanation.

---

## Verification & Testing Strategy

1. **Unit Testing**: Test context extraction functions and prompt text generation algorithms.
2. **Integration Testing**: Test Ollama API calls with mock server responses.
3. **Interactive Manual Tests**:
   - Trigger `Ctrl+Shift+Space`, type *"go to courage folder"*, verify generated `cd courage` command preview, press `Enter`, and verify shell path updates.
   - Speak *"list all files"*, verify speech-to-text pipeline generates `ls -la`.

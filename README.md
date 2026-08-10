<p align="center">
    <img width="180" alt="Kourage Logo" src="https://raw.githubusercontent.com/alacritty/alacritty/master/extra/logo/compat/alacritty-term%2Bscanlines.png">
</p>

<h1 align="center">Kourage — The All-Knowing AI Terminal Emulator</h1>

<p align="center">
  <strong>Blazing fast GPU-accelerated terminal with 100% local Voice-to-Command & on-device LLM intelligence. Inspired by the Attic Computer from <em>Courage the Cowardly Dog</em>.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Language-Rust-orange.svg" alt="Rust">
  <img src="https://img.shields.io/badge/Inference-100%25%20Local-success.svg" alt="Local AI">
  <img src="https://img.shields.io/badge/STT-Whisper.cpp%20(Offline)-blue.svg" alt="Whisper STT">
  <img src="https://img.shields.io/badge/LLM-Ollama%20%2F%20llama.cpp-purple.svg" alt="Ollama">
  <img src="https://img.shields.io/badge/GPU%20Renderer-OpenGL-green.svg" alt="OpenGL">
  <img src="https://img.shields.io/badge/License-Apache%202.0-blue.svg" alt="License">
</p>

---

## ⚡ What is Kourage?

**Kourage** is a fork of the ultra-fast, OpenGL-accelerated [Alacritty](https://github.com/alacritty/alacritty) terminal that adds **embedded voice commands and local natural language understanding**.

Just like the legendary Attic Computer from *Courage the Cowardly Dog* that always had the answer to every impossible crisis, **Kourage** lets you type in plain English or simply speak your intent, translating it into executable shell commands directly in your active terminal session.

> [!IMPORTANT]
> **100% Privacy & Zero Cloud Dependency**: Speech recognition runs entirely on your CPU/GPU using Whisper GGML models, and language generation connects to local LLM daemons (e.g. Ollama). No voice audio or command data ever leaves your computer.

---

## 🌟 Key Features

- 💬 **Natural Language AI Prompt Bar**: Press `Ctrl+Shift+Space` to open an inline AI prompt bar (`🤖 AI: `), type your intent in plain English, and hit `Enter`.
- 🎙️ **Hands-Free Voice-to-Command**: Press `Ctrl+Shift+S` and speak naturally (e.g., *"show all hidden files sorted by modification time"*).
- 🧠 **Local LLM Intent Parsing**: Automatically turns natural language into valid shell commands (`ls -lat`) using local models like `qwen2.5-coder` or `llama3`.
- 🔇 **Smart RMS Silence Detection**: Automatically detects when you stop speaking and begins transcription with zero manual cutoffs.
- ⚡ **Pure High-Performance Rust**: Non-blocking asynchronous worker threads ensure that voice and LLM processing never freeze Alacritty's 60+ FPS GPU render loop.
- 🔒 **Safe & Offline**: No API keys, no subscriptions, and no internet connection required.

---

## 🔄 How It Works

```
+--------------------------+       +-----------------------+       +----------------------+
| 💬 Text Prompt Bar        |       | 🔊 Whisper STT Engine  |       | 🧠  Local LLM Server  |
| (Ctrl+Shift+Space)       | ----+ | (whisper-rs / GGML)   | ----> | (Ollama / REST API)  |
+--------------------------+     | +-----------------------+       +----------------------+
                                 |             ^                               |
+--------------------------+     |             |                               v
| 🎙️  Microphone            | ----+-------------+                   +----------------------+
| (Ctrl+Shift+S / cpal)    |                                       | 💻  Alacritty PTY    |
+--------------------------+                                       | (Automatic Execution)|
                                                                   +----------------------+
```

1. **Text Mode**: Press `Ctrl+Shift+Space`, type your prompt (e.g. *"find all files bigger than 50MB"*), and hit `Enter`.
2. **Voice Mode**: Press `Ctrl+Shift+S`, speak your command naturally, and Whisper transcribes it.
3. **Inference**: The prompt is processed locally by your LLM (e.g. Ollama).
4. **Execution**: The resulting shell command is injected straight into the terminal PTY.

---

## 🚀 Quick Start Guide

### 1. Prerequisites

Make sure the native system audio and build tools are installed:

```bash
# Ubuntu / Debian
sudo apt install -y libasound2-dev libopenblas-dev libclang-dev cmake pkg-config fontconfig
```

### 2. Download a Local Whisper Model (Optional for Voice)

Download a compact GGML model (e.g. `tiny` or `base`):

```bash
mkdir -p ~/.config/alacritty/models
wget -O ~/.config/alacritty/models/ggml-tiny.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin
```

### 3. Start your Local LLM (Ollama)

Make sure [Ollama](https://ollama.com) is installed and running:

```bash
ollama run qwen2.5-coder:3b # or: ollama run llama3
```

### 4. Configure Alacritty

Add the `[voice]` section to your `alacritty.toml` (located at `~/.config/alacritty/alacritty.toml`):

```toml
[voice]
whisper_model_path = "~/.config/alacritty/models/ggml-tiny.bin"
llm_api_url = "http://localhost:11434/api/generate"
model = "qwen2.5-coder:3b" # or "llama3"
temperature = 0.0
timeout_secs = 15
```

### 5. Build and Run

```bash
# Clone the repository
git clone https://github.com/Sunidhi-Satika/alacritty.git
cd alacritty

# Build & launch
cargo run --release --bin alacritty
```

- Press **`Ctrl+Shift+Space`** to open the AI prompt bar and type any command!
- Press **`Ctrl+Shift+S`** to speak your command!

---

## ⌨️ Default Keybindings

| Keybinding | Action | Description |
| :--- | :--- | :--- |
| `Ctrl + Shift + Space` | `ToggleAiPrompt` | Opens/closes inline natural language AI prompt bar (`🤖 AI: `) |
| `Ctrl + Shift + S` | `VoiceCommand` | Starts microphone recording and executes transcribed AI command |
| `Enter` (in prompt) | `AiPromptConfirm` | Submits prompt to LLM for shell command generation |
| `Esc` (in prompt) | `AiPromptCancel` | Cancels and closes prompt overlay |
| `Up` / `Down` (in prompt) | History Nav | Cycles through previous AI prompt history |
| `Ctrl + W` (in prompt) | `AiPromptDeleteWord` | Deletes previous word in prompt |
| `Ctrl + Shift + +` | `IncreaseFontSize` | Increase terminal font size |
| `Ctrl + Shift + -` | `DecreaseFontSize` | Decrease terminal font size |

---

## 🗺️ Roadmap & In-Development Features

- [x] **Context Awareness Engine**: Automatically extracts active CWD (`/proc/<pid>/cwd`), directory files, tech stack detection (Rust/Cargo, Node/npm, Python, Go, Docker, Git), active shell, OS distro, and recent terminal buffer output to provide hyper-accurate command generation.
- [x] **Natural Language Text Prompt Bar (`Ctrl+Shift+Space`)**: Inline prompt overlay with typing, cursor, history (`Up`/`Down`), word deletion (`Ctrl+W`), and execution.
- [x] **Local Voice Capture & STT (`Ctrl+Shift+S`)**: Hands-free voice capture with silence auto-cutoff (`cpal` + `whisper-rs`).
- [x] **Local LLM Engine**: REST API client querying local LLM daemon (Ollama / Qwen / Llama).
- [x] **Visual Feedback**: Real-time stage updates (*"🎙️ Recording..."*, *"⚙️ Transcribing..."*, *"🤖 Generating LLM..."*) and error notifications in the message bar.
- [x] **Configurable LLM Settings**: Configurable model, temperature, timeout, and tilde `~` path resolution.
- [ ] **Interactive Preview & Safety Guard**: Preview command with `Enter` (run), `Tab` (edit), and `Esc` (cancel) with warnings for destructive commands (`rm -rf`).
- [ ] **Error Diagnosis & Fix (`Ctrl+Shift+E`)**: Analyze recent terminal errors and suggest automated fixes.

---

## 📄 License

Kourage is released under the [Apache License, Version 2.0](LICENSE-APACHE).

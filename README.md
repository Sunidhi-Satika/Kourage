<p align="center">
    <img width="180" alt="Alacritty AI Logo" src="https://raw.githubusercontent.com/alacritty/alacritty/master/extra/logo/compat/alacritty-term%2Bscanlines.png">
</p>

<h1 align="center">Alacritty AI — Intelligent, Voice-Enabled Terminal Emulator</h1>

<p align="center">
  <strong>Blazing fast GPU-accelerated terminal with 100% local Voice-to-Command & on-device LLM intelligence.</strong>
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

## ⚡ What is Alacritty AI?

**Alacritty AI** is a fork of the ultra-fast, OpenGL-accelerated [Alacritty](https://github.com/alacritty/alacritty) terminal that adds **embedded voice commands and local natural language understanding**.

Instead of typing long, complex shell commands or remembering obscure flags, you can simply **speak or express your intent in plain English**, and the built-in AI pipeline translates it into executable shell commands directly in your active terminal session.

> [!IMPORTANT]
> **100% Privacy & Zero Cloud Dependency**: Speech recognition runs entirely on your CPU/GPU using Whisper GGML models, and language generation connects to local LLM daemons (e.g. Ollama). No voice audio or command data ever leaves your computer.

---

## 🌟 Key Features

- 🎙️ **Hands-Free Voice-to-Command**: Press `Ctrl+Shift+S` and speak naturally (e.g., *"show all hidden files sorted by modification time"*).
- 🧠 **Local LLM Intent Parsing**: Automatically turns natural language into valid shell commands (`ls -lat`) using local models like `qwen2.5-coder` or `llama3`.
- 🔇 **Smart RMS Silence Detection**: Automatically detects when you stop speaking and begins transcription with zero manual cutoffs.
- ⚡ **Pure High-Performance Rust**: Non-blocking asynchronous worker threads ensure that voice and LLM processing never freeze Alacritty's 60+ FPS GPU render loop.
- 🔒 **Safe & Offline**: No API keys, no subscriptions, and no internet connection required.

---

## 🔄 How It Works

```
+-------------------+       +-----------------------+       +----------------------+
| 🎙️  Microphone     | ----> | 🔊 Whisper STT Engine  | ----> | 🧠  Local LLM Server  |
| (cpal capture)    |       | (whisper-rs / GGML)   |       | (Ollama / REST API)  |
+-------------------+       +-----------------------+       +----------------------+
                                                                        |
                                                                        v
                                                            +----------------------+
                                                            | 💻  Alacritty PTY    |
                                                            | (Automatic Execution)|
                                                            +----------------------+
```

1. **Trigger**: You press `Ctrl+Shift+S`.
2. **Record**: `cpal` captures audio from your microphone with RMS silence detection.
3. **Transcribe**: `whisper-rs` transcribes speech into text using local GGML Whisper weights.
4. **Translate**: The transcribed text is sent to your local LLM daemon (e.g. `http://localhost:11434`).
5. **Execute**: The generated shell command is written straight into the terminal PTY.

---

## 🚀 Quick Start Guide

### 1. Prerequisites

Make sure the native system audio and build tools are installed:

```bash
# Ubuntu / Debian
sudo apt install -y libasound2-dev libopenblas-dev libclang-dev cmake pkg-config fontconfig
```

### 2. Download a Local Whisper Model

Download a compact GGML model (e.g. `tiny` or `base`):

```bash
mkdir -p ~/.config/alacritty/models
wget -O ~/.config/alacritty/models/ggml-tiny.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin
```

### 3. Start your Local LLM (Ollama)

Make sure [Ollama](https://ollama.com) is installed and running:

```bash
ollama run llama3 # or: ollama run qwen2.5-coder:3b
```

### 4. Configure Alacritty

Add the `[voice]` section to your `alacritty.toml` (located at `~/.config/alacritty/alacritty.toml`):

```toml
[voice]
whisper_model_path = "~/.config/alacritty/models/ggml-tiny.bin"
llm_api_url = "http://localhost:11434/api/generate"
```

### 5. Build and Run

```bash
# Clone the repository
git clone https://github.com/Sunidhi-Satika/alacritty.git
cd alacritty

# Build & launch
cargo run --release --bin alacritty
```

Press **`Ctrl+Shift+S`**, speak your command, and watch the terminal execute it!

---

## ⌨️ Default Keybindings

| Keybinding | Action | Description |
| :--- | :--- | :--- |
| `Ctrl + Shift + S` | `VoiceCommand` | Starts microphone recording and executes transcribed AI command |
| `Ctrl + Shift + +` | `IncreaseFontSize` | Increase terminal font size |
| `Ctrl + Shift + -` | `DecreaseFontSize` | Decrease terminal font size |
| `Ctrl + Shift + Space` | *Prompt Overlay (Coming Soon)* | Opens inline natural language text prompt |

---

## 🗺️ Roadmap & In-Development Features

- [x] Local voice capture with silence auto-cutoff (`cpal`)
- [x] Local Speech-to-Text inference (`whisper-rs`)
- [x] Local LLM REST API client (`reqwest`)
- [x] PTY command event injection
- [ ] **Visual Feedback**: On-screen status indicators (*"Listening..."*, *"Transcribing..."*, *"Generating..."*) in the message bar.
- [ ] **Context Awareness**: Inject active CWD (`/proc/<pid>/cwd`), active shell, and OS info into LLM prompts.
- [ ] **Interactive Preview & Safety Guard**: Preview command with `Enter` (run), `Tab` (edit), and `Esc` (cancel) with warnings for destructive commands (`rm -rf`).
- [ ] **Error Diagnosis (`Ctrl+Shift+E`)**: Analyze recent terminal errors and automatically suggest fixes.

---

## 📄 License

Alacritty AI is released under the [Apache License, Version 2.0](LICENSE-APACHE).

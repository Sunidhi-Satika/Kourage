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

- 💬 **Natural Language AI Prompt Bar**: Press `Ctrl+Shift+A` to open an inline AI prompt bar (`AI: `), type your intent in plain English, and hit `Enter`.
- 🎙️ **Hands-Free Voice-to-Command**: Press `Ctrl+Shift+S` and speak naturally (e.g., *"show all hidden files sorted by modification time"*).
- 🧠 **Local LLM Intent Parsing**: Automatically turns natural language into valid shell commands (`ls -lat`) using local models like `qwen2.5-coder` or `llama3`.
- 🔇 **Smart RMS Silence Detection**: Automatically detects when you stop speaking and begins transcription with zero manual cutoffs.
- ⚡ **Pure High-Performance Rust**: Non-blocking asynchronous worker threads ensure that voice and LLM processing never freeze Alacritty's 60+ FPS GPU render loop.
- 👀 **Always-Visible Status**: One line on the last terminal row shows *listening* (with a live mic meter), *transcribing*, *sent to the model, waiting* and any error, each with a seconds counter.
- 🛡️ **Preview & Safety Guard**: Nothing runs until you confirm. Risky commands (`rm -rf`, `sudo`, `curl | sh`, `git reset --hard`, anything that ends your shell...) are flagged as `DANGER`.
- 🔒 **Safe & Offline**: No API keys, no subscriptions, and no internet connection required.

---

## 🔄 How It Works

```
+--------------------------+       +-----------------------+       +----------------------+
| 💬 Text Prompt Bar        |       | 🔊 Whisper STT Engine  |       | 🧠  Local LLM Server  |
| (Ctrl+Shift+A)           | ----+ | (whisper-rs / GGML)   | ----> | (Ollama / REST API)  |
+--------------------------+     | +-----------------------+       +----------------------+
                                 |             ^                               |
+--------------------------+     |             |                               v
| 🎙️  Microphone            | ----+-------------+                   +----------------------+
| (Ctrl+Shift+S / cpal)    |                                       | 💻  Alacritty PTY    |
+--------------------------+                                       | (Automatic Execution)|
                                                                   +----------------------+
```

1. **Text Mode**: Press `Ctrl+Shift+A`, type your prompt (e.g. *"find all files bigger than 50MB"*), and hit `Enter`.
2. **Voice Mode**: Press `Ctrl+Shift+S`, speak your command naturally, and Whisper transcribes it.
3. **Inference**: The prompt is processed locally by your LLM (e.g. Ollama).
4. **Preview & confirm**: The command appears in a preview line. `Enter` runs it, `Tab` pastes it into your prompt for editing, `Esc` cancels. Nothing runs without your confirmation.

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

Add the `[ai]` section to your `alacritty.toml` (located at `~/.config/alacritty/alacritty.toml`; the old name `[voice]` still works):

```toml
[ai]
whisper_model_path = "~/.config/alacritty/models/ggml-tiny.bin"
llm_api_url = "http://localhost:11434/api/generate"
model = "qwen2.5-coder:3b" # or "llama3"
temperature = 0.0
timeout_secs = 15
```

### 5. Build and Run

```bash
# Clone the repository
git clone https://github.com/Sunidhi-Satika/Kourage.git
cd Kourage

# Build & launch
cargo run --release --bin alacritty
```

- Press **`Ctrl+Shift+A`** to open the AI prompt bar and type any command!
- Press **`Ctrl+Shift+S`** to speak your command!

---

## ⌨️ Default Keybindings

| Keybinding | Action | Description |
| :--- | :--- | :--- |
| `Ctrl + Shift + A` | `ToggleAiPrompt` | Opens/closes inline natural language AI prompt bar (`🤖 AI: `) |
| `Ctrl + Shift + S` | `VoiceCommand` | Starts microphone recording and executes transcribed AI command |
| `Esc` (while recording) | Cancel voice | Stops the microphone and discards the recording |
| `Ctrl + Shift + V` (in prompt) | Paste | Pastes into the prompt; line breaks become spaces |
| `Enter` (in prompt) | `AiPromptConfirm` | Submits prompt to LLM for shell command generation |
| `Esc` (in prompt) | `AiPromptCancel` | Cancels and closes prompt overlay |
| `Up` / `Down` (in prompt) | History Nav | Cycles through previous AI prompt history |
| `Ctrl + W` (in prompt) | `AiPromptDeleteWord` | Deletes previous word in prompt |
| `Enter` (in preview) | `AiPreviewConfirm` | **Execute** previewed command immediately into shell |
| `Tab` (in preview) | `AiPreviewEdit` | **Paste into terminal prompt** for manual editing before running |
| `Esc` (in preview) | `AiPreviewCancel` | **Dismiss** preview without running anything |
| `Ctrl + Shift + +` | `IncreaseFontSize` | Increase terminal font size |
| `Ctrl + Shift + -` | `DecreaseFontSize` | Decrease terminal font size |

---

## 👀 What the status line shows

While the AI works, one line on the last terminal row always says what is happening, with a live seconds counter:

```
LISTENING [####------] 4s  speak now - Esc: cancel
TRANSCRIBING speech... 2s
SENT to qwen2.5-coder:3b - waiting for reply... 5s
PREVIEW: df -h [Enter: Run | Tab: Edit | Esc: Cancel]
ERROR: Can't reach the AI server ...  (press any key)
```

The `#` meter moves with your voice; if it stays empty while you speak, the microphone is not hearing you. Errors and notices stay until you press a key (the key still goes through).

---

## 🩺 Troubleshooting

Run `./target/debug/alacritty -v` to see log output. Kourage's own messages tell you what to fix; the usual ones:

| Message | Cause and fix |
| :--- | :--- |
| `Can't reach the AI server at …` | Ollama isn't running. Start it with `ollama serve` (install it inside WSL if you use WSL, not on Windows). Check with `curl localhost:11434/api/tags`. |
| `The model "…" isn't installed` | Run `ollama pull qwen2.5-coder:3b` (or the model named in `[ai] model`). |
| `The AI server … did not answer within …s` | The model is still loading, or the machine is slow. Try again, or raise `[ai] timeout_secs`. |
| `The model returned N lines but a single command is required` | The model ignored the one-line rule. Rephrase the request. |
| `Whisper model not found at …` | Download a model such as `ggml-base.en.bin` and set `[ai] whisper_model_path`. |
| `No audio input device found` / ALSA errors | No microphone is visible. On WSL2, install `alsa-utils pulseaudio-utils libasound2-plugins` and check `pactl list short sources`. |
| Voice transcribes "Bye." or nothing | The recording probably stopped before you spoke. Run with `-vv`, then listen to `/tmp/kourage-voice-<pid>-<time>.wav` and report what you hear. |

Commands that look destructive (recursive `rm`, `sudo`, `curl | sh`, `git reset --hard`, anything that ends your shell, and more) are flagged in the preview. Read the preview before pressing Enter; the guard is a safety net, not a guarantee.

---

## 📄 License

Kourage is released under the [Apache License, Version 2.0](LICENSE-APACHE).

# Stage 0 to-do list: how to solve each problem

Order: A (safety and output), B (first run), C (voice and input), D (quality). Every fix is test-first on branch `stage0-next-up`.

## A. Safety and output

**1. Safety guard is a keyword list**
- Replace substring matching with a tokenizer: split on `;`, `&&`, `||`, `|`, newline, `$(...)` and backticks. Take the command word of each segment, skipping wrappers (`sudo`, `env`, `command`, `xargs`, `nohup`, `time`).
- Classify per command word plus its flags, parsed as flags rather than substrings. `rm` is dangerous with `-r`/`-R`/`--recursive`, a broad target (`*`, `/`, `~`, `dir/*`), or a system path or home dotfile. Plain `rm -f file` is deliberately safe. This fixes the `rm my-report.txt` false positive and the dead `-R` checks, since matching stops lowercasing the whole line.
- Substitutions (`$(...)`, backticks, `<(...)`) are analysed as their own commands and leave a placeholder in the surrounding command; an unknown command name (`$(which rm) -rf ~`) is flagged. A shell, `eval` or `source` fed by a download (`sh -c "$(curl …)"`, `bash <(curl …)`) is flagged.
- Add rules: `sudo` anything, `curl|wget … | sh`, `find … -delete` and `-exec rm`, `>` truncation of home dotfiles and system paths (truncating arbitrary user files is not flagged, since the guard can't see the filesystem), `chmod/chown -R` on `/` or `~`, `kill -9 -1`, `mv`/`cp` onto system paths.
- Three levels (safe, warn, blocked) instead of a boolean.
- A table-driven test file of about 100 dangerous and safe commands is the spec. Every future bug adds a row.
- Stage 2 later swaps the tokenizer for a real shell AST (`tree-sitter-bash`).

**2. Multi-line output runs several commands on one confirm**
- Collapse or reject: if the cleaned reply has more than one non-empty line, show all lines in the preview, mark it multi-command, and require an explicit confirm. Never write embedded newlines to the PTY silently.
- Also ask Ollama for a constrained reply (`"format"` JSON schema `{command: string}`), which removes fence and backtick problems at the source. Keep `clean_llm_reply` as a fallback.

**4. No length cap, no cancel**
- Send `num_predict: 128`, `repeat_penalty: 1.1` and a `stop` list (`"\n\n"`) in the request options.
- Cancel: run the request on a thread with a cancel flag, switch to streaming (`stream: true`) and drop the connection on Esc. Show a "thinking…" indicator while waiting.

## B. First run

**5. Ollama or model missing, opaque errors**
- A health check before the first request: `GET /api/tags`. Connection refused means "Ollama isn't running (`ollama serve`)". A missing model means "run `ollama pull <model>`". Map HTTP and parse errors to these messages.
- Same for Whisper: if `whisper_model_path` is the placeholder or missing, say exactly where to download it. Run the checks once at startup and show one clear message.

**8. README**
- Add a troubleshooting section (no mic, Ollama down, model not pulled, Whisper path) and a keybinding table for prompt and preview (open, confirm, edit, cancel).

## C. Voice and input

**6. Voice fragility**
- `i16`/`u16` devices: match on `config.sample_format()` and convert to `f32` in each callback (`cpal::Sample`).
- Cancel and push-to-talk: Esc cancels. Optional hold-to-talk, which needs key release events (the Kitty keyboard protocol path). Fall back to toggle.
- Model reload: load `WhisperContext` once, lazily, and keep it in a `OnceLock`/`Mutex`.
- "Bye." hallucination: listen to the `-vv` dump first. Likely fixes are a higher `SPEECH_RMS_THRESHOLD`, a minimum speech duration before accepting, and discarding known hallucinations ("Bye.", "Thanks for watching") on near-silent input. Whisper's `no_speech_thold` can help too.
- macOS binding: add a voice binding to the macOS keybindings.

**7. Pasting into the prompt**
- While the overlay is open, route `Paste` to the prompt's input buffer instead of blocking it. Strip newlines (or reject them) so a paste can't submit.

## D. Quality

**9. Clippy errors**: fix all 10 and add `cargo clippy --all-targets -- -D warnings` to CI.

**10. Thin tests**: extract the prompt and preview state machines into pure types, and test transitions (open, type, submit, preview, confirm, edit, cancel). Add a `Backend` trait with a mock for end-to-end tests. This is a small first step toward Stage 1's `Backend`.

**11. Config naming**: add `[ai]` as the canonical section. Keep `[voice]` as a deprecated alias that still parses and logs a one-time warning.

**3. Wrong answers from a 3B model**
- Few-shot examples in the prompt: 10 to 15 pairs of request and command, chosen for the common failure cases (hidden files by mtime, disk usage, find by name and size).
- Retrieval of examples by similarity is possible later. Consider a larger default model (7B) for machines that can run it.
- Fine-tuning is the longer-term answer (see below).

## Not now
12 (architecture) is Stage 1. 13 (branding) and 14 (release path) need owner decisions first.

---

## Can we use fine-tuned models, and can we fine-tune them?

**Using one: yes, easily.** Ollama runs any GGUF model through a `Modelfile`, and it can apply a LoRA adapter on top of a base model. Kourage only needs the model name in config, so a fine-tuned model is a config change.

**Fine-tuning one: yes, and it suits this task.** NL to shell command is narrow, so a small model with LoRA/QLoRA improves a lot.
- **Data:** public NL-to-bash pairs (NL2Bash, tldr pages) plus our own logs. The preview gives a free labelling signal: commands the user ran as proposed, edited or rejected. Opt-in, local only, in line with the roadmap's local-first rule.
- **Tooling:** Unsloth or axolotl for QLoRA, then convert to GGUF with llama.cpp and load into Ollama.
- **Hardware:** a 3B model with QLoRA fits on about 8 GB of VRAM, or a free Colab GPU. WSL here has limited GPU access, so train elsewhere and bring the GGUF back.
- **Caveats:**
  - Check the base model's licence before redistributing. I believe the Qwen2.5-Coder 3B model carries a non-commercial research licence, while the larger sizes are Apache 2.0. Please verify before we bundle anything.
  - A fine-tune can drift toward dangerous commands too, so the safety guard stays in front of it.
  - Build an eval set first (about 100 requests with known-good commands). Otherwise we can't tell whether tuning helped.

**My recommendation:** don't fine-tune yet. Do few-shot prompting, constrained output and the eval set first. The eval set is what makes any later fine-tune measurable. Start collecting opt-in logs in Stage 2, and fine-tune once we have a few thousand real examples. Whisper doesn't need fine-tuning for this: a larger model or better silence handling is enough.

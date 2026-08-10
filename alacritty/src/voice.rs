use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use log::{error, info};
use serde::{Deserialize, Serialize};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::WavWriter;
use tempfile::NamedTempFile;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::config::ui_config::UiConfig;
use crate::config::voice::Voice;
use crate::event::{Event, EventType};
use crate::message_bar::{Message, MessageType};

pub const LOG_TARGET_VOICE: &str = "alacritty_voice";

pub fn init() {
    // Initialization logic if needed
}

fn send_status(
    proxy: &EventLoopProxy<Event>,
    window_id: WindowId,
    text: impl Into<String>,
    ty: MessageType,
) {
    let mut message = Message::new(text.into(), ty);
    message.set_target(LOG_TARGET_VOICE.to_string());
    let _ = proxy.send_event(Event::new(EventType::Message(message), window_id));
}

fn clear_status(proxy: &EventLoopProxy<Event>, window_id: WindowId) {
    let _ = proxy.send_event(Event::new(
        EventType::ClearMessageTarget(LOG_TARGET_VOICE.to_string()),
        window_id,
    ));
}

fn resolve_path(path_str: &str) -> PathBuf {
    if let Some(stripped) = path_str.strip_prefix("~/") {
        if let Some(home) = home::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path_str)
}

fn record_audio(path: &Path) -> Result<(), anyhow::Error> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No audio input device found"))?;
    let config = device.default_input_config()?;

    let spec = hound::WavSpec {
        channels: config.channels() as _,
        sample_rate: config.sample_rate().0,
        bits_per_sample: (config.sample_format().sample_size() * 8) as _,
        sample_format: hound::SampleFormat::Float,
    };

    let writer = Arc::new(Mutex::new(Some(WavWriter::create(path, spec)?)));
    let writer_clone = writer.clone();

    let should_stop = Arc::new(AtomicBool::new(false));
    let should_stop_clone = should_stop.clone();
    let silence_count = Arc::new(Mutex::new(0));
    let silence_count_clone = silence_count.clone();

    // Parameters for silence detection
    let threshold = 0.005; // RMS threshold
    let silence_limit = 30; // Number of consecutive silent callbacks (roughly 1-2 seconds)

    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            let mut sum_sq = 0.0;
            if let Some(writer) = writer_clone.lock().unwrap().as_mut() {
                for &sample in data.iter() {
                    writer
                        .write_sample(sample)
                        .unwrap_or_else(|e| error!("Failed to write sample: {}", e));
                    sum_sq += sample * sample;
                }
            }

            let rms = (sum_sq / data.len() as f32).sqrt();
            if rms < threshold {
                let mut count = silence_count_clone.lock().unwrap();
                *count += 1;
                if *count > silence_limit {
                    should_stop_clone.store(true, Ordering::SeqCst);
                }
            } else {
                let mut count = silence_count_clone.lock().unwrap();
                *count = 0;
            }
        },
        |err| {
            error!("An error occurred on the input stream: {}", err);
        },
        None,
    )?;

    stream.play()?;
    info!("Recording audio... Speak now!");

    let start_time = std::time::Instant::now();
    let max_duration = Duration::from_secs(10);

    while !should_stop.load(Ordering::SeqCst) && start_time.elapsed() < max_duration {
        thread::sleep(Duration::from_millis(100));
    }

    stream.pause()?;
    *writer.lock().unwrap() = None;
    info!("Recording stopped.");

    Ok(())
}

fn transcribe_audio(path: &Path, model_path_str: &str) -> Result<String, anyhow::Error> {
    let resolved_model_path = resolve_path(model_path_str);
    if !resolved_model_path.exists() {
        return Err(anyhow::anyhow!(
            "Whisper model not found at path: \"{}\" (check [voice.whisper_model_path] in alacritty.toml)",
            resolved_model_path.display()
        ));
    }

    let model_path_os = resolved_model_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid model path encoding"))?;

    let ctx = WhisperContext::new_with_params(model_path_os, WhisperContextParameters::default())
        .map_err(|e| anyhow::anyhow!("Failed to create Whisper context: {:?}", e))?;
    let mut state = ctx
        .create_state()
        .map_err(|e| anyhow::anyhow!("Failed to create Whisper state: {:?}", e))?;

    let file = File::open(path)?;
    let mut reader = hound::WavReader::new(std::io::BufReader::new(file))?;
    let samples: Vec<i16> = reader.samples().collect::<Result<_, _>>()?;
    let mut audio_samples: Vec<f32> = samples
        .into_iter()
        .map(|s| s as f32 / 32768.0)
        .collect();

    if reader.spec().channels == 2 {
        audio_samples = whisper_rs::convert_stereo_to_mono_audio(&audio_samples)?;
    }

    let params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    state
        .full(params, &audio_samples[..])
        .map_err(|e| anyhow::anyhow!("Failed to run Whisper model: {:?}", e))?;

    let num_segments = state.full_n_segments();
    let mut result = String::new();
    for i in 0..num_segments {
        if let Some(segment) = state.get_segment(i) {
            result.push_str(&segment.to_string());
        }
    }

    Ok(result)
}

#[derive(Serialize)]
struct LlmOptions {
    temperature: f32,
}

#[derive(Serialize)]
struct LlmRequest {
    model: String,
    prompt: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<LlmOptions>,
}

#[derive(Deserialize)]
struct LlmResponse {
    response: String,
}

use crate::ai::AiContext;

fn call_llm(
    instruction: &str,
    context: &AiContext,
    voice_config: &Voice,
) -> Result<String, anyhow::Error> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(voice_config.timeout_secs))
        .build()?;

    let prompt = context.format_prompt(instruction);

    let request = LlmRequest {
        model: voice_config.model.clone(),
        prompt,
        stream: false,
        options: Some(LlmOptions {
            temperature: voice_config.temperature,
        }),
    };

    let res = client
        .post(&voice_config.llm_api_url)
        .json(&request)
        .send()
        .map_err(|e| anyhow::anyhow!("HTTP request to \"{}\" failed: {}", voice_config.llm_api_url, e))?
        .json::<LlmResponse>()
        .map_err(|e| anyhow::anyhow!("Failed to parse response from {}: {}", voice_config.llm_api_url, e))?;

    let mut cmd = res.response.trim().to_string();
    if cmd.starts_with("```") {
        if let Some(first_newline) = cmd.find('\n') {
            cmd = cmd[first_newline + 1..].to_string();
        }
        if let Some(last_ticks) = cmd.rfind("```") {
            cmd = cmd[..last_ticks].to_string();
        }
        cmd = cmd.trim().to_string();
    }

    if cmd.is_empty() {
        return Err(anyhow::anyhow!("LLM returned an empty command"));
    }

    Ok(cmd)
}

pub fn handle_voice_command(
    context: AiContext,
    config: &UiConfig,
    proxy: &EventLoopProxy<Event>,
    window_id: WindowId,
) {
    let voice_config = config.voice.clone();
    let proxy = proxy.clone();

    thread::spawn(move || {
        // Stage 1: Audio recording
        send_status(
            &proxy,
            window_id,
            "🎙️ Recording audio... Speak your command",
            MessageType::Warning,
        );

        let temp_file = match NamedTempFile::new() {
            Ok(file) => file,
            Err(e) => {
                let err_msg = format!("Failed to create temporary audio file: {e}");
                error!("{err_msg}");
                send_status(&proxy, window_id, format!("❌ {err_msg}"), MessageType::Error);
                return;
            }
        };

        let temp_path = temp_file.path().to_path_buf();

        if let Err(e) = record_audio(&temp_path) {
            let err_msg = format!("Audio recording failed: {e}");
            error!("{err_msg}");
            send_status(&proxy, window_id, format!("❌ {err_msg}"), MessageType::Error);
            return;
        }

        // Stage 2: Speech transcription
        send_status(
            &proxy,
            window_id,
            "⚙️ Transcribing speech with Whisper...",
            MessageType::Warning,
        );

        let transcribed_text = match transcribe_audio(&temp_path, &voice_config.whisper_model_path) {
            Ok(text) => text.trim().to_string(),
            Err(e) => {
                let err_msg = format!("Whisper transcription failed: {e}");
                error!("{err_msg}");
                send_status(&proxy, window_id, format!("❌ {err_msg}"), MessageType::Error);
                return;
            }
        };

        if transcribed_text.is_empty() {
            let warn_msg = "⚠️ No speech detected or transcription was empty.";
            info!("{warn_msg}");
            send_status(&proxy, window_id, warn_msg, MessageType::Warning);
            return;
        }

        info!("Transcribed text: {}", transcribed_text);

        // Stage 3: LLM command inference
        send_status(
            &proxy,
            window_id,
            format!(
                "🤖 Generating command via LLM ({}) for: \"{}\"...",
                voice_config.model, transcribed_text
            ),
            MessageType::Warning,
        );

        let command_from_llm = match call_llm(&transcribed_text, &context, &voice_config) {
            Ok(cmd) => cmd,
            Err(e) => {
                let err_msg = format!("LLM command generation failed: {e}");
                error!("{err_msg}");
                send_status(&proxy, window_id, format!("❌ {err_msg}"), MessageType::Error);
                return;
            }
        };

        info!("Command from LLM: {}", command_from_llm);

        // Clear status from message bar upon success
        clear_status(&proxy, window_id);

        // Send command to interactive preview & safety guard
        let is_destructive = crate::ai::is_destructive_command(&command_from_llm);
        let event = Event::new(
            EventType::AiPreview {
                command: command_from_llm,
                is_destructive,
            },
            window_id,
        );
        let _ = proxy.send_event(event);
    });
}

pub fn handle_text_prompt(
    prompt_text: String,
    context: AiContext,
    config: &UiConfig,
    proxy: &EventLoopProxy<Event>,
    window_id: WindowId,
) {
    let voice_config = config.voice.clone();
    let proxy = proxy.clone();

    thread::spawn(move || {
        send_status(
            &proxy,
            window_id,
            format!(
                "🤖 Generating command via LLM ({}) for: \"{}\"...",
                voice_config.model, prompt_text
            ),
            MessageType::Warning,
        );

        let command_from_llm = match call_llm(&prompt_text, &context, &voice_config) {
            Ok(cmd) => cmd,
            Err(e) => {
                let err_msg = format!("LLM command generation failed: {e}");
                error!("{err_msg}");
                send_status(&proxy, window_id, format!("❌ {err_msg}"), MessageType::Error);
                return;
            }
        };

        info!("Command from LLM: {}", command_from_llm);

        // Clear status from message bar upon success
        clear_status(&proxy, window_id);

        // Send command to interactive preview & safety guard
        let is_destructive = crate::ai::is_destructive_command(&command_from_llm);
        let event = Event::new(
            EventType::AiPreview {
                command: command_from_llm,
                is_destructive,
            },
            window_id,
        );
        let _ = proxy.send_event(event);
    });
}



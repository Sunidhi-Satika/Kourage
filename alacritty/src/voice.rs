use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::config::ui_config::UiConfig;
use crate::config::voice::Voice;
use crate::event::{Event, EventType};
use crate::message_bar::{Message, MessageType};

pub const LOG_TARGET_VOICE: &str = "alacritty_voice";

/// Held while a voice session runs; a second session is refused until it is dropped.
struct BusyGuard(&'static AtomicBool);

impl BusyGuard {
    fn try_acquire(flag: &'static AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).ok()?;
        Some(Self(flag))
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Set while a voice command is recording/transcribing/generating.
static VOICE_BUSY: AtomicBool = AtomicBool::new(false);

/// Sample rate Whisper expects.
const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Convert interleaved device audio to the 16 kHz mono `f32` Whisper expects.
fn to_whisper_input(samples: &[f32], channels: u16, sample_rate: u32) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    let mono: Vec<f32> = if channels == 1 {
        samples.to_vec()
    } else {
        samples
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };

    if sample_rate == WHISPER_SAMPLE_RATE || mono.is_empty() {
        return mono;
    }

    // Linear interpolation is enough for speech recognition input.
    let out_len =
        (mono.len() as u64 * u64::from(WHISPER_SAMPLE_RATE) / u64::from(sample_rate)) as usize;
    let step = f64::from(sample_rate) / f64::from(WHISPER_SAMPLE_RATE);
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * step;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = mono[idx];
            let b = *mono.get(idx + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

/// RMS above which a chunk counts as speech.
const SPEECH_RMS_THRESHOLD: f32 = 0.005;
/// Trailing silence that ends a recording once the user has spoken.
const TRAILING_SILENCE: Duration = Duration::from_secs(1);
/// How long to wait for the user to start speaking before giving up.
const NO_SPEECH_TIMEOUT: Duration = Duration::from_secs(6);

/// Decides when to stop recording based on per-chunk loudness.
///
/// Silence only ends a recording after speech was heard, so the user has time to start
/// talking; if they never do, recording gives up after [`NO_SPEECH_TIMEOUT`].
struct SilenceDetector {
    heard_speech: bool,
    silence: Duration,
}

impl SilenceDetector {
    fn new() -> Self {
        Self { heard_speech: false, silence: Duration::ZERO }
    }

    /// Feed one audio chunk's RMS and duration; returns `true` when recording should stop.
    fn push(&mut self, rms: f32, chunk: Duration) -> bool {
        if rms >= SPEECH_RMS_THRESHOLD {
            self.heard_speech = true;
            self.silence = Duration::ZERO;
            return false;
        }

        self.silence += chunk;
        if self.heard_speech {
            self.silence >= TRAILING_SILENCE
        } else {
            self.silence >= NO_SPEECH_TIMEOUT
        }
    }
}

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

/// Audio captured from the default input device, still in the device's own format.
struct Recording {
    samples: Vec<f32>,
    channels: u16,
    sample_rate: u32,
}

fn record_audio() -> Result<Recording, anyhow::Error> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No audio input device found"))?;
    let config = device.default_input_config()?;
    let channels = config.channels();
    let sample_rate = config.sample_rate().0;

    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let samples_clone = samples.clone();

    let should_stop = Arc::new(AtomicBool::new(false));
    let should_stop_clone = should_stop.clone();
    let mut detector = SilenceDetector::new();

    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if data.is_empty() {
                return;
            }
            samples_clone.lock().unwrap().extend_from_slice(data);

            let rms = (data.iter().map(|s| s * s).sum::<f32>() / data.len() as f32).sqrt();
            let frames = data.len() / usize::from(channels.max(1));
            let chunk = Duration::from_secs_f64(frames as f64 / f64::from(sample_rate));
            if detector.push(rms, chunk) {
                should_stop_clone.store(true, Ordering::SeqCst);
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
    drop(stream);
    info!("Recording stopped.");

    let samples = std::mem::take(&mut *samples.lock().unwrap());
    Ok(Recording { samples, channels, sample_rate })
}

fn transcribe_audio(recording: &Recording, model_path_str: &str) -> Result<String, anyhow::Error> {
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

    let audio_samples =
        to_whisper_input(&recording.samples, recording.channels, recording.sample_rate);

    if log::max_level() >= log::LevelFilter::Debug {
        let path = format!("/tmp/kourage-voice-{}.wav", std::process::id());
        match std::fs::write(&path, encode_wav_16k_mono(&audio_samples)) {
            Ok(()) => info!("Wrote voice debug audio to {}", path),
            Err(err) => warn!("Failed to write voice debug audio to {}: {}", path, err),
        }
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

/// Encode 16 kHz mono samples as a 16-bit PCM WAV file (for debug dumps).
fn encode_wav_16k_mono(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&WHISPER_SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(WHISPER_SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    wav
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
        options: Some(LlmOptions { temperature: voice_config.temperature }),
    };

    let res = client
        .post(&voice_config.llm_api_url)
        .json(&request)
        .send()
        .map_err(|e| {
            anyhow::anyhow!("HTTP request to \"{}\" failed: {}", voice_config.llm_api_url, e)
        })?
        .json::<LlmResponse>()
        .map_err(|e| {
            anyhow::anyhow!("Failed to parse response from {}: {}", voice_config.llm_api_url, e)
        })?;

    let cmd = crate::ai::clean_llm_reply(&res.response);

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
    // Holding the hotkey auto-repeats it; only one voice session may own the mic.
    let Some(busy) = BusyGuard::try_acquire(&VOICE_BUSY) else {
        info!("Voice command already in progress; ignoring trigger");
        return;
    };

    let voice_config = config.voice.clone();
    let proxy = proxy.clone();

    thread::spawn(move || {
        let _busy = busy;

        // Stage 1: Audio recording
        send_status(
            &proxy,
            window_id,
            "🎙️ Recording audio... Speak your command",
            MessageType::Warning,
        );

        let recording = match record_audio() {
            Ok(recording) => recording,
            Err(e) => {
                let err_msg = format!("Audio recording failed: {e}");
                error!("{err_msg}");
                send_status(&proxy, window_id, format!("❌ {err_msg}"), MessageType::Error);
                return;
            },
        };

        // Stage 2: Speech transcription
        send_status(
            &proxy,
            window_id,
            "⚙️ Transcribing speech with Whisper...",
            MessageType::Warning,
        );

        let transcribed_text = match transcribe_audio(&recording, &voice_config.whisper_model_path)
        {
            Ok(text) => text.trim().to_string(),
            Err(e) => {
                let err_msg = format!("Whisper transcription failed: {e}");
                error!("{err_msg}");
                send_status(&proxy, window_id, format!("❌ {err_msg}"), MessageType::Error);
                return;
            },
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
            },
        };

        info!("Command from LLM: {}", command_from_llm);

        // Clear status from message bar upon success
        clear_status(&proxy, window_id);

        // Send command to interactive preview & safety guard
        let is_destructive = crate::ai::is_destructive_command(&command_from_llm);
        let event = Event::new(
            EventType::AiPreview { command: command_from_llm, is_destructive },
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
            },
        };

        info!("Command from LLM: {}", command_from_llm);

        // Clear status from message bar upon success
        clear_status(&proxy, window_id);

        // Send command to interactive preview & safety guard
        let is_destructive = crate::ai::is_destructive_command(&command_from_llm);
        let event = Event::new(
            EventType::AiPreview { command: command_from_llm, is_destructive },
            window_id,
        );
        let _ = proxy.send_event(event);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHUNK: Duration = Duration::from_millis(10);

    #[test]
    fn whisper_input_passes_16k_mono_through() {
        let samples = vec![0.1, -0.2, 0.3];
        assert_eq!(to_whisper_input(&samples, 1, 16_000), samples);
    }

    #[test]
    fn whisper_input_downmixes_stereo_by_averaging() {
        let out = to_whisper_input(&[1.0, 0.0, 0.5, 0.5], 2, 16_000);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    #[test]
    fn whisper_input_downmixes_any_channel_count() {
        let out = to_whisper_input(&[0.3, 0.3, 0.3, 0.6, 0.6, 0.6], 3, 16_000);
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.3).abs() < 1e-6 && (out[1] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn whisper_input_resamples_48k_to_16k() {
        let one_second = vec![0.25; 48_000];
        let out = to_whisper_input(&one_second, 1, 48_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn whisper_input_resamples_44_1k_to_16k() {
        let out = to_whisper_input(&vec![0.0; 44_100], 1, 44_100);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn silence_before_speech_does_not_stop_recording() {
        let mut detector = SilenceDetector::new();
        // Three seconds of the user getting ready to talk.
        for _ in 0..300 {
            assert!(!detector.push(0.0, CHUNK));
        }
    }

    #[test]
    fn silence_after_speech_stops_recording() {
        let mut detector = SilenceDetector::new();
        for _ in 0..50 {
            assert!(!detector.push(0.1, CHUNK));
        }
        let mut stopped_after = None;
        for i in 1..=300 {
            if detector.push(0.0, CHUNK) {
                stopped_after = Some(i);
                break;
            }
        }
        // Stops after roughly a second of trailing silence, not instantly and not never.
        let chunks = stopped_after.expect("recording never stopped");
        assert!((90..=110).contains(&chunks), "stopped after {chunks} quiet chunks");
    }

    #[test]
    fn no_speech_at_all_eventually_stops() {
        let mut detector = SilenceDetector::new();
        let stopped = (0..1000).any(|_| detector.push(0.0, CHUNK));
        assert!(stopped, "should give up after waiting several seconds for speech");
    }

    #[test]
    fn busy_guard_rejects_second_session_while_first_runs() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let first = BusyGuard::try_acquire(&FLAG);
        assert!(first.is_some());
        assert!(BusyGuard::try_acquire(&FLAG).is_none());
    }

    #[test]
    fn busy_guard_releases_on_drop() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        drop(BusyGuard::try_acquire(&FLAG));
        assert!(BusyGuard::try_acquire(&FLAG).is_some());
    }

    #[test]
    fn wav_header_is_valid() {
        let wav = encode_wav_16k_mono(&[0.0, 0.5, -0.5]);
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 6);
    }

    #[test]
    fn wav_clamps_and_scales() {
        let wav = encode_wav_16k_mono(&[1.0, -1.0, 2.0]);
        let pcm: Vec<i16> =
            wav[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(pcm, vec![32767, -32767, 32767]);
    }
}

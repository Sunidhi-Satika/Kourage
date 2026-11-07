use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use log::{error, info, warn};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use kourage_core::{ask, Backend, LlmSettings, OllamaBackend, Proposal};

use crate::ai::{diagnostics, AiContext, AiPhase, AiStatus};
use crate::config::ui_config::UiConfig;
use crate::config::voice::Voice;
use crate::event::{Event, EventType};

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

/// Tracks whether the microphone is open and whether the user asked to cancel it.
struct VoiceSession {
    recording: AtomicBool,
    cancelled: AtomicBool,
}

impl VoiceSession {
    const fn new() -> Self {
        Self { recording: AtomicBool::new(false), cancelled: AtomicBool::new(false) }
    }

    fn begin_recording(&self) {
        self.cancelled.store(false, Ordering::SeqCst);
        self.recording.store(true, Ordering::SeqCst);
    }

    fn end_recording(&self) {
        self.recording.store(false, Ordering::SeqCst);
    }

    /// Ask the recorder to stop and discard its audio. Only succeeds while recording.
    fn request_cancel(&self) -> bool {
        if self.recording.load(Ordering::SeqCst) {
            self.cancelled.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

static VOICE_SESSION: VoiceSession = VoiceSession::new();

/// Cancel the recording in progress, if any. Returns whether there was one to cancel.
pub fn cancel_recording() -> bool {
    VOICE_SESSION.request_cancel()
}

/// Strip Whisper's non-speech markers (`[BLANK_AUDIO]`, `(music)`) and known silence
/// hallucinations from a transcript. An empty result means nothing intelligible was said.
fn clean_transcript(raw: &str) -> String {
    let mut kept = String::new();
    let mut closing: Option<char> = None;
    for c in raw.chars() {
        match (closing, c) {
            (None, '[') => closing = Some(']'),
            (None, '(') => closing = Some(')'),
            (None, '*') => closing = Some('*'),
            (Some(close), c) if c == close => closing = None,
            (Some(_), _) => (),
            (None, c) => kept.push(c),
        }
    }

    let text = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = text.trim_end_matches(['.', '!', '?', ',']).to_lowercase();
    // What Whisper tends to "hear" in near-silence.
    const SILENCE_HALLUCINATIONS: [&str; 5] =
        ["bye", "you", "thank you", "thanks", "thanks for watching"];
    if SILENCE_HALLUCINATIONS.contains(&normalized.as_str()) {
        return String::new();
    }
    text
}

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

/// RMS of the loudest 100 ms window in interleaved device audio. Used to tell a quiet
/// microphone from a muted one when no speech is detected.
fn loudest_window_rms(samples: &[f32], channels: u16, sample_rate: u32) -> f32 {
    let window = (sample_rate as usize / 10).max(1) * usize::from(channels.max(1));
    samples
        .chunks(window)
        .map(|chunk| (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt())
        .fold(0.0, f32::max)
}

/// Live microphone level for the status bar's meter: jumps up with the signal and falls away
/// smoothly, so the meter is readable at the status bar's refresh rate.
fn smooth_level(previous: f32, rms: f32) -> f32 {
    rms.max(previous * 0.85)
}

/// The most recent (smoothed) microphone level, as `f32` bits.
static INPUT_LEVEL_BITS: AtomicU32 = AtomicU32::new(0);

/// Current microphone level for the status bar's meter; 0 when not recording.
pub fn input_level() -> f32 {
    f32::from_bits(INPUT_LEVEL_BITS.load(Ordering::Relaxed))
}

fn set_input_level(level: f32) {
    INPUT_LEVEL_BITS.store(level.to_bits(), Ordering::Relaxed);
}

pub fn init() {
    // Initialization logic if needed
}

/// Sends the AI's progress to the window and keeps the elapsed-seconds counter moving.
struct StatusReporter {
    proxy: EventLoopProxy<Event>,
    window_id: WindowId,
    ticking: Arc<AtomicBool>,
}

impl StatusReporter {
    fn start(proxy: &EventLoopProxy<Event>, window_id: WindowId) -> Self {
        let ticking = Arc::new(AtomicBool::new(true));
        let (ticker_proxy, ticker_flag) = (proxy.clone(), ticking.clone());
        thread::spawn(move || {
            while ticker_flag.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(250));
                let _ = ticker_proxy.send_event(Event::new(EventType::AiStatusTick, window_id));
            }
        });
        Self { proxy: proxy.clone(), window_id, ticking }
    }

    fn show(&self, phase: AiPhase) {
        let event = EventType::AiStatus(Some(AiStatus::new(phase)));
        let _ = self.proxy.send_event(Event::new(event, self.window_id));
    }

    fn clear(&self) {
        let _ = self.proxy.send_event(Event::new(EventType::AiStatus(None), self.window_id));
    }

    /// Show an error that stays until the user presses a key.
    fn fail(&self, message: String) {
        error!("{message}");
        self.show(AiPhase::Error(message));
    }
}

impl Drop for StatusReporter {
    fn drop(&mut self) {
        self.ticking.store(false, Ordering::Relaxed);
    }
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

/// Convert device samples of any supported format to `f32` in `[-1.0, 1.0]`, replacing `out`.
fn convert_to_f32<T>(data: &[T], out: &mut Vec<f32>)
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    out.clear();
    out.extend(data.iter().map(|sample| <f32 as cpal::FromSample<T>>::from_sample_(*sample)));
}

/// Open an input stream for sample type `T`, handing every chunk to `on_chunk` as `f32`.
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut on_chunk: impl FnMut(&[f32]) + Send + 'static,
) -> Result<cpal::Stream, anyhow::Error>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let mut converted = Vec::<f32>::new();
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            convert_to_f32(data, &mut converted);
            on_chunk(&converted);
        },
        |err| error!("An error occurred on the input stream: {}", err),
        None,
    )?;
    Ok(stream)
}

/// Record from the default microphone. `Ok(None)` means the user cancelled.
fn record_audio() -> Result<Option<Recording>, anyhow::Error> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No audio input device found"))?;
    let config = device.default_input_config()?;
    info!(
        "Microphone: {} ({} Hz, {} ch, {:?})",
        device.name().unwrap_or_else(|_| "unknown".into()),
        config.sample_rate().0,
        config.channels(),
        config.sample_format()
    );
    let channels = config.channels();
    let sample_rate = config.sample_rate().0;

    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let samples_clone = samples.clone();

    let should_stop = Arc::new(AtomicBool::new(false));
    let should_stop_clone = should_stop.clone();
    let mut detector = SilenceDetector::new();

    let on_chunk = move |data: &[f32]| {
        if data.is_empty() {
            return;
        }
        samples_clone.lock().unwrap().extend_from_slice(data);

        let rms = (data.iter().map(|s| s * s).sum::<f32>() / data.len() as f32).sqrt();
        set_input_level(smooth_level(input_level(), rms));
        let frames = data.len() / usize::from(channels.max(1));
        let chunk = Duration::from_secs_f64(frames as f64 / f64::from(sample_rate));
        if detector.push(rms, chunk) {
            should_stop_clone.store(true, Ordering::SeqCst);
        }
    };

    let stream_config: cpal::StreamConfig = config.clone().into();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &stream_config, on_chunk)?,
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &stream_config, on_chunk)?,
        cpal::SampleFormat::U16 => build_stream::<u16>(&device, &stream_config, on_chunk)?,
        cpal::SampleFormat::I32 => build_stream::<i32>(&device, &stream_config, on_chunk)?,
        other => {
            return Err(anyhow::anyhow!(
                "Unsupported microphone sample format {other:?}; please report your device."
            ))
        },
    };

    stream.play()?;
    info!("Recording audio... Speak now!");

    let start_time = std::time::Instant::now();
    let max_duration = Duration::from_secs(10);

    while !should_stop.load(Ordering::SeqCst)
        && !VOICE_SESSION.is_cancelled()
        && start_time.elapsed() < max_duration
    {
        thread::sleep(Duration::from_millis(100));
    }

    stream.pause()?;
    drop(stream);
    set_input_level(0.0);
    info!("Recording stopped.");

    if VOICE_SESSION.is_cancelled() {
        return Ok(None);
    }

    let samples = std::mem::take(&mut *samples.lock().unwrap());
    info!(
        "Recording stopped. Loudest 100 ms input level: {:.4} (speech is detected at {})",
        loudest_window_rms(&samples, channels, sample_rate),
        SPEECH_RMS_THRESHOLD
    );
    Ok(Some(Recording { samples, channels, sample_rate }))
}

/// The loaded Whisper model, kept between commands because loading it from disk is slow.
static WHISPER_CONTEXT: Mutex<Option<(String, Arc<WhisperContext>)>> = Mutex::new(None);

fn whisper_context(model_path: &str) -> Result<Arc<WhisperContext>, anyhow::Error> {
    let mut cached = WHISPER_CONTEXT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((path, ctx)) = cached.as_ref() {
        if path == model_path {
            return Ok(ctx.clone());
        }
    }

    info!("Loading Whisper model from {}", model_path);
    let ctx = Arc::new(
        WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .map_err(|e| anyhow::anyhow!("Failed to create Whisper context: {:?}", e))?,
    );
    *cached = Some((model_path.to_string(), ctx.clone()));
    Ok(ctx)
}

fn transcribe_audio(recording: &Recording, model_path_str: &str) -> Result<String, anyhow::Error> {
    let resolved_model_path = resolve_path(model_path_str);
    if !resolved_model_path.exists() {
        return Err(anyhow::anyhow!(diagnostics::whisper_missing_message(
            &resolved_model_path.display().to_string()
        )));
    }

    let model_path_os = resolved_model_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid model path encoding"))?;

    let ctx = whisper_context(model_path_os)?;
    let mut state = ctx
        .create_state()
        .map_err(|e| anyhow::anyhow!("Failed to create Whisper state: {:?}", e))?;

    let audio_samples =
        to_whisper_input(&recording.samples, recording.channels, recording.sample_rate);

    if log::max_level() >= log::LevelFilter::Debug {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = std::path::PathBuf::from(format!(
            "/tmp/kourage-voice-{}-{}.wav",
            std::process::id(),
            secs
        ));
        match write_debug_wav(&path, &encode_wav_16k_mono(&audio_samples)) {
            Ok(()) => info!("Wrote voice debug audio to {}", path.display()),
            Err(err) => warn!("Failed to write voice debug audio to {}: {}", path.display(), err),
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

/// Write a debug recording that only the current user can read, never overwriting a file.
fn write_debug_wav(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(bytes)
}

fn llm_settings(voice_config: &Voice) -> LlmSettings {
    LlmSettings {
        api_url: voice_config.llm_api_url.clone(),
        model: voice_config.model.clone(),
        temperature: voice_config.temperature,
        timeout_secs: voice_config.timeout_secs,
    }
}

/// Load the model and prime Ollama's prompt cache in the background, so the first real request
/// skips the ~11 s cold start. Failures are only logged: Ollama may simply not be running yet.
pub fn warm_up_llm(voice_config: &Voice) {
    let backend = OllamaBackend::new(llm_settings(voice_config));
    thread::spawn(move || {
        let started = std::time::Instant::now();
        // The fixed prompt prefix is what Ollama caches between requests.
        match backend.warm_up(&AiContext::default().format_prompt("")) {
            Ok(()) => info!("LLM warm-up finished in {:.1}s", started.elapsed().as_secs_f32()),
            Err(err) => info!("LLM warm-up skipped: {err}"),
        }
    });
}

/// Ask `backend` for a command. The error is the text shown in the status line.
fn run_prompt(
    backend: &dyn Backend,
    instruction: &str,
    context: &AiContext,
    temperature: f32,
) -> Result<Proposal, String> {
    let started = std::time::Instant::now();
    let result = ask(backend, context, instruction, temperature);
    info!("LLM request took {:.1}s", started.elapsed().as_secs_f32());

    let proposal = result.map_err(|e| format!("LLM command generation failed: {e}"))?;
    info!("Command from LLM: {}", proposal.command);
    Ok(proposal)
}

/// Run `instruction` against the configured Ollama server. On success, sends the preview event;
/// on error, calls `reporter.fail`.
fn propose_command(
    instruction: &str,
    context: &AiContext,
    voice_config: &Voice,
    proxy: &EventLoopProxy<Event>,
    window_id: WindowId,
    reporter: &StatusReporter,
) {
    let backend = OllamaBackend::new(llm_settings(voice_config));
    match run_prompt(&backend, instruction, context, voice_config.temperature) {
        Ok(proposal) => send_preview(proxy, window_id, reporter, proposal),
        Err(message) => reporter.fail(message),
    }
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

    let voice_config = config.ai.clone();
    let reporter = StatusReporter::start(proxy, window_id);
    let proxy = proxy.clone();

    thread::spawn(move || {
        let _busy = busy;

        // Stage 1: Audio recording
        reporter.show(AiPhase::Listening);
        VOICE_SESSION.begin_recording();
        let recorded = record_audio();
        VOICE_SESSION.end_recording();

        let recording = match recorded {
            Ok(Some(recording)) => recording,
            Ok(None) => {
                info!("Voice command cancelled");
                reporter.show(AiPhase::Notice("Recording cancelled.".into()));
                return;
            },
            Err(e) => {
                reporter.fail(format!("Audio recording failed: {e}"));
                return;
            },
        };

        // Stage 2: Speech transcription
        reporter.show(AiPhase::Transcribing);
        let loudest =
            loudest_window_rms(&recording.samples, recording.channels, recording.sample_rate);

        let transcribed_text = match transcribe_audio(&recording, &voice_config.whisper_model_path)
        {
            Ok(text) => clean_transcript(&text),
            Err(e) => {
                reporter.fail(format!("Whisper transcription failed: {e}"));
                return;
            },
        };

        if transcribed_text.is_empty() {
            let message = if loudest < SPEECH_RMS_THRESHOLD {
                format!(
                    "No speech heard (loudest input level {loudest:.3}, speech needs \
                     {SPEECH_RMS_THRESHOLD}). Speak closer or check the microphone."
                )
            } else {
                "Heard sound but no words were recognised. Try again.".to_string()
            };
            info!("{message}");
            reporter.show(AiPhase::Notice(message));
            return;
        }

        info!("Transcribed text: {}", transcribed_text);

        // Stage 3: LLM command inference
        reporter.show(AiPhase::Thinking { model: voice_config.model.clone() });
        propose_command(&transcribed_text, &context, &voice_config, &proxy, window_id, &reporter);
    });
}

pub fn handle_text_prompt(
    prompt_text: String,
    context: AiContext,
    config: &UiConfig,
    proxy: &EventLoopProxy<Event>,
    window_id: WindowId,
) {
    let voice_config = config.ai.clone();
    let reporter = StatusReporter::start(proxy, window_id);
    // Shown before the thread starts so the user sees "sent" immediately.
    reporter.show(AiPhase::Thinking { model: voice_config.model.clone() });
    let proxy = proxy.clone();

    thread::spawn(move || {
        propose_command(&prompt_text, &context, &voice_config, &proxy, window_id, &reporter);
    });
}

/// Replace the status line with the command preview and the safety check's verdict.
fn send_preview(
    proxy: &EventLoopProxy<Event>,
    window_id: WindowId,
    reporter: &StatusReporter,
    proposal: Proposal,
) {
    reporter.clear();
    let Proposal { command, is_destructive } = proposal;
    let event = Event::new(EventType::AiPreview { command, is_destructive }, window_id);
    let _ = proxy.send_event(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    use kourage_core::{BackendError, MockBackend};

    const CHUNK: Duration = Duration::from_millis(10);

    fn prompt_with(backend: &MockBackend) -> Result<Proposal, String> {
        run_prompt(backend, "do it", &AiContext::default(), 0.1)
    }

    #[test]
    fn text_prompt_flow_yields_proposal_with_mock() {
        let safe = prompt_with(&MockBackend::replying(&["`du -sh .`"])).unwrap();
        assert_eq!(safe, Proposal { command: "du -sh .".into(), is_destructive: false });

        let destructive = prompt_with(&MockBackend::replying(&["rm -rf ~"])).unwrap();
        assert_eq!(destructive, Proposal { command: "rm -rf ~".into(), is_destructive: true });

        let error =
            BackendError::Timeout { url: "http://localhost:11434".into(), timeout_secs: 15 };
        let failed = prompt_with(&MockBackend::failing(error.clone())).unwrap_err();
        assert_eq!(failed, format!("LLM command generation failed: {error}"));
    }

    #[test]
    fn text_prompt_flow_reports_bad_replies_with_existing_wording() {
        assert_eq!(
            prompt_with(&MockBackend::replying(&["ls\nrm -rf /"])).unwrap_err(),
            "LLM command generation failed: The model returned 2 lines but a single command is \
             required. Try rephrasing."
        );
        assert_eq!(
            prompt_with(&MockBackend::replying(&["  "])).unwrap_err(),
            "LLM command generation failed: The model returned an empty command. Try rephrasing."
        );
    }

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

    #[test]
    fn debug_wav_is_private_and_never_overwrites() {
        let path = std::env::temp_dir().join(format!("kourage-test-{}.wav", std::process::id()));
        let _ = std::fs::remove_file(&path);
        write_debug_wav(&path, b"abc").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(write_debug_wav(&path, b"xyz").is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn samples_convert_to_f32_from_i16_and_u16() {
        let mut out = Vec::new();
        convert_to_f32(&[i16::MIN, 0, i16::MAX], &mut out);
        assert!((out[0] + 1.0).abs() < 1e-3 && out[1].abs() < 1e-6 && (out[2] - 1.0).abs() < 1e-3);

        // Unsigned 16-bit audio is centred on 32768.
        convert_to_f32(&[0u16, 32768, 65535], &mut out);
        assert!((out[0] + 1.0).abs() < 1e-3 && out[1].abs() < 1e-3 && (out[2] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn convert_to_f32_replaces_previous_contents() {
        let mut out = vec![9.0; 5];
        convert_to_f32(&[0.5f32], &mut out);
        assert_eq!(out, vec![0.5]);
    }

    #[test]
    fn cancel_is_refused_when_nothing_is_recording() {
        let session = VoiceSession::new();
        assert!(!session.request_cancel());
        assert!(!session.is_cancelled());
    }

    #[test]
    fn cancel_while_recording_is_seen_by_the_recorder() {
        let session = VoiceSession::new();
        session.begin_recording();
        assert!(session.request_cancel());
        assert!(session.is_cancelled());
    }

    #[test]
    fn beginning_a_new_recording_clears_an_old_cancel() {
        let session = VoiceSession::new();
        session.begin_recording();
        session.request_cancel();
        session.end_recording();
        session.begin_recording();
        assert!(!session.is_cancelled());
    }

    #[test]
    fn cancel_after_recording_ended_is_refused() {
        let session = VoiceSession::new();
        session.begin_recording();
        session.end_recording();
        assert!(!session.request_cancel());
    }

    #[test]
    fn transcript_keeps_real_speech() {
        assert_eq!(clean_transcript("  list all files "), "list all files");
        assert_eq!(clean_transcript("show disk usage [BLANK_AUDIO]"), "show disk usage");
    }

    #[test]
    fn transcript_drops_non_speech_markers() {
        for raw in ["[BLANK_AUDIO]", " [ Silence ] ", "(music)", "[MUSIC] (soft music)", "*sigh*"] {
            assert_eq!(clean_transcript(raw), "", "input: {raw:?}");
        }
    }

    #[test]
    fn transcript_drops_known_silence_hallucinations() {
        for raw in ["Bye.", "bye", "Thank you.", "Thanks for watching!", "you"] {
            assert_eq!(clean_transcript(raw), "", "input: {raw:?}");
        }
    }

    #[test]
    fn transcript_keeps_commands_that_merely_contain_those_words() {
        assert_eq!(clean_transcript("say bye to the old files"), "say bye to the old files");
    }

    #[test]
    fn loudest_window_of_silence_is_zero() {
        assert_eq!(loudest_window_rms(&vec![0.0; 4800], 1, 48_000), 0.0);
        assert_eq!(loudest_window_rms(&[], 1, 48_000), 0.0);
    }

    #[test]
    fn loudest_window_finds_a_burst_in_the_middle() {
        // 48 kHz mono: 0.3 s of silence, 0.1 s at +-0.2, 0.3 s of silence.
        let mut samples = vec![0.0f32; 14_400];
        samples.extend(std::iter::repeat_n(0.2, 4_800));
        samples.extend(vec![0.0f32; 14_400]);
        let rms = loudest_window_rms(&samples, 1, 48_000);
        assert!((rms - 0.2).abs() < 0.01, "rms was {rms}");
    }

    #[test]
    fn loudest_window_handles_interleaved_channels() {
        // 2 channels at 1 kHz: 100 ms = 100 frames = 200 samples, all 0.5.
        let rms = loudest_window_rms(&vec![0.5; 200], 2, 1_000);
        assert!((rms - 0.5).abs() < 1e-3, "rms was {rms}");
    }

    #[test]
    fn level_jumps_up_immediately() {
        assert_eq!(smooth_level(0.0, 0.1), 0.1);
        assert_eq!(smooth_level(0.1, 0.5), 0.5);
    }

    #[test]
    fn level_decays_gradually_when_the_signal_drops() {
        let next = smooth_level(0.1, 0.0);
        assert!((next - 0.085).abs() < 1e-6, "{next}");
        assert_eq!(smooth_level(0.0, 0.0), 0.0);
    }
}

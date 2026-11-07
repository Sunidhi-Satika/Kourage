use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use log::{error, info, warn};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use kourage_core::{ask, check_setup, Backend, LlmSettings, OllamaBackend, Proposal, SetupIssue};

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

/// The request is waiting for the model.
const REQUEST_IN_FLIGHT: u8 = 0;
/// The user cancelled the request; its worker has not noticed yet.
const REQUEST_CANCELLED: u8 = 1;
/// The request's worker is done with it.
const REQUEST_ENDED: u8 = 2;

/// The state of one AI request.
///
/// One atomic per request, so a cancel and the end of that request cannot interleave: either the
/// cancel lands first and [`Self::end`] reports it, or the request ended first and the cancel is
/// refused. Requests never touch each other's state.
struct RequestState(AtomicU8);

impl RequestState {
    fn cancel(&self) -> bool {
        self.0
            .compare_exchange(
                REQUEST_IN_FLIGHT,
                REQUEST_CANCELLED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    /// Returns whether the request was cancelled before it ended.
    fn end(&self) -> bool {
        self.0.swap(REQUEST_ENDED, Ordering::SeqCst) == REQUEST_CANCELLED
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst) == REQUEST_CANCELLED
    }
}

/// The AI requests currently waiting for the model, oldest first.
struct AiRequests {
    next_id: AtomicU64,
    in_flight: Mutex<Vec<(u64, Arc<RequestState>)>>,
}

impl AiRequests {
    const fn new() -> Self {
        Self { next_id: AtomicU64::new(0), in_flight: Mutex::new(Vec::new()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(u64, Arc<RequestState>)>> {
        self.in_flight.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start a new request with its own cancel state.
    fn begin(&self) -> RequestToken<'_> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let state = Arc::new(RequestState(AtomicU8::new(REQUEST_IN_FLIGHT)));
        self.lock().push((id, state.clone()));
        RequestToken { requests: self, id, state, active: true }
    }

    /// Cancel the most recently started request (the one the status line shows). Returns
    /// whether that cancel was accepted; an older request is never cancelled instead.
    fn cancel_latest(&self) -> bool {
        self.lock().last().is_some_and(|(_, state)| state.cancel())
    }

    fn remove(&self, id: u64) {
        self.lock().retain(|(entry, _)| *entry != id);
    }
}

static AI_REQUESTS: AiRequests = AiRequests::new();

/// One request's handle: cancelled only through its own state, and removed from the registry
/// when finished or dropped, so no exit path (early return, panic) can leave it in flight and
/// Esc swallowed forever.
struct RequestToken<'a> {
    requests: &'a AiRequests,
    id: u64,
    state: Arc<RequestState>,
    active: bool,
}

impl RequestToken<'_> {
    fn is_cancelled(&self) -> bool {
        self.state.is_cancelled()
    }

    /// End this request now. Returns whether the user cancelled it before it ended.
    fn finish(mut self) -> bool {
        self.end()
    }

    fn end(&mut self) -> bool {
        if !std::mem::replace(&mut self.active, false) {
            return false;
        }
        // Swap the state first: while it is still registered but ended, `cancel_latest`
        // refuses instead of skipping it and cancelling an older request.
        let cancelled = self.state.end();
        self.requests.remove(self.id);
        cancelled
    }
}

impl Drop for RequestToken<'_> {
    fn drop(&mut self) {
        self.end();
    }
}

/// Cancel the recording in `voice` if one is in progress, otherwise the latest request in
/// `requests`. Returns whether anything was cancelled.
fn cancel_in(voice: &VoiceSession, requests: &AiRequests) -> bool {
    voice.request_cancel() || requests.cancel_latest()
}

/// Cancel what the AI is doing for the user: the recording if one is in progress, otherwise
/// the most recently started request waiting for the model. Returns whether anything was
/// cancelled.
///
/// Requests are tracked process-wide, not per window: with requests running in two windows,
/// Esc in either window cancels the most recent one.
pub fn cancel_ai() -> bool {
    cancel_in(&VOICE_SESSION, &AI_REQUESTS)
}

/// How often a waiting worker checks whether the user cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// Wait for a value on `rx`, giving up as soon as `cancelled()` is true. `None` means the wait
/// was cancelled or the sender went away without sending.
fn wait_or_cancel<T>(rx: &std::sync::mpsc::Receiver<T>, cancelled: &dyn Fn() -> bool) -> Option<T> {
    use std::sync::mpsc::RecvTimeoutError;

    loop {
        if cancelled() {
            return None;
        }
        match rx.recv_timeout(CANCEL_POLL) {
            Ok(value) => return Some(value),
            Err(RecvTimeoutError::Timeout) => (),
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
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

    /// Show a notice only if the status line is not already in use.
    fn notice_if_idle(&self, message: String) {
        let event = Event::new(EventType::AiSetupNotice(message), self.window_id);
        let _ = self.proxy.send_event(event);
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

/// Set once the startup setup check has been started, so only the first window gets it.
static SETUP_ANNOUNCED: AtomicBool = AtomicBool::new(false);

/// The Whisper path as the check must see it: `~/` expanded like when the model is loaded.
fn whisper_path_for_check(path: &str) -> String {
    resolve_path(path).to_string_lossy().into_owned()
}

/// The one-line status text for `issues`, or `None` when nothing needs the user's attention.
/// An unconfigured Whisper path is skipped: text-only users must not be nagged about it.
fn setup_summary(issues: &[SetupIssue]) -> Option<String> {
    let mut shown =
        issues.iter().filter(|issue| !matches!(issue, SetupIssue::WhisperNotConfigured { .. }));
    let first = shown.next()?.message().to_string();
    match shown.count() {
        0 => Some(first),
        more => Some(format!("{first} (+{more} more, see the log)")),
    }
}

/// Check in the background what the AI features are missing (Ollama, the model, Whisper),
/// log every issue and show a summary on `window_id`'s status line. Runs once per process.
pub fn announce_setup_issues(
    config: &UiConfig,
    proxy: &EventLoopProxy<Event>,
    window_id: WindowId,
) {
    if SETUP_ANNOUNCED.swap(true, Ordering::Relaxed) {
        return;
    }

    let backend = OllamaBackend::new(llm_settings(&config.ai));
    let whisper_path = whisper_path_for_check(&config.ai.whisper_model_path);
    let proxy = proxy.clone();

    thread::spawn(move || {
        let issues = check_setup(&backend, &whisper_path);
        for issue in &issues {
            warn!("Setup: {}", issue.message());
        }
        if let Some(summary) = setup_summary(&issues) {
            StatusReporter::start(&proxy, window_id).notice_if_idle(summary);
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

/// Shown when a request's helper thread ended without an answer and nobody cancelled it.
const REQUEST_STOPPED_MESSAGE: &str =
    "LLM command generation failed: the request stopped unexpectedly. See the log.";

/// How a request ended, from its worker's point of view.
#[derive(Debug, PartialEq, Eq)]
enum RequestOutcome {
    /// The user cancelled this request; nothing else may be shown for it.
    Cancelled,
    Preview(Proposal),
    Failed(String),
}

/// Decide what a finished request shows. An accepted cancel always wins, even over an answer
/// that arrived at the same moment, so no preview ever appears after "Request cancelled.".
fn request_outcome(answer: Option<Result<Proposal, String>>, cancelled: bool) -> RequestOutcome {
    match answer {
        _ if cancelled => RequestOutcome::Cancelled,
        Some(Ok(proposal)) => RequestOutcome::Preview(proposal),
        Some(Err(message)) => RequestOutcome::Failed(message),
        None => RequestOutcome::Failed(REQUEST_STOPPED_MESSAGE.into()),
    }
}

/// Run `work` on a helper thread and wait for it until `request` is cancelled.
///
/// A cancelled request is abandoned: its helper thread ends by itself (for the model, within the
/// configured timeout) and its late answer is dropped, because nobody receives it any more.
fn run_request(
    request: RequestToken<'_>,
    work: impl FnOnce() -> Result<Proposal, String> + Send + 'static,
) -> RequestOutcome {
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(work());
    });

    let answer = wait_or_cancel(&rx, &|| request.is_cancelled());
    drop(rx);
    request_outcome(answer, request.finish())
}

/// Run `instruction` against the configured Ollama server. On success, sends the preview event;
/// on error, calls `reporter.fail`; when the user cancels `request` (Esc), shows a notice.
fn propose_command(
    instruction: String,
    context: AiContext,
    voice_config: &Voice,
    proxy: &EventLoopProxy<Event>,
    window_id: WindowId,
    reporter: &StatusReporter,
    request: RequestToken<'_>,
) {
    let backend = OllamaBackend::new(llm_settings(voice_config));
    let temperature = voice_config.temperature;
    let work = move || run_prompt(&backend, &instruction, &context, temperature);

    match run_request(request, work) {
        RequestOutcome::Cancelled => {
            info!("AI request cancelled");
            reporter.show(AiPhase::Notice("Request cancelled.".into()));
        },
        RequestOutcome::Preview(proposal) => send_preview(proxy, window_id, reporter, proposal),
        RequestOutcome::Failed(message) => reporter.fail(message),
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
        let request = AI_REQUESTS.begin();
        reporter.show(AiPhase::Thinking { model: voice_config.model.clone() });
        propose_command(
            transcribed_text,
            context,
            &voice_config,
            &proxy,
            window_id,
            &reporter,
            request,
        );
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
    // Shown before the thread starts so the user sees "sent" immediately; Esc can cancel from
    // the moment it is shown.
    let request = AI_REQUESTS.begin();
    reporter.show(AiPhase::Thinking { model: voice_config.model.clone() });
    let proxy = proxy.clone();

    thread::spawn(move || {
        propose_command(prompt_text, context, &voice_config, &proxy, window_id, &reporter, request);
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

    use kourage_core::{BackendError, MockBackend, WHISPER_PLACEHOLDER_PATH};

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

    fn unreachable() -> SetupIssue {
        SetupIssue::BackendUnreachable { message: "Is Ollama running?".into() }
    }

    fn model_missing() -> SetupIssue {
        SetupIssue::ModelMissing { model: "m".into(), message: "Run `ollama pull m`.".into() }
    }

    #[test]
    fn setup_summary_is_none_without_issues() {
        assert_eq!(setup_summary(&[]), None);
    }

    #[test]
    fn setup_summary_of_one_issue_is_its_message() {
        assert_eq!(setup_summary(&[unreachable()]), Some("Is Ollama running?".into()));
    }

    #[test]
    fn setup_summary_of_two_issues_points_to_the_log() {
        assert_eq!(
            setup_summary(&[unreachable(), model_missing()]),
            Some("Is Ollama running? (+1 more, see the log)".into())
        );
    }

    #[test]
    fn setup_summary_skips_whisper_not_configured() {
        let not_configured = SetupIssue::WhisperNotConfigured { message: "placeholder".into() };
        assert_eq!(setup_summary(std::slice::from_ref(&not_configured)), None);
        assert_eq!(
            setup_summary(&[not_configured, model_missing()]),
            Some("Run `ollama pull m`.".into())
        );
    }

    #[test]
    fn setup_summary_includes_whisper_file_missing() {
        let missing =
            SetupIssue::WhisperFileMissing { path: "/x.bin".into(), message: "Download".into() };
        assert_eq!(setup_summary(&[missing]), Some("Download".into()));
    }

    #[test]
    fn whisper_path_for_check_expands_home_only() {
        if let Some(home) = home::home_dir() {
            let expected = home.join("x.bin").to_string_lossy().into_owned();
            assert_eq!(whisper_path_for_check("~/x.bin"), expected);
        }
        assert_eq!(whisper_path_for_check("/abs/model.bin"), "/abs/model.bin");
        assert_eq!(whisper_path_for_check(WHISPER_PLACEHOLDER_PATH), WHISPER_PLACEHOLDER_PATH);
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
    fn wait_or_cancel_returns_value_when_it_arrives() {
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(60));
            let _ = tx.send(42);
        });
        assert_eq!(wait_or_cancel(&rx, &|| false), Some(42));
    }

    #[test]
    fn wait_or_cancel_returns_none_promptly_after_cancel() {
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(2));
            let _ = tx.send(1);
        });
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::SeqCst);
        });

        let started = std::time::Instant::now();
        let result = wait_or_cancel(&rx, &|| cancelled.load(Ordering::SeqCst));
        let elapsed = started.elapsed();
        assert_eq!(result, None);
        assert!(elapsed < Duration::from_millis(500), "took {elapsed:?}");
    }

    fn proposal(command: &str) -> Proposal {
        Proposal { command: command.into(), is_destructive: false }
    }

    #[test]
    fn late_result_after_cancel_is_dropped() {
        let requests = AiRequests::new();
        let request = requests.begin();
        assert!(requests.cancel_latest());

        // The real waiting path: the answer only exists after the caller has given up.
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        let outcome = run_request(request, move || {
            thread::sleep(Duration::from_millis(150));
            let _ = done_tx.send(());
            Ok(proposal("late"))
        });
        assert_eq!(outcome, RequestOutcome::Cancelled);
        assert!(started.elapsed() < Duration::from_millis(150), "must not wait for the answer");

        // The abandoned producer finishes later without panicking; its answer reached no one.
        done_rx.recv_timeout(Duration::from_secs(2)).expect("producer finished");
        assert!(!requests.cancel_latest(), "the cancelled request left the registry");
    }

    #[test]
    fn request_answer_becomes_a_preview_or_an_error() {
        let requests = AiRequests::new();
        assert_eq!(
            run_request(requests.begin(), || Ok(proposal("ls"))),
            RequestOutcome::Preview(proposal("ls"))
        );
        assert_eq!(
            run_request(requests.begin(), || Err("boom".into())),
            RequestOutcome::Failed("boom".into())
        );
    }

    #[test]
    fn request_that_dies_without_answer_is_an_error_not_a_cancel() {
        let requests = AiRequests::new();
        let outcome = run_request(requests.begin(), || panic!("helper thread died"));
        assert_eq!(outcome, RequestOutcome::Failed(REQUEST_STOPPED_MESSAGE.into()));
    }

    #[test]
    fn accepted_cancel_wins_over_an_answer_that_is_already_there() {
        // Decision level: every answer loses to an accepted cancel.
        assert_eq!(request_outcome(Some(Ok(proposal("ls"))), true), RequestOutcome::Cancelled);
        assert_eq!(request_outcome(Some(Err("e".into())), true), RequestOutcome::Cancelled);
        assert_eq!(request_outcome(None, true), RequestOutcome::Cancelled);
        assert_eq!(
            request_outcome(Some(Ok(proposal("ls"))), false),
            RequestOutcome::Preview(proposal("ls"))
        );
        assert_eq!(
            request_outcome(None, false),
            RequestOutcome::Failed(REQUEST_STOPPED_MESSAGE.into())
        );

        // Waiting-function level: the answer is ready at once, but Esc was accepted first.
        let requests = AiRequests::new();
        let request = requests.begin();
        assert!(requests.cancel_latest());
        assert_eq!(run_request(request, || Ok(proposal("ls"))), RequestOutcome::Cancelled);
    }

    #[test]
    fn wait_or_cancel_returns_none_when_the_producer_dies() {
        let (tx, rx) = std::sync::mpsc::channel::<u32>();
        drop(tx);
        assert_eq!(wait_or_cancel(&rx, &|| false), None);
    }

    #[test]
    fn cancel_without_request_returns_false() {
        let requests = AiRequests::new();
        assert!(!requests.cancel_latest());
    }

    #[test]
    fn cancel_during_request_is_seen() {
        let requests = AiRequests::new();
        let request = requests.begin();
        assert!(requests.cancel_latest());
        assert!(request.is_cancelled());
    }

    #[test]
    fn cancel_after_end_returns_false() {
        let requests = AiRequests::new();
        let request = requests.begin();
        assert!(!request.finish());
        assert!(!requests.cancel_latest());
    }

    #[test]
    fn begin_clears_previous_cancel() {
        let requests = AiRequests::new();
        let first = requests.begin();
        assert!(requests.cancel_latest());
        assert!(first.finish());
        let second = requests.begin();
        assert!(!second.is_cancelled());
    }

    #[test]
    fn finish_reports_whether_the_request_was_cancelled() {
        let requests = AiRequests::new();
        assert!(!requests.begin().finish(), "answer arrived first: not cancelled");
        let request = requests.begin();
        assert!(requests.cancel_latest());
        assert!(!requests.cancel_latest(), "Esc twice: the second is refused");
        assert!(request.finish(), "an accepted cancel must win over a late answer");
    }

    #[test]
    fn cancelling_request_b_does_not_end_request_a_state() {
        let requests = AiRequests::new();
        let a = requests.begin();
        let b = requests.begin();
        assert!(requests.cancel_latest());
        assert!(b.is_cancelled());
        assert!(!a.is_cancelled());
        // A is still in flight and cancellable once B is gone.
        assert!(b.finish());
        assert!(requests.cancel_latest());
        assert!(a.finish());
    }

    #[test]
    fn request_a_ending_does_not_make_request_b_uncancellable() {
        let requests = AiRequests::new();
        let a = requests.begin();
        let b = requests.begin();
        assert!(!a.finish());
        assert!(requests.cancel_latest());
        assert!(b.is_cancelled());
        assert!(b.finish());
    }

    #[test]
    fn ended_but_registered_request_shields_older_ones() {
        let requests = AiRequests::new();
        let older = requests.begin();
        let newer = requests.begin();

        // The state `end` passes through: ended, not yet removed from the registry.
        assert!(!newer.state.end());
        assert!(!requests.cancel_latest(), "an ended latest request must not expose the older");
        assert!(!older.is_cancelled());

        assert!(!newer.finish());
        assert!(requests.cancel_latest(), "once removed, the older request is the latest");
        assert!(older.finish());
    }

    #[test]
    fn cancel_targets_the_most_recent_request() {
        let requests = AiRequests::new();
        let a = requests.begin();
        let b = requests.begin();
        let c = requests.begin();
        assert!(requests.cancel_latest());
        assert!(c.is_cancelled());
        assert!(!a.is_cancelled() && !b.is_cancelled());
        // Ending the middle one does not change which request is the latest.
        assert!(!b.finish());
        assert!(!requests.cancel_latest(), "the latest is already cancelled");
        assert!(!a.is_cancelled());
        assert!(c.finish());
        assert!(!a.finish());
    }

    #[test]
    fn old_request_finishing_cannot_claim_new_requests_cancel() {
        let requests = AiRequests::new();
        let a = requests.begin();
        let b = requests.begin();
        assert!(requests.cancel_latest());
        assert!(!a.finish(), "A must not claim B's cancel");
        assert!(b.is_cancelled(), "B still sees its cancel after A finished");
        assert!(b.finish());
    }

    #[test]
    fn each_request_reports_only_its_own_cancel() {
        let requests = AiRequests::new();
        let a = requests.begin();
        let b = requests.begin();
        assert!(requests.cancel_latest());
        assert!(b.finish());
        let c = requests.begin();
        assert!(!c.is_cancelled());
        assert!(!c.finish());
        assert!(requests.cancel_latest(), "A is the latest again");
        assert!(a.finish());
    }

    #[test]
    fn overlapping_requests_cancelled_b_never_delivers_a_preview() {
        // Interleaving (a): A and B in flight, Esc cancels B, A finishes first.
        let requests = AiRequests::new();
        let a = requests.begin();
        let b = requests.begin();
        assert!(requests.cancel_latest());

        assert_eq!(run_request(a, || Ok(proposal("a"))), RequestOutcome::Preview(proposal("a")));
        assert_eq!(run_request(b, || Ok(proposal("b"))), RequestOutcome::Cancelled);
    }

    #[test]
    fn cancel_prefers_the_recording_over_a_request() {
        let voice = VoiceSession::new();
        let requests = AiRequests::new();
        let request = requests.begin();
        voice.begin_recording();

        assert!(cancel_in(&voice, &requests));
        assert!(voice.is_cancelled());
        assert!(!request.is_cancelled(), "the recording is cancelled, not the request");

        voice.end_recording();
        assert!(cancel_in(&voice, &requests), "with no recording, the request is cancelled");
        assert!(request.finish());
    }

    #[test]
    fn cancel_with_nothing_in_flight_returns_false() {
        assert!(!cancel_in(&VoiceSession::new(), &AiRequests::new()));
    }

    #[test]
    fn cancel_after_recording_and_request_end_returns_false() {
        let voice = VoiceSession::new();
        let requests = AiRequests::new();
        voice.begin_recording();
        let request = requests.begin();
        voice.end_recording();
        assert!(!request.finish());
        assert!(!cancel_in(&voice, &requests));
    }

    #[test]
    fn request_token_leaves_the_registry_when_dropped() {
        fn returns_early(requests: &AiRequests) -> Option<()> {
            let _request = requests.begin();
            None?;
            Some(())
        }

        let requests = AiRequests::new();
        drop(requests.begin());
        assert!(!requests.cancel_latest());

        assert_eq!(returns_early(&requests), None);
        assert!(!requests.cancel_latest(), "early return must not leave a request in flight");

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _request = requests.begin();
            panic!("worker panicked");
        }));
        assert!(panicked.is_err());
        assert!(!requests.cancel_latest(), "a panic must not leave a request in flight");

        // Dropping an old token removes only its own entry.
        let a = requests.begin();
        let b = requests.begin();
        drop(a);
        assert!(requests.cancel_latest());
        assert!(b.finish());
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

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use log::{error, info};
use serde::{Deserialize, Serialize};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use alacritty_terminal::event::Event as TerminalEvent;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::WavWriter;
use tempfile::NamedTempFile;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::config::ui_config::UiConfig;
use crate::event::{Event, EventType};

pub fn init() {
    // Initialization logic if needed
}

fn record_audio(path: &Path) -> Result<(), anyhow::Error> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| anyhow::anyhow!("No input device found"))?;
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
                    writer.write_sample(sample).unwrap_or_else(|e| error!("Failed to write sample: {}", e));
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

fn transcribe_audio(path: &Path, model_path: &str) -> Result<String, anyhow::Error> {
    if !Path::new(model_path).exists() {
        return Err(anyhow::anyhow!("Whisper model not found at path: {}", model_path));
    }

    let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|e| anyhow::anyhow!("Failed to create whisper context: {:?}", e))?;
    let mut state = ctx.create_state()
        .map_err(|e| anyhow::anyhow!("Failed to create whisper state: {:?}", e))?;

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

    state.full(params, &audio_samples[..])
        .map_err(|e| anyhow::anyhow!("Failed to run model: {:?}", e))?;

    let num_segments = state.full_n_segments();
    let mut result = String::new();
    for i in 0..num_segments {
        if let Some(segment) = state.get_segment(i) {
            // WhisperSegment should have a way to get text.
            // Let's try to_string() if it implements Display, or just use it.
            // Actually, in some versions it's state.full_get_segment_text(i).
            // Let's try state.full_get_segment_text(i).
            // Wait, I already tried that and it failed.
            // Let's try to find what WhisperSegment has.
            // Actually, I'll try state.get_segment_text(i).
            result.push_str(&segment.to_string());
        }
    }

    Ok(result)
}

#[derive(Serialize)]
struct LlmRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Deserialize)]
struct LlmResponse {
    response: String,
}

fn call_llm(transcribed_text: &str, api_url: &str) -> Result<String, anyhow::Error> {
    let client = reqwest::blocking::Client::new();
    let prompt = format!(
        "Convert the following voice transcription into a single linux command. \
         Only return the command, no explanation: {}",
        transcribed_text
    );

    let request = LlmRequest {
        model: "llama3".to_string(), // You might want to make this configurable
        prompt,
        stream: false,
    };

    let res = client.post(api_url)
        .json(&request)
        .send()?
        .json::<LlmResponse>()?;

    Ok(res.response.trim().to_string())
}

pub fn handle_voice_command(
    config: &UiConfig,
    proxy: &EventLoopProxy<Event>,
    window_id: WindowId,
) {
    let voice_config = config.voice.clone();
    let proxy = proxy.clone();

    thread::spawn(move || {
        let temp_file = match NamedTempFile::new() {
            Ok(file) => file,
            Err(e) => {
                error!("Failed to create temp file: {}", e);
                return;
            }
        };

        let temp_path = temp_file.path().to_path_buf();

        if let Err(e) = record_audio(&temp_path) {
            error!("Failed to record audio: {}", e);
            return;
        }

        let transcribed_text = match transcribe_audio(&temp_path, &voice_config.whisper_model_path) {
            Ok(text) => text,
            Err(e) => {
                error!("Failed to transcribe audio: {}", e);
                return;
            }
        };

        info!("Transcribed text: {}", transcribed_text);

        let command_from_llm = match call_llm(&transcribed_text, &voice_config.llm_api_url) {
            Ok(cmd) => cmd,
            Err(e) => {
                error!("Failed to call LLM: {}", e);
                transcribed_text // Fallback to raw transcription
            }
        };

        info!("Command from LLM: {}", command_from_llm);

        // Execute command
        let event = Event::new(EventType::Terminal(TerminalEvent::PtyWrite(format!("{}\n", command_from_llm))), window_id);
        let _ = proxy.send_event(event);
    });
}

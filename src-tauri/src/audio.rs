//! Microphone capture (cpal) and conversion to 16 kHz mono f32.

use anyhow::{anyhow, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SizedSample, Stream,
};
use std::{
    path::Path,
    sync::{
        mpsc::{self, Sender},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub const SAMPLE_RATE: u32 = 16_000;
const MIN_SAMPLES: usize = SAMPLE_RATE as usize * 3 / 10;
const SILENCE_PEAK: f32 = 0.015;

pub fn list_microphones() -> Vec<String> {
    let devices = cpal::default_host().input_devices().map(|d| d.collect::<Vec<_>>()).unwrap_or_default();
    devices.iter().filter_map(|d| d.name().ok()).collect()
}

/// False for clips under 0.3 s or clips that hold only silence.
pub fn has_speech(samples: &[f32]) -> bool {
    samples.len() >= MIN_SAMPLES && samples.iter().any(|s| s.abs() >= SILENCE_PEAK)
}

pub struct Recording {
    stop: Sender<()>,
    thread: JoinHandle<Vec<f32>>,
}

impl Recording {
    /// Starts capturing. `on_level` gets a 0..1 loudness value about 20 times per second.
    pub fn start(microphone: Option<&str>, on_level: impl Fn(f32) + Send + 'static) -> Result<Self> {
        let microphone = microphone.map(str::to_string);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        // cpal streams cannot move between threads, so one thread owns the stream.
        let thread = std::thread::spawn(move || {
            let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
            let opened = open_stream(microphone.as_deref(), buffer.clone(), on_level);
            let (stream, rate) = match opened {
                Ok(opened) => {
                    let _ = ready_tx.send(Ok(()));
                    opened
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("{error:#}")));
                    return vec![];
                }
            };
            let _ = stop_rx.recv();
            drop(stream);
            let samples = std::mem::take(&mut *buffer.lock().unwrap());
            resample(&samples, rate, SAMPLE_RATE)
        });

        ready_rx.recv()?.map_err(|e| anyhow!(e))?;
        Ok(Self { stop: stop_tx, thread })
    }

    /// Stops capturing and returns 16 kHz mono samples.
    pub fn finish(self) -> Vec<f32> {
        let _ = self.stop.send(());
        let samples = self.thread.join().unwrap_or_default();
        // Test hook: replaces the microphone audio with a WAV file.
        match std::env::var("MURMUR_FAKE_WAV") {
            Ok(path) => read_wav(Path::new(&path)).unwrap_or(samples),
            Err(_) => samples,
        }
    }
}

fn open_stream(microphone: Option<&str>, buffer: Arc<Mutex<Vec<f32>>>, on_level: impl Fn(f32) + Send + 'static) -> Result<(Stream, u32)> {
    let host = cpal::default_host();
    let device = match microphone {
        Some(name) => host.input_devices()?.find(|d| d.name().is_ok_and(|n| n == name)),
        None => None,
    }
    .or_else(|| host.default_input_device())
    .ok_or_else(|| anyhow!("No microphone found"))?;

    let config = device.default_input_config()?;
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let stream_config = config.config();
    log::info!("Recording from {:?} at {rate} Hz, {channels} ch", device.name().unwrap_or_default());

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &stream_config, channels, buffer, on_level)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &stream_config, channels, buffer, on_level)?,
        cpal::SampleFormat::I32 => build::<i32>(&device, &stream_config, channels, buffer, on_level)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &stream_config, channels, buffer, on_level)?,
        other => return Err(anyhow!("Unsupported sample format {other:?}")),
    };
    stream.play()?;
    Ok((stream, rate))
}

fn build<T: SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    buffer: Arc<Mutex<Vec<f32>>>,
    on_level: impl Fn(f32) + Send + 'static,
) -> Result<Stream>
where
    f32: FromSample<T>,
{
    let mut last_level = Instant::now();
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _| {
            let mono: Vec<f32> = data
                .chunks(channels)
                .map(|frame| frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / channels as f32)
                .collect();
            if last_level.elapsed() >= Duration::from_millis(50) {
                last_level = Instant::now();
                let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len().max(1) as f32).sqrt();
                on_level((rms * 12.0).sqrt().min(1.0));
            }
            buffer.lock().unwrap().extend_from_slice(&mono);
        },
        |error| log::error!("Microphone stream error: {error}"),
        None,
    );
    stream.context("Could not open the microphone")
}

/// Averages the source samples that fall into each output sample. Good enough for speech.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let count = (input.len() as f64 / ratio) as usize;
    (0..count)
        .map(|i| {
            let start = (i as f64 * ratio) as usize;
            let end = (((i + 1) as f64 * ratio) as usize).clamp(start + 1, input.len());
            input[start..end].iter().sum::<f32>() / (end - start) as f32
        })
        .collect()
}

/// Reads any WAV file as 16 kHz mono.
pub fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.map(|v| v as f32 / scale)).collect::<Result<_, _>>()?
        }
    };
    let channels = spec.channels as usize;
    let mono: Vec<f32> = samples.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect();
    Ok(resample(&mono, spec.sample_rate, SAMPLE_RATE))
}

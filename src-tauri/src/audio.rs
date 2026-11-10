//! Microphone capture (cpal) and conversion to 16 kHz mono f32.

use anyhow::{anyhow, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SizedSample, Stream,
};
use std::{
    fs::File,
    io::ErrorKind,
    path::Path,
    sync::{
        mpsc::{self, Sender},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use symphonia::core::{audio::SampleBuffer, codecs::CODEC_TYPE_NULL, errors::Error as SymphoniaError, io::MediaSourceStream, probe::Hint};

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
        // Test hook: replaces the microphone audio with an audio file.
        match std::env::var("MURMUR_FAKE_WAV") {
            Ok(path) => read_file(Path::new(&path)).unwrap_or(samples),
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
struct Resampler {
    ratio: f64,
    /// Source samples pushed so far.
    seen: usize,
    /// Index of the output sample that is being built.
    index: usize,
    sum: f32,
    count: usize,
    last: f32,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self { ratio: from as f64 / to as f64, seen: 0, index: 0, sum: 0.0, count: 0, last: 0.0 }
    }

    /// Appends the output samples that this source sample completes.
    fn push(&mut self, sample: f32, out: &mut Vec<f32>) {
        self.sum += sample;
        self.count += 1;
        self.seen += 1;
        self.last = sample;
        loop {
            let start = (self.index as f64 * self.ratio) as usize;
            let end = (((self.index + 1) as f64 * self.ratio) as usize).max(start + 1);
            if self.seen < end {
                break;
            }
            // Upsampling repeats a source sample, so a window can have no new samples.
            out.push(if self.count == 0 { self.last } else { self.sum / self.count as f32 });
            self.sum = 0.0;
            self.count = 0;
            self.index += 1;
        }
    }
}

fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    let mut resampler = Resampler::new(from, to);
    let mut out = Vec::with_capacity((input.len() as f64 / resampler.ratio) as usize);
    for &sample in input {
        resampler.push(sample, &mut out);
    }
    out
}

/// Decodes an audio file (mp3, wav, m4a, aac, flac, ogg) into 16 kHz mono samples.
pub fn read_file(path: &Path) -> Result<Vec<f32>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe().format(&hint, source, &Default::default(), &Default::default())?;
    let mut format = probed.format;
    // Video files list the video track too, which has no codec that symphonia knows.
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no audio track"))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &Default::default())?;

    let mut out = vec![];
    let mut resampler = None;
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::ResetRequired) => break,
            Err(error) => return Err(error.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(error.into()),
        };
        let spec = *decoded.spec();
        let samples = match &mut buffer {
            Some(buffer) if buffer.capacity() >= decoded.capacity() * spec.channels.count() => buffer,
            slot => slot.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
        };
        samples.copy_interleaved_ref(decoded);
        let resampler = resampler.get_or_insert_with(|| Resampler::new(spec.rate, SAMPLE_RATE));
        let channels = spec.channels.count();
        for frame in samples.samples().chunks(channels) {
            resampler.push(frame.iter().sum::<f32>() / channels as f32, &mut out);
        }
    }
    Ok(out)
}

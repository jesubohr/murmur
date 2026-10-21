//! Speech to text: NVIDIA Parakeet TDT 0.6B v3 (int8 ONNX) with greedy TDT decoding.
//! The mel preprocessor and the small decoder run on the CPU; the encoder runs on DirectML or CPU.

use crate::{gpu, settings};
use anyhow::{anyhow, Context, Result};
use ort::{
    ep::{DirectML, ExecutionProviderDispatch, CPU},
    inputs,
    session::{builder::GraphOptimizationLevel, Session},
    value::{Tensor, TensorRef},
};
use std::{fs, path::Path, time::Instant};

/// Part of the cache file name, because the saved graph depends on the ONNX Runtime version.
const ORT_VERSION: &str = "2.0.0-rc.12";
const MAX_TOKENS_PER_FRAME: usize = 10;
/// Silence added before the audio. The model drops the first word without it.
const LEAD_SILENCE_SAMPLES: usize = 4000;
/// DirectML recompiles the encoder for every new input length and keeps only the first compiled
/// shape. So on the GPU the encoder always gets exactly this many frames: shorter audio is padded,
/// longer audio is split into chunks of at most `MAX_CHUNK_SAMPLES`.
const FIXED_FRAMES: usize = 1280;
const MAX_CHUNK_SAMPLES: usize = 192_000;
const SPLIT_WINDOW_SAMPLES: usize = 32_000;
const SPLIT_FRAME_SAMPLES: usize = 800;

pub struct Stt {
    preprocessor: Session,
    encoder: Session,
    decoder: Session,
    vocab: Vec<String>,
    blank: i32,
    state_shapes: [Vec<i64>; 2],
    fixed_length: bool,
    pub device_label: String,
}

impl Stt {
    pub fn load(dir: &Path, device: &str) -> Result<Self> {
        let load_started = Instant::now();
        let (encoder_eps, device_label, device_key) = match (device, gpu::pick_discrete()) {
            ("gpu", Some(adapter)) => {
                log::info!("STT encoder on DirectML adapter {} ({})", adapter.index, adapter.name);
                let dml = DirectML::default().with_device_id(adapter.index).build().error_on_failure();
                (vec![dml], format!("DirectML · {}", adapter.name), format!("dml-{}", adapter.name.replace(|c: char| !c.is_ascii_alphanumeric(), "_")))
            }
            ("gpu", None) => {
                log::warn!("No discrete GPU found, STT runs on the CPU");
                (vec![], "CPU".to_string(), "cpu".to_string())
            }
            _ => (vec![], "CPU".to_string(), "cpu".to_string()),
        };
        let on_gpu = !encoder_eps.is_empty();

        let started = Instant::now();
        let (preprocessor, encoder, decoder) = std::thread::scope(|scope| {
            let preprocessor = scope.spawn(|| build_session("preprocessor", &dir.join("nemo128.onnx"), vec![], false, GraphOptimizationLevel::Level3, None));
            let decoder = scope.spawn(|| build_session("decoder", &dir.join("decoder_joint-model.int8.onnx"), vec![], false, GraphOptimizationLevel::Level3, None));
            let encoder = load_encoder(&dir.join("encoder-model.int8.onnx"), encoder_eps, on_gpu, &device_key);
            let joined = |name: &str, thread: std::thread::ScopedJoinHandle<Result<Session>>| thread.join().unwrap_or_else(|_| Err(anyhow!("{name} loading panicked")));
            (joined("preprocessor", preprocessor), encoder, joined("decoder", decoder))
        });
        let (preprocessor, encoder, decoder) = (preprocessor?, encoder?, decoder?);
        let sessions_ms = started.elapsed().as_millis();

        let state_shape = |name: &str| -> Result<Vec<i64>> {
            let input = decoder.inputs().iter().find(|i| i.name() == name);
            let shape = input.and_then(|i| i.dtype().tensor_shape()).ok_or_else(|| anyhow!("decoder input {name} missing"))?;
            Ok(vec![shape[0], 1, shape[2]])
        };
        let state_shapes = [state_shape("input_states_1")?, state_shape("input_states_2")?];

        let vocab = load_vocab(&dir.join("vocab.txt"))?;
        let blank = vocab.iter().position(|t| t == "<blk>").ok_or_else(|| anyhow!("<blk> missing in vocab"))? as i32;
        log::info!("STT loaded in {} ms (sessions {} ms)", load_started.elapsed().as_millis(), sessions_ms);

        Ok(Self { preprocessor, encoder, decoder, vocab, blank, state_shapes, fixed_length: on_gpu, device_label })
    }

    /// The first run compiles the encoder, so do it before the user dictates.
    pub fn warm_up(&mut self) -> Result<()> {
        self.transcribe(&vec![0.0; 32_000]).map(|_| ())
    }

    /// Takes 16 kHz mono samples.
    pub fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        if !self.fixed_length {
            return self.transcribe_chunk(samples);
        }
        let mut texts = vec![];
        let mut rest = samples;
        while !rest.is_empty() {
            let end = if rest.len() <= MAX_CHUNK_SAMPLES { rest.len() } else { quietest_split(rest) };
            let (chunk, tail) = rest.split_at(end);
            let text = self.transcribe_chunk(chunk)?;
            if !text.is_empty() {
                texts.push(text);
            }
            rest = tail;
        }
        Ok(texts.join(" "))
    }

    fn transcribe_chunk(&mut self, samples: &[f32]) -> Result<String> {
        let mut padded = vec![0.0f32; LEAD_SILENCE_SAMPLES];
        padded.extend_from_slice(samples);
        let (features, feature_shape, feature_len) = self.preprocess(&padded)?;
        let (encoded, frames, dim) = self.encode(&features, feature_shape, feature_len)?;
        let tokens = self.decode(&encoded, frames, dim)?;
        Ok(self.tokens_to_text(&tokens))
    }

    fn preprocess(&mut self, samples: &[f32]) -> Result<(Vec<f32>, Vec<i64>, i64)> {
        let wave = TensorRef::from_array_view(([1usize, samples.len()], samples))?;
        let lens = Tensor::from_array(([1usize], vec![samples.len() as i64]))?;
        let out = self.preprocessor.run(inputs!["waveforms" => wave, "waveforms_lens" => lens])?;
        let (shape, features) = out["features"].try_extract_tensor::<f32>()?;
        let (_, lens) = out["features_lens"].try_extract_tensor::<i64>()?;
        Ok((features.to_vec(), shape.to_vec(), lens[0]))
    }

    /// Returns the encoder output laid out as `[dim][frames]`, as the model emits it.
    fn encode(&mut self, features: &[f32], shape: Vec<i64>, len: i64) -> Result<(Vec<f32>, usize, usize)> {
        let (features, shape) = if self.fixed_length { pad_frames(features, &shape) } else { (features.to_vec(), shape) };
        let signal = TensorRef::from_array_view((shape, &*features))?;
        let length = Tensor::from_array(([1usize], vec![len]))?;
        let out = self.encoder.run(inputs!["audio_signal" => signal, "length" => length])?;
        let (shape, data) = out["outputs"].try_extract_tensor::<f32>()?;
        let (_, lens) = out["encoded_lengths"].try_extract_tensor::<i64>()?;
        let dim = shape[1] as usize;
        let total_frames = shape[2] as usize;
        let frames = (lens[0] as usize).min(total_frames);
        // Keep only the valid frames, still as [dim][frames].
        let mut trimmed = Vec::with_capacity(dim * frames);
        for d in 0..dim {
            trimmed.extend_from_slice(&data[d * total_frames..d * total_frames + frames]);
        }
        Ok((trimmed, frames, dim))
    }

    /// Greedy TDT decoding: each step predicts a token and how many frames to skip.
    fn decode(&mut self, encoded: &[f32], frames: usize, dim: usize) -> Result<Vec<i32>> {
        let vocab_size = self.vocab.len();
        let mut state1 = vec![0.0f32; self.state_shapes[0].iter().product::<i64>() as usize];
        let mut state2 = vec![0.0f32; self.state_shapes[1].iter().product::<i64>() as usize];
        let mut tokens: Vec<i32> = vec![];
        let (mut t, mut emitted) = (0usize, 0usize);
        let mut frame = vec![0.0f32; dim];

        while t < frames {
            for (d, value) in frame.iter_mut().enumerate() {
                *value = encoded[d * frames + t];
            }
            let last = tokens.last().copied().unwrap_or(self.blank);
            let frame_t = TensorRef::from_array_view(([1usize, dim, 1], &*frame))?;
            let targets = Tensor::from_array(([1usize, 1], vec![last]))?;
            let target_len = Tensor::from_array(([1usize], vec![1i32]))?;
            let s1 = TensorRef::from_array_view((self.state_shapes[0].clone(), &*state1))?;
            let s2 = TensorRef::from_array_view((self.state_shapes[1].clone(), &*state2))?;
            let out = self.decoder.run(inputs![
                "encoder_outputs" => frame_t,
                "targets" => targets,
                "target_length" => target_len,
                "input_states_1" => s1,
                "input_states_2" => s2,
            ])?;
            let (_, logits) = out["outputs"].try_extract_tensor::<f32>()?;
            let token = argmax(&logits[..vocab_size]) as i32;
            let skip = argmax(&logits[vocab_size..]);

            if token != self.blank {
                state1 = out["output_states_1"].try_extract_tensor::<f32>()?.1.to_vec();
                state2 = out["output_states_2"].try_extract_tensor::<f32>()?.1.to_vec();
                tokens.push(token);
                emitted += 1;
            }
            if skip > 0 {
                t += skip;
                emitted = 0;
            } else if token == self.blank || emitted == MAX_TOKENS_PER_FRAME {
                t += 1;
                emitted = 0;
            }
        }
        Ok(tokens)
    }

    fn tokens_to_text(&self, tokens: &[i32]) -> String {
        let joined: String = tokens
            .iter()
            .filter_map(|&id| self.vocab.get(id as usize))
            .filter(|token| !token.starts_with("<|"))
            .map(|token| token.replace('▁', " "))
            .collect();
        joined.trim().to_string()
    }
}

/// The first load saves the optimized encoder graph. Later loads use that file and skip the optimization.
fn load_encoder(original: &Path, providers: Vec<ExecutionProviderDispatch>, directml: bool, device_key: &str) -> Result<Session> {
    let cached = settings::app_dir().join("cache").join(format!("encoder-{device_key}-ort{ORT_VERSION}.onnx"));
    if cached.exists() {
        match build_session("encoder (saved graph)", &cached, providers.clone(), directml, GraphOptimizationLevel::Disable, None) {
            Ok(session) => return Ok(session),
            Err(error) => {
                log::warn!("Saved encoder graph failed ({error:#}), loading the original");
                let _ = fs::remove_file(&cached);
            }
        }
    }
    let _ = fs::create_dir_all(cached.parent().unwrap());
    build_session("encoder", original, providers, directml, GraphOptimizationLevel::Level3, Some(&cached))
}

fn build_session(
    name: &str,
    path: &Path,
    providers: Vec<ExecutionProviderDispatch>,
    directml: bool,
    level: GraphOptimizationLevel,
    save_optimized_to: Option<&Path>,
) -> Result<Session> {
    let started = Instant::now();
    let mut builder = Session::builder()
        .map_err(oe)?
        .with_optimization_level(level)
        .map_err(oe)?;
    if let Some(path) = save_optimized_to {
        builder = builder.with_optimized_model_path(path).map_err(oe)?;
    }
    if directml {
        // Required by the DirectML execution provider.
        builder = builder.with_parallel_execution(false).map_err(oe)?.with_memory_pattern(false).map_err(oe)?;
    }
    let mut all = providers;
    all.push(CPU::default().build());
    let session = builder
        .with_execution_providers(all)
        .map_err(oe)?
        .commit_from_file(path)
        .with_context(|| format!("loading {}", path.display()))?;
    log::info!("STT {name} session loaded in {} ms", started.elapsed().as_millis());
    Ok(session)
}

/// Some ort errors carry the session builder and are not Send, so keep only the message.
fn oe<T>(error: ort::Error<T>) -> anyhow::Error {
    anyhow!("{error}")
}

fn load_vocab(path: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut vocab: Vec<String> = vec![];
    for line in text.lines() {
        let Some((token, id)) = line.rsplit_once(' ') else { continue };
        let id: usize = id.parse()?;
        if vocab.len() <= id {
            vocab.resize(id + 1, String::new());
        }
        vocab[id] = token.to_string();
    }
    Ok(vocab)
}

/// Zero-pads `[1, bins, frames]` features to `FIXED_FRAMES` frames.
fn pad_frames(features: &[f32], shape: &[i64]) -> (Vec<f32>, Vec<i64>) {
    let (bins, frames) = (shape[1] as usize, shape[2] as usize);
    let padded = FIXED_FRAMES;
    let mut out = vec![0.0f32; bins * padded];
    for b in 0..bins {
        out[b * padded..b * padded + frames].copy_from_slice(&features[b * frames..(b + 1) * frames]);
    }
    (out, vec![1, bins as i64, padded as i64])
}

/// Picks the quietest 50 ms near the end of the chunk, so words are rarely cut.
fn quietest_split(samples: &[f32]) -> usize {
    let energy = |start: usize| samples[start..start + SPLIT_FRAME_SAMPLES].iter().map(|s| s * s).sum::<f32>();
    let first = MAX_CHUNK_SAMPLES - SPLIT_WINDOW_SAMPLES;
    let quietest = (first..MAX_CHUNK_SAMPLES - SPLIT_FRAME_SAMPLES)
        .step_by(SPLIT_FRAME_SAMPLES)
        .min_by(|&a, &b| energy(a).total_cmp(&energy(b)))
        .unwrap_or(first);
    quietest + SPLIT_FRAME_SAMPLES / 2
}

fn argmax(values: &[f32]) -> usize {
    let mut best = 0;
    for (i, v) in values.iter().enumerate() {
        if *v > values[best] {
            best = i;
        }
    }
    best
}

//! Samples in, text out: speech to text, optional LLM cleanup, replacements.

use crate::{audio, llm::Llm, settings::Settings, stt::Stt};
use anyhow::Result;
use serde::Serialize;
use std::time::Instant;

/// The LLM context holds 4096 tokens and the few-shot prompt uses a big part of it.
const CLEANUP_CHUNK_CHARS: usize = 1200;

#[derive(Clone, Copy)]
pub enum Stage {
    Transcribing,
    Polishing,
}

#[derive(Serialize, Clone, Debug)]
pub struct Output {
    pub raw: String,
    pub cleaned: String,
    pub audio_s: f32,
    pub cleanup_used: bool,
    pub stt_ms: u64,
    pub llm_ms: u64,
    pub total_ms: u64,
    pub llm_tokens_per_s: f64,
}

/// Returns `None` when there is nothing to paste (too short, silent or empty).
/// `on_progress` gets each stage with a 0..1 value. Every stage starts with 0.0. An error from it stops the work.
pub fn run(stt: &mut Stt, llm: Option<&Llm>, settings: &Settings, samples: &[f32], on_progress: impl Fn(Stage, f32) -> Result<()>) -> Result<Option<Output>> {
    if !audio::has_speech(samples) {
        return Ok(None);
    }
    let start = Instant::now();

    on_progress(Stage::Transcribing, 0.0)?;
    let raw = stt.transcribe(samples, |progress| on_progress(Stage::Transcribing, progress))?;
    let stt_ms = start.elapsed().as_millis() as u64;
    if raw.is_empty() {
        return Ok(None);
    }

    let mut output = polish(llm, settings, &raw, |progress| on_progress(Stage::Polishing, progress))?;
    output.audio_s = samples.len() as f32 / audio::SAMPLE_RATE as f32;
    output.stt_ms = stt_ms;
    output.total_ms = start.elapsed().as_millis() as u64;
    Ok(Some(output))
}

/// Cleans a raw transcript with the LLM (when enabled) and applies the replacements.
/// `on_progress` gets 0..1 before each part of the transcript, only when the LLM runs. An error from it stops the work.
pub fn polish(llm: Option<&Llm>, settings: &Settings, raw: &str, on_progress: impl Fn(f32) -> Result<()>) -> Result<Output> {
    let start = Instant::now();
    let mut cleaned = raw.to_string();
    let mut speeds = vec![];
    if let (true, Some(llm), true) = (settings.cleanup_enabled, llm, raw.split_whitespace().count() > 2) {
        let parts = split_for_cleanup(raw);
        let mut cleaned_parts = Vec::with_capacity(parts.len());
        for (i, part) in parts.iter().enumerate() {
            on_progress(i as f32 / parts.len() as f32)?;
            match llm.cleanup(part, &settings.vocabulary) {
                Ok(result) if is_plausible(&result.text, part) => {
                    speeds.push(result.tokens_per_s);
                    cleaned_parts.push(result.text);
                    continue;
                }
                Ok(result) => log::warn!("Discarded LLM output {:?} for {:?}", result.text, part),
                Err(error) => log::warn!("LLM cleanup failed, using the raw text: {error:#}"),
            }
            cleaned_parts.push(part.to_string());
        }
        cleaned = cleaned_parts.join("\n\n");
    }
    let cleanup_used = !speeds.is_empty();
    let llm_tokens_per_s = speeds.iter().fold(0.0, |total, speed| total + speed) / speeds.len().max(1) as f64;
    let llm_ms = start.elapsed().as_millis() as u64;

    for replacement in settings.replacements.iter().filter(|r| !r.from.is_empty()) {
        cleaned = cleaned.replace(&replacement.from, &replacement.to);
    }
    Ok(Output { raw: raw.to_string(), cleaned, audio_s: 0.0, cleanup_used, stt_ms: 0, llm_ms, total_ms: llm_ms, llm_tokens_per_s })
}

/// Splits the text into parts of at most `CLEANUP_CHUNK_CHARS` characters, after a sentence end when possible.
fn split_for_cleanup(text: &str) -> Vec<&str> {
    let mut parts = vec![];
    let mut rest = text;
    while let Some((limit, _)) = rest.char_indices().nth(CLEANUP_CHUNK_CHARS) {
        let window = &rest[..limit];
        let sentence_end = [". ", "? ", "! "].iter().filter_map(|end| window.rfind(end)).max().map(|i| i + 2);
        let cut = sentence_end.or_else(|| window.rfind(' ').map(|i| i + 1)).unwrap_or(limit);
        let (part, tail) = rest.split_at(cut);
        parts.push(part.trim());
        rest = tail.trim_start();
    }
    parts.push(rest);
    parts.retain(|part| !part.is_empty());
    parts
}

fn is_plausible(cleaned: &str, raw: &str) -> bool {
    !cleaned.is_empty() && cleaned.len() <= raw.len() * 2 + 40
}

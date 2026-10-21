//! Samples in, text out: speech to text, optional LLM cleanup, replacements.

use crate::{audio, llm::Llm, settings::Settings, stt::Stt};
use anyhow::Result;
use serde::Serialize;
use std::time::Instant;

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
pub fn run(stt: &mut Stt, llm: Option<&Llm>, settings: &Settings, samples: &[f32], on_stage: impl Fn(Stage)) -> Result<Option<Output>> {
    if !audio::has_speech(samples) {
        return Ok(None);
    }
    let start = Instant::now();

    on_stage(Stage::Transcribing);
    let raw = stt.transcribe(samples)?;
    let stt_ms = start.elapsed().as_millis() as u64;
    if raw.is_empty() {
        return Ok(None);
    }

    if settings.cleanup_enabled && llm.is_some() && raw.split_whitespace().count() > 2 {
        on_stage(Stage::Polishing);
    }
    let mut output = polish(llm, settings, &raw);
    output.audio_s = samples.len() as f32 / audio::SAMPLE_RATE as f32;
    output.stt_ms = stt_ms;
    output.total_ms = start.elapsed().as_millis() as u64;
    Ok(Some(output))
}

/// Cleans a raw transcript with the LLM (when enabled) and applies the replacements.
pub fn polish(llm: Option<&Llm>, settings: &Settings, raw: &str) -> Output {
    let start = Instant::now();
    let mut cleaned = raw.to_string();
    let mut llm_tokens_per_s = 0.0;
    let mut cleanup_used = false;
    if let (true, Some(llm), true) = (settings.cleanup_enabled, llm, raw.split_whitespace().count() > 2) {
        match llm.cleanup(raw, &settings.vocabulary) {
            Ok(result) if is_plausible(&result.text, raw) => {
                cleaned = result.text;
                cleanup_used = true;
                llm_tokens_per_s = result.tokens_per_s;
            }
            Ok(result) => log::warn!("Discarded LLM output {:?} for {:?}", result.text, raw),
            Err(error) => log::warn!("LLM cleanup failed, using the raw text: {error:#}"),
        }
    }
    let llm_ms = start.elapsed().as_millis() as u64;

    for replacement in settings.replacements.iter().filter(|r| !r.from.is_empty()) {
        cleaned = cleaned.replace(&replacement.from, &replacement.to);
    }
    Output { raw: raw.to_string(), cleaned, audio_s: 0.0, cleanup_used, stt_ms: 0, llm_ms, total_ms: llm_ms, llm_tokens_per_s }
}

fn is_plausible(cleaned: &str, raw: &str) -> bool {
    !cleaned.is_empty() && cleaned.len() <= raw.len() * 2 + 40
}

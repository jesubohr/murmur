//! Command line test modes (no window):
//!   murmur.exe --test <wav>... [--stt cpu|gpu] [--no-llm] [--out file.json]
//!   murmur.exe --bench <wav> [--out file.json]

use crate::{audio, llm::Llm, models, pipeline, settings::Settings, stt::Stt};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{path::Path, time::Instant};
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

pub fn run(args: &[String], llama_dir: &Path) -> Result<()> {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    let flag_value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let files: Vec<String> = args[1..]
        .iter()
        .enumerate()
        .filter(|(i, a)| !(a.starts_with("--") || *i > 0 && matches!(args[*i].as_str(), "--stt" | "--out")))
        .map(|(_, a)| a.clone())
        .collect();
    if files.is_empty() {
        bail!("Pass at least one WAV file");
    }

    let result = if args[0] == "--bench" {
        bench(&files[0])?
    } else {
        test(&files, llama_dir, &flag_value("--stt"), !args.iter().any(|a| a == "--no-llm"))?
    };
    let text = serde_json::to_string_pretty(&result)?;
    if let Some(out) = flag_value("--out") {
        std::fs::write(out, &text)?;
    }
    println!("{text}");
    Ok(())
}

fn test(files: &[String], llama_dir: &Path, device: &Option<String>, use_llm: bool) -> Result<Value> {
    let mut settings = Settings::load();
    if let Some(device) = device {
        settings.stt_device = device.clone();
    }

    let started = Instant::now();
    let mut stt = Stt::load(&models::parakeet_dir(), &settings.stt_device)?;
    stt.warm_up()?;
    let stt_load_ms = started.elapsed().as_millis();

    let started = Instant::now();
    let llm = if use_llm { Some(Llm::start(llama_dir)?) } else { None };
    let llm_start_ms = started.elapsed().as_millis();

    let mut clips = vec![];
    for file in files {
        // Anything that is not a WAV file is used as a typed transcript, which skips the speech model.
        let output = if file.ends_with(".wav") {
            let samples = audio::read_wav(Path::new(file))?;
            pipeline::run(&mut stt, llm.as_ref(), &settings, &samples, |_| {})?
        } else {
            Some(pipeline::polish(llm.as_ref(), &settings, file))
        };
        clips.push(json!({ "file": file, "result": output }));
    }
    Ok(json!({
        "stt_device": stt.device_label,
        "stt_load_and_warmup_ms": stt_load_ms,
        "llm_start_ms": llm_start_ms,
        "clips": clips,
    }))
}

/// Compares the encoder on CPU and on the GPU.
fn bench(file: &str) -> Result<Value> {
    let samples = audio::read_wav(Path::new(file))?;
    let mut runs = vec![];
    for device in ["cpu", "gpu"] {
        let started = Instant::now();
        let mut stt = Stt::load(&models::parakeet_dir(), device)?;
        let load_ms = started.elapsed().as_millis();
        let started = Instant::now();
        stt.transcribe(&samples)?;
        let first_ms = started.elapsed().as_millis();
        let mut times = vec![];
        let mut text = String::new();
        for _ in 0..5 {
            let started = Instant::now();
            text = stt.transcribe(&samples)?;
            times.push(started.elapsed().as_millis());
        }
        runs.push(json!({
            "device": stt.device_label,
            "audio_s": samples.len() as f32 / audio::SAMPLE_RATE as f32,
            "load_ms": load_ms,
            "first_run_ms": first_ms,
            "warm_runs_ms": times,
            "text": text,
        }));
    }
    Ok(json!(runs))
}

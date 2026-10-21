//! Model files: where they live and how to download them.

use crate::{llm::MODEL_FILE, settings};
use anyhow::{bail, Result};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::PathBuf,
};

const PARAKEET_URL: &str = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main";

pub struct ModelFile {
    pub path: PathBuf,
    pub url: String,
}

pub fn parakeet_dir() -> PathBuf {
    settings::models_dir().join("parakeet")
}

fn all_files() -> Vec<ModelFile> {
    let parakeet = |name: &str| ModelFile { path: parakeet_dir().join(name), url: format!("{PARAKEET_URL}/{name}") };
    vec![
        ModelFile {
            path: settings::models_dir().join(MODEL_FILE),
            url: format!("https://huggingface.co/bartowski/Qwen_Qwen3-4B-Instruct-2507-GGUF/resolve/main/{MODEL_FILE}"),
        },
        parakeet("encoder-model.int8.onnx"),
        parakeet("decoder_joint-model.int8.onnx"),
        parakeet("nemo128.onnx"),
        parakeet("vocab.txt"),
    ]
}

pub fn missing() -> Vec<ModelFile> {
    all_files().into_iter().filter(|f| !f.path.exists()).collect()
}

/// Downloads every missing file. `on_progress` gets (file name, bytes done, bytes total).
pub fn download_missing(mut on_progress: impl FnMut(&str, u64, u64)) -> Result<()> {
    let client = reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(20)).build()?;
    for file in missing() {
        let name = file.path.file_name().unwrap().to_string_lossy().to_string();
        let mut response = client.get(&file.url).send()?;
        if !response.status().is_success() {
            bail!("Download of {name} failed: HTTP {}", response.status());
        }
        let total = response.content_length().unwrap_or(0);
        fs::create_dir_all(file.path.parent().unwrap())?;
        let partial = file.path.with_extension("part");
        let mut out = File::create(&partial)?;
        let (mut done, mut buf) = (0u64, vec![0u8; 256 * 1024]);
        loop {
            let n = response.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            on_progress(&name, done, total);
        }
        if total > 0 && done != total {
            bail!("Download of {name} was cut short");
        }
        drop(out);
        fs::rename(&partial, &file.path)?;
    }
    Ok(())
}

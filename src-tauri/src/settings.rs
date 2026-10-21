//! Settings, history and well-known folders.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

const APP_ID: &str = "com.murmur.app";

/// `%APPDATA%\com.murmur.app`: settings, history and models.
pub fn app_dir() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join(APP_ID)
}

pub fn models_dir() -> PathBuf {
    app_dir().join("models")
}

/// `%LOCALAPPDATA%\com.murmur.app\logs`
pub fn log_dir() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
        .join(APP_ID)
        .join("logs")
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: String,
    /// `None` means the system default microphone.
    pub input_device: Option<String>,
    /// "gpu" or "cpu"
    pub stt_device: String,
    pub cleanup_enabled: bool,
    pub vocabulary: Vec<String>,
    pub replacements: Vec<Replacement>,
    /// "paste" or "clipboard"
    pub paste_mode: String,
    pub sounds: bool,
    pub launch_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Shift+Space".into(),
            input_device: None,
            stt_device: "gpu".into(),
            cleanup_enabled: true,
            vocabulary: vec![],
            replacements: vec![],
            paste_mode: "paste".into(),
            sounds: true,
            launch_at_login: false,
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        app_dir().join("settings.json")
    }

    pub fn exists() -> bool {
        Self::path().exists()
    }

    pub fn load() -> Self {
        fs::read_to_string(Self::path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        fs::create_dir_all(app_dir())?;
        fs::write(Self::path(), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    /// RFC 3339, UTC
    pub timestamp: String,
    pub raw: String,
    pub cleaned: String,
    pub stt_ms: u64,
    pub llm_ms: u64,
    pub total_ms: u64,
    pub cleanup_used: bool,
}

/// Formats the current time as RFC 3339 (UTC) and returns it with the Unix time in milliseconds.
pub fn now_rfc3339() -> (String, u64) {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let secs = (ms / 1000) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let text = format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60);
    (text, ms)
}

fn history_path() -> PathBuf {
    app_dir().join("history.jsonl")
}

pub fn append_history(entry: &HistoryEntry) -> anyhow::Result<()> {
    fs::create_dir_all(app_dir())?;
    let mut file = OpenOptions::new().create(true).append(true).open(history_path())?;
    writeln!(file, "{}", serde_json::to_string(entry)?)?;
    Ok(())
}

/// Newest first.
pub fn read_history(limit: usize) -> Vec<HistoryEntry> {
    let text = fs::read_to_string(history_path()).unwrap_or_default();
    text.lines()
        .rev()
        .filter_map(|line| serde_json::from_str(line).ok())
        .take(limit)
        .collect()
}

pub fn clear_history() {
    let _ = fs::remove_file(history_path());
}

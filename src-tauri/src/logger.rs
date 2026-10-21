//! Minimal logger: `murmur.log` in the log folder and the console, with millisecond timestamps (UTC).

use crate::settings;
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    sync::{Mutex, OnceLock},
    time::Instant,
};
use windows::Win32::{
    Foundation::FILETIME,
    System::{
        SystemInformation::GetSystemTimeAsFileTime,
        Threading::{GetCurrentProcess, GetProcessTimes},
    },
};

const MAX_LOG_BYTES: u64 = 1_000_000;
static MAIN_START: OnceLock<Instant> = OnceLock::new();
static LOGGER: OnceLock<Logger> = OnceLock::new();

struct Logger {
    file: Mutex<Option<File>>,
    to_stderr: bool,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let (time, ms) = settings::now_rfc3339();
        let line = format!("[{}][{}.{:03}][{}][{}] {}\n", &time[..10], &time[11..19], ms % 1000, record.target(), record.level(), record.args());
        if let Some(file) = self.file.lock().unwrap().as_mut() {
            let _ = file.write_all(line.as_bytes());
        }
        if self.to_stderr {
            let _ = std::io::stderr().write_all(line.as_bytes());
        } else {
            let _ = std::io::stdout().write_all(line.as_bytes());
        }
    }

    fn flush(&self) {}
}

/// Call first in `main`. The command line test modes pass `to_stderr` to keep stdout free for their JSON.
pub fn init(to_stderr: bool) {
    let _ = MAIN_START.set(Instant::now());
    let path = settings::log_dir().join("murmur.log");
    let _ = fs::create_dir_all(settings::log_dir());
    let file = if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) { File::create(&path) } else { OpenOptions::new().create(true).append(true).open(&path) };
    let logger = LOGGER.get_or_init(|| Logger { file: Mutex::new(file.ok()), to_stderr });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
}

/// Milliseconds since `init` ran at the start of `main`.
pub fn since_main_ms() -> u128 {
    MAIN_START.get().map_or(0, |start| start.elapsed().as_millis())
}

/// Milliseconds between the creation of this process and now.
pub fn since_process_start_ms() -> u64 {
    let (mut created, mut unused) = (FILETIME::default(), FILETIME::default());
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    unsafe {
        if GetProcessTimes(GetCurrentProcess(), &mut created, &mut unused, &mut unused, &mut unused).is_err() {
            return 0;
        }
        // FILETIME counts 100 ns ticks.
        (ticks(GetSystemTimeAsFileTime()).saturating_sub(ticks(created))) / 10_000
    }
}

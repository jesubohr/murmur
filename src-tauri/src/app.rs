//! The Tauri app: windows, tray, hotkey state machine, commands and the dictation flow.

use crate::{
    audio::{self, Recording},
    llm::Llm,
    logger, models,
    pipeline::{self, Stage},
    settings::{self, HistoryEntry, Settings},
    stt::Stt,
};
use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use std::{
    path::PathBuf,
    process::Command,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex, OnceLock, RwLock,
    },
    time::{Duration, Instant},
};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use windows::{
    core::w,
    Win32::{
        Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
        System::{Diagnostics::Debug::Beep, Threading::CreateMutexW},
    },
};

const TAP_MAX: Duration = Duration::from_millis(350);
const OVERLAY_BOTTOM_GAP: f64 = 80.0;

#[derive(Serialize, Clone)]
struct ComponentStatus {
    state: &'static str,
    message: Option<String>,
}

fn component(state: &'static str, message: &str) -> ComponentStatus {
    ComponentStatus { state, message: (!message.is_empty()).then(|| message.to_string()) }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Status {
    stt: ComponentStatus,
    llm: ComponentStatus,
    stt_device_active: String,
    llm_device: String,
    models_missing: bool,
    version: String,
}

enum Msg {
    Hotkey(ShortcutState),
    Escape,
    ProcessingDone,
}

pub struct App {
    settings: RwLock<Settings>,
    stt: Mutex<Option<Stt>>,
    llm: Mutex<Option<Llm>>,
    status: Mutex<Status>,
    messages: Sender<Msg>,
    overlay_generation: AtomicU64,
    hotkey_capture: AtomicBool,
    downloading: AtomicBool,
    llama_dir: PathBuf,
    /// Set in `setup`. The engines can start earlier and then only update the status without events.
    handle: OnceLock<AppHandle>,
}

type Shared<'a> = State<'a, Arc<App>>;

pub fn run(llama_dir: PathBuf) -> Result<()> {
    let (messages, inbox) = mpsc::channel();
    let first_run = !Settings::exists();
    if first_run {
        let _ = Settings::default().save();
    }
    let app = Arc::new(App {
        settings: RwLock::new(Settings::load()),
        stt: Mutex::new(None),
        llm: Mutex::new(None),
        status: Mutex::new(Status {
            stt: component("loading", ""),
            llm: component("loading", ""),
            stt_device_active: String::new(),
            llm_device: String::new(),
            models_missing: false,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
        messages,
        overlay_generation: AtomicU64::new(0),
        hotkey_capture: AtomicBool::new(false),
        downloading: AtomicBool::new(false),
        llama_dir,
        handle: OnceLock::new(),
    });

    // Loading the models does not need Tauri, so it runs while Tauri builds the windows.
    let early_start = can_start_early();
    if early_start {
        log::info!("Starting the engines {} ms after main", logger::since_main_ms());
        let early_state = app.clone();
        std::thread::spawn(move || start_engines(&early_state));
    }

    let state = app.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|handle, args, _cwd| {
            // A second launch opens Settings; `murmur.exe --quit` stops the running app.
            if args.iter().any(|a| a == "--quit") {
                handle.exit(0);
            } else {
                show_settings(handle);
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(app.clone())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_status,
            list_input_devices,
            get_history,
            clear_history,
            set_hotkey_capture,
            download_models,
            open_log_folder
        ])
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();
            log::info!("Murmur {} setup started {} ms after main", env!("CARGO_PKG_VERSION"), logger::since_main_ms());
            let _ = state.handle.set(handle.clone());
            let _ = handle.emit("status-changed", current_status(&state));
            setup_windows(&handle)?;
            setup_tray(&handle, &state)?;
            let hotkey = state.settings.read().unwrap().hotkey.clone();
            match apply_hotkey(&handle, &state, &hotkey) {
                Ok(()) => log::info!("Hotkey {hotkey} registered"),
                Err(error) => log::error!("Could not register hotkey {hotkey}: {error}"),
            }
            let (worker_state, worker_handle) = (state.clone(), handle.clone());
            std::thread::spawn(move || hotkey_worker(worker_handle, worker_state, inbox));
            if !early_start {
                init_engines(handle.clone(), state.clone());
            }
            if first_run || !models::missing().is_empty() {
                show_settings(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())?
        .run(|handle, event| {
            if let RunEvent::Exit = event {
                handle.state::<Arc<App>>().llm.lock().unwrap().take();
            }
        });
    Ok(())
}

// ---------------------------------------------------------------- windows, tray, overlay

fn setup_windows(app: &AppHandle) -> tauri::Result<()> {
    let overlay = app.get_webview_window("overlay").expect("overlay window");
    overlay.set_ignore_cursor_events(true)?;
    overlay.set_focusable(false)?;
    Ok(())
}

fn setup_tray(app: &AppHandle, state: &Arc<App>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let cleanup = CheckMenuItem::with_id(app, "cleanup", "AI cleanup", true, state.settings.read().unwrap().cleanup_enabled, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &cleanup, &quit])?;

    let toggle_state = state.clone();
    TrayIconBuilder::with_id("tray")
        .icon(app.default_window_icon().expect("window icon").clone())
        .tooltip("Murmur")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |handle, event| match event.id.as_ref() {
            "settings" => show_settings(handle),
            "cleanup" => {
                let mut settings = toggle_state.settings.write().unwrap();
                settings.cleanup_enabled = cleanup.is_checked().unwrap_or(settings.cleanup_enabled);
                let _ = settings.save();
                let _ = handle.emit("settings-changed", settings.clone());
            }
            "quit" => handle.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_settings(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// The Settings window is created when it is first opened. After that, closing only hides it.
fn show_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        return focus(&window);
    }
    // Building a window blocks until the event loop answers, so never do it on the event loop thread.
    let app = app.clone();
    std::thread::spawn(move || match create_settings_window(&app) {
        Ok(window) => focus(&window),
        Err(error) => log::error!("Could not create the Settings window: {error}"),
    });
}

fn create_settings_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Murmur")
        .inner_size(760.0, 560.0)
        .min_inner_size(640.0, 480.0)
        .center()
        .build()?;
    let hider = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hider.hide();
        }
    });
    Ok(window)
}

fn focus(window: &WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn show_overlay(app: &AppHandle) {
    let Some(window) = app.get_webview_window("overlay") else { return };
    if let (Ok(Some(monitor)), Ok(size)) = (app.primary_monitor(), window.outer_size()) {
        let work = monitor.work_area();
        let gap = (OVERLAY_BOTTOM_GAP * monitor.scale_factor()) as i32;
        let x = work.position.x + (work.size.width as i32 - size.width as i32) / 2;
        let y = work.position.y + work.size.height as i32 - size.height as i32 - gap;
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.show();
}

/// Tells the windows about the pipeline state and shows or hides the overlay.
/// After "done", "error" and "cancelled" the overlay stays a moment so the UI can show the result.
fn set_pipeline_state(app: &AppHandle, state: &Arc<App>, stage: &str, message: Option<&str>, mode: Option<&str>) {
    let generation = state.overlay_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = app.emit("pipeline-state", json!({ "state": stage, "message": message, "mode": mode }));
    if stage == "idle" {
        if let Some(window) = app.get_webview_window("overlay") {
            let _ = window.hide();
        }
        return;
    }
    show_overlay(app);
    let linger = match stage {
        "done" | "cancelled" => Duration::from_millis(600),
        "error" => Duration::from_millis(1800),
        _ => return,
    };
    let (app, state) = (app.clone(), state.clone());
    std::thread::spawn(move || {
        std::thread::sleep(linger);
        if state.overlay_generation.load(Ordering::SeqCst) == generation {
            set_pipeline_state(&app, &state, "idle", None, None);
        }
    });
}

fn show_error(app: &AppHandle, state: &Arc<App>, message: &str) {
    log::error!("{message}");
    set_pipeline_state(app, state, "error", Some(message), None);
}

// ---------------------------------------------------------------- hotkey and dictation flow

/// Registers the dictation hotkey, replacing an earlier registration of the same key.
fn apply_hotkey(app: &AppHandle, state: &Arc<App>, hotkey: &str) -> Result<()> {
    let shortcut = Shortcut::from_str(hotkey)?;
    let _ = app.global_shortcut().unregister(shortcut);
    let messages = state.messages.clone();
    app.global_shortcut().on_shortcut(shortcut, move |_, _, event| {
        let _ = messages.send(Msg::Hotkey(event.state));
    })?;
    Ok(())
}

fn register_escape(app: &AppHandle, state: &Arc<App>) {
    let messages = state.messages.clone();
    let result = app.global_shortcut().on_shortcut("Escape", move |_, _, event| {
        if event.state == ShortcutState::Pressed {
            let _ = messages.send(Msg::Escape);
        }
    });
    if let Err(error) = result {
        log::warn!("Could not register Escape: {error}");
    }
}

fn unregister_escape(app: &AppHandle) {
    let _ = app.global_shortcut().unregister("Escape");
}

enum Phase {
    Idle,
    Recording { recording: Recording, since: Instant, hands_free: bool },
    Busy,
}

fn hotkey_worker(app: AppHandle, state: Arc<App>, inbox: Receiver<Msg>) {
    let mut phase = Phase::Idle;
    for msg in inbox {
        phase = match (std::mem::replace(&mut phase, Phase::Idle), msg) {
            (Phase::Idle, Msg::Hotkey(ShortcutState::Pressed)) => start_recording(&app, &state),
            (Phase::Recording { recording, since, .. }, Msg::Hotkey(ShortcutState::Released)) if since.elapsed() < TAP_MAX => {
                set_pipeline_state(&app, &state, "listening", None, Some("toggle"));
                Phase::Recording { recording, since, hands_free: true }
            }
            (Phase::Recording { recording, .. }, Msg::Hotkey(ShortcutState::Released)) => stop_recording(&app, &state, recording),
            (Phase::Recording { recording, hands_free: true, .. }, Msg::Hotkey(ShortcutState::Pressed)) => stop_recording(&app, &state, recording),
            (Phase::Recording { recording, .. }, Msg::Escape) => {
                unregister_escape(&app);
                drop(recording.finish());
                log::info!("Recording cancelled");
                set_pipeline_state(&app, &state, "cancelled", None, None);
                Phase::Idle
            }
            (Phase::Busy, Msg::ProcessingDone) => Phase::Idle,
            (other, _) => other,
        };
    }
}

fn start_recording(app: &AppHandle, state: &Arc<App>) -> Phase {
    if state.stt.lock().map(|stt| stt.is_none()).unwrap_or(true) {
        show_error(app, state, "Speech model is still loading");
        return Phase::Idle;
    }
    let (device, sounds) = {
        let settings = state.settings.read().unwrap();
        (settings.input_device.clone(), settings.sounds)
    };
    let level_app = app.clone();
    match Recording::start(device.as_deref(), move |level| {
        let _ = level_app.emit("mic-level", level);
    }) {
        Ok(recording) => {
            if sounds {
                beep(880);
            }
            set_pipeline_state(app, state, "listening", None, Some("hold"));
            register_escape(app, state);
            Phase::Recording { recording, since: Instant::now(), hands_free: false }
        }
        Err(error) => {
            show_error(app, state, &format!("Microphone: {error:#}"));
            Phase::Idle
        }
    }
}

fn stop_recording(app: &AppHandle, state: &Arc<App>, recording: Recording) -> Phase {
    unregister_escape(app);
    if state.settings.read().unwrap().sounds {
        beep(660);
    }
    let samples = recording.finish();
    let (app, state) = (app.clone(), state.clone());
    std::thread::spawn(move || {
        process(&app, &state, &samples);
        let _ = state.messages.send(Msg::ProcessingDone);
    });
    Phase::Busy
}

fn process(app: &AppHandle, state: &Arc<App>, samples: &[f32]) {
    let settings = state.settings.read().unwrap().clone();
    let mut stt = state.stt.lock().unwrap();
    let llm = state.llm.lock().unwrap();
    let Some(stt) = stt.as_mut() else {
        return show_error(app, state, "Speech model is not loaded");
    };

    let result = pipeline::run(stt, llm.as_ref(), &settings, samples, |stage| match stage {
        Stage::Transcribing => set_pipeline_state(app, state, "transcribing", None, None),
        Stage::Polishing => set_pipeline_state(app, state, "polishing", None, None),
    });
    match result {
        Ok(Some(output)) => {
            log::info!("Dictated {:.1}s: stt {} ms, llm {} ms: {:?}", output.audio_s, output.stt_ms, output.llm_ms, output.cleaned);
            let (timestamp, time_ms) = settings::now_rfc3339();
            let entry = HistoryEntry {
                id: time_ms.to_string(),
                timestamp,
                raw: output.raw,
                cleaned: output.cleaned.clone(),
                stt_ms: output.stt_ms,
                llm_ms: output.llm_ms,
                total_ms: output.total_ms,
                cleanup_used: output.cleanup_used,
            };
            let _ = settings::append_history(&entry);
            let _ = app.emit("history-added", entry);
            if let Err(error) = deliver(&output.cleaned, &settings.paste_mode) {
                return show_error(app, state, &format!("Paste failed: {error:#}"));
            }
            set_pipeline_state(app, state, "done", None, None);
        }
        Ok(None) => {
            log::info!("Nothing to paste (too short, silent or empty)");
            set_pipeline_state(app, state, "idle", None, None);
        }
        Err(error) => show_error(app, state, &format!("Transcription failed: {error:#}")),
    }
}

/// Puts the text on the clipboard and, in paste mode, sends Ctrl+V and restores the old clipboard.
fn deliver(text: &str, paste_mode: &str) -> Result<()> {
    use enigo::{Direction, Enigo, Key, Keyboard};
    let mut clipboard = arboard::Clipboard::new()?;
    let previous = clipboard.get_text().ok();
    clipboard.set_text(text)?;
    if paste_mode != "paste" {
        return Ok(());
    }
    std::thread::sleep(Duration::from_millis(60));
    let mut keyboard = Enigo::new(&enigo::Settings::default())?;
    keyboard.key(Key::Control, Direction::Press)?;
    keyboard.key(Key::Other(0x56), Direction::Click)?;
    keyboard.key(Key::Control, Direction::Release)?;
    std::thread::sleep(Duration::from_millis(300));
    if let Some(previous) = previous {
        clipboard.set_text(previous)?;
    }
    Ok(())
}

fn beep(frequency: u32) {
    std::thread::spawn(move || unsafe {
        let _ = Beep(frequency, 60);
    });
}

// ---------------------------------------------------------------- engines

fn current_status(state: &App) -> Status {
    let mut status = state.status.lock().unwrap().clone();
    status.models_missing = !models::missing().is_empty();
    status
}

fn set_status(state: &App, update: impl FnOnce(&mut Status)) {
    update(&mut state.status.lock().unwrap());
    if let Some(app) = state.handle.get() {
        let _ = app.emit("status-changed", current_status(state));
    }
}

/// True when the models are present, no other Murmur process runs and this is not a `--quit` call.
/// Only then it is safe to load the models before the single instance plugin has decided.
fn can_start_early() -> bool {
    if std::env::args().any(|a| a == "--quit") || !models::missing().is_empty() {
        return false;
    }
    // The mutex handle stays open until the process ends.
    unsafe { CreateMutexW(None, false, w!("Local\\com.murmur.app-engines")).is_ok() && GetLastError() != ERROR_ALREADY_EXISTS }
}

fn init_engines(app: AppHandle, state: Arc<App>) {
    std::thread::spawn(move || {
        if !models::missing().is_empty() && !download(&app, &state) {
            return;
        }
        start_engines(&state);
    });
}

/// Loads the speech model on a new thread and starts the LLM on this one.
fn start_engines(state: &Arc<App>) {
    let stt_state = state.clone();
    std::thread::spawn(move || load_stt(&stt_state));
    load_llm(state);
}

/// Downloads the missing models and reports progress. Returns false when it failed.
fn download(app: &AppHandle, state: &Arc<App>) -> bool {
    if state.downloading.swap(true, Ordering::SeqCst) {
        return false;
    }
    set_status(state, |s| {
        s.stt = component("missing", "Downloading models");
        s.llm = component("missing", "Downloading models");
    });
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let result = models::download_missing(|file, downloaded, total| {
        let done = total > 0 && downloaded == total;
        if done || last_emit.elapsed() >= Duration::from_millis(250) {
            last_emit = Instant::now();
            let _ = app.emit("download-progress", json!({ "file": file, "downloaded": downloaded, "total": total, "done": done, "error": null }));
        }
    });
    state.downloading.store(false, Ordering::SeqCst);
    if let Err(error) = result {
        let message = format!("Model download failed: {error:#}");
        log::error!("{message}");
        let _ = app.emit("download-progress", json!({ "file": "", "downloaded": 0, "total": 0, "done": false, "error": message }));
        set_status(state, |s| {
            s.stt = component("error", &message);
            s.llm = component("error", &message);
        });
        return false;
    }
    true
}

fn load_stt(state: &Arc<App>) {
    set_status(state, |s| s.stt = component("loading", ""));
    state.stt.lock().unwrap().take();
    let device = state.settings.read().unwrap().stt_device.clone();
    let loaded = Stt::load(&models::parakeet_dir(), &device).or_else(|error| {
        log::warn!("Loading STT on {device} failed ({error:#}), trying the CPU");
        Stt::load(&models::parakeet_dir(), "cpu")
    });
    match loaded {
        Ok(mut stt) => {
            let started = Instant::now();
            if let Err(error) = stt.warm_up() {
                log::warn!("STT warm-up failed: {error:#}");
            }
            log::info!("STT ready on {} (warm-up {} ms), {} ms after main", stt.device_label, started.elapsed().as_millis(), logger::since_main_ms());
            let label = stt.device_label.clone();
            *state.stt.lock().unwrap() = Some(stt);
            set_status(state, |s| {
                s.stt = component("ready", "");
                s.stt_device_active = label;
            });
        }
        Err(error) => {
            log::error!("STT failed to load: {error:#}");
            set_status(state, |s| s.stt = component("error", &format!("{error:#}")));
        }
    }
}

fn load_llm(state: &Arc<App>) {
    set_status(state, |s| s.llm = component("loading", ""));
    match Llm::start(&state.llama_dir) {
        Ok(llm) => {
            log::info!("LLM ready, {} ms after main", logger::since_main_ms());
            let device = llm.device.clone();
            *state.llm.lock().unwrap() = Some(llm);
            set_status(state, |s| {
                s.llm = component("ready", "");
                s.llm_device = device;
            });
        }
        Err(error) => {
            log::error!("llama-server failed to start: {error:#}");
            set_status(state, |s| s.llm = component("error", &format!("{error:#}")));
        }
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_settings(state: Shared) -> Settings {
    state.settings.read().unwrap().clone()
}

/// Saves the settings and applies them right away.
#[tauri::command]
fn save_settings(app: AppHandle, state: Shared, settings: Settings) -> Result<Settings, String> {
    let old = state.settings.read().unwrap().clone();
    if settings.hotkey != old.hotkey {
        Shortcut::from_str(&settings.hotkey).map_err(|e| format!("Invalid hotkey: {e}"))?;
        // While the UI records a key combination the hotkey stays unregistered.
        if !state.hotkey_capture.load(Ordering::SeqCst) {
            let _ = app.global_shortcut().unregister(old.hotkey.as_str());
            if let Err(error) = apply_hotkey(&app, &state, &settings.hotkey) {
                let _ = apply_hotkey(&app, &state, &old.hotkey);
                return Err(format!("Could not use {}: {error}", settings.hotkey));
            }
        }
    }
    if settings.launch_at_login != old.launch_at_login {
        let autolaunch = app.autolaunch();
        let result = if settings.launch_at_login { autolaunch.enable() } else { autolaunch.disable() };
        result.map_err(|e| format!("Launch at login: {e}"))?;
    }
    settings.save().map_err(|e| e.to_string())?;
    *state.settings.write().unwrap() = settings.clone();
    if settings.stt_device != old.stt_device {
        let state = state.inner().clone();
        std::thread::spawn(move || load_stt(&state));
    }
    Ok(settings)
}

#[tauri::command]
fn get_status(state: Shared) -> Status {
    current_status(&state)
}

#[tauri::command]
fn list_input_devices() -> Vec<String> {
    audio::list_microphones()
}

#[tauri::command]
fn get_history(limit: u32) -> Vec<HistoryEntry> {
    settings::read_history(limit as usize)
}

#[tauri::command]
fn clear_history() {
    settings::clear_history();
}

/// While active, the global hotkey is off so the UI can read the key combination.
#[tauri::command]
fn set_hotkey_capture(app: AppHandle, state: Shared, active: bool) -> Result<(), String> {
    state.hotkey_capture.store(active, Ordering::SeqCst);
    let hotkey = state.settings.read().unwrap().hotkey.clone();
    if active {
        let shortcut = Shortcut::from_str(&hotkey).map_err(|e| e.to_string())?;
        let _ = app.global_shortcut().unregister(shortcut);
        Ok(())
    } else {
        apply_hotkey(&app, &state, &hotkey).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn download_models(app: AppHandle, state: Shared) {
    if !models::missing().is_empty() && !state.downloading.load(Ordering::SeqCst) {
        init_engines(app, state.inner().clone());
    }
}

#[tauri::command]
fn open_log_folder() {
    let _ = Command::new("explorer").arg(settings::log_dir()).spawn();
}

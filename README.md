# Murmur

Local dictation for Windows 10. Hold a hotkey, speak, release. Murmur turns your speech into clean text and pastes it into the active window. Nothing leaves your computer.

Pipeline: hotkey, microphone, 16 kHz mono, speech to text (NVIDIA Parakeet TDT 0.6B v3, int8 ONNX, English, Spanish and 23 more languages), cleanup with a local LLM (Qwen3 4B Instruct 2507 through llama.cpp and Vulkan), paste.

## Use

| Action | Result |
| --- | --- |
| Hold `Ctrl+Shift+Space` | Push to talk. Recording stops when you release the keys. |
| Tap the hotkey (under 350 ms) | Hands-free. Recording continues until you tap again. |
| `Esc` while recording | Cancel. Nothing is pasted. |
| Tray icon, left click | Open Settings. |
| Tray icon, right click | Settings, AI cleanup on/off, Quit. |
| Start `murmur.exe` again | Open Settings. |
| `murmur.exe --quit` | Stop the running Murmur. |

Murmur starts in the tray. The overlay at the bottom of the screen shows Listening (with live microphone level), Transcribing, Polishing or an error. Clips shorter than 0.3 s or without sound are ignored.

## What the cleanup does

It removes fillers (um, uh, mhm, filler "like", "you know"), stutters and repeated words, and applies self-corrections ("at 5, no wait, at 6" becomes "at 6"). It fixes grammar and punctuation, and writes spoken forms the written way ("github dot com" becomes `github.com`, "five hundred megabytes" becomes `500 MB`). It fixes the casing of technical terms (JavaScript, PostgreSQL, macOS), turns dictated lists into lists, and turns "new line" and "new paragraph" into line breaks. Spanish stays Spanish. A question or command is cleaned, never answered. For results of one or two words, or when the LLM fails, you get the raw transcript.

## Settings

Settings are stored in `%APPDATA%\com.murmur.app\settings.json`.

- **Hotkey**: record a new key combination in the Settings window.
- **Microphone**: system default or a chosen input device.
- **STT device**: GPU (DirectML on the discrete GPU) or CPU.
- **AI cleanup**: on or off (also in the tray menu).
- **Vocabulary**: names and terms the LLM must spell exactly.
- **Replacements**: exact text replacements applied after the LLM.
- **Paste mode**: paste (Ctrl+V, then restore your clipboard) or clipboard only.
- **Sounds**: start and stop beeps.
- **Launch at login**: off by default.
- **History**: the last results with copy button, raw text and timings. Stored in `%APPDATA%\com.murmur.app\history.jsonl`.

Files:

- Models: `%APPDATA%\com.murmur.app\models` (downloaded on first run if missing, about 3.2 GB).
- Cache: `%APPDATA%\com.murmur.app\cache` (saved speech encoder graph, about 650 MB, and saved LLM prompt, about 170 MB) and `llama-device.json` (GPU choice for llama-server). Delete them to rebuild them on the next start.
- Logs: `%LOCALAPPDATA%\com.murmur.app\logs` (`murmur.log`, `llama-server.log`). `murmur.log` has millisecond timestamps (UTC) and is cut at 1 MB on start.

## Rebuild

Needs Rust (MSVC), Node.js, and Visual Studio Build Tools.

```
npm install
powershell -File tools/fetch_binaries.ps1     # llama.cpp and DirectML, use -Force to download again
npx tauri build          # NSIS installer: src-tauri/target/release/bundle/nsis/
```

Install per user: `Murmur_0.1.0_x64-setup.exe /S`. The app goes to `%LOCALAPPDATA%\Murmur`.

`src-tauri/resources/llama` holds `llama-server.exe` and its DLLs (llama.cpp b11741, Vulkan). `src-tauri/dml/DirectML.dll` (version 1.15.4) is shipped next to `murmur.exe`; the system copy of DirectML is too old. Both folders are not in the repository. `tools/fetch_binaries.ps1` downloads them and checks their SHA256.

Regenerate the icon: `powershell -File tools/make_icon.ps1 -Out tools/app-icon.png`, then `npx tauri icon tools/app-icon.png` (delete the extra mobile folders it creates).

## Test modes (same binary, no window)

```
murmur.exe --test clip1.wav "typed transcript" [--stt cpu|gpu] [--no-llm] [--out result.json]
murmur.exe --bench clip.wav [--out result.json]     # encoder on CPU and on the GPU
```

`--test` prints raw text, cleaned text and timings as JSON. Log lines go to stderr in these modes. Arguments that are not WAV files are used as typed transcripts and skip speech to text. `tools/run_tests.sh` runs all clips in `tests/audio` (made by `tests/make_audio.ps1` with Windows SAPI). `tools/text_tests.sh` runs typed transcripts for prompt work. `tools/e2e.ps1` presses the hotkey and reads what arrives in Notepad. It needs `MURMUR_FAKE_WAV=<wav>` in the environment of the running Murmur, which replaces the microphone audio.

## Design notes

- **STT**: `ort` 2.0.0-rc.12 with the DirectML provider on the DXGI adapter with the most dedicated memory (the RX 6600). The mel preprocessor and the small decoder run on the CPU. DirectML recompiles the encoder for every new input length and keeps only one compiled shape, so on the GPU the encoder always gets 1024 frames (padded), and longer audio is split at the quietest point into chunks of at most 9.5 s.
- **LLM**: `llama-server.exe` on `127.0.0.1:8178`, `-ngl 99 -c 4096 -np 1`, device chosen by matching the DXGI adapter name against `--list-devices`. It runs without a console window inside a Windows Job Object with `KILL_ON_JOB_CLOSE`, so it never survives Murmur, even after a crash.
- **Start**: `main` starts the logger first. When the models exist and no other Murmur runs (named mutex), the models load before Tauri builds its windows. The Settings window is created when it is first opened. The saved encoder graph and the saved LLM prompt (llama-server `--slot-save-path`) make later starts faster.
- **Threads**: the hotkey handler only sends messages. One worker thread owns the recording state machine; STT and LLM run on separate threads.

## UI contract (backend to frontend)

The frontend is in `ui/` (plain HTML, CSS and JS, `withGlobalTauri`). Windows: `settings` (`settings.html`) and `overlay` (`overlay.html`, transparent, always on top, no focus, ignores the mouse, shown only while the pipeline is active).

Commands (call with `window.__TAURI__.core.invoke`, arguments in camelCase):

- `get_settings() -> Settings`
- `save_settings({ settings }) -> Settings` (applies live: hotkey, microphone, STT device, autostart; error string for an invalid hotkey)
- `get_status() -> Status`
- `list_input_devices() -> string[]`
- `get_history({ limit }) -> HistoryEntry[]` (newest first)
- `clear_history()`
- `set_hotkey_capture({ active })` (while true, the global hotkey is unregistered so the UI can record a key combination)
- `download_models()` (progress through events)
- `open_log_folder()`

Types (camelCase):

```
Settings { hotkey, inputDevice: string|null, sttDevice: "gpu"|"cpu", cleanupEnabled, vocabulary: string[],
           replacements: { from, to }[], pasteMode: "paste"|"clipboard", sounds, launchAtLogin }
Status { stt: ComponentStatus, llm: ComponentStatus, sttDeviceActive, llmDevice, modelsMissing, version }
ComponentStatus { state: "loading"|"ready"|"error"|"missing", message: string|null }
HistoryEntry { id, timestamp /* RFC 3339, UTC */, raw, cleaned, sttMs, llmMs, totalMs, cleanupUsed }
```

Events (to all windows):

- `pipeline-state`: `{ state: "idle"|"listening"|"transcribing"|"polishing"|"done"|"error"|"cancelled", message, mode: "hold"|"toggle"|null }`
- `mic-level`: number 0 to 1, about 20 per second, only while listening
- `status-changed`: `Status`
- `download-progress`: `{ file, downloaded, total, done, error }` (`done` is true when one file is complete; all models are present when `status-changed` reports `modelsMissing: false`)
- `history-added`: `HistoryEntry`
- `settings-changed`: `Settings` (for example when the tray toggles AI cleanup)

## Third-party components

| Component | License |
| --- | --- |
| [llama.cpp](https://github.com/ggml-org/llama.cpp) (`llama-server`) | MIT |
| [ONNX Runtime](https://github.com/microsoft/onnxruntime) through the [`ort`](https://github.com/pykeio/ort) crate | MIT (ONNX Runtime), MIT or Apache-2.0 (`ort`) |
| [DirectML](https://www.nuget.org/packages/Microsoft.AI.DirectML/1.15.4) (Microsoft) | Microsoft Software License Terms, which allow redistribution in applications that run on Windows |
| [NVIDIA Parakeet TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) (ONNX conversion by istupakov) | CC-BY-4.0 |
| [Qwen3-4B-Instruct-2507](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507) (GGUF conversion by bartowski) | Apache-2.0 |

The models are not part of this repository. Murmur downloads them on first run.

## Limits

- Only text is restored on the clipboard after pasting (not images or files).
- While Murmur processes a clip, a new hotkey press is ignored.
- The first start after an install loads the models and builds the cache (about 10 s). Later starts take about 3 s; recording before that shows "Speech model is still loading".

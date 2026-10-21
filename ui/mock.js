// Browser-only stand-in for the Tauri backend. Does nothing inside the real app.
// Options (query string): ?status=missing|loading|error  ?state=listening&mode=toggle (overlay)
(() => {
  if (window.__TAURI__) return;

  const params = new URLSearchParams(location.search);
  const handlers = {};
  const emit = (name, payload) => (handlers[name] || []).forEach((fn) => fn({ payload }));
  const ago = (minutes) => new Date(Date.now() - minutes * 60000).toISOString();

  let settings = {
    hotkey: "Ctrl+Shift+Space",
    inputDevice: null,
    sttDevice: "gpu",
    cleanupEnabled: true,
    vocabulary: ["Murmur", "Tauri", "Parakeet", "Acme"],
    replacements: [
      { from: "ack me", to: "Acme" },
      { from: "my address", to: "221B Baker Street" },
    ],
    pasteMode: "paste",
    sounds: true,
    launchAtLogin: false,
  };

  const ready = { state: "ready" };
  const statusVariants = {
    missing: { stt: { state: "missing", message: "Speech model not downloaded" }, llm: { state: "missing", message: "Cleanup model not downloaded" }, modelsMissing: true },
    loading: { stt: ready, llm: { state: "loading" }, modelsMissing: false },
    error: { stt: ready, llm: { state: "error", message: "Vulkan device lost. Restart Murmur or switch to CPU." }, modelsMissing: false },
  };
  let status = {
    stt: ready,
    llm: ready,
    sttDeviceActive: "DirectML · AMD Radeon RX 6600",
    llmDevice: "Vulkan · AMD Radeon RX 6600",
    modelsMissing: false,
    version: "0.1.0",
    ...statusVariants[params.get("status")],
  };

  let history = [
    {
      id: "6", timestamp: ago(0.5),
      raw: "Mm, lets do this first and then I don't now, put these things here to to mhm be able to have good",
      cleaned: "Let's do this first, then put these things here so we can have this set up.",
      sttMs: 412, llmMs: 880, totalMs: 1342, cleanupUsed: true,
    },
    {
      id: "5", timestamp: ago(12),
      raw: "hey can you send me the invoice for october when you get a chance thanks",
      cleaned: "Hey, can you send me the invoice for October when you get a chance? Thanks.",
      sttMs: 380, llmMs: 760, totalMs: 1190, cleanupUsed: true,
    },
    {
      id: "4", timestamp: ago(65),
      raw: "git checkout dash b feature slash overlay",
      cleaned: "git checkout -b feature/overlay",
      sttMs: 340, llmMs: 0, totalMs: 352, cleanupUsed: false,
    },
    {
      id: "3", timestamp: ago(60 * 5),
      raw: "um so the meeting is moved to thursday at three p m and uh please bring the ack me report",
      cleaned: "The meeting is moved to Thursday at 3 PM. Please bring the Acme report.",
      sttMs: 455, llmMs: 1010, totalMs: 1510, cleanupUsed: true,
    },
    {
      id: "2", timestamp: ago(60 * 30),
      raw: "remember to water the plants before leaving",
      cleaned: "Remember to water the plants before leaving.",
      sttMs: 300, llmMs: 540, totalMs: 868, cleanupUsed: true,
    },
  ];

  const devices = ["Microphone Array (Realtek Audio)", "Headset Microphone (Jabra Evolve2 65)", "Line In (USB Audio Interface)"];

  function simulateDownload() {
    const files = [
      { file: "parakeet-tdt-v3.onnx", total: 640e6 },
      { file: "qwen3-4b-q4_k_m.gguf", total: 2500e6 },
    ];
    let tick = 0;
    const timer = setInterval(() => {
      tick += 1;
      files.forEach((f, i) => {
        const downloaded = Math.min(f.total, f.total * Math.max(0, (tick - i * 4) / 20));
        emit("download-progress", { file: f.file, downloaded, total: f.total, done: downloaded >= f.total });
      });
      if (tick >= 28) {
        clearInterval(timer);
        status = { ...status, stt: ready, llm: ready, modelsMissing: false };
        emit("status-changed", status);
      }
    }, 250);
  }

  const commands = {
    get_settings: () => settings,
    save_settings: ({ settings: next }) => {
      if (next.hotkey === "Ctrl+Alt+Delete") throw "Ctrl+Alt+Delete is reserved by Windows";
      settings = next;
      setTimeout(() => emit("settings-changed", settings), 20);
      return settings;
    },
    get_status: () => status,
    list_input_devices: () => devices,
    get_history: ({ limit }) => history.slice(0, limit),
    clear_history: () => { history = []; },
    set_hotkey_capture: () => {},
    download_models: () => simulateDownload(),
    open_log_folder: () => {},
  };

  window.__TAURI__ = {
    core: {
      invoke: (name, args) =>
        new Promise((resolve, reject) =>
          setTimeout(() => {
            try { resolve(commands[name](args)); } catch (error) { reject(error); }
          }, 60)),
    },
    event: {
      listen: (name, fn) => {
        (handlers[name] ||= []).push(fn);
        return Promise.resolve(() => { handlers[name] = handlers[name].filter((h) => h !== fn); });
      },
    },
    window: { getCurrentWindow: () => ({}) },
  };

  // Overlay: send fake mic levels while listening and cycle through the states
  if (!location.pathname.endsWith("overlay.html")) return;

  setInterval(() => {
    const syllable = Math.max(0, Math.sin(performance.now() / 180));
    emit("mic-level", Math.min(1, 0.04 + 0.5 * syllable * (0.3 + 0.7 * Math.random())));
  }, 50);

  const fixed = params.get("state");
  const mode = params.get("mode") || "hold";
  if (fixed) {
    setTimeout(() => emit("pipeline-state", { state: fixed, mode, message: "No speech detected. Check that the right microphone is selected." }), 150);
    return;
  }

  const script = [
    ["idle", 1200], ["listening", 4000, "hold"], ["transcribing", 1500], ["polishing", 1500], ["done", 1200],
    ["idle", 1000], ["listening", 3000, "toggle"], ["cancelled", 1200],
    ["idle", 1000], ["listening", 1500, "hold"], ["error", 2400, null, "No speech detected. Check that the right microphone is selected."],
  ];
  let step = 0;
  function next() {
    const [state, ms, stepMode, message] = script[step++ % script.length];
    emit("pipeline-state", { state, mode: stepMode || undefined, message });
    setTimeout(next, ms);
  }
  setTimeout(next, 200);
})();

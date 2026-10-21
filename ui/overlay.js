(() => {
  const { listen } = window.__TAURI__.event;

  const BAR_COUNT = 14;
  const body = document.body;
  const barsBox = document.getElementById("bars");
  const timeLabel = document.getElementById("time");
  const bars = Array.from({ length: BAR_COUNT }, () => barsBox.appendChild(document.createElement("span")));

  const targets = new Array(BAR_COUNT).fill(0);
  const shown = new Array(BAR_COUNT).fill(0);
  let frame = 0;
  let startedAt = 0;
  let lastSecond = -1;

  // Speech sits low on a 0-1 scale, so a square root makes the bars readable.
  listen("mic-level", ({ payload }) => {
    targets.shift();
    targets.push(Math.sqrt(Math.min(1, Math.max(0, payload))));
  });

  function draw() {
    for (let i = 0; i < BAR_COUNT; i++) {
      shown[i] += (targets[i] - shown[i]) * 0.3;
      bars[i].style.transform = `scaleY(${0.12 + 0.88 * shown[i]})`;
    }
    const seconds = Math.floor((performance.now() - startedAt) / 1000);
    if (seconds !== lastSecond) {
      lastSecond = seconds;
      timeLabel.textContent = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
    }
    frame = requestAnimationFrame(draw);
  }

  function startListening(mode) {
    document.getElementById("hint").textContent = mode === "toggle" ? "hands-free" : "hold";
    if (frame) return;
    targets.fill(0);
    shown.fill(0);
    startedAt = performance.now();
    lastSecond = -1;
    frame = requestAnimationFrame(draw);
  }

  function stopListening() {
    cancelAnimationFrame(frame);
    frame = 0;
  }

  listen("pipeline-state", ({ payload }) => {
    const { state, message, mode } = payload;
    body.dataset.state = state;
    if (state === "listening") {
      startListening(mode);
    } else {
      stopListening();
    }
    if (state === "transcribing" || state === "polishing") {
      document.getElementById("busy-label").textContent = state === "polishing" ? "Polishing" : "Transcribing";
    }
    if (state === "error") {
      document.getElementById("error-message").textContent = message || "Something went wrong";
    }
  });
})();

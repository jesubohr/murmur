(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  const $ = (selector, root = document) => root.querySelector(selector);
  const el = (tag, props = {}, ...children) => {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...children);
    return node;
  };
  const closeIcon = () => {
    const wrapper = el("span");
    wrapper.innerHTML = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3 3l6 6M9 3l-6 6"/></svg>';
    return wrapper.firstChild;
  };

  let settings; // last value confirmed by the backend
  let draft; // what the UI shows; may be ahead of `settings` while a save is pending
  let status;
  let devices = [];
  let saveTimer = 0;

  /* Navigation */

  function showSection() {
    const sections = [...document.querySelectorAll("main section")];
    const wanted = location.hash.slice(1);
    const id = sections.some((s) => s.dataset.section === wanted) ? wanted : "general";
    sections.forEach((s) => (s.hidden = s.dataset.section !== id));
    document.querySelectorAll("nav a").forEach((a) => {
      if (a.hash === `#${id}`) a.setAttribute("aria-current", "page");
      else a.removeAttribute("aria-current");
    });
    $("main").scrollTop = 0;
  }

  /* Feedback */

  let savedTimer = 0;
  function flashSaved() {
    const saved = $("#saved");
    saved.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7"/></svg>Saved';
    saved.classList.add("visible");
    clearTimeout(savedTimer);
    savedTimer = setTimeout(() => saved.classList.remove("visible"), 1800);
  }

  let toastTimer = 0;
  function fail(message) {
    const toast = $("#toast");
    toast.textContent = message;
    toast.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast.hidden = true), 6000);
  }

  /* Saving */

  function change(patch, delay = 0) {
    Object.assign(draft, patch);
    clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      const error = await save();
      if (error) fail(`Could not save settings. ${error}`);
    }, delay);
  }

  // Resolves to an error message, or null when saved. On error the UI goes back to the last saved settings.
  async function save() {
    const next = { ...draft, replacements: draft.replacements.filter((r) => r.from.trim()) };
    try {
      settings = await invoke("save_settings", { settings: next });
      renderHotkey();
      flashSaved();
      return null;
    } catch (error) {
      apply(settings);
      return String(error);
    }
  }

  function apply(next) {
    settings = next;
    draft = structuredClone(next);
    renderSettings();
  }

  /* Settings form */

  const controls = {
    mic: $("#mic"),
    paste: $("#paste-mode"),
    sounds: $("#sounds"),
    launch: $("#launch"),
    cleanup: $("#cleanup-toggle"),
  };

  function renderSettings() {
    renderMic();
    controls.paste.value = draft.pasteMode;
    controls.sounds.checked = draft.sounds;
    controls.launch.checked = draft.launchAtLogin;
    controls.cleanup.checked = draft.cleanupEnabled;
    document.querySelectorAll('[name="stt-device"]').forEach((r) => (r.checked = r.value === draft.sttDevice));
    renderHotkey();
    renderChips();
    renderReplacements();
    renderHistory();
  }

  function renderMic() {
    const names = [...devices];
    if (draft.inputDevice && !names.includes(draft.inputDevice)) names.push(draft.inputDevice);
    controls.mic.replaceChildren(
      el("option", { value: "", textContent: "System default" }),
      ...names.map((name) => el("option", { value: name, textContent: name })),
    );
    controls.mic.value = draft.inputDevice ?? "";
  }

  controls.mic.onchange = () => change({ inputDevice: controls.mic.value || null });
  controls.paste.onchange = () => change({ pasteMode: controls.paste.value });
  controls.sounds.onchange = () => change({ sounds: controls.sounds.checked });
  controls.launch.onchange = () => change({ launchAtLogin: controls.launch.checked });
  controls.cleanup.onchange = () => change({ cleanupEnabled: controls.cleanup.checked });
  document.querySelectorAll('[name="stt-device"]').forEach((radio) => {
    radio.onchange = () => change({ sttDevice: radio.value });
  });

  /* Hotkey */

  const caps = $("#caps");
  const hotkeyButton = $("#hotkey-btn");
  const hotkeyError = $("#hotkey-error");
  const keyLabels = { Super: "Win" };
  let capturing = false;

  const capElements = (parts) => parts.map((part) => el("kbd", { textContent: keyLabels[part] || part }));

  function renderHotkey() {
    if (capturing) return;
    caps.classList.remove("capturing");
    caps.replaceChildren(...capElements((draft?.hotkey || settings.hotkey).split("+")));
    hotkeyButton.textContent = "Change";
  }

  function renderCapturePrompt(heldModifiers = []) {
    caps.classList.add("capturing");
    const prompt = heldModifiers.length ? "" : "Press the new shortcut";
    caps.replaceChildren(...capElements(heldModifiers), el("span", { className: "prompt", textContent: prompt }));
    hotkeyButton.textContent = "Cancel";
  }

  function keyName(e) {
    if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
    if (/^Digit\d$/.test(e.code)) return e.code.slice(5);
    if (/^F([1-9]|1\d|2[0-4])$/.test(e.code)) return e.code;
    if (e.code === "Space") return "Space";
    return null;
  }

  const heldModifiers = (e) =>
    [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);

  function onCaptureKeydown(e) {
    e.preventDefault();
    e.stopPropagation();
    if (e.repeat) return;
    if (e.key === "Escape") return stopCapture();

    const modifiers = heldModifiers(e);
    const key = keyName(e);
    if (!key) return renderCapturePrompt(modifiers);
    if (!modifiers.length && !/^F\d+$/.test(key)) {
      hotkeyError.textContent = "Add Ctrl, Alt, Shift or Win. Only an F key works alone.";
      hotkeyError.hidden = false;
      return;
    }
    finishCapture([...modifiers, key].join("+"));
  }

  const onCaptureKeyup = (e) => {
    e.preventDefault();
    renderCapturePrompt(heldModifiers(e));
  };

  async function startCapture() {
    hotkeyError.hidden = true;
    try {
      await invoke("set_hotkey_capture", { active: true });
    } catch (error) {
      return fail(`Could not record a shortcut. ${error}`);
    }
    capturing = true;
    renderCapturePrompt();
    window.addEventListener("keydown", onCaptureKeydown, true);
    window.addEventListener("keyup", onCaptureKeyup, true);
    window.addEventListener("blur", stopCapture);
  }

  async function stopCapture() {
    if (!capturing) return;
    capturing = false;
    window.removeEventListener("keydown", onCaptureKeydown, true);
    window.removeEventListener("keyup", onCaptureKeyup, true);
    window.removeEventListener("blur", stopCapture);
    renderHotkey();
    await invoke("set_hotkey_capture", { active: false }).catch(() => {});
  }

  async function finishCapture(accelerator) {
    await stopCapture();
    if (accelerator === settings.hotkey) return;
    draft.hotkey = accelerator;
    renderHotkey();
    const error = await save();
    if (error) {
      hotkeyError.textContent = error;
      hotkeyError.hidden = false;
    }
  }

  hotkeyButton.onclick = () => (capturing ? stopCapture() : startCapture());

  /* Vocabulary */

  const chipBox = $("#vocab-chips");
  const vocabInput = $("#vocab-input");

  function renderChips() {
    chipBox.querySelectorAll(".chip").forEach((chip) => chip.remove());
    draft.vocabulary.forEach((term, index) => {
      const remove = el("button", { className: "icon-btn", type: "button", onclick: () => {
        draft.vocabulary.splice(index, 1);
        renderChips();
        change({});
      } }, closeIcon());
      remove.setAttribute("aria-label", `Remove ${term}`);
      chipBox.insertBefore(el("span", { className: "chip" }, term, remove), vocabInput);
    });
    renderSuggestions();
  }

  vocabInput.onkeydown = (e) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    const term = vocabInput.value.trim();
    vocabInput.value = "";
    if (!term || draft.vocabulary.some((t) => t.toLowerCase() === term.toLowerCase())) return;
    draft.vocabulary.push(term);
    renderChips();
    change({});
  };

  /* Find terms in text */

  const importPanel = $("#import-panel");
  const importToggle = $("#import-toggle");
  const importText = $("#import-text");
  const suggestionBox = $("#suggestions");

  function startsSentence(text, index) {
    let i = index - 1;
    while (i >= 0 && /[ \t\r"'(\[*•-]/.test(text[i])) i--;
    return i < 0 || ".!?¿¡\n".includes(text[i]);
  }

  // Names (capitalized inside a sentence), acronyms, mixed case (PgBouncer) and letters with digits (K8s).
  function findTerms(text, known) {
    const counts = new Map();
    for (const match of text.matchAll(/[\p{L}\p{N}]+(?:[._-][\p{L}\p{N}]+)*[+#]*/gu)) {
      const word = match[0];
      const key = word.toLowerCase();
      if (word.length < 2 || known.has(key) || !/\p{L}/u.test(word)) continue;
      const name = /^\p{Lu}/u.test(word) && !startsSentence(text, match.index);
      const unusual = /\p{Ll}\p{Lu}|^\p{Lu}[\p{Lu}\p{N}]+$|\p{N}/u.test(word);
      if (!name && !unusual) continue;
      const entry = counts.get(key) ?? { word, count: 0 };
      entry.count++;
      counts.set(key, entry);
    }
    return [...counts.values()].sort((a, b) => b.count - a.count).slice(0, 20).map((entry) => entry.word);
  }

  function renderSuggestions() {
    const known = new Set(draft.vocabulary.map((term) => term.toLowerCase()));
    const terms = importText.value ? findTerms(importText.value, known) : [];
    if (!terms.length) {
      suggestionBox.replaceChildren(el("span", { className: "none", textContent: importText.value ? "No new terms found." : "" }));
      return;
    }
    suggestionBox.replaceChildren(...terms.map((term) => el("button", {
      className: "chip suggestion",
      type: "button",
      textContent: `+ ${term}`,
      onclick: () => {
        draft.vocabulary.push(term);
        renderChips();
        change({});
      },
    })));
  }

  importToggle.onclick = () => {
    importPanel.hidden = !importPanel.hidden;
    importToggle.setAttribute("aria-expanded", String(!importPanel.hidden));
    if (!importPanel.hidden) importText.focus();
  };
  importText.oninput = renderSuggestions;
  $("#import-file").onclick = () => $("#import-file-input").click();
  $("#import-file-input").onchange = async (e) => {
    const [file] = e.target.files;
    e.target.value = "";
    if (!file) return;
    importText.value = await file.text();
    renderSuggestions();
  };

  /* Replacements */

  const replacementBox = $("#replacements");

  function renderReplacements() {
    if (!draft.replacements.length) {
      replacementBox.replaceChildren(el("p", { className: "none", textContent: "No replacements yet." }));
      return;
    }
    replacementBox.replaceChildren(...draft.replacements.map((item, index) => {
      const input = (key, label, placeholder) => {
        const field = el("input", { type: "text", value: item[key], placeholder, autocomplete: "off", spellcheck: false });
        field.setAttribute("aria-label", label);
        field.oninput = () => {
          item[key] = field.value;
          change({}, 600);
        };
        return field;
      };
      const remove = el("button", { className: "icon-btn", type: "button", onclick: () => {
        draft.replacements.splice(index, 1);
        renderReplacements();
        change({});
      } }, closeIcon());
      remove.setAttribute("aria-label", `Remove replacement ${index + 1}`);
      return el("div", { className: "replacement" },
        input("from", "When I say", "When I say"),
        el("span", { className: "arrow", textContent: "→" }),
        input("to", "Write", "Write this instead"),
        remove);
    }));
  }

  $("#add-replacement").onclick = () => {
    draft.replacements.push({ from: "", to: "" });
    renderReplacements();
    replacementBox.querySelector(".replacement:last-child input").focus();
  };

  /* Status and downloads */

  const stateLabels = { loading: "Loading", ready: "Ready", error: "Error", missing: "Not downloaded" };
  const progressList = $("#progress-list");
  const downloadButton = $("#download-btn");
  const progressItems = new Map();

  function renderRow(selector, part, detail) {
    const row = $(selector);
    $(".state", row).dataset.state = part.state;
    $(".state-label", row).textContent = stateLabels[part.state];
    $("p", row).textContent = (part.state === "error" || part.state === "missing") && part.message ? part.message : detail;
  }

  function renderStatus() {
    renderRow("#stt-row", status.stt, `Parakeet v3 · ${status.sttDeviceActive}`);
    renderRow("#llm-row", status.llm, `Qwen3 4B · ${status.llmDevice}`);
    $("#downloads").hidden = !status.modelsMissing;
    if (!status.modelsMissing) {
      progressList.replaceChildren();
      progressItems.clear();
    }
    $("#stt-active").textContent = `Running on ${status.sttDeviceActive}`;
    $("#llm-active").textContent = `Running on ${status.llmDevice}`;
    $("#version").textContent = status.version;
  }

  const formatBytes = (bytes) =>
    bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;

  function progressItem(file) {
    if (progressItems.has(file)) return progressItems.get(file);
    const info = el("span");
    const fill = el("div", { className: "fill" });
    const root = el("li", { className: "progress-item" },
      el("div", { className: "meta" }, el("span", { className: "name", textContent: file }), info),
      el("div", { className: "track" }, fill));
    progressList.append(root);
    const item = { root, info, fill };
    progressItems.set(file, item);
    return item;
  }

  function showProgress({ file, downloaded, total, done, error }) {
    const item = progressItem(file);
    const ratio = done ? 1 : total ? downloaded / total : 0;
    item.fill.style.transform = `scaleX(${ratio})`;
    item.root.classList.toggle("failed", Boolean(error));
    if (error) item.info.textContent = error;
    else item.info.textContent = done ? "Done" : `${formatBytes(downloaded)} of ${formatBytes(total)}`;
    if (error) {
      downloadButton.disabled = false;
      downloadButton.textContent = "Retry download";
    }
  }

  downloadButton.onclick = async () => {
    downloadButton.disabled = true;
    downloadButton.textContent = "Downloading";
    try {
      await invoke("download_models");
    } catch (error) {
      downloadButton.disabled = false;
      downloadButton.textContent = "Retry download";
      fail(`Could not download the models. ${error}`);
    }
  };

  $("#open-logs").onclick = () => invoke("open_log_folder").catch((e) => fail(`Could not open the log folder. ${e}`));

  /* History */

  let entries = [];
  let query = "";
  let confirmingClear = false;
  const expanded = new Set();
  const list = $("#entries");

  function timeAgo(timestamp) {
    const minutes = Math.floor((Date.now() - new Date(timestamp)) / 60000);
    if (minutes < 1) return "Just now";
    if (minutes < 60) return `${minutes} min ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours} h ago`;
    return new Date(timestamp).toLocaleDateString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  }

  const formatMs = (ms) => (ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`);

  function entryElement(entry) {
    const open = expanded.has(entry.id);
    const meta = el("div", { className: "meta entry-meta" },
      el("time", { textContent: timeAgo(entry.timestamp), dateTime: entry.timestamp }),
      el("span", { textContent: formatMs(entry.totalMs) }));
    if (entry.cleanupUsed) meta.append(el("span", { className: "badge", textContent: "AI" }));

    const copy = el("button", { className: "btn small", type: "button", textContent: "Copy" });
    copy.onclick = async () => {
      try {
        await navigator.clipboard.writeText(entry.cleaned);
        copy.textContent = "Copied";
        setTimeout(() => (copy.textContent = "Copy"), 1200);
      } catch {
        fail("Could not copy to the clipboard.");
      }
    };

    const toggle = el("button", { className: "icon-btn", type: "button" });
    toggle.innerHTML = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2.5 4.5L6 8l3.5-3.5"/></svg>';
    toggle.setAttribute("aria-label", "Show details");
    toggle.setAttribute("aria-expanded", String(open));

    const timings = el("dl", { className: "timings" });
    const timing = (name, value) => timings.append(el("div", {}, el("dt", { textContent: name }), el("dd", { textContent: value })));
    timing("Speech to text", formatMs(entry.sttMs));
    timing("AI cleanup", entry.cleanupUsed ? formatMs(entry.llmMs) : "Skipped");
    timing("Total", formatMs(entry.totalMs));

    const detail = el("div", { className: "entry-detail", hidden: !open },
      el("p", { textContent: entry.raw }), timings);

    const root = el("li", { className: open ? "entry open" : "entry" },
      el("p", { className: "entry-text", textContent: entry.cleaned }),
      meta,
      el("div", { className: "entry-actions" }, copy, toggle),
      detail);

    toggle.onclick = () => {
      const isOpen = root.classList.toggle("open");
      detail.hidden = !isOpen;
      toggle.setAttribute("aria-expanded", String(isOpen));
      if (isOpen) expanded.add(entry.id);
      else expanded.delete(entry.id);
    };
    return root;
  }

  function renderHistory() {
    const needle = query.trim().toLowerCase();
    const shown = needle
      ? entries.filter((e) => e.cleaned.toLowerCase().includes(needle) || e.raw.toLowerCase().includes(needle))
      : entries;
    list.replaceChildren(...shown.map(entryElement));

    const empty = $("#history-empty");
    empty.hidden = shown.length > 0;
    if (!empty.hidden) {
      const hotkey = settings ? settings.hotkey : "your shortcut";
      $("h3", empty).textContent = entries.length ? "No matches" : "No dictations yet";
      $("p", empty).textContent = entries.length
        ? "Try a different word."
        : `Hold ${hotkey} in any app and start talking. Your recent dictations show up here.`;
    }
    renderClearArea();
  }

  function renderClearArea() {
    const area = $("#clear-area");
    if (confirmingClear) {
      const cancel = el("button", { className: "btn", type: "button", textContent: "Cancel", onclick: () => setConfirm(false) });
      const confirm = el("button", { className: "btn danger", type: "button", textContent: "Delete all", onclick: clearHistory });
      area.replaceChildren(el("div", { className: "confirm" }, el("span", { textContent: "Delete the whole history?" }), cancel, confirm));
      confirm.focus();
      return;
    }
    const clear = el("button", { className: "btn", type: "button", textContent: "Clear history", disabled: !entries.length, onclick: () => setConfirm(true) });
    area.replaceChildren(clear);
  }

  function setConfirm(value) {
    confirmingClear = value;
    renderClearArea();
    if (!value) $("#clear-area button").focus();
  }

  async function clearHistory() {
    try {
      await invoke("clear_history");
      entries = [];
      expanded.clear();
      confirmingClear = false;
      renderHistory();
    } catch (error) {
      fail(`Could not clear the history. ${error}`);
    }
  }

  $("#history-search").oninput = (e) => {
    query = e.target.value;
    renderHistory();
  };

  setInterval(() => {
    list.querySelectorAll("time").forEach((t) => (t.textContent = timeAgo(t.dateTime)));
  }, 30000);

  /* Startup */

  window.addEventListener("hashchange", showSection);
  showSection();

  listen("settings-changed", ({ payload }) => {
    if (JSON.stringify(payload) !== JSON.stringify(settings)) apply(payload);
  });
  listen("status-changed", ({ payload }) => {
    status = payload;
    renderStatus();
  });
  listen("download-progress", ({ payload }) => showProgress(payload));
  listen("history-added", ({ payload }) => {
    entries.unshift(payload);
    renderHistory();
  });

  (async () => {
    try {
      let initialSettings;
      [initialSettings, status, devices, entries] = await Promise.all([
        invoke("get_settings"),
        invoke("get_status"),
        invoke("list_input_devices"),
        invoke("get_history", { limit: 200 }),
      ]);
      apply(initialSettings);
      renderStatus();
    } catch (error) {
      fail(`Could not load Murmur. ${error}`);
    }
  })();
})();

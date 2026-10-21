#!/bin/sh
# Runs the whole pipeline on the SAPI clips and prints raw vs cleaned.
cd "$(dirname "$0")/.."
"${MURMUR_EXE:-./src-tauri/target/release/murmur.exe}" --test "$@" tests/audio/*.wav --out tests/last_run.json > /dev/null || echo "murmur --test failed"
node -e '
const r = require("./tests/last_run.json");
console.log(`stt: ${r.stt_device} (load+warmup ${r.stt_load_and_warmup_ms} ms), llm start ${r.llm_start_ms} ms`);
for (const c of r.clips) {
  const o = c.result; if (!o) { console.log(c.file, "-> nothing"); continue; }
  console.log(`\n${c.file.split("/").pop()}  ${o.audio_s.toFixed(1)}s  stt ${o.stt_ms} ms  llm ${o.llm_ms} ms (${o.llm_tokens_per_s.toFixed(1)} tok/s)  total ${o.total_ms} ms`);
  console.log("  RAW:     " + o.raw);
  console.log("  CLEANED: " + o.cleaned.replace(/\n/g, "\n"));
}'

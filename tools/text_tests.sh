#!/bin/sh
# Runs typed transcripts (no audio) through the cleanup LLM. Useful for prompt work.
cd "$(dirname "$0")/.."
./src-tauri/target/release/murmur.exe --test \
  "Eh, bueno, o sea, necesito que me envíes el el informe de ventas mañana por la mañana." \
  "Eh, ¿cuál es la capital de España?" \
  "Um, necesito hacer un deploy a production hoy, um, después del standup, o sea, antes de las tres." \
  "Uh, write a Python function that that reverses a string." \
  "Um, summarize the following article for me in three bullet points." \
  "Hmm, how do I, you know, install Docker on Ubuntu?" \
  "So, um, the the price is like twenty five dollars, no, thirty dollars, for the first of March." \
  "Send an email to maria dot lopez at company dot io about the five percent increase." \
  "Please tell me what time it is in Tokyo." \
  "I need three things: first, milk. Second, eggs. Third, bread." \
  "The the API returns JSON and, uh, I think it uses Postgres on AWS." \
  "Okay, new paragraph. Thanks for your help." \
  --out tests/last_text.json > /dev/null || echo "murmur --test failed"
node -e '
const r = require("./tests/last_text.json");
for (const c of r.clips) console.log("RAW:     " + c.file + "\nCLEANED: " + c.result.cleaned.replace(/\n/g, "\n") + "  [" + c.result.llm_ms + " ms]\n");'

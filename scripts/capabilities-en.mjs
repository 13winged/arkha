// Разовая правка: дописывает английские тексты в реестр возможностей.
// Ключ — идентификатор возможности, значение — переводы.
// Запуск: node scripts/capabilities-en.mjs. После прогона удалить.

import { readFileSync, writeFileSync } from "node:fs";

const путь = "data/capabilities.json";
const реестр = JSON.parse(readFileSync(путь, "utf8"));

// Пустая строка означает «перевода не нужно»: поле в этом языке
// остаётся пустым, а не подставляется русским. Так честнее — видно,
// что перевода нет, а не что его забыли.
const EN = {
  "chat.local": {
    name: "Conversation",
    what: "Answers questions in Russian and English, keeps the conversation within the context window",
    evidence: "Gate S2 on live hardware: T1, T2 and T3, Russian answer without errors, streaming works, no refusal on a harmless task",
  },
  "files.read": {
    name: "Reading files",
    what: "Reads text and documents from folders the user has allowed",
    evidence: "Gate S6, 12.10.2026, T3 RTX 5060: reading by a known path 5 out of 5, refusal reaches the surface and is not invented",
  },
  "skill.execute": {
    name: "Skills",
    what: "Runs a procedure described in advance by a human",
    evidence: "Gate S6, 12.10.2026, T3 RTX 5060: a skill on disk was called by the model, its steps were completed",
  },
  "files.write": {
    name: "Writing files",
    what: "Creates and modifies files in allowed folders, asking for confirmation",
    refusal: "writing files is not wired up yet — the tool exists but does not work",
  },
  "windows.read": {
    name: "Reading windows",
    what: "Shows open windows and finds the one by its title. Changes nothing",
    evidence: "Gate S8, 13.10.2026, T3 RTX 5060: window list 9 out of 9 scenarios, the found window is named in the answer",
    refusal: "Sees window titles only, not their contents",
  },
  "apps.launch": {
    name: "Launching programs",
    what: "Opens programs the user has allowed",
    evidence: "Gate S8, 13.10.2026, T3 RTX 5060: notepad launched and its window appeared; a forbidden program did not launch",
    refusal: "Looks only in program install folders. If a program lives somewhere unusual, give the full path to the .exe",
  },
  "windows.control": {
    name: "Controlling windows",
    what: "Switches windows, types text, presses buttons by their label",
    evidence: "Gate S8, 13.10.2026, T3 RTX 5060: focus, Russian input via unicode and pressing a button — 9 out of 9; pressing a non-existent button was refused",
    refusal: "Presses only elements with a visible label. In Chrome, Telegram and other programs that draw their own interface the target is not found",
  },
  "web.fetch": {
    name: "Reading a page",
    what: "Opens a page by address and returns its text",
    refusal: "page reading is not wired up yet",
  },
  "web.search": {
    name: "Searching the web",
    what: "Searches the web through a SearXNG instance whose address the user sets",
    refusal: "search is not configured: no SearXNG address is set. Reading a page by its address works without it",
  },
  "docs.office": {
    name: "Office documents",
    what: "Reads docx, xlsx, pptx and pdf",
    refusal: "reading Office documents is not wired up yet",
  },
  "vision.read": {
    name: "Reading images",
    what: "Makes sense of the contents of images and screenshots",
    evidence: "Vision gate, 13 October 2026, T3 RTX 5060 8 GB: 4 out of 4 on Qwen3-VL-4B Q4_K_M. Cyrillic «КОТ» and «ПРИВЕТ» read correctly in 5 out of 5 runs, the answer is in Russian, vision 0.8–2 s per image. Vision and the agent run at the same time: 2118 MiB for vision plus the agent on 30 layers, no swapping",
    refusal: "Vision runs as a separate Qwen3-VL-4B model, layers in RAM, only the viewer on the graphics card. Spaces between characters are lost: «7 3 9» reads as «739»",
  },
  "voice.stt": {
    name: "Speech recognition",
    what: "Turns speech into text",
    refusal: "speech recognition is not wired up yet, it is at stage S9",
  },
  "voice.tts": {
    name: "Speaking",
    what: "Reads answers aloud",
    refusal: "speaking is not wired up yet, it is at stage S9",
  },
  "voice.wake": {
    name: "Waking by name",
    what: "Responds to its name instead of recording everything",
    refusal: "waking by name is not wired up yet, it is at stage S9",
  },
  "memory.recall": {
    name: "Memory",
    what: "Remembers facts and decisions between conversations and finds them later",
    refusal: "memory is not wired up yet",
  },
  "memory.search": {
    name: "Searching the corpus",
    what: "Searches your documents and answers from what it found",
    refusal: "corpus search is not wired up yet, embeddings are needed",
  },
  "browser.dom": {
    name: "Controlling the browser",
    what: "Reads a page as a tree of elements rather than as a picture",
    refusal: "browser control is not wired up yet",
  },
  "workflow.run": {
    name: "Workflows",
    what: "Runs multi-step records: open, find, fill in, save",
    refusal: "workflows are not wired up yet, they are built from the other tools",
  },
};

let заполнено = 0;
let пропущено = [];

for (const в of реестр.capabilities) {
  const перевод = EN[в.id];
  if (!перевод) {
    пропущено.push(в.id);
    continue;
  }
  // Пустое поле в русском переводить незачем: если отказа нет, то и
  // на английском его не будет.
  в.name_en = перевод.name ?? "";
  в.what_en = перевод.what ?? "";
  в.evidence_en = в.evidence ? (перевод.evidence ?? "") : "";
  в.refusal_en = в.refusal ? (перевод.refusal ?? "") : "";
  заполнено++;
}

writeFileSync(путь, JSON.stringify(реестр, null, 2) + "\n", "utf8");
console.log(`заполнено: ${заполнено}`);
console.log(`пропущено: ${пропущено.length}${пропущено.length ? " — " + пропущено.join(", ") : ""}`);
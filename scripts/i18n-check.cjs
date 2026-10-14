// Разовая проверка: какие строки вызываются через t(), но нет в словаре.
// Запуск: node scripts/i18n-check.mjs
const fs = require("node:fs");

const app = fs.readFileSync("src/App.tsx", "utf8");
const i18n = fs.readFileSync("src/i18n.ts", "utf8");

const ключи = new Set();
for (const m of app.matchAll(/t\("((?:[^"\\]|\\.)*)"\)/g)) ключи.add(m[1]);

const есть = new Set();
for (const m of i18n.matchAll(/^\s*"((?:[^"\\]|\\.)+)"\s*:/gm)) есть.add(m[1]);

const нет = [...ключи].filter((к) => !есть.has(к));
console.log(`вызовов t(): ${ключи.size}`);
console.log(`ключей в словаре: ${есть.size}`);
console.log(`без перевода: ${нет.length}`);
for (const к of нет) console.log(`  |${к}|`);
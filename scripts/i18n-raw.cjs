// Разовая проверка: кириллица в разметке, мимо t().
// Запуск: node scripts/i18n-raw.cjs
const fs = require("node:fs");

const строки = fs.readFileSync("src/App.tsx", "utf8").split("\n");
const кириллица = /[А-Яа-яЁё]/;
const код = /[<>{}()=;|`$]|=>|\bconst\b|\blet\b|\breturn\b|\bimport\b|\binterface\b|\btype\b|\bfunction\b/;

let найдено = 0;
for (let i = 0; i < строки.length; i++) {
  const с = строки[i];
  const t = с.trim();
  if (t === "") continue;
  if (t.startsWith("//") || t.startsWith("/*") || t.startsWith("*")) continue;
  if (!кириллица.test(с)) continue;
  // Внутри t(...) — переведено.
  if (/^\s*\{?t\(/.test(с)) continue;
  // Объектные ключи и значения в словарях режимов.
  if (/^\s*(id|title|note):/.test(с)) continue;
  // className и прочие атрибуты.
  if (/className=|aria-|data-/.test(с)) continue;
  if (код.test(с)) continue;
  найдено++;
  console.log(`${i + 1}: ${t}`);
}
console.log(`\nвсего сырых строк: ${найдено}`);
// Разовая проверка: какие строки вызываются через t(), но нет в словаре.
// Запуск: node scripts/i18n-check.cjs
//
// Регулярка вызова требует, чтобы перед `t(` не стояло ни буквы,
// ни точки. Без этого условия она ловит чужие вызовы: `split("/")`,
// `format("|")` и подобные заканчиваются на `t("...")`, и чек
// объявлял их несуществующими переводами. На файле интерфейса это
// давало два ложных срабатывания — `|` и `/`.
//
// Проверяются все экраны, а не только App.tsx: Dashboard, AgentSelect
// и другие файлы тоже зовут t(), и раньше их строки проходили мимо.
const fs = require("node:fs");
const path = require("node:path");

const i18n = fs.readFileSync("src/i18n.ts", "utf8");

const файлы = fs
  .readdirSync("src", { recursive: true })
  .filter((file) => file.endsWith(".tsx"))
  .map((file) => path.join("src", file));

const ключи = new Set();
for (const файл of файлы) {
  const app = fs.readFileSync(файл, "utf8");
  for (const m of app.matchAll(/(?<![\w.$])t\("((?:[^"\\]|\\.)+)"\)/g)) ключи.add(m[1]);
}

const есть = new Set();
for (const m of i18n.matchAll(/^\s*"((?:[^"\\]|\\.)+)"\s*:/gm)) есть.add(m[1]);

const нет = [...ключи].filter((к) => !есть.has(к));
console.log(`вызовов t(): ${ключи.size}`);
console.log(`ключей в словаре: ${есть.size}`);
console.log(`без перевода: ${нет.length}`);
for (const к of нет) console.log(`  |${к}|`);

// Пустой ключ тоже считается недочётом: он означает либо опечатку в
// вызове, либо строку, которую кто-то добавил в интерфейс и забыл
// перевести.
if (нет.length > 0) process.exit(1);

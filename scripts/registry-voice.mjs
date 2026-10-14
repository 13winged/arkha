// Разовая правка: проставляет voice.stt и voice.tts как проверенные
// по замерам из docs/10-замеры.md. Реестр врал: модели на диске,
// код написан, замеры сняты — а в реестре значилось «не подключено».
// Запуск: node scripts/registry-voice.mjs. После прогона удалить.

import { readFileSync, writeFileSync } from "node:fs";

const путь = "data/capabilities.json";
const реестр = JSON.parse(readFileSync(путь, "utf8"));

// Формулировки взяты из замеров, а не придуманы здесь. Дата и числа
// те же, что в docs/10-замеры.md, раздел «Голос на карте из трёх
// моделей».
const ПРАВКА = {
  "voice.stt": {
    evidence:
      "Гейт голоса 13.10.2026, T3 RTX 5060: эталон произнесён синтезом «Проверка распознавания речи. Сегодня вторник.» и сверялся дословно. 5 из 5 точных совпадений на процессоре (медиана 274 мс) и 5 из 5 на карте (медиана 86 мс). Живёт на процессоре: 143 МиБ против 2594 МиБ на карте — при поднятом распознавании агент на контексте 65536 сваливается в разделяемую память уже на 16 слоях",
    refusal:
      "работает с файлом, а не с микрофона: живой диалог ещё не собран. Задержка от конца речи до первого звука ответа не измерена, в неё входит генерация ответа агентом",
    what_en: "Turns speech into text",
    evidence_en:
      "Voice gate 13.10.2026, T3 RTX 5060: the reference was spoken by synthesis — “Проверка распознавания речи. Сегодня вторник.” — and compared word for word. 5 out of 5 exact matches on the processor (median 274 ms) and 5 out of 5 on the GPU (median 86 ms). Lives on the processor: 143 MiB against 2594 MiB on the GPU — with recognition up, the agent at a 65536 context spills into shared memory already at 16 layers",
    refusal_en:
      "it works on a file, not a microphone: the live conversation loop is not assembled yet. The delay from the end of speech to the first sound of the answer is not measured — it includes the agent generating its answer",
  },
  "voice.tts": {
    evidence:
      "Гейт голоса 13.10.2026, T3 RTX 5060: синтез на живой машине 3.60 с звука за 0.93 с вычислений, то есть 3.87× реального времени",
    refusal:
      "синтез работает с текстом и пишет файл; озвучивание в ответе агента ещё не собрано — это следующий шаг вместе с живым диалогом",
    evidence_en:
      "Voice gate 13.10.2026, T3 RTX 5060: synthesis on live hardware produced 3.60 s of audio in 0.93 s of compute, which is 3.87× real time",
    refusal_en:
      "synthesis takes text and writes a file; speaking the agent’s answers is not assembled yet — that is the next step together with the live conversation loop",
  },
};

let тронуто = 0;
const пропущено = [];

for (const в of реестр.capabilities) {
  const правка = ПРАВКА[в.id];
  if (!правка) continue;
  Object.assign(в, правка);
  в.verified = true;
  тронуто++;
}

// Кого правка не тронула — так и должно остаться: `voice.wake` не
// реализован, и помечать его проверенным было бы ровно той ложью,
// ради которой реестр и сделан.
for (const в of реестр.capabilities) {
  if (!ПРАВКА[в.id] && в.id.startsWith("voice.")) пропущено.push(в.id);
}

реестр._обновлено = "2026-10-14, этап S9: голос помечен проверенным по замерам 13.10";

writeFileSync(путь, JSON.stringify(реестр, null, 2) + "\n", "utf8");
console.log(`помечено проверенным: ${тронуто}`);
console.log(`оставлено как есть: ${пропущено.join(", ") || "—"}`);
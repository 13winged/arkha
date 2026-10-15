// Язык интерфейса.
//
// Ключом служит сама русская строка. Так диф остаётся читаемым: видно,
// что именно переводили, а не по загадочным идентификаторам. Плата за
// это — опечатку в ключе не поймает компилятор, поэтому есть `t()` с
// тихим откатом на русский: лишняя непереведённая строка не ломает
// страницу, а просто остаётся по-русски.
//
// Строки, которые видит человек, приходят из трёх мест: отсюда, из
// реестра возможностей и из промптов знакомства. Первые живут здесь,
// вторые и третьи — в Rust, и язык передаётся в команды. Пока Rust
// отвечает на выбранном языке, здесь тоже надо ответить на нём же,
// иначе половина экрана заговорит по-русски посреди английского.

export type Lang = "ru" | "en";

/**
 * Переводы. Ключ — русская строка без схлопнутых пробелов.
 *
 * Строки в несколько строк хранятся слитно: JSX при выводе схлопывает
 * переносы, значит и ключ должен быть слитным, иначе он не совпадёт.
 */
const EN: Record<string, string> = {
  // Шапка и подвал
  "локальный секретарь": "local secretary",
  "модель": "model",
  "секретарь": "secretary",
  "чат": "chat",
  "загрузка": "download",
  "возможности": "capabilities",
  "Знакомство": "Welcome",
  "профиль": "profile",
  "настройки": "settings",
  "вкладки": "tabs",
  "всё считается на этой машине, облако выключено":
    "everything is computed on this machine, cloud is off",

  // Полоса ресурсов
  "ОЗУ": "RAM",
  "память: …": "memory: …",
  "голос: …": "voice: …",
  "включить голос": "turn on voice",
  "выключить голос и освободить место": "turn voice off and free the memory",
  "включить голос: поднимется распознавание":
    "turn on voice: speech recognition will start",

  // Модель
  "Запустить модель": "Start model",
  "остановить модель": "stop model",
  "Сначала запустите модель": "Start the model first",
  "Что мешает запуску": "What blocks the start",
  "Рецепт": "Recipe",
  "KV-кэш": "KV cache",
  "Контекст": "Context",
  "Размещение": "Placement",
  "Устройство": "Device",
  "скорость": "speed",
  "не измерена": "not measured",
  "· рецепт не замерян": " · recipe was never benchmarked",
  "не запущена": "not running",
  "не выбрана": "not chosen",
  "запущена": "running",
  "остановлено": "stopped",
  "закрыто": "blocked",
  "работает": "works",
  "не вышло": "failed",
  "журнал запуска": "startup log",
  "скрыть журнал": "hide log",
  "журнал пуст": "log is empty",
  "Проверяем и запускаем…": "Checking and starting…",
  "Поднимаем интерфейс…": "Starting the interface…",

  // Секретарь и чат
  "Инструменты:": "Tools:",
  "Сообщение": "Message",
  "Напишите сообщение": "Write a message",
  "остановить ответ": "stop the answer",
  "отправить": "send",
  "Режим": "Mode",
  "Не понял ответ. Попробуем тот же вопрос ещё раз.":
    "Did not understand. Let's ask the same question again.",
  "Не понял ответ": "Did not understand",

  // Возможности
  "Возможности": "Capabilities",
  "пока недоступно": "not available yet",
  "Возможностей в реестре нет.": "The registry is empty.",
  "проверено на железе": "verified on hardware",
  "Ничего не проверено. Это значит «ещё не сделано», а не «сломалось».":
    "Nothing is verified yet. That means “not built yet”, not “broken”.",
  "Секретарь знает про эти возможности и честно скажет, что не может. Не молчит и не обещает.":
    "The secretary knows about these and will honestly say what it cannot do. It neither stays silent nor promises.",
  "Читаем реестр…": "Reading the registry…",
  "Проверено": "Verified",
  "не сделано": "not built",

  // Знакомство
  "Реши, насколько широко секретарь сможет читать. На этом шаге содержимое не читается — только названия папок.":
    "Decide how broadly the secretary may read. Nothing is read at this step — only folder names.",
  "читать весь диск": "read the whole disk",
  "Секретарь сможет читать что угодно в любой папке. Удобно, если документы разбросаны. Он всё равно не напишет и не удалит ничего без твоего вопроса.":
    "The secretary will be able to read anything in any folder. Handy when documents are scattered. It still will not write or delete anything without asking you.",
  "или отметь отдельные папки": "or pick individual folders",
  "Папок по умолчанию не нашлось. Можно назвать свою в поле ниже.":
    "No default folders found. You can name one in the field below.",
  "Отметь, что секретарю можно делать. Недоступное сейчас всё равно отметить можно — включишь, когда появится.":
    "Mark what the secretary may do. What is unavailable now can still be marked — you will turn it on when it appears.",
  "Два решения, и оба про то, насколько секретарь будет действовать сам. Сначала — показывать или делать.":
    "Two decisions, both about how far the secretary acts on its own. First: show or do.",
  "режим": "mode",
  "уровень доступа": "access level",
  "только показывать": "show only",
  "Секретарь пишет, что собирается сделать, и ждёт. Ничего не происходит без твоего слова.":
    "The secretary writes what it is about to do and waits. Nothing happens without your word.",
  "выполнять": "execute",
  "Согласные действия выполняются сразу. Опасные — всё равно спросит.":
    "Safe actions run immediately. Risky ones still ask.",
  "опасные — спросить": "ask for risky ones",
  "Удаление, запуск программ и переписывание файлов требуют твоего подтверждения.":
    "Deleting, launching programs and overwriting files require your confirmation.",
  "спросить всё": "ask for everything",
  "Подтверждения на каждое действие. Медленнее, зато предсказуемо.":
    "A confirmation for every action. Slower, but predictable.",
  "не спрашивать": "ask for nothing",
  "Делает всё, что разрешено. Опасно: промах в команде он не остановит.":
    "It does everything allowed. Risky: it will not stop a wrong command.",
  "дальше": "next",
  "уровни выбраны": "levels chosen",
  "Итог": "Summary",
  "Как буду работать": "How I will work",
  "Папки": "Folders",
  "Ничего ещё не применено. Проверь и нажми «сохранить», если всё верно.":
    "Nothing has been applied yet. Check it and press “save” if all is right.",
  "сохранить": "save",
  "начать заново": "start over",
  "твой ответ": "your answer",
  "сказать": "say",
  "путь к папке": "path to a folder",
  "Выбрать этот профиль": "Choose this profile",
  "вернуть автоматическое определение": "return to automatic detection",

  // Профиль
  "Ваш компьютер": "Your computer",
  "Видеокарта": "Graphics card",
  "Видеопамять": "Video memory",
  "Процессор": "Processor",
  "Оперативная память": "System memory",
  "Профиль": "Profile",
  "выбран": "selected",
  "Сколько токенов модель держит в голове за раз. Больше — больше помнит, но дольше думает: на 24 000 токенах первый ответ приходит через 11 секунд.":
    "How many tokens the model holds at once. More means a better memory but slower thinking: at 24,000 tokens the first answer takes 11 seconds.",

  // Загрузка
  "Скачать модель": "Download a model",

  // Движок. Раздел на вкладке «Скачать» идёт перед моделями: без
  // движка модель не запустится, и человек должен видеть это раньше,
  // чем список моделей.
  "Движок": "Engine",
  "Движок найден. Работает на процессоре.":
    "Engine found. Running on the processor.",
  "Движок не найден ни рядом с программой, ни в папке проекта. Модель не запустится.":
    "No engine found next to the program or in the project folder. The model will not start.",
  "Видеокарта: ": "Video card: ",
  "Библиотеки карты на месте — движок работает на видеокарте.":
    "Card libraries are in place — the engine runs on the graphics card.",
  "Видеокарта есть, а библиотек для неё нет: ":
    "A graphics card is present, but its libraries are not: ",
  ". Без них агент пойдёт на процессоре, медленно. Скачивание делаете вы, и только один раз.":
    ". Without them the agent runs on the processor, slowly. You start the download yourself, and only once.",
  "Скачать библиотеки карты": "Download the card libraries",
  "Идёт загрузка…": "Downloading…",
  "Качаем: ": "Downloading: ",
  "Распаковываем: ": "Unpacking: ",
  "Библиотеки карты не нужны: видеокарты NVIDIA на этой машине нет. Модель работает на процессоре.":
    "Card libraries are not needed: there is no NVIDIA card on this machine. The model runs on the processor.",
  "Искали здесь:": "Looked here:",
  "Под текущий профиль нужен квант из списка. Показан размер файла, а не скорость: она станет известна после сверки сводки.":
    "The current profile needs a quant from the list. The file size is shown, not the speed: that only becomes known after checking the summary.",
  "Репозиторий": "Repository",
  "показать файлы": "show files",
  "ищем…": "searching…",
  "по этому слову моделей не нашлось. Проверь написание или укажи репозиторий целиком.":
    "No models found for this word. Check the spelling or give the full repository.",
  "Скачать": "Download",
  "начни набирать: gemma, qwen, llama…":
    "start typing: gemma, qwen, llama…",
  "если не скачивается: нужны разрешённые домены":
    "if it does not download: allowed domains are required",

  // Настройки
  "Размер шрифта": "Font size",
  "Меняет весь интерфейс сразу. Если текст мелкий — поставь побольше, секретарь от этого не тупеет.":
    "Changes the whole interface at once. If the text is too small, make it bigger — the secretary does not get any dumber.",
  "мельче": "smaller",
  "крупнее": "larger",
  "Образец: Проверка распознавания речи. Сегодня вторник.":
    "Sample: Speech recognition check. Today is Tuesday.",

  // Разрозненные подписи, которые поодиночке ни за что не цепляются,
  // но в интерфейсе бросаются в глаза.
  "Модель": "Model",
  "Скорость": "Speed",
  "статус": "status",
  "Готовлюсь…": "Getting ready…",
  "Отправить": "Send",
  "отменить": "cancel",
  "загрузка…": "downloading…",
  "голос включён": "voice on",
};

/** Текущий язык. Модульная переменная, а не состояние React. */
let текущий: Lang = "ru";

/**
 * Подписчики на смену языка.
 *
 * Нужны потому, что `t()` читает модульную переменную: React о такой
 * переменной не знает и сам не перерисуется. Смена языка шлёт событие,
 * подписчики ставят состояние, и дерево перерисовывается целиком.
 */
const слушатели = new Set<() => void>();

export function следить(слушатель: () => void): () => void {
  слушатели.add(слушатель);
  return () => слушатели.delete(слушатель);
}

export function язык(): Lang {
  return текущий;
}

export function поставитьЯзык(новый: Lang) {
  if (новый === текущий) return;
  текущий = новый;
  document.documentElement.lang = новый;
  localStorage.setItem("arkha-lang", новый);
  for (const с of слушатели) с();
}

/** Прочитать язык при запуске. Некорректное значение игнорируем. */
export function восстановитьЯзык(): Lang {
  const сохранён = localStorage.getItem("arkha-lang");
  if (сохранён === "en" || сохранён === "ru") {
    текущий = сохранён;
    document.documentElement.lang = сохранён;
  }
  return текущий;
}

/**
 * Перевести строку.
 *
 * `{}` — место для подстановки. Не используется `{имя}`, потому что
 * ключом служит русский текст, и фигурные скобки внутри него ничего
 * не сломали бы, но помешали бы искать строку глазами.
 *
 * Пропущенный перевод не ошибка: строка остаётся по-русски. Так
 * забытая строка видна, но не ломает страницу.
 */
export function t(русский: string, ...значения: (string | number)[]): string {
  let s = текущий === "en" ? EN[русский] ?? русский : русский;
  for (const v of значения) {
    s = s.replace("{}", String(v));
  }
  return s;
}
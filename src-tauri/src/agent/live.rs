// Живой сайдкар как модель для цикла агента.
//
// Тонкий слой намеренно. Всё, что можно проверить без модели, вынесено
// в [`crate::agent::executor`] и проверено там. Здесь остаётся только
// три вещи: собрать запрос, разобрать поток, отдать [`Turn`].
//
// Стрим переиспользуется из чата: разбор `data:` строк и склейка
// аргументов уже написаны и покрыты тестами в `chat.rs`. Дублировать
// их значило бы со временем разойтись с чатом, а расхождение в разборе
// вызовов инструментов выглядит снаружи как «иногда работает».
//
// **Клиент блокирующий, и это не выбор вкуса.** `block_on` внутри
// команды Tauri упал бы: вызов уже идёт в рантайме tokio, а
// вложенный рантайм там запрещён. Поэтому цикл целиком запускается в
// отдельном потоке (см. [`crate::agent_commands::agent_chat`]), и
// блокирующий клиент там законен. Один поток на один диалог.
//
// Отличие от `chat::chat`: здесь запрос несёт `tools`, а ответ может
// содержать `tool_calls`, и поток читается до конца, а не
// одновременно показывается пользователю. Текст всё равно уходит в
// интерфейс по окончании захода: пользователь должен видеть, что
// секретарь думает, а не зависает на молчании.

use serde_json::Value;
use std::io::Read;

use tauri::{AppHandle, Emitter};

use super::executor::{Message, Model, Turn};
use crate::chat::{self, EVENT_CHUNK, ChatChunk};
use crate::model_manager::Sampling;

/// Что секретарь знает о себе.
///
/// Правило здесь одно и обязательное: **скажи модели, что инструменты
/// есть**. На живом гейте S6 без этой строки модель отвечала «я не могу
/// найти файл в доступных директориях», не вызвав ни одного
/// инструмента: список инструментов она видит, но не знает, что это
/// приказ действовать, а не предложение.
///
/// Три части, и каждая отвечает на реальную ошибку:
///
/// | Часть | Что чинит |
/// |---|---|
/// | «у тебя есть инструменты, используй их» | модель не звонит вообще |
/// | «если инструмент отказал — скажи об этом» | модель молчит про отказ |
/// | «не выдумывай содержимое» | модель отвечает «в файле написано…» без чтения |
/// Основа системной строки.
///
/// Постоянная часть. Динамическая — в [`системная_строка`], потому что
/// список слепых зон зависит от профиля и от того, что загружено.
pub const ОСНОВА: &str = "\
Ты — локальный ИИ-секретарь для Windows. Отвечай по-русски, кратко.

У тебя есть инструменты. Если задача требует узнать что-то о файлах или \
папках — вызови инструмент, а не отвечай по памяти и не отвечай, что у тебя \
нет доступа: доступ есть ровно у тех папок, что перечислены в описании \
инструментов.

Если инструмент отказал, скажи пользователю, что не вышло и почему. Не \
повторяй тот же вызов.

Список папки показывает только имена и размеры. Чтобы узнать, что внутри \
файла, вызови чтение — по одному списку ответить нельзя.

Никогда не выдумывай содержимое файла. Прочитать файл можно только \
инструментом.

Если пользователь называет навык — сначала вызови навык этим именем и \
иди по его шагам. Не выполняй шаги по своему догадку: человек написал \
их заранее.

Открыть программу и переключиться на окно — разные действия. \
«Открой блокнот» означает запустить его (apps.launch), даже если окна \
блокнота сейчас нет. Ищи готовое окно (windows.find) только когда \
человек просит переключиться на уже открытое. Если после запуска \
окна нет, скажи об этом, а не ищи дальше.

Кликнуть можно только по элементу, который ты нашёл по его тексту \
(windows.click). Никогда не угадывай координаты и не кликай наугад.";

/// Системная строка целиком: основа плюс слепые зоны.
///
/// Слепые зоны — то, что на этой машине **не работает**, с причиной.
///
/// Зачем это. Отказы лежат в `capabilities.json`, но в промпт не
/// попадали: модель видела только инструменты, которых у неё нет, и
/// про слепые места не знала. На живой модели задача «посмотри на
/// картинку» дала 2 ответа из 3 «у меня нет доступа к папке» — то есть
/// человек пошёл бы чинить права, которые ни при чём. Не выдумала
/// содержимое, но назвала не ту причину.
///
/// Список строится из реестра, а не пишется руками: запрет может
/// сняться, и тогда строка обязана исчезнуть. Список в промпте,
/// разошедшийся с реальностью, врёт хуже отсутствия списка.
pub fn системная_строка(tier: crate::profile::Tier) -> String {
    let основа = ОСНОВА.to_string();
    let слепые = слепые_зоны(tier);
    if слепые.is_empty() {
        return основа;
    }
    format!(
        "{основа}\n\n\
         Чего ты не умеешь на этой машине. Проверено, а не предположено:\n\
         {слепые}\n\n\
         Если задача требует этого — скажи сразу, что именно не умеешь, \
         и назови причину оттуда. Не выдумывай другую причину: «нет доступа \
         к папке» про изображение неправда, и человек пойдёт чинить права, \
         которые тут ни при чём."
    )
}

/// Слепые зоны по профилю, готовые к вставке.
fn слепые_зоны(tier: crate::profile::Tier) -> String {
    let Ok(реестр) = crate::capabilities::Registry::load() else {
        // Каталог не прочитан. Молчаливый пустой список врёт хуже
        // отсутствия блока: модель решит, что умеет всё.
        return "- список возможностей не прочитан, не обещай ничего \
                незнакомого\n"
            .to_string();
    };
    let слепые: Vec<String> = реестр
        .capabilities
        .iter()
        .filter_map(|в| {
            let блокер = в.blocked_for(tier)?;
            Some(format!("- {}: {}", в.name, в.refusal_for(блокер)))
        })
        .collect();
    if слепые.is_empty() {
        return String::new();
    }
    слепые.join("\n")
}

/// Модель через запущенный сайдкар.
pub struct Sidecar {
    base: String,
    sampling: Sampling,
    app: AppHandle,
    /// Уже показанный текст, чтобы события шли с приростом, а не
    /// повтором целого ответа.
    показан: usize,
}

impl Sidecar {
    pub fn new(base: String, sampling: Sampling, app: AppHandle) -> Self {
        Self { base, sampling, app, показан: 0 }
    }
}

/// Один заход к сайдкару: запрос, поток, ответ.
/// Профиль машины для системной строки.
///
/// Определяется один раз за процесс: слепые зоны зависят от профиля,
/// а профиль за время разговора не меняется. Определять его на каждом
/// заходе — значит спрашивать видеопамять столько же раз, сколько
/// модель сделала запросов.
fn профиль() -> crate::profile::Tier {
    static КЭШ: std::sync::OnceLock<crate::profile::Tier> = std::sync::OnceLock::new();
    *КЭШ.get_or_init(|| crate::profile::detect_and_persist().effective())
}

/// Профиль для тестов и гейтов.
///
/// Гейты задают профиль через `ARSHA_TEST_TIER`, а [`профиль`]
/// читает железо. Читать железо в гейте бессмысленно: гейт проверяет
/// поведение на той машине, где запущен, но проверять поведение для
/// профиля, которого тут нет, — обратное дело.
pub fn профиль_для_тестов() -> crate::profile::Tier {
    match std::env::var("ARSHA_TEST_TIER").unwrap_or_default().as_str() {
        "T1" => crate::profile::Tier::T1,
        "T2" => crate::profile::Tier::T2,
        "T4" => crate::profile::Tier::T4,
        "T5" => crate::profile::Tier::T5,
        _ => crate::profile::Tier::T3,
    }
}

/// Один заход к сайдкару: запрос, поток, ответ.
///
/// Системная строка собирается здесь, а не хранится константой:
/// слепые зоны зависят от профиля.
pub fn one_turn(
    base: &str,
    sampling: &Sampling,
    messages: &[Message],
    tools: &[Value],
) -> Result<Turn, String> {
    one_turn_с(base, sampling, messages, tools, &системная_строка(профиль()))
}

/// Как [`one_turn`], но со строкой от вызывающего.
pub fn one_turn_с(
    base: &str,
    sampling: &Sampling,
    messages: &[Message],
    tools: &[Value],
    system: &str,
) -> Result<Turn, String> {
    // Сообщения агента сериализуются в формате OpenAI: у вызова есть
    // `tool_calls`, у результата — `tool_call_id`. Ровно то, что
    // понимает llama.cpp.
    let mut msgs: Vec<Value> = Vec::with_capacity(messages.len() + 1);
    // Системная строка идёт ПЕРВОЙ и только на первом заходе. На
    // последующих заходах она повторялась бы перед историей, и
    // модель начала бы читать её как часть диалога. Первой она
    // нужна один раз: дальше роль уже понятна из вызовов.
    msgs.push(serde_json::json!({"role": "system", "content": system}));
    for m in messages {
        let mut o = serde_json::Map::new();
        o.insert("role".into(), Value::String(m.role.clone()));
        o.insert("content".into(), Value::String(m.content.clone()));
        if let Some(tc) = &m.tool_calls {
            o.insert("tool_calls".into(), tc.clone());
        }
        if let Some(id) = &m.tool_call_id {
            o.insert("tool_call_id".into(), Value::String(id.clone()));
        }
        if let Some(n) = &m.name {
            o.insert("name".into(), Value::String(n.clone()));
        }
        msgs.push(Value::Object(o));
    }

    let mut payload = serde_json::json!({
        "messages": msgs,
        "max_tokens": 1024,
        "stream": true,
    });
    if let Some(s) = payload.as_object_mut() {
        if let Some(sm) = sampling.to_json().as_object() {
            for (k, v) in sm {
                s.insert(k.clone(), v.clone());
            }
        }
    }
    // Список инструментов — только когда он не пуст. Пустым полем
    // некоторые сборки считают «модель обязана вызвать инструмент».
    if !tools.is_empty() {
        payload["tools"] = Value::Array(tools.to_vec());
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("не удалось создать HTTP-клиент: {e}"))?;

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&payload)
        .send()
        .map_err(|e| format!("запрос к модели не ушёл: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!(
            "модель ответила {status}: {}",
            chat::truncate_pub(&body, 400)
        ));
    }

    let mut resp = resp;
    let mut buffer: Vec<u8> = Vec::new();
    let mut raw = [0u8; 8192];
    let mut text = String::new();
    let mut tool_calls: Option<Value> = None;
    let mut leftovers = String::new();

    loop {
        let n = match resp.read(&mut raw) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(format!("поток прервался: {e}")),
        };
        buffer.extend_from_slice(&raw[..n]);

        // Разбираем только целые строки. Остаток ждёт следующего
        // чтения: резать по буферу нельзя, многострочный JSON
        // развалится.
        while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line).to_string();
            leftovers.clear();
            if let Some(p) = parse_line(&line) {
                let (t, c) = p;
                text.push_str(&t);
                if let Some(calls) = c {
                    tool_calls = Some(match tool_calls {
                        Some(prev) => chat::merge_tool_calls_pub(&prev, &calls),
                        None => calls,
                    });
                }
            }
        }
    }

    Ok(Turn { text, tool_calls })
}

/// Разобрать одну строку `data:` потока.
///
/// Возвращает `(текст, вызовы)`; строка не из потока даёт `None`.
/// Вынесено отдельно, чтобы разбор проверялся тестом без сети.
pub fn parse_line(line: &str) -> Option<(String, Option<Value>)> {
    let payload = line.trim().strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    let obj: Value = serde_json::from_str(payload).ok()?;
    let delta = obj.get("choices")?.get(0)?.get("delta")?;

    let text = delta.get("content").and_then(|c| c.as_str()).unwrap_or_default().to_string();
    let calls = delta
        .get("tool_calls")
        .filter(|c| !c.is_null())
        .cloned();
    Some((text, calls))
}

impl Model for Sidecar {
    fn turn(&mut self, messages: &[Message], tools: &[Value], request_id: u64) -> Result<Turn, String> {
        let t = one_turn(&self.base, &self.sampling, messages, tools)?;

        // Показываем только новое. Иначе на втором заходе пользователь
        // увидит весь ответ заново и решит, что текст задваивается.
        let всего = t.text.chars().count();
        if всего > self.показан {
            let новое: String = t.text.chars().skip(self.показан).collect();
            self.показан = всего;
            let _ = self.app.emit(EVENT_CHUNK, ChatChunk { request_id, delta: новое });
        } else if !t.text.is_empty() {
            self.показан = всего;
        }

        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Слепые зоны обязаны быть в промпте.
    ///
    /// Без них модель на задаче «посмотри на картинку» отвечала «нет
    /// доступа к папке»: инструмента у неё нет, но и знания об этом
    /// нет тоже, поэтому она выдумывала правдоподобную причину.
    #[test]
    fn слепые_зоны_попадают_в_промпт() {
        let строка = системная_строка(crate::profile::Tier::T3);
        assert!(
            строка.contains("Чего ты не умеешь"),
            "в промпте нет блока слепых зон:\n{строка}"
        );
        assert!(
            строка.contains("зрение") || строка.contains("изображен"),
            "зрение должно быть названо своей слепой зоной"
        );
        assert!(
            строка.starts_with(ОСНОВА),
            "основа обязана остаться в начале"
        );
    }

    /// Причина обязана быть названа прямо.
    ///
    /// Без этого блок читается как отписка, и модель продолжает
    /// придумывать своё: «у вас нет прав», «папка не разрешена» —
    /// всё это вызывает у человека поход чинить несуществующую
    /// проблему.
    #[test]
    fn блок_запрещает_выдумывать_причину() {
        let строка = системная_строка(crate::profile::Tier::T3);
        assert!(
            строка.contains("Не выдумывай другую причину"),
            "в промпте нет запрета выдумывать причину"
        );
    }

    /// Зрение не проверено ни на одном профиле — значит, оно должно
    /// быть названо везде.
    ///
    /// Тест на «списки разных профилей различаются» был бы написан
    /// неверно: `vision.read` требует T4, но не проверена, поэтому
    /// попадает в список и на T4, и на T1 — как «ещё не сделано».
    /// Списки совпадут ровно до того момента, когда зрение будет
    /// проверено гейтом; тогда T4 выпадет из списка, а T1 останется.
    #[test]
    fn непроверенное_называется_на_любом_профиле() {
        for профиль in [
            crate::profile::Tier::T1,
            crate::profile::Tier::T2,
            crate::profile::Tier::T3,
            crate::profile::Tier::T4,
            crate::profile::Tier::T5,
        ] {
            let строка = системная_строка(профиль);
            assert!(
                строка.contains("изображен") || строка.contains("зрение"),
                "на профиле {профиль:?} слепое зрение не названо"
            );
        }
    }

    /// Проверенные возможности в список не попадают.
    ///
    /// Иначе секретарь со знакомыми возможностями отказывался бы от
    /// того, что умеет, — и отказ был бы ложью.
    #[test]
    fn проверенные_в_список_не_попадают() {
        let строка = системная_строка(crate::profile::Tier::T3);
        assert!(
            !строка.contains("Управление окнами:"),
            "проверенная возможность не должна быть в слепых зонах:\n{строка}"
        );
        assert!(
            !строка.contains("Навыки:"),
            "навыки проверены гейтом и не должны быть в слепых зонах"
        );
    }
}

#[cfg(test)]
mod tests_потока {
    use super::*;

    #[test]
    fn строка_потока_даёт_текст() {
        let (t, c) = parse_line(r#"data: {"choices":[{"delta":{"content":"привет"}}]}"#).unwrap();
        assert_eq!(t, "привет");
        assert!(c.is_none());
    }

    #[test]
    fn вызов_инструмента_читается_из_потока() {
        let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"fs.read","arguments":"{\"path\":"}}]}}]}"#;
        let (_, c) = parse_line(line).unwrap();
        let c = c.unwrap();
        assert_eq!(c[0]["function"]["name"], "fs.read");
        assert_eq!(c[0]["function"]["arguments"], "{\"path\":");
    }

    #[test]
    fn мусор_в_потоке_не_ломает_разбор() {
        assert!(parse_line("не data").is_none());
        assert!(parse_line("data: {не json").is_none());
        assert!(parse_line("data: [DONE]").is_none());
        assert!(parse_line("data: ").is_none());
        assert!(parse_line("").is_none());
    }

    /// Пустой дельты не должно ломать разбор: сервер шлёт usage в
    /// отдельном сообщении без choices.
    #[test]
    fn сообщение_без_choices_пропускается() {
        assert!(parse_line(r#"data: {"usage":{"prompt_tokens":10}}"#).is_none());
    }
}
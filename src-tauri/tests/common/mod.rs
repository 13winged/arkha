//! Общее для гейтов: подъём сайдкара и разговор с живой моделью.
//!
//! Вынесено из `s6_gate.rs`, потому что S8 требует того же самого
//! адаптера. Дублирование с S6 — зафиксированный долг: две копии
//! сериализатора, который однажды потерял `tool_call_id` и показал
//! 0 из 10. В S6 своя копия осталась нетронутой намеренно, чтобы не
//! сломать проходящий гейт; при объединении S6 проверять первым.

#![allow(dead_code)]

use arkha_lib::agent::executor::{Message, Model, Turn};
use arkha_lib::model_manager::{self, Catalog};
use arkha_lib::profile::Tier;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Parameters of selection. Only one is changed between runs.
#[derive(Debug, Clone, Copy)]
pub struct Выборка {
    pub top_p: bool,
    pub penalty: bool,
}

pub fn выборка() -> Выборка {
    match std::env::var("GATE_SAMPLING").unwrap_or_default().as_str() {
        "min" => Выборка { top_p: false, penalty: false },
        "full" => Выборка { top_p: true, penalty: true },
        _ => Выборка { top_p: true, penalty: true },
    }
}

/// Profile under test.
pub fn tier() -> Tier {
    match std::env::var("ARKHA_TEST_TIER").unwrap_or_default().as_str() {
        "T1" => Tier::T1,
        "T2" => Tier::T2,
        "T4" => Tier::T4,
        "T5" => Tier::T5,
        _ => Tier::T3,
    }
}

/// Environment variable or default.
pub fn env_dir(имя: &str, по_умолчанию: &str) -> PathBuf {
    match std::env::var_os(имя) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(по_умолчанию),
    }
}

/// Model file for the profile. `None` means the gate is skipped.
///
/// `ARKHA_GATE_MODEL` переопределяет файл. Нужно для проверки
/// кандидата на замену: пока гейты не пройдены, ставить `verified:
/// true` в каталоге рано — это заявление, а не измерение, и
/// `recommended()` начал бы выбирать непроверенное как лучшее.
pub fn model_файл() -> Option<PathBuf> {
    let dir = env_dir("ARKHA_MODELS_DIR", "data/models");
    if let Some(путь) = std::env::var_os("ARKHA_GATE_MODEL") {
        let кандидат = PathBuf::from(путь);
        if кандидат.exists() {
            eprintln!("гейт на модели {}", кандидат.display());
            return Some(кандидат);
        }
        eprintln!("ПРОПУЩЕНО: ARKHA_GATE_MODEL={} не найден", кандидат.display());
        return None;
    }
    let catalog = Catalog::load().ok()?;
    let recipe = catalog.recommended(tier()).or_else(|| catalog.fallback(tier()))?;
    let candidate = dir.join(&recipe.file);
    if candidate.exists() {
        return Some(candidate);
    }
    let msg = format!(
        "для профиля {} нет модели в {}. Скачайте загрузчиком или ARKHA_ALLOW_SKIP=1",
        tier().as_str(),
        dir.display()
    );
    if std::env::var("ARKHA_ALLOW_SKIP").is_ok() {
        eprintln!("ПРОПУЩЕНО: {msg}");
        return None;
    }
    panic!("{msg}");
}

/// Поднять сайдкар на порту.
pub fn поднять(порт: u16, метка: &str) -> Option<(model_manager::Sidecar, PathBuf)> {
    let Some(model) = model_файл() else {
        return None;
    };
    let recipe = рецепт();
    eprintln!(
        "{метка}: рецепт {} (verified={})",
        recipe.name, recipe.verified
    );
    let started = model_manager::start(&recipe, &model, порт)
        .unwrap_or_else(|e| panic!("сайдкар не поднялся: {e}"));
    eprintln!(
        "{метка}: размещение {:?}/{:?}",
        started.placement.gpu_layers, started.placement.total_layers
    );
    Some((started, model))
}

fn рецепт() -> model_manager::Recipe {
    let catalog = Catalog::load().expect("каталог рецептов");
    catalog
        .recommended(tier())
        .or_else(|| catalog.fallback(tier()))
        .expect("нужен рецепт")
        .clone()
}

/// Настоящая модель через OpenAI-совместимый API сайдкара.
pub struct Живая {
    pub base: String,
    /// Последнее тело запроса. Для отладки: без него провал гейта
    /// нечем отличить от «модель не позвала».
    pub сырой: Option<String>,
}

impl Живая {
    pub fn новый(base: String) -> Self {
        Self { base, сырой: None }
    }
}

impl Model for Живая {
    fn turn(
        &mut self,
        messages: &[Message],
        tools: &[Value],
        _id: u64,
    ) -> Result<Turn, String> {
        // ВАЖНО: сюда идут ВСЕ роли, включая `tool`. Отбрасывание
        // результатов инструментов делает модель слепой к тому, что
        // уже сделано, и она начинает повторять вызовы.
        let system = arkha_lib::agent::live::системная_строка(arkha_lib::agent::live::профиль_для_тестов());
        self.один_ход(&system, messages, tools)
    }
}

impl Живая {
    fn один_ход(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Value],
    ) -> Result<Turn, String> {
        let mut всё: Vec<Value> = vec![json!({"role": "system", "content": system})];
        for m in messages {
            // Сериализация ровно как в `agent::live`, включая
            // `tool_call_id`: без него результат инструмента не
            // связывается с вызовом, и модель продолжает так, будто
            // вызова не было.
            let mut o = serde_json::Map::new();
            o.insert("role".into(), Value::String(m.role.clone()));
            if let Some(tc) = &m.tool_calls {
                // У сообщения модели с вызовами `content` обязан быть
                // `null`, а не пустой строкой: разницу видно шаблону.
                o.insert("content".into(), Value::Null);
                o.insert("tool_calls".into(), tc.clone());
            } else {
                o.insert("content".into(), Value::String(m.content.clone()));
            }
            if let Some(id) = &m.tool_call_id {
                o.insert("tool_call_id".into(), Value::String(id.clone()));
            }
            if let Some(n) = &m.name {
                o.insert("name".into(), Value::String(n.clone()));
            }
            всё.push(Value::Object(o));
        }

        let mut payload = json!({
            "messages": всё,
            "max_tokens": 1024,
            "stream": false,
            "temperature": 0.3,
        });
        let в = выборка();
        if в.top_p {
            payload["top_p"] = json!(0.9);
        }
        if в.penalty {
            payload["repeat_penalty"] = json!(1.1);
        }
        if !tools.is_empty() {
            payload["tools"] = Value::Array(tools.to_vec());
        }
        if std::env::var("GATE_DEBUG").is_ok() {
            eprintln!("--- ТЕЛО ЗАПРОСА ---");
            eprintln!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| format!("клиент: {e}"))?;
        let resp = client
            .post(format!("{}/v1/chat/completions", self.base))
            .json(&payload)
            .send()
            .map_err(|e| format!("запрос не ушёл: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("сайдкар ответил {}", resp.status()));
        }
        let body: Value = resp.json().map_err(|e| format!("ответ не разобран: {e}"))?;

        let message = &body["choices"][0]["message"];
        let text = message["content"].as_str().unwrap_or_default().to_string();
        let calls = message
            .get("tool_calls")
            .filter(|c| !c.is_null())
            .cloned();
        Ok(Turn { text, tool_calls: calls })
    }
}

/// Порт агента для гейта зрения.
///
/// Отдельный от портов S2/S6/S8: гейт зрения держит **две** модели
/// одновременно, и его нельзя запускать, пока другие гейты заняты
/// своими серверами. Отдельный порт ничего не решает сам по себе —
/// решает требование одного потока.
pub const ПОРТ_АГЕНТА_ДЛЯ_ЗРЕНИЯ: u16 = 18113;

/// Спросить модель текстом и вернуть ответ.
///
/// Без инструментов: гейт зрения проверяет, что агент **не зовёт**
/// инструменты там, где их нет, и отвечает по-русски. С инструментами
/// ответ был бы о том, откажет ли он, а это другой вопрос.
///
/// Системная строка — та же, что у живого агента, вместе с блоком
/// слепых зон. Проверяется ровно то, что увидит человек: если блока
/// нет в этой строке, его нет и в приложении.
pub fn спросить_текстом(base_url: &str, вопрос: &str) -> String {
    let system = arkha_lib::agent::live::системная_строка(
        arkha_lib::agent::live::профиль_для_тестов(),
    );
    let payload = json!({
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": вопрос}
        ],
        "max_tokens": 512,
        "stream": false,
        "temperature": 0.3,
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .expect("клиент");
    let resp = client
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&payload)
        .send()
        .expect("запрос не ушёл");
    assert!(resp.status().is_success(), "сайдкар ответил {}", resp.status());
    let body: Value = resp.json().expect("ответ не разобран");
    body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

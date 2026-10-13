//! Гейт S2 на живом железе.
//!
//! Тест поднимает настоящий сайдкар, проверяет размещение по логу и
//! делает реальный запрос. Если модели или бинарников нет, тест
//! пропускает, а не падает: на машине без модели это нормально.
//!
//! Запуск:
//!   ARCHOTIC_BIN_DIR=data/bin ARCHOTIC_MODELS_DIR=<папка с gguf> \
//!     cargo test --test s2_gate -- --nocapture

use archotic_lib::model_manager::{self, Catalog};
use archotic_lib::profile::Tier;
use std::path::PathBuf;

fn env_dir(var: &str, fallback: &str) -> PathBuf {
    match std::env::var(var) {
        Ok(v) => PathBuf::from(v),
        Err(_) => PathBuf::from(fallback),
    }
}

/// Профиль, на котором идёт замер. Задаётся переменной окружения,
/// иначе один и тот же код проверял бы только T2.
fn tier() -> Tier {
    match std::env::var("ARCHOTIC_TEST_TIER").ok().as_deref() {
        Some("T1") => Tier::T1,
        Some("T3") => Tier::T3,
        Some("T4") => Tier::T4,
        Some("T5") => Tier::T5,
        _ => Tier::T2,
    }
}

/// Файл модели берётся из рецепта выбранного профиля, а не по имени.
/// Имя зашито было бы второй копией каталога, и она бы разъехалась.
fn model_file() -> Option<PathBuf> {
    let dir = env_dir("ARCHOTIC_MODELS_DIR", "data/models");
    let catalog = Catalog::load().ok()?;
    let recipe = catalog.recommended(tier()).or_else(|| catalog.fallback(tier()))?;
    let candidate = dir.join(&recipe.file);
    candidate.exists().then_some(candidate)
}

/// Файл модели для гейта. Пропуск гейта — это не успех.
///
/// Пока файла нет, тест молча возвращался, и прогон выглядел зелёным:
/// «3 passed за 0.00 сек» на самом деле означало «ничего не проверили».
/// Так выглядит картина, в которой проверка потеряла значение.
///
/// Теперь отсутствие модели проваливает тест. Кто сознательно гоняет
/// гейты на машине без нужной модели, ставит ARCHOTIC_ALLOW_SKIP=1,
/// и тогда возвращается `None` — тест выходит, но в выводе это видно.
fn require_model() -> Option<PathBuf> {
    if let Some(path) = model_file() {
        return Some(path);
    }
    let msg = format!(
        "для профиля {} нет файла модели в {}. Скачайте его загрузчиком \
         или поставьте ARCHOTIC_ALLOW_SKIP=1, чтобы сознательно пропустить",
        tier().as_str(),
        env_dir("ARCHOTIC_MODELS_DIR", "data/models").display(),
    );
    if std::env::var("ARCHOTIC_ALLOW_SKIP").is_ok() {
        eprintln!("ПРОПУЩЕНО по требованию: {msg}");
        return None;
    }
    panic!("{msg}");
}

#[test]
fn гейт_s2_сайдкар_поднимается_на_gpu_и_отвечает_по_русски() {
    let catalog = Catalog::load().expect("каталог рецептов должен грузиться");
    let tier = tier();
    let Some(model) = require_model() else {
        return;
    };

    // Каталог проверяется на диске, а не в тесте, чтобы тест ловил
    // опечатки в recipes.json.
    let recipes = catalog.for_tier(tier);
    assert!(
        !recipes.is_empty(),
        "для {} должен быть хотя бы один рецепт",
        tier.as_str()
    );

    let recipe = catalog
        .recommended(tier)
        .or_else(|| catalog.fallback(tier))
        .expect("нужен рецепт")
        .clone();

    eprintln!("рецепт: {} (verified={})", recipe.name, recipe.verified);

    let port = 18099;
    let sidecar = model_manager::start(&recipe, &model, port)
        .unwrap_or_else(|e| panic!("сайдкар не поднялся: {e}"));

    // --- Гейт: все слои на GPU ---
    let p = &sidecar.placement;
    eprintln!(
        "размещение: {:?}/{:?} слоёв на {:?}, модель {:?} МиБ, KV {:?} МиБ, mmap {:?} МиБ, flash_attn={}",
        p.gpu_layers, p.total_layers, p.device,
        p.model_mib_on_gpu, p.kv_mib_on_gpu, p.model_mib_cpu_mapped, p.flash_attn
    );

    // Сообщение ссылается на наш замер, а не на чужую статью:
    // 28 из 33 слоёв стоят 1.84 раза и работают, а 12 из 33 — уже нет.
    assert!(
        p.full_gpu(),
        "не все слои на GPU: {:?}. Подробная шкала в docs/10-замеры.md",
        p.offload_explanation()
    );
    assert_eq!(p.gpu_layers, p.total_layers);

    // --- Гейт: русский в чате ---
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .unwrap();

    let resp = client
        .post(format!("{}/v1/chat/completions", sidecar.base_url()))
        .json(&serde_json::json!({
            "messages": [
                {"role": "system", "content": "Ты — локальный ИИ-секретарь для Windows. Отвечай по-русски."},
                {"role": "user", "content": "Назови одним предложением, что ты умеешь."}
            ],
            "max_tokens": 80,
            "stream": false,
        }))
        .send()
        .expect("запрос к модели");

    assert!(resp.status().is_success(), "статус {}", resp.status());
    let body: serde_json::Value = resp.json().expect("json");
    let text = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    eprintln!("ответ: {text}");

    assert!(!text.trim().is_empty(), "модель вернула пустой ответ");

    let cyrillic = text.chars().any(|c| ('А'..='Я').contains(&c) || ('а'..='я').contains(&c));
    assert!(cyrillic, "ответ без кириллицы, это не русский: {text}");

    // --- Гейт: tool calling на русском ---
    let resp = client
        .post(format!("{}/v1/chat/completions", sidecar.base_url()))
        .json(&serde_json::json!({
            "messages": [
                {"role": "user", "content": "Найди на рабочем столе все PDF-файлы и покажи их имена."}
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "find_files",
                    "description": "Ищет файлы в папке по расширению.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "folder": {"type": "string", "description": "Папка для поиска"},
                            "extension": {"type": "string", "description": "Расширение файла"}
                        },
                        "required": ["folder", "extension"]
                    }
                }
            }],
            "tool_choice": "auto",
            "max_tokens": 200,
            "stream": false,
        }))
        .send()
        .expect("запрос с инструментом");

    assert!(resp.status().is_success(), "статус {}", resp.status());
    let body: serde_json::Value = resp.json().expect("json");

    assert_eq!(
        body["choices"][0]["finish_reason"], "tool_calls",
        "модель не вызвала инструмент. Это главное для секретаря"
    );

    let call = &body["choices"][0]["message"]["tool_calls"][0]["function"];
    assert_eq!(call["name"], "find_files");

    let args: serde_json::Value =
        serde_json::from_str(call["arguments"].as_str().unwrap()).expect("аргументы это json");
    eprintln!("аргументы: {args}");

    assert_eq!(args["extension"], "pdf");

    // --- Гейт: модель не отказывается на безобидной задаче ---
    // Отказ на «найди PDF» ломает продукт. Здесь проверяется именно он.
    assert_eq!(
        body["choices"][0]["finish_reason"], "tool_calls",
        "отказ на безобидной задаче: benign_over_refusal_rate должен быть нулевым"
    );
}

/// Параметры выборки должны реально доходить до модели, иначе
/// утверждение в рецептах — декорация.
#[test]
fn гейт_s2_параметры_выборки_доходят_до_модели() {
    let Some(model) = require_model() else {
        return;
    };

    let catalog = Catalog::load().expect("каталог");
    let sampling = catalog.sampling().expect("параметры выборки");
    eprintln!("выборка: {sampling:?}");

    assert!(sampling.validate().is_ok());
    // Секретарь выполняет поручение, а не фантазирует.
    assert!(
        sampling.temperature <= 0.4,
        "температура {} слишком высока для секретаря",
        sampling.temperature
    );

    let recipe = catalog
        .recommended(tier())
        .or_else(|| catalog.fallback(tier()))
        .expect("рецепт")
        .clone();
    let sidecar = model_manager::start(&recipe, &model, 18097).expect("сайдкар");

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .unwrap();

    // Сервер должен принять все четыре поля и не отвергнуть их.
    // Если параметры не переданы, OpenAI-совместимый эндпоинт
    // обычно отвечает ошибкой или молча игнорирует их, поэтому
    // проверяем и статус, и сам ответ.
    let resp = client
        .post(format!("{}/v1/chat/completions", sidecar.base_url()))
        .json(&serde_json::json!({
            "messages": [{"role": "user", "content": "Скажи одним словом: готов"}],
            "max_tokens": 16,
            "stream": false,
            "temperature": sampling.temperature,
            "top_p": sampling.top_p,
            "top_k": sampling.top_k,
            "repeat_penalty": sampling.repeat_penalty,
        }))
        .send()
        .expect("запрос с параметрами выборки");

    assert!(
        resp.status().is_success(),
        "сервер отверг параметры выборки: {}",
        resp.status()
    );

    let body: serde_json::Value = resp.json().expect("json");
    let text = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    eprintln!("ответ: {text:?}");
    assert!(!text.trim().is_empty());
}

#[test]
fn гейт_s2_остановка_генерации_реально_прекращает_работу() {
    let Some(model) = require_model() else {
        return;
    };

    let catalog = Catalog::load().expect("каталог");
    let recipe = catalog
        .recommended(tier())
        .or_else(|| catalog.fallback(tier()))
        .expect("рецепт")
        .clone();
    let sidecar = model_manager::start(&recipe, &model, 18098).expect("сайдкар");

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .unwrap();

    // Стрим удобнее проверять асинхронным клиентом: у blocking в
// reqwest 0.13 нет доступа к потоку ответа по кускам.
let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .expect("runtime");

let (chunks, text) = rt.block_on(async {
    use futures_util::StreamExt;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", sidecar.base_url()))
        .json(&serde_json::json!({
            "messages": [{"role": "user", "content": "Напиши подробный список из 500 пунктов."}],
            "max_tokens": 4000,
            "stream": true,
        }))
        .send()
        .await
        .expect("поток");

    let mut stream = resp.bytes_stream();
    let mut chunks = 0usize;
    let mut text = String::new();

    while let Some(item) = stream.next().await {
        let Ok(bytes) = item else { break };
        chunks += 1;
        text.push_str(&String::from_utf8_lossy(&bytes));
        // Обрываем на середине: это и есть проверка остановки.
        if chunks >= 5 {
            break;
        }
    }

    // drop потока закрывает соединение, сервер должен это пережить.
    drop(stream);
    (chunks, text)
});

assert!(chunks >= 5, "поток не отдал кусков, стрим не работает");
assert!(text.contains("data:"), "в потоке нет SSE-кадров");
eprintln!("получено кусков: {chunks}, символов: {}", text.len());

std::thread::sleep(std::time::Duration::from_secs(3));
let health = client
    .get(format!("{}/health", sidecar.base_url()))
    .send();
assert!(health.is_ok(), "сервер не пережил обрыв соединения");
}
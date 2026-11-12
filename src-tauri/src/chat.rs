// S2  Чат: запрос к сайдкару, стрим, остановка.
//
// Поток идёт из Rust в интерфейс по событиям Tauri, а не через
// опрос: пользователь должен видеть первый токен сразу.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::path::{Path, PathBuf};
// `Manager` нужен ради `AppHandle::state`: работа запуска ушла в
// отдельный поток и достаёт состояние уже оттуда, а не из параметров
// команды. Без него состояние не достать вовсе.
use tauri::{AppHandle, Emitter, Manager, State};

use std::sync::Arc;

use crate::downloader;
use crate::model_manager::{self, SidecarState};

/// Идентификатор текущего запроса. Нужен чтобы события старого
/// запроса не дописывали в новый.
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);
/// Флаг отмены текущего запроса.
static CANCELLED: AtomicBool = AtomicBool::new(false);
/// Флаг отмены текущей загрузки модели.
static DOWNLOAD_CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct ChatChunk {
    pub request_id: u64,
    pub delta: String,
}

#[derive(Clone, Serialize)]
pub struct ChatDone {
    pub request_id: u64,
    /// Причина завершения: done, stop, error.
    pub outcome: String,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    /// Токенов в секунду по данным сервера, если он их прислал.
    pub tok_s: Option<f32>,
    pub error: Option<String>,
    pub tool_calls: Option<serde_json::Value>,
}

#[derive(Clone, Serialize)]
pub struct ModelStatus {
    pub running: bool,
    pub recipe: Option<String>,
    pub kv_cache: Option<String>,
    pub context: Option<u32>,
    pub tok_s: Option<f32>,
    pub verified: bool,
    pub full_gpu: bool,
    pub gpu_layers: Option<u32>,
    pub total_layers: Option<u32>,
    pub device: Option<String>,
    pub flash_attn: bool,
    pub error: Option<String>,
}

pub const EVENT_CHUNK: &str = "chat:chunk";
pub const EVENT_DONE: &str = "chat:done";

/// Состояние модели, о которой рассказываем интерфейсу.
pub struct ModelState(pub std::sync::Mutex<Option<ModelStatus>>);

impl ModelState {
    pub fn new() -> Self {
        Self(std::sync::Mutex::new(None))
    }

    pub fn get(&self) -> ModelStatus {
        self.0
            .lock()
            .expect("model lock poisoned")
            .clone()
            .unwrap_or(ModelStatus {
                running: false,
                recipe: None,
                kv_cache: None,
                context: None,
                tok_s: None,
                verified: false,
                full_gpu: false,
                gpu_layers: None,
                total_layers: None,
                device: None,
                flash_attn: false,
                error: None,
            })
    }
}

// ---------------------------------------------------------------------------
// Команды
// ---------------------------------------------------------------------------

/// Запускает модель по профилю. Транзакционно: если рецепт не
/// подошёл, пробуется следующий, и пользователю показывается что
/// именно произошло.
/// Запустить модель.
///
/// Вся работа уходит в отдельный блокирующий поток, и это не
/// оптимизация, а требование. Внутри используется
/// `reqwest::blocking`, а он создаёт собственное окружение tokio
/// ради каждого запроса. Ронять такое окружение в асинхронном
/// контексте запрещено, и процесс падал с
/// «Cannot drop a runtime in a context where blocking is not
/// allowed» — то есть модель не запускалась вовсе, а человек видел
/// только панику в консоли.
///
/// Второй довод: модель грузится десятки секунд. Если команда
/// выполнялась бы прямо в асинхронном потоке, окно замёрзло бы на
/// всё это время.
#[tauri::command]
pub async fn start_model(app: AppHandle) -> Result<ModelStatus, String> {
    crate::auth::require_authenticated()?;
    tauri::async_runtime::spawn_blocking(move || старт_на_потоке(&app))
        .await
        .map_err(|e| format!("поток запуска модели оборвался: {e}"))?
}

/// Сама работа запуска. Вынесена отдельно, чтобы и её сигнатура не
/// совпадала с командой: иначе легко снова вызвать блокирующее из
/// асинхронного, не заметив.
fn старт_на_потоке(app: &AppHandle) -> Result<ModelStatus, String> {
    let sidecar = app.state::<SidecarState>();
    let status = app.state::<ModelState>();

    let tier = crate::profile::detect_and_persist().effective();
    let catalog = model_manager::Catalog::load()?;

    let recipes: Vec<model_manager::Recipe> = catalog.for_tier(tier).to_vec();
    if recipes.is_empty() {
        let err = format!("в каталоге нет рецептов для профиля {}", tier.as_str());
        set_error(&status, &err);
        return Err(err);
    }

    let models = model_manager::models_dir();
    std::fs::create_dir_all(&models)
        .map_err(|e| format!("не удалось создать папку моделей: {e}"))?;

    let port = 18080;
    let selected_model = crate::settings::settings_get()?.active_model;
    let (started, outcome) = model_manager::start_transactional_selected(
        tier,
        &catalog,
        &models,
        port,
        selected_model.as_deref(),
    )
        .map_err(|e| {
            set_error(&status, &e);
            e
        })?;

    *sidecar.inner.lock().expect("sidecar lock poisoned") = Some(started);

    let s = sidecar.with(|s| {
        let p = &s.placement;
        ModelStatus {
            running: true,
            recipe: Some(s.recipe.name.clone()),
            kv_cache: Some(s.recipe.kv_cache.clone()),
            context: Some(s.recipe.context),
            tok_s: s.recipe.tok_s,
            verified: s.recipe.verified,
            full_gpu: p.full_gpu(),
            gpu_layers: p.gpu_layers,
            total_layers: p.total_layers,
            device: p.device.clone(),
            flash_attn: p.flash_attn,
            error: None,
        }
    })?;

    // Если выбран неверифицированный рецепт, это видно пользователю.
    if !s.verified {
        let _ = set_error(
            &status,
            "рецепт не проходил замер, скорость неизвестна",
        );
    }
    let _ = outcome;

    *status.0.lock().expect("model lock poisoned") = Some(s.clone());
    Ok(s)
}

#[tauri::command]
pub async fn models_open_window(app: AppHandle) -> Result<(), String> {
    crate::auth::require_authenticated()?;
    if let Some(window) = app.get_webview_window("models") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        "models",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("ArkHa — Модели")
    .inner_size(980.0, 720.0)
    .min_inner_size(720.0, 480.0)
    .build()
    .map_err(|error| format!("не удалось открыть окно моделей: {error}"))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub active: bool,
    pub supported: bool,
}

#[tauri::command]
pub fn models_list() -> Result<Vec<LocalModel>, String> {
    crate::auth::require_authenticated()?;
    let directory = model_manager::models_dir();
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let settings = crate::settings::settings_get()?;
    let catalog = model_manager::Catalog::load().ok();
    let mut models = Vec::new();
    for entry in std::fs::read_dir(&directory).map_err(|error| error.to_string())?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("gguf") {
            continue;
        }
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or_default().to_string();
        let supported = catalog.as_ref().is_some_and(|catalog| {
            catalog
                .for_tier(crate::profile::detect_and_persist().effective())
                .iter()
                .any(|recipe| recipe.file == name)
        });
        let size_bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        models.push(LocalModel {
            active: settings.active_model.as_deref() == Some(name.as_str()),
            name,
            path: path.to_string_lossy().to_string(),
            size_bytes,
            supported,
        });
    }
    models.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(models)
}

#[tauri::command]
pub fn model_set_active(name: String) -> Result<Vec<LocalModel>, String> {
    crate::auth::require_authenticated()?;
    if name.contains(['/', '\\', ':']) || name != Path::new(&name).file_name().and_then(|value| value.to_str()).unwrap_or_default() {
        return Err("недопустимое имя модели".to_string());
    }
    let directory = model_manager::models_dir();
    let path = directory.join(&name);
    if path.extension().and_then(|extension| extension.to_str()) != Some("gguf") || !path.is_file() {
        return Err("модель не найдена".to_string());
    }
    let mut settings = crate::settings::settings_get()?;
    settings.active_model = Some(name);
    crate::settings::settings_save(settings)?;
    models_list()
}

/// Добавить модель из локального файла в общую папку моделей.
///
/// Нужна, когда файл уже скачан другим способом: качать 5 ГБ заново
/// только потому, что приложение его не видело, — бессмысленно.
#[tauri::command]
pub async fn model_add(path: String) -> Result<Vec<LocalModel>, String> {
    crate::auth::require_authenticated()?;
    let source = PathBuf::from(path.trim());
    if !source.is_file() {
        return Err(format!("файл не найден: {}", source.display()));
    }
    if source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("gguf"))
        != Some(true)
    {
        return Err("нужен файл .gguf".to_string());
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "у файла нет имени".to_string())?
        .to_string();

    let directory = model_manager::models_dir();
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let destination = directory.join(&name);
    if destination.exists() {
        return Err(format!("модель {name} уже есть в библиотеке"));
    }
    if source == destination {
        return Err("файл уже лежит в папке моделей".to_string());
    }

    // Копирование гигабайтов не должно занимать поток интерфейса.
    let from = source.clone();
    let to = destination.clone();
    tauri::async_runtime::spawn_blocking(move || std::fs::copy(&from, &to))
        .await
        .map_err(|error| format!("копирование прервано: {error}"))?
        .map_err(|error| format!("не удалось скопировать {}: {error}", source.display()))?;

    models_list()
}

#[tauri::command]
pub fn model_delete(name: String, status: State<'_, ModelState>) -> Result<Vec<LocalModel>, String> {    crate::auth::require_authenticated()?;
    let _ = (name, status);
    Err("общие модели доступны только для чтения и не могут быть удалены из профиля".into())
}

#[tauri::command]
pub fn stop_model(sidecar: State<'_, SidecarState>, status: State<'_, ModelState>) -> Result<(), String> {
    crate::auth::require_authenticated()?;
    sidecar.stop();
    *status.0.lock().expect("model lock poisoned") = None;
    Ok(())
}

/// Что с моделью сейчас.
///
/// Не возвращает запомненное состояние вслепую. Модель могла упасть
/// сама - нехватка видеопамяти, сбой драйвера, ошибка в весах, - и
/// тогда состояние в памяти продолжало врать «запущена», а
/// интерфейс показывал работающую модель и кнопку «остановить»,
/// которая ничего не останавливала. Именно это случилось на живой
/// машине.
#[tauri::command]
pub fn model_status(
    status: State<'_, ModelState>,
    sidecar: State<'_, SidecarState>,
) -> Result<ModelStatus, String> {
    crate::auth::require_authenticated()?;
    // Причина забирается ДО проверки живости. Обратный порядок ничего
    // не даёт: проверка живости убирает мёртвый сайдкар, а причина
    // падения хранилась именно в нём, и на второй запрос её уже
    // было негде взять. На живой машине интерфейс молчал о падении
    // именно поэтому.
    if let Some(почему) = sidecar.упал_сам() {
        // Снять прежнее состояние раньше записи: иначе человек увидит
        // упавшую модель, все равно помеченную как работающая.
        *status.0.lock().expect("model lock poisoned") = None;
        set_error(&status, &почему);
        return Ok(status.get());
    }

    if !sidecar.is_running() {
        // Сообщение о падении, если оно уже записано, остаётся.
        //
        // Без этой оговорки оно жило пять секунд: интерфейс опрашивает
        // статус каждые пять секунд, и на втором опросе падение уже не
        // повторялось — сайдкара не было — и состояние затиралось.
        // Человек как раз в этот момент читал, что случилось, и
        // текст исчезал у него перед глазами.
        let прежнее = status.get();
        if прежнее.error.is_some() {
            return Ok(прежнее);
        }
        *status.0.lock().expect("model lock poisoned") = None;
    }
    Ok(status.get())
}

/// Последние строки лога сайдкара. Пользователь должен видеть, что
/// происходит: «грузится», «слои на GPU», «ошибка».
#[tauri::command]
pub fn model_log(sidecar: State<'_, SidecarState>, lines: Option<usize>) -> Result<Vec<String>, String> {
    crate::auth::require_authenticated()?;
    let n = lines.unwrap_or(80);
    Ok(sidecar.with(|s| s.log.tail(n)).unwrap_or_default())
}

/// Проверка перед запуском: показывает все проблемы разом.
///
/// Раньше пользователь нажимал «Запустить модель» и получал одну
/// ошибку, потом нажимал снова и получал следующую. Теперь список
/// полный с первого раза.
#[tauri::command]
pub fn preflight_model() -> Result<model_manager::Preflight, String> {
    crate::auth::require_authenticated()?;
    let tier = crate::profile::detect_and_persist().effective();
    let catalog = model_manager::Catalog::load()?;
    let models = model_manager::models_dir();
    Ok(model_manager::preflight(tier, &catalog, &models))
}

/// Список файлов в репозитории с размерами и сводками.
/// Подсказки по названию модели. Пустой запрос даёт пустой список,
/// а не ошибку: человек ещё не начал печатать.
#[tauri::command]
pub async fn search_models(query: String) -> Result<Vec<downloader::ModelHit>, String> {
    crate::auth::require_authenticated()?;
    downloader::search_models(&query).await
}

#[tauri::command]
pub async fn repo_files(repo: String) -> Result<Vec<downloader::RemoteFile>, String> {
    crate::auth::require_authenticated()?;
    Ok(downloader::fetch_repo(&repo).await?.files)
}

/// Домены, которые должны быть разрешены в сети. Файл отдаётся не
/// с huggingface.co, а с CDN после переадресации, и во многих
/// корпоративных сетях именно он заблокирован.
#[tauri::command]
pub async fn download_hosts() -> Result<Vec<String>, String> {
    crate::auth::require_authenticated()?;
    downloader::required_hosts().await
}

const EVENT_DOWNLOAD: &str = "download:progress";

/// Скачивает модель для текущего профиля.
///
/// Отдельная команда, а не часть запуска: скачивание занимает
/// минуты, а пользователь должен видеть прогресс и уметь отменить.
#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    file: String,
    repo: Option<String>,
) -> Result<String, String> {
    crate::auth::require_authenticated()?;
    let tier = crate::profile::detect_and_persist().effective();
    let catalog = model_manager::Catalog::load()?;
    let recipe = catalog
        .recommended(tier)
        .or_else(|| catalog.fallback(tier))
        .ok_or_else(|| format!("нет рецептов для профиля {}", tier.as_str()))?;

    let repo = repo.unwrap_or_else(|| {
        catalog
            .llm_repo(tier)
            .unwrap_or("unsloth/Qwen3.5-9B-GGUF")
            .to_string()
    });

    let remote = downloader::fetch_repo(&repo).await?;
    let target = downloader::find_file(&remote, &file)?;

    let app_for_cb = app.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_cb = cancel.clone();

    let path = downloader::download(
        &repo,
        target,
        &model_manager::models_dir(),
        cancel,
        Box::new(move |p| {
            let _ = app_for_cb.emit(EVENT_DOWNLOAD, &p);
            let _ = &cancel_for_cb;
        }),
    )
    .await?;

    // Имя файла должно совпасть с тем, что ждёт рецепт, иначе
    // приложение скачает модель и не сможет её запустить.
    let expected = recipe.file.rsplit('/').next().unwrap_or(&recipe.file);
    let got = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    if got != expected {
        return Err(format!(
            "скачан файл {got}, а рецепту «{}» нужен {expected}",
            recipe.name
        ));
    }

    Ok(path.display().to_string())
}

#[tauri::command]
pub fn cancel_download() -> Result<(), String> {
    crate::auth::require_authenticated()?;
    DOWNLOAD_CANCEL.store(true, Ordering::SeqCst);
    Ok(())
}

/// Останавливает текущую генерацию. Модель остаётся запущенной,
/// останавливается только ответ.
#[tauri::command]
pub fn stop_generation() -> Result<(), String> {
    crate::auth::require_authenticated()?;
    CANCELLED.store(true, Ordering::SeqCst);
    Ok(())
}

// ---------------------------------------------------------------------------
// Стрим
// ---------------------------------------------------------------------------

/// Результат короткого замера скорости модели.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchResult {
    pub tok_s: f32,
    pub completion_tokens: u32,
    pub prompt_tokens: u32,
    pub elapsed_ms: u64,
}

/// Приблизительная скорость модели прямо сейчас.
///
/// Один короткий запрос к уже запущенному сайдкару. Скорость берём из
/// `timings`, которые считает сам llama-server; если их нет — делим
/// число токенов на время ответа. Число приблизительное по построению:
/// 64 токена, один прогон, без прогрева, — и подписано именно так,
/// чтобы его не приняли за паспортную цифру.
#[tauri::command]
pub async fn model_bench(sidecar: State<'_, SidecarState>) -> Result<BenchResult, String> {
    crate::auth::require_authenticated()?;
    let base = sidecar.base_url()?;
    let started = std::time::Instant::now();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("не удалось создать HTTP-клиент: {e}"))?;

    let payload = serde_json::json!({
        "messages": [{
            "role": "user",
            "content": "Перечисли числа от 1 до 24 через запятую, без пояснений."
        }],
        "max_tokens": 64,
        "temperature": 0.0,
        "stream": false,
    });

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("запрос к модели не ушёл: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "модель ответила {status}: {}",
            body.chars().take(300).collect::<String>()
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("ответ модели не разобран: {e}"))?;

    let usage = body.get("usage");
    let completion = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let prompt = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let elapsed = started.elapsed();

    let tok_s = body
        .get("timings")
        .and_then(|t| t.get("predicted_per_second"))
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .filter(|v| *v > 0.0)
        .or_else(|| {
            let secs = elapsed.as_secs_f32();
            (completion > 0 && secs > 0.0).then(|| completion as f32 / secs)
        })
        .ok_or_else(|| "модель не сообщила ни токенов, ни времени".to_string())?;

    Ok(BenchResult {
        tok_s,
        completion_tokens: completion,
        prompt_tokens: prompt,
        elapsed_ms: elapsed.as_millis() as u64,
    })
}

/// Отправляет сообщение и стримит ответ.
///
/// Останавливается по флагу отмены: пользователь ждёт до 15 секунд
/// разбора длинного контекста, и это время он должен контролировать.
#[tauri::command]
pub async fn chat(
    app: AppHandle,
    sidecar: State<'_, SidecarState>,
    messages: Vec<ChatMessage>,
    max_tokens: Option<u32>,
    request_id: Option<u64>,
) -> Result<u64, String> {
    crate::auth::require_authenticated()?;
    let base = sidecar.base_url()?;
    let user = crate::settings::settings_get()?.user;
    let mut messages = messages;
    if !user.login.is_empty() || !user.display_name.is_empty() {
        let identity = format!(
            "Локальный профиль пользователя: имя «{}», login «{}». Используй это как устойчивый контекст пользователя. Пароль и секреты не передаются в модель.",
            if user.display_name.is_empty() { "не указано" } else { &user.display_name },
            if user.login.is_empty() { "не указано" } else { &user.login },
        );
        messages.insert(0, ChatMessage { role: "system".into(), content: identity, attachments: Vec::new() });
    }
    let attachment_ids = messages.iter().flat_map(|message| message.attachments.iter().cloned()).collect::<Vec<_>>();
    if let Some(context) = crate::attachments::context(&attachment_ids)? {
        messages.insert(0, ChatMessage { role: "system".into(), content: context, attachments: Vec::new() });
    }
    // Номер запроса может прийти из интерфейса, и это не мелочь.
    //
    // Раньше он выдавался здесь и возвращался в конце функции, то
    // есть уже после того, как весь поток прочитан и события
    // отправлены. Интерфейс записывал полученное значение уже
    // после того, как все события пришли, и проверка «это мой
    // запрос?» отбрасывала их все. Ответ не появлялся никогда,
    // а кнопка «остановить» висела до перезапуска. Замечено
    // 15.10.2026.
    //
    // Теперь номер задаёт интерфейс, до отправки запроса, и
    // события приходят уже с тем же номером. Свой счётчик здесь
    // остаётся для вызовов без номера — из гейтов и тестов.
    let свой = REQUEST_ID.fetch_add(1, Ordering::SeqCst);
    let request_id = request_id.unwrap_or(свой);
    CANCELLED.store(false, Ordering::SeqCst);

    let max_tokens = max_tokens.unwrap_or(1024);

    // Параметры выборки приходят из каталога рецептов, а не из шаблона
    // модели. При температуре по умолчанию секретарь будет выдумывать:
    // называть несуществующие файлы и объявлять задачу выполненной.
    let sampling = match model_manager::Catalog::load().and_then(|c| c.sampling()) {
        Ok(s) => s,
        Err(e) => {
            // Каталог сломан, но чат должен работать: подставляем
            // безопасные значения и говорим, что прописаны не они.
            eprintln!("параметры выборки не из каталога, беру безопасные: {e}");
            model_manager::Sampling::default_for_secrets()
        }
    };

    // Параметры выборки собираются в одном месте — `to_json`, — а не
    // перечисляются здесь поимённо. Список здесь означал, что новое
    // поле можно забыть: `enable_thinking` так и был забыт, и чат
    // не отвечал (замер 15.10.2026 в docs/10).
    let mut payload = sampling.to_json();
    payload["messages"] = serde_json::json!(messages);
    payload["max_tokens"] = serde_json::json!(max_tokens);
    payload["stream"] = serde_json::json!(true);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("не удалось создать HTTP-клиент: {e}"))?;

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("запрос к модели не ушёл: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let _ = app.emit(
            EVENT_DONE,
            ChatDone {
                request_id,
                outcome: "error".into(),
                prompt_tokens: None,
                completion_tokens: None,
                tok_s: None,
                error: Some(format!("модель ответила {status}: {}", truncate(&body, 400))),
                tool_calls: None,
            },
        );
        return Err(format!("модель ответила {status}"));
    }

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut tool_calls: Option<serde_json::Value> = None;
    let mut prompt_tokens: Option<u32> = None;
    let mut completion_tokens: Option<u32> = None;
    let mut tok_s: Option<f32> = None;

    use futures_util::StreamExt;

    loop {
        if CANCELLED.load(Ordering::SeqCst) {
            let _ = app.emit(
                EVENT_DONE,
                ChatDone {
                    request_id,
                    outcome: "stop".into(),
                    prompt_tokens,
                    completion_tokens,
                    tok_s,
                    error: None,
                    tool_calls,
                },
            );
            return Ok(request_id);
        }

        let chunk = match stream.next().await {
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                let _ = app.emit(
                    EVENT_DONE,
                    ChatDone {
                        request_id,
                        outcome: "error".into(),
                        prompt_tokens,
                        completion_tokens,
                        tok_s,
                        error: Some(format!("поток прервался: {e}")),
                        tool_calls,
                    },
                );
                return Err(format!("поток прервался: {e}"));
            }
            None => break,
        };

        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find('\n') {
            let line: String = buffer.drain(..=pos).collect();
            let line = line.trim();
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload.is_empty() {
                continue;
            }
            if payload == "[DONE]" {
                continue;
            }

            let Ok(obj) = serde_json::from_str::<serde_json::Value>(payload) else {
                continue;
            };

            if let Some(t) = obj.get("timings") {
                if let Some(v) = t.get("predicted_per_second").and_then(|v| v.as_f64()) {
                    tok_s = Some(v as f32);
                }
            }

            let Some(delta) = obj
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("delta"))
            else {
                continue;
            };

            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
                    let _ = app.emit(
                        EVENT_CHUNK,
                        ChatChunk {
                            request_id,
                            delta: content.to_string(),
                        },
                    );
                }
            }

            // Вызовы инструментов приходят отдельным полем.
            if let Some(calls) = delta.get("tool_calls") {
                if !calls.is_null() {
                    tool_calls = Some(match tool_calls {
                        Some(prev) => merge_tool_calls(&prev, calls),
                        None => calls.clone(),
                    });
                }
            }

            if let Some(usage) = obj.get("usage") {
                if let Some(v) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                    prompt_tokens = Some(v as u32);
                }
                if let Some(v) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                    completion_tokens = Some(v as u32);
                }
            }
        }
    }

    let _ = app.emit(
        EVENT_DONE,
        ChatDone {
            request_id,
            outcome: "done".into(),
            prompt_tokens,
            completion_tokens,
            tok_s,
            error: None,
            tool_calls,
        },
    );

    Ok(request_id)
}

/// Склеивает части одного вызова инструмента, приходящие потоком.
fn merge_tool_calls(prev: &serde_json::Value, next: &serde_json::Value) -> serde_json::Value {
    let mut merged: Vec<serde_json::Value> = prev.as_array().cloned().unwrap_or_default();
    let incoming = next.as_array().cloned().unwrap_or_default();

    for call in incoming {
        let index = call.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
        while merged.len() <= index {
            merged.push(serde_json::json!({}));
        }
        let slot = &mut merged[index];
        if let Some(f) = call.get("function").and_then(|f| f.get("name")).and_then(|n| n.as_str())
        {
            slot["function"]["name"] = serde_json::Value::String(f.to_string());
        }
        // Аргументы приходят кусками, их надо склеить, а не заменить.
        if let Some(args) = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
        {
            let existing = slot["function"]["arguments"].as_str().unwrap_or("").to_string();
            slot["function"]["arguments"] = serde_json::Value::String(format!("{existing}{args}"));
        }
    }

    serde_json::Value::Array(merged)
}

/// Обрезка для сообщений об ошибке. Публичная, потому что агентный
/// слой показывает пользователю те же обрезки: разные правила обрезки
/// в двух местах выглядят как «иногда сообщение обрывается».
pub fn truncate_pub(s: &str, n: usize) -> String {
    truncate(s, n)
}

/// Склейка вызовов инструментов для агентного слоя. Та же функция, что
/// и в чате: две реализации разбора расходятся, и расхождение видно
/// только как «иногда инструмент не вызывается».
pub fn merge_tool_calls_pub(prev: &serde_json::Value, next: &serde_json::Value) -> serde_json::Value {
    merge_tool_calls(prev, next)
}

/// Сбросить флаг отмены. Отдельная функция, чтобы агентный слой не
/// трогал внутренний флаг напрямую.
pub fn reset_cancel() {
    CANCELLED.store(false, Ordering::SeqCst);
}

/// Отменена ли генерация.
pub fn is_cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

/// Новый идентификатор запроса для агентного слоя.
pub fn next_request_id() -> u64 {
    REQUEST_ID.fetch_add(1, Ordering::SeqCst)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect::<String>() + "…"
}

fn set_error(status: &State<'_, ModelState>, message: &str) {
    let mut guard = status.0.lock().expect("model lock poisoned");
    let mut s = guard.clone().unwrap_or(ModelStatus {
        running: false,
        recipe: None,
        kv_cache: None,
        context: None,
        tok_s: None,
        verified: false,
        full_gpu: false,
        gpu_layers: None,
        total_layers: None,
        device: None,
        flash_attn: false,
        error: None,
    });
    s.error = Some(message.to_string());
    *guard = Some(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn части_вызова_инструмента_склеиваются() {
        let first = serde_json::json!([
            {"index": 0, "function": {"name": "find_files", "arguments": "{\"folder\":"}}
        ]);
        let second = serde_json::json!([
            {"index": 0, "function": {"arguments": "\"Рабочий стол\"}"}}
        ]);

        let merged = merge_tool_calls(&first, &second);
        let call = &merged[0];
        assert_eq!(call["function"]["name"], "find_files");
        assert_eq!(call["function"]["arguments"], "{\"folder\":\"Рабочий стол\"}");
    }

    #[test]
    fn два_вызова_не_слипаются() {
        let first = serde_json::json!([
            {"index": 0, "function": {"name": "a", "arguments": "{}"}},
            {"index": 1, "function": {"name": "b", "arguments": "{}"}}
        ]);
        let second = serde_json::json!([
            {"index": 1, "function": {"arguments": ",\"x\":1}"}}
        ]);
        let merged = merge_tool_calls(&first, &second);
        assert_eq!(merged.as_array().unwrap().len(), 2);
        assert_eq!(merged[1]["function"]["name"], "b");
    }

    #[test]
    fn длинный_ответ_обрезается() {
        assert_eq!(truncate("привет", 100), "привет");
        assert_eq!(truncate("абвгд", 3), "абв…");
        // Обрезка по символам, а не по байтам: кириллица не ломается.
        assert_eq!(truncate("привет мир", 6), "привет…");
    }
}

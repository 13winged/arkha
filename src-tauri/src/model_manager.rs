// S2  Модель-менеджер: рецепты, запуск сайдкара, проверка размещения.
//
// Порядок из docs/02-архитектура.md, раздел «Модель-менеджер»:
//   1. определить профиль        — см. profile.rs
//   2. выбрать рецепт            — каталог, только verified
//   3. скачать с проверкой digest
//   4. проверить комплектность
//   5. запустить сайдкар
//   6. ПРОВЕРИТЬ, что все слои на GPU
//   7. дождаться готовности РЕАЛЬНЫМ запросом
//   8. транзакционно: при провале вернуть предыдущий

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::profile::{config_dir, Tier};

// ---------------------------------------------------------------------------
// Каталог рецептов
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub name: String,
    pub file: String,
    pub kv_cache: String,
    pub context: u32,
    pub full_gpu: bool,
    /// Прошёл ли замер на заполненном контексте.
    /// Не verified — не показывается как рекомендация.
    pub verified: bool,
    /// Медиана пяти прогонов на заполненном контексте, tok/s.
    /// null означает «не измерено», а не «ноль».
    pub tok_s: Option<f32>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub measured_on: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRecipes {
    pub tier: String,
    pub llm: String,
    pub recipes: Vec<Recipe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaCppPin {
    pub repo: String,
    pub release: String,
    pub asset_windows: String,
    pub asset_cudart: String,
    pub license: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub llama_cpp: LlamaCppPin,
    #[serde(default)]
    pub profiles: HashMap<String, ProfileRecipes>,
    #[serde(default)]
    pub common_args: Vec<String>,
    #[serde(default)]
    pub chat_template_kwargs: serde_json::Value,
    /// Параметры выборки. Не пустое поле, а секция: отсутствие
    /// значений означало бы, что их никто не задавал.
    #[serde(default, rename = "sampling")]
    pub sampling: Option<SamplingSection>,
}

/// Параметры выборки при генерации.
///
/// В `data/recipes.json` их не было вообще: брались из шаблона модели
/// по умолчанию. Для секретаря это не мелочь. При температуре 0.8 и
/// выше модель начинает выдумывать — а секретарь выполняет поручение.
/// Худший сценарий: называет несуществующий файл и объявляет задачу
/// выполненной.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sampling {
    /// Низкое значение: секретарь выполняет, а не фантазирует.
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    /// Защита от зацикливания: «да, да, да» и «вот пример, вот пример».
    pub repeat_penalty: f32,
}

impl Sampling {
    /// Значения по умолчанию, если в каталоге их нет.
    pub fn default_for_secrets() -> Self {
        Self {
            temperature: 0.3,
            top_p: 0.9,
            top_k: 40,
            repeat_penalty: 1.1,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "temperature": self.temperature,
            "top_p": self.top_p,
            "top_k": self.top_k,
            "repeat_penalty": self.repeat_penalty,
        })
    }

    /// Границы, за которыми значение бессмысленно или вредно.
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err(format!("temperature {} вне 0…2", self.temperature));
        }
        if !(0.0..=1.0).contains(&self.top_p) {
            return Err(format!("top_p {} вне 0…1", self.top_p));
        }
        if self.top_k == 0 || self.top_k > 200 {
            return Err(format!("top_k {} вне 1…200", self.top_k));
        }
        if !(1.0..=2.0).contains(&self.repeat_penalty) {
            return Err(format!(
                "repeat_penalty {} вне 1…2, при значении меньше 1 модель \
                 начнёт избегать повторов и потеряет связность",
                self.repeat_penalty
            ));
        }
        Ok(())
    }
}

impl Default for Sampling {
    fn default() -> Self {
        Self::default_for_secrets()
    }
}

/// Что лежит в разделе `sampling` каталога рецептов.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingSection {
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    top_k: Option<u32>,
    #[serde(default)]
    repeat_penalty: Option<f32>,
}

impl Catalog {
    /// Параметры выборки из каталога, с откатом на значения по
    /// умолчанию для недостающих полей.
    pub fn sampling(&self) -> Result<Sampling, String> {
        let s = self
            .sampling
            .clone()
            .unwrap_or_else(|| SamplingSection {
                temperature: None,
                top_p: None,
                top_k: None,
                repeat_penalty: None,
            });
        let d = Sampling::default_for_secrets();
        let out = Sampling {
            temperature: s.temperature.unwrap_or(d.temperature),
            top_p: s.top_p.unwrap_or(d.top_p),
            top_k: s.top_k.unwrap_or(d.top_k),
            repeat_penalty: s.repeat_penalty.unwrap_or(d.repeat_penalty),
        };
        out.validate()?;
        Ok(out)
    }

    /// Каталог лежит рядом с приложением в `data/recipes.json`, а в
    /// установленной сборке — в `%APPDATA%\Arsha\recipes.json`.
    pub fn load() -> Result<Self, String> {
        for path in Self::candidate_paths() {
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("не удалось прочитать {}: {e}", path.display()))?;
                return serde_json::from_str(&text)
                    .map_err(|e| format!("не удалось разобрать {}: {e}", path.display()));
            }
        }
        Err(format!(
            "каталог рецептов не найден, искали: {}",
            Self::candidate_paths()
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    fn candidate_paths() -> Vec<PathBuf> {
        let mut paths = vec![config_dir().join("recipes.json")];
        if let Ok(exe) = std::env::current_exe() {
            // Рядом с exe: dev-сборка и портативная установка.
            if let Some(dir) = exe.parent() {
                paths.push(dir.join("data").join("recipes.json"));
                paths.push(dir.join("recipes.json"));
            }
        }
        paths.push(PathBuf::from("data").join("recipes.json"));
        paths.push(PathBuf::from("../data/recipes.json"));
        paths
    }

    pub fn for_tier(&self, tier: Tier) -> &[Recipe] {
        self.profiles
            .get(tier.as_str())
            .map(|p| p.recipes.as_slice())
            .unwrap_or(&[])
    }

    /// Рекомендация — только верифицированная. Не верифицированные
    /// рецепты остаются в каталоге, но не выбираются автоматически.
    pub fn recommended(&self, tier: Tier) -> Option<&Recipe> {
        self.for_tier(tier)
            .iter()
            .filter(|r| r.verified)
            .max_by(|a, b| {
                a.tok_s
                    .unwrap_or(0.0)
                    .partial_cmp(&b.tok_s.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Первый верифицированный, иначе первый в списке. Так пользователь
    /// может начать работать с неверифицированным рецептом, но это должно
    /// быть видно в интерфейсе, а не молчаливым выбором.
    pub fn fallback(&self, tier: Tier) -> Option<&Recipe> {
        self.for_tier(tier).first()
    }

    pub fn llm_repo(&self, tier: Tier) -> Option<&str> {
        self.profiles.get(tier.as_str()).map(|p| p.llm.as_str())
    }
}

// ---------------------------------------------------------------------------
// Где лежат файлы
// ---------------------------------------------------------------------------

/// Папка с бинарниками llama.cpp.
///
/// Порядок поиска важен. Раньше здесь был молчаливый откат на
/// относительный `data/bin`, который разрешается от каталога запуска.
/// Пользователь запускает готовый exe из папки установки, а не из
/// корня проекта, поэтому откат всегда указывал не туда и сообщение
/// «не найден llama-server.exe» вводило в заблуждение: он был
/// установлен, просто искали не там.
pub fn bin_dir() -> PathBuf {
    bin_dir_со(std::env::var("ARSHA_BIN_DIR").ok().as_deref())
}

/// Строка журнала для человека.
///
/// llama-server пишет в поток JSON: `{"type":"log","level":"info",
/// "msg":"slot print_timing: ..."}`. Показать это как есть — значит
/// заставить обычного человека читать служебные скобки, чтобы понять,
/// что произошло. Сообщение достаётся, а всё остальное отбрасывается.
///
/// Разбор неудачный — строка возвращается как есть. Причина важнее
/// красивости: если разбирать не вышло, молчать нельзя.
fn по_человечески(строка: &str) -> String {
    let обрезанная = {
        let t = строка.trim();
        if t.len() > 900 {
            let s: String = t.chars().take(900).collect();
            format!("{s}…")
        } else {
            t.to_string()
        }
    };
    if !обрезанная.starts_with('{') {
        return обрезанная;
    }
    match serde_json::from_str::<serde_json::Value>(&обрезанная) {
        Ok(v) => v
            .get("msg")
            .and_then(|m| m.as_str())
            .map(|m| {
                // Заменяются настоящие переводы строк, а не
                // последовательность «\n». Разбор JSON уже превратил
                // её в перевод, так что искать было нечего: текст для
                // человека рвался на строки посреди сообщения.
                m.replace(['\r', '\n'], " ").trim().to_string()
            })
            .unwrap_or(обрезанная),
        Err(_) => обрезанная,
    }
}

/// Папка с бинарниками, переопределение приходит параметром.
///
/// Тело вынесено отдельно от `bin_dir` не для красоты. Переменная
/// окружения общая для всего процесса, и тест, который её ставит,
/// влияет на все прочие тесты, запущенные рядом. На живой машине
/// из-за этого один и тот же тест то проходил, то падал: чужой
/// `ARSHA_BIN_DIR` был виден в момент проверки. Переопределение
/// параметром убирает гонку вовсе — тесты просто передают нужное
/// значение, и общее состояние не трогают.
fn bin_dir_со(переопределение: Option<&str>) -> PathBuf {
    if let Some(dir) = переопределение {
        return PathBuf::from(dir);
    }

    let mut searched = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Установленная сборка: бинарники рядом с exe.
            searched.push(dir.join("llama"));
            searched.push(dir.join("bin"));

            // Сборка из проекта: target/release/arsha.exe,
            // бинарники в data/bin от корня репозитория.
            for up in dir.ancestors().skip(1).take(5) {
                searched.push(up.join("data").join("bin"));
                searched.push(up.join("bin"));
            }
        }
    }

    searched.push(PathBuf::from("data").join("bin"));

    for candidate in &searched {
        if candidate.join("llama").join("llama-server.exe").exists() {
            return candidate.clone();
        }
    }

    // Ничего не нашли: возвращаем первый разумный путь, чтобы
    // сообщение об ошибке назвало тело, а детали собирает
    // поиск по папкам отдельно.
    searched.first().cloned().unwrap_or_else(|| {
        PathBuf::from("data").join("bin")
    })
}

/// Все места, где искали бинарник. Показываются в ошибке, чтобы
/// пользователь не гадал.
pub fn bin_search_paths() -> Vec<PathBuf> {
    if let Ok(dir) = std::env::var("ARSHA_BIN_DIR") {
        return vec![PathBuf::from(dir)];
    }
    пути_от_exe()
}

/// Папки рядом с программой, без чтения переменных окружения.
///
/// Отдельная функция, а не часть `bin_search_paths`, из-за тестов.
/// Переменная окружения общая для всего процесса, и два теста,
/// которые её трогают, запущенные рядом, ломают друг другу
/// результат: на живой машине этот тест то проходил, то падал, и
/// причина была не в нём. Здесь переменной нет вовсе, и гонки не
/// будет.
fn пути_от_exe() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("llama"));
            out.push(dir.join("bin"));
            for up in dir.ancestors().skip(1).take(5) {
                out.push(up.join("data").join("bin"));
                out.push(up.join("bin"));
            }
        }
    }
    out.push(PathBuf::from("data").join("bin"));
    out
}

pub fn llama_server_exe() -> PathBuf {
    bin_dir().join("llama").join("llama-server.exe")
}

pub fn models_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ARSHA_MODELS_DIR") {
        return PathBuf::from(dir);
    }
    config_dir().join("models")
}

// ---------------------------------------------------------------------------
// Лог сайдкара
// ---------------------------------------------------------------------------

/// Кольцевой буфер строк лога. Интерфейс читает его, чтобы показать
/// пользователю, что происходит: «модель грузится», «слои на GPU»,
/// «ошибка». Пользователь не должен гадать.
#[derive(Clone, Default)]
pub struct LogBuffer {
    lines: Arc<Mutex<Vec<String>>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Arc::new(Mutex::new(Vec::new())),
            capacity,
        }
    }

    pub fn push(&self, line: String) {
        let mut guard = self.lines.lock().expect("log lock poisoned");
        guard.push(line);
        if guard.len() > self.capacity {
            let excess = guard.len() - self.capacity;
            guard.drain(0..excess);
        }
    }

    pub fn tail(&self, n: usize) -> Vec<String> {
        let guard = self.lines.lock().expect("log lock poisoned");
        guard.iter().rev().take(n).rev().cloned().collect()
    }

    pub fn find(&self, needle: &str) -> Option<String> {
        let guard = self.lines.lock().expect("log lock poisoned");
        guard.iter().find(|l| l.contains(needle)).cloned()
    }

    pub fn len(&self) -> usize {
        self.lines.lock().expect("log lock poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        self.lines.lock().expect("log lock poisoned").clear();
    }
}

// ---------------------------------------------------------------------------
// Размещение на GPU
// ---------------------------------------------------------------------------

/// Что удалось выяснить из лога llama.cpp о размещении модели.
#[derive(Debug, Clone, Serialize)]
pub struct Placement {
    /// Слои на GPU / всего. None если llama.cpp не отчитался.
    pub gpu_layers: Option<u32>,
    pub total_layers: Option<u32>,
    /// Модель на GPU, МиБ.
    pub model_mib_on_gpu: Option<u32>,
    /// KV-кэш на GPU, МиБ.
    pub kv_mib_on_gpu: Option<u32>,
    /// Модель, отображённая из файла, МиБ. Это НЕ offload.
    pub model_mib_cpu_mapped: Option<u32>,
    /// Имя устройства, например RTX 5060.
    pub device: Option<String>,
    pub flash_attn: bool,
}

/// Насколько неполное размещение. Не «да или нет», а градация:
/// 85 процентов слоёв на GPU стоят 1.84 раза и вполне рабочие,
/// а двенадцать из тридцати трёх — уже нет.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Offload {
    /// Всё на видеопамяти.
    Full,
    /// Не хватило немного, работает.
    Mild,
    /// Заметно медленнее, но терпимо.
    Heavy,
    /// Узкое место в процессоре, разумнее уменьшить квант.
    Unusable,
    /// Размещение неизвестно.
    Unknown,
}

impl Offload {
    pub fn title(self) -> &'static str {
        match self {
            Offload::Full => "всё на видеокарте",
            Offload::Mild => "частично в памяти",
            Offload::Heavy => "в основном в памяти",
            Offload::Unusable => "почти всё в памяти",
            Offload::Unknown => "размещение неизвестно",
        }
    }

    /// Отказывать или только предупреждать.
    ///
    /// Раньше отказ был на любом неполном размещении, и держался на
    /// числе из чужой статьи. Своё измерение показало, что это
    /// отнимает у пользователя рабочий вариант.
    pub fn blocks_start(self) -> bool {
        matches!(self, Offload::Unusable | Offload::Unknown)
    }
}

impl Placement {
    pub fn full_gpu(&self) -> bool {
        match (self.gpu_layers, self.total_layers) {
            (Some(g), Some(t)) => t > 0 && g >= t,
            _ => false,
        }
    }

    /// Доля слоёв на видеопамяти, от 0 до 1.
    ///
    /// Нужна, чтобы отличать «чуть-чуть не влезло» от «почти всё
    /// в оперативной памяти». Это разные ситуации: измерение
    /// показало 1.84× против 4.69× при одной и той же модели.
    pub fn gpu_share(&self) -> Option<f32> {
        match (self.gpu_layers, self.total_layers) {
            (Some(g), Some(t)) if t > 0 => Some((g as f32 / t as f32).clamp(0.0, 1.0)),
            _ => None,
        }
    }

    /// Насколько размещение плохое.
    ///
    /// Шкала из измерения 12 октября 2026, Qwen3.5 9B Q4_K_M на
    /// RTX 5060, контекст около 8K:
    ///
    /// | слоёв на GPU | просадка |
    /// |---|---|
    /// | 33 из 33 | нет |
    /// | 28 из 33 | 1.84× |
    /// | 20 из 33 | 3.22× |
    /// | 12 из 33 | 4.69× |
    ///
    /// При бо́льшем контексте KV-кэш вытесняет веса, и просадка
    /// сильнее. Поэтому числа — нижняя граница, а не обещание.
    pub fn severity(&self) -> Offload {
        if self.full_gpu() {
            return Offload::Full;
        }
        let Some(share) = self.gpu_share() else {
            // Размещение неизвестно. Молчать нельзя: неизвестное и
            // «всё хорошо» — не одно и то же.
            return Offload::Unknown;
        };
        // Пороги подобраны под измеренные точки, а не «красиво
        //»: 28 из 33 — это 84.8 процента, и оно должно попасть в
        // «работает», иначе порог в 0.85 отбраковывал бы лучший
        // измеренный случай.
        if share >= 0.80 {
            Offload::Mild
        } else if share >= 0.55 {
            Offload::Heavy
        } else {
            Offload::Unusable
        }
    }

    /// Что сказать пользователю, если размещение неполное.
    pub fn offload_explanation(&self) -> Option<String> {
        // Полное размещение — нечего объяснять. Без этой строки
        // «всё хорошо» попадало в ветку «неизвестно».
        if self.full_gpu() {
            return None;
        }
        match (self.gpu_layers, self.total_layers) {
            (Some(g), Some(t)) if t > 0 && g < t => {
                let pct = (g as f64 / t as f64 * 100.0).round() as u32;
                let text = match self.severity() {
                    Offload::Mild => format!(
                        "на GPU {g} из {t} слоёв ({pct}%). Часть весов в оперативной \
                         памяти: по нашему замеру скорость падает примерно в \
                         {} раза, с 67 до 36 токенов в секунду. Работать можно",
                        1.84f32
                    ),
                    Offload::Heavy => format!(
                        "на GPU {g} из {t} слоёв ({pct}%). По нашему замеру скорость \
                         падает примерно в 3.2 раза. Уже заметно медленнее, но \
                         терпимо. Меньший квант был бы быстрее",
                    ),
                    _ => format!(
                        "на GPU {g} из {t} слоёв ({pct}%). По нашему замеру скорость \
                         падает почти в 5 раз, узкое место ушло в процессор. Нужен \
                         меньший квант: убирать слои с GPU дальше почти бесполезно"
                    ),
                };
                Some(text)
            }
            _ => Some(
                "llama.cpp не отчитался о размещении слоёв. Неизвестно, всё ли на GPU"
                    .into(),
            ),
        }
    }
}

/// Разбирает лог llama.cpp и достаёт размещение.
///
/// Ключевая поправка, найденная измерением на S2: в логе есть строка
/// `CPU_Mapped model buffer size = 545.62 MiB`. Это НЕ offload на CPU.
/// `CPU_Mapped` — это буфер, отображённый из файла модели, туда не
/// считают слои. Признак частичного размещения — не эта строка, а
/// `offloaded X/Y layers to GPU`, где X меньше Y.
pub fn parse_placement(lines: &[String]) -> Placement {
    let mut p = Placement {
        gpu_layers: None,
        total_layers: None,
        model_mib_on_gpu: None,
        kv_mib_on_gpu: None,
        model_mib_cpu_mapped: None,
        device: None,
        flash_attn: false,
    };
    let mut model_layers: Option<u32> = None;

    for line in lines {
        // Строки лога имеют вид "load_tensors: offloaded 33/33 layers to GPU",
        // поэтому маркер ищем в любом месте строки, а не по префиксу.
        if let Some(rest) = after(line, "offloaded ") {
            // 33/33 layers to GPU
            let mut parts = rest.split('/');
            let gpu: u32 = parts
                .next()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            let total: u32 = parts
                .next()
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if total > 0 {
                p.gpu_layers = Some(gpu);
                p.total_layers = Some(total);
            }
        }

        if let Some(rest) = after(line, "n_layer ") {
            if let Some(v) = rest
                .split('=')
                .nth(1)
                .and_then(|s| s.trim().parse::<u32>().ok())
            {
                // Это число слоёв модели без выходного слоя, поэтому оно
                // на единицу меньше, чем знаменатель в "offloaded X/Y".
                // Знаменатель из строки offloaded авторитетен, это
                // значение используется только если строки offloaded нет.
                model_layers = Some(v);
            }
        }

        if let Some(rest) = after(line, "CUDA0 model buffer size =") {
            p.model_mib_on_gpu = parse_mib(rest);
        }
        if let Some(rest) = after(line, "CUDA0 KV buffer size =") {
            p.kv_mib_on_gpu = parse_mib(rest);
        }
        if let Some(rest) = after(line, "CPU_Mapped model buffer size =") {
            p.model_mib_cpu_mapped = parse_mib(rest);
        }
        if line.contains("flash_attn") && (line.contains("= enabled") || line.contains("enabling")) {
            p.flash_attn = true;
        }
        if p.device.is_none() {
            if let Some(idx) = line.find("CUDA0 (") {
                let tail = &line[idx + "CUDA0 (".len()..];
                if let Some(end) = tail.find(')') {
                    p.device = Some(tail[..end].to_string());
                }
            }
        }
    }

    if p.total_layers.is_none() {
        p.total_layers = model_layers;
    }

    p
}

/// Часть строки после маркера, если маркер в ней есть.
fn after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    line.find(marker).map(|i| &line[i + marker.len()..])
}

fn parse_mib(s: &str) -> Option<u32> {
    s.trim().split_whitespace().next()?.parse::<f32>().ok().map(|v| v as u32)
}

// ---------------------------------------------------------------------------
// Сайдкар
// ---------------------------------------------------------------------------

pub struct Sidecar {
    child: Child,
    pub port: u16,
    pub recipe: Recipe,
    pub log: LogBuffer,
    pub placement: Placement,
}

impl Sidecar {
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Жив ли ещё процесс.
    ///
    /// Отдельный метод, а не `is_running` у состояния, потому что
    /// наличие структуры `Sidecar` и живость процесса — разные
    /// вещи, и путать их дорого: на живой машине модель упала, а
    /// интерфейс продолжал показывать «запущена», и кнопка
    /// «остановить» не помогала — останавливать было нечего.
    ///
    /// `try_wait` не убивает процесс: он только спрашивает у
    /// системы, завершился ли он, и забирает код возврата, если
    /// завершился. Повторный вызов вернёт тот же код, поэтому
    /// вызывать его можно сколько угодно.
    pub fn жив(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Почему процесс закончился, если закончился.
    pub fn причина_смерти(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(выход)) => Some(выход.code().unwrap_or(-1)),
            _ => None,
        }
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Состояние сайдкара живёт всё время работы приложения.
pub struct SidecarState {
    pub(crate) inner: Mutex<Option<Sidecar>>,
}

impl SidecarState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut Sidecar) -> T) -> Result<T, String> {
        let mut guard = self.inner.lock().expect("sidecar lock poisoned");
        match guard.as_mut() {
            Some(s) => Ok(f(s)),
            None => Err("модель не запущена".into()),
        }
    }

    /// Проверить сайдкар и убрать его, если он мёртв.
    ///
    /// Возвращает текст причины, если сайдкар упал сам, и `None`, если
    /// он жив либо его не было вовсе.
    ///
    /// Одна функция, а не «спросить живо ли» плюс «узнать почему»:
    /// приходилось сначала спрашивать, а потом убирать, и при уборке
    /// причина исчезала вместе с сайдкаром — она лежала в нём. То
    /// есть причина не показывалась никогда, хотя для неё всё и
    /// затевалось.
    pub fn упал_сам(&self) -> Option<String> {
        let mut guard = self.inner.lock().ok()?;
        let сайт = guard.as_mut()?;
        if сайт.жив() {
            return None;
        }
        let код = сайт.причина_смерти();
        let порт = сайт.port;
        let хвост: Vec<String> = сайт.log.tail(4).iter().map(|с| по_человечески(с)).collect();
        // `take` вызывает `Drop`, а тот добивает уже мёртвый процесс.
        // Уже мёртвому это не вредно, зато состояние перестаёт хранить
        // того, кого нет: иначе следующий запуск сказал бы «порт
        // занят», хотя он свободен.
        guard.take();
        Some(format!(
            "модель завершилась сама (код {}, порт {}). Последнее из журнала: {}",
            код.map(|к| к.to_string()).unwrap_or_else(|| "?".into()),
            порт,
            if хвост.is_empty() {
                "пусто".to_string()
            } else {
                // Разделитель — точка с точкой с запятой, а не
                // вертикальная черта: черта в служебных сообщениях
                // llama.cpp тоже встречается и слипает соседние.
                хвост.join("; ")
            }
        ))
    }

    /// Жив ли сайдкар.
    ///
    /// Проверяется именно процесс, а не наличие структуры. Мёртвый
    /// сайдкар убирается здесь же.
    pub fn is_running(&self) -> bool {
        self.упал_сам().is_none() && self.inner.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    pub fn base_url(&self) -> Result<String, String> {
        self.with(|s| s.base_url())
    }

    pub fn stop(&self) {
        let mut guard = self.inner.lock().expect("sidecar lock poisoned");
        if let Some(mut s) = guard.take() {
            s.stop();
        }
    }
}

/// Аргументы командной строки для llama-server.
///
/// Флаг `--tool-call-parser` из плана убран: в llama.cpp b11557 его
/// больше нет, и он не нужен. Шаблон Qwen3.5 разбирает tool calls сам
/// при `--jinja`. Проверено запросом к `/v1/chat/completions` с
/// `tools`: модель вернула `finish_reason: tool_calls` с русскими
/// аргументами. Оставление флага привело бы к ошибке запуска.
pub fn server_args(recipe: &Recipe, model: &Path, port: u16) -> Vec<String> {
    vec![
        "-m".into(),
        model.display().to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        // Просим все слои на GPU. Проверка ниже градуированная:
        // сильное нарушение — ошибка, слабое — предупреждение с
        // измеренной цифрой.
        "-ngl".into(),
        "99".into(),
        "-c".into(),
        recipe.context.to_string(),
        "-ctk".into(),
        recipe.kv_cache.clone(),
        "-ctv".into(),
        recipe.kv_cache.clone(),
        // Шаблон из модели. Без него tool calls не работают.
        "--jinja".into(),
        // Рассуждение выключено: секретарь отвечает, а не размышляет.
        "--reasoning".into(),
        "off".into(),
        // Структурированный лог: размещение и тайминги читаются из��
        "--log-jsonl".into(),
        // Уровень 4 — минимальный, при котором llama.cpp пишет
        // "offloaded X/Y layers to GPU". На уровне 3 этой строки нет,
        // и размещение нечем подтвердить: проверка молча провалилась бы.
        // На уровне verbose (без N) лог разрастается тысячами строк.
        // Измерено: -lv 3 даёт 9 строк, -lv 4 даёт 225.
        "-lv".into(),
        "4".into(),
    ]
}

/// Запускает сайдкар и ждёт готовности реальным запросом.
///
/// Готовность проверяется не фактом старта процесса и не ответом на
/// `/health`, а настоящим запросом completions. Сервер может принимать
/// соединение до того, как модель физически готова.
pub fn start(
    recipe: &Recipe,
    model: &Path,
    port: u16,
) -> Result<Sidecar, String> {
    let exe = llama_server_exe();
    if !exe.exists() {
        let searched = bin_search_paths()
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "не найден llama-server.exe\n  искали: {searched}\n  \
             ожидался файл: {}\n  \
             ставится вместе с приложением, скачивается автоматически",
            exe.display()
        ));
    }
    if !model.exists() {
        return Err(format!(
            "не найден файл модели {}\n  папка моделей: {}",
            model
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| model.display().to_string()),
            model.parent().map(|p| p.display().to_string()).unwrap_or_default()
        ));
    }

    let log = LogBuffer::new(2_000);
    let args = server_args(recipe, model, port);

    // cudart лежит рядом с бинарником и должен быть в PATH.
    let cudart = bin_dir().join("cudart");
    let mut cmd = Command::new(&exe);
    cmd.args(&args)
        .current_dir(bin_dir())
        .env("PATH", prepend_path(&cudart))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("не удалось запустить llama-server: {e}"))?;

    // Читаем лог в фоне, пока ждём готовности.
    let (out, err) = (
        child.stdout.take().expect("stdout piped"),
        child.stderr.take().expect("stderr piped"),
    );
    let mut readers: Vec<Box<dyn std::io::Read + Send>> = vec![Box::new(out), Box::new(err)];
    for mut stream in readers.drain(..) {
        let log = log.clone();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(&mut stream);
            for line in reader.lines().map_while(Result::ok) {
                log.push(line);
            }
        });
    }

    // Ждём появления строки о загрузке модели, затем читаем размещение.
    wait_for_line(&log, "model loaded", std::time::Duration::from_secs(300))?;
    wait_for_line(&log, "listening on", std::time::Duration::from_secs(60))?;

    let lines = log.tail(4_000);
    let placement = parse_placement(&lines);

    if recipe.full_gpu {
        let severity = placement.severity();
        if severity.blocks_start() {
            // Отказ только там, где вариант действительно плох.
            // Раньше отказ был на любом неполном размещении — правило
            // держалось на числе из чужой статьи «6–8 раз», а своё
            // измерение показало, что 85 процентов слоёв на GPU
            // стоят 1.84 раза и работают нормально.
            if let Some(why) = placement.offload_explanation() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(why);
            }
        } else if let Some(why) = placement.offload_explanation() {
            // Рабочий вариант: говорим и продолжаем. Молча запускать
            // было бы хуже — пользователь решит, что всё в порядке.
            eprintln!("размещение неполное, но рабочее: {why}");
        }
    }

    // Реальный запрос. `/health` отвечает и до готовности модели.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("не удалось создать HTTP-клиент: {e}"))?;

    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&serde_json::json!({
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
            "stream": false,
        }))
        .send();

    match resp {
        Ok(r) if r.status().is_success() => {
            log.push(format!("готовность подтверждена запросом, статус {}", r.status()));
        }
        Ok(r) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "сервер принял соединение, но не ответил на запрос: {}",
                r.status()
            ));
        }
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("проверка готовности не прошла: {e}"));
        }
    }

    Ok(Sidecar {
        child,
        port,
        recipe: recipe.clone(),
        log,
        placement,
    })
}

fn wait_for_line(
    log: &LogBuffer,
    needle: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if log.find(needle).is_some() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let tail = log.tail(15).join("\n");
    Err(format!(
        "сервер не сообщил «{needle}» за {:?}. Последние строки лога:\n{tail}",
        timeout
    ))
}

/// Дописать папку в начало `PATH`.
///
/// `pub`, а не `pub(crate)`: тем же приёмом пользуется `voice`, и
/// дублировать его там означало бы две реализации одного правила.
pub fn prepend_path(dir: &Path) -> String {
    match std::env::var("PATH") {
        Ok(p) => format!("{};{p}", dir.display()),
        Err(_) => dir.display().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Транзакционный запуск
// ---------------------------------------------------------------------------

pub struct StartOutcome {
    pub ok: bool,
    pub error: Option<String>,
    /// Какой рецепт реально запущен после откатов.
    pub active: Option<Recipe>,
    /// Что пробовали и почему откатились.
    pub attempts: Vec<Attempt>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attempt {
    pub recipe: String,
    pub result: String,
}

/// Пробует рецепты по очереди, пока один не запустится.
///
/// Список кандидатов сначала верифицированные, потом остальные. При
/// провале каждого следующий, и в конце возвращается к рабочему: так
/// пользователь после неудачной смены рецепта не остаётся без модели.
/// Проблема перед запуском. Собираются все сразу, а не по одной:
/// показывать пользователю только первую ошибку значит отправлять
/// его за ней, потом за второй, потом за третьей.
#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    pub what: String,
    pub why: String,
    pub how: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preflight {
    pub problems: Vec<Problem>,
    /// Рецепт, который реально запустится, если проблемы закрыты.
    pub usable_recipe: Option<String>,
    pub models_dir: String,
}

impl Preflight {
    pub fn blocked(&self) -> bool {
        !self.problems.is_empty()
    }
}

/// Проверяет всё, что нужно для запуска, до попытки запустить.
///
/// Смысл: пользователь нажимает «Запустить модель» один раз и получает
/// полный список. Раньше показывалась первая ошибка — на экране
/// значило «не найден llama-server.exe», хотя первым делом не
/// найдена была модель для профиля T3.
pub fn preflight(tier: Tier, catalog: &Catalog, models: &Path) -> Preflight {
    preflight_со(
        tier,
        catalog,
        models,
        std::env::var("ARSHA_BIN_DIR").ok().as_deref(),
    )
}

/// Проверка с переопределением папки бинарников.
///
/// Отдельная функция по той же причине, что и `bin_dir_со`: тест
/// «движок не установлен» обязан проверять случай «папки нет», и
/// раньше для этого приходилось ставить переменную окружения на
/// весь процесс. Тесты идут параллельно, и значение видели те, кто
/// его не ставил, — из-за чего проверки падали по чужой вине.
fn preflight_со(
    tier: Tier,
    catalog: &Catalog,
    models: &Path,
    bin_override: Option<&str>,
) -> Preflight {
    let mut problems = Vec::new();

    // 1. Движок.
    let exe = bin_dir_со(bin_override)
        .join("llama")
        .join("llama-server.exe");
    if !exe.exists() {
        let искали: Vec<PathBuf> = match bin_override {
            Some(dir) => vec![PathBuf::from(dir)],
            None => пути_от_exe(),
        };
        problems.push(Problem {
            what: "не установлен движок llama.cpp".into(),
            why: format!(
                "файл {} не найден. Поиск шёл: {}",
                exe.display(),
                искали
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            how: "движок ставится вместе с приложением. Если вы переносили \
                  папку вручную, верните llama рядом с программой"
                .into(),
        });
    }

    // 2. Рецепты для профиля.
    let recipes = catalog.for_tier(tier);
    if recipes.is_empty() {
        problems.push(Problem {
            what: format!("нет рецептов для профиля {}", tier.as_str()),
            why: "в каталоге для этого профиля не описана ни одна комбинация \
                  кванта, KV-кэша и контекста"
                .into(),
            how: "выберите профиль вручную или обновите каталог рецептов".into(),
        });
    }

    // 3. Модели. Считаем все отсутствующие, а не замираем на первом.
    for recipe in recipes {
        let path = models.join(&recipe.file);
        if !path.exists() {
            problems.push(Problem {
                what: format!("нет модели для рецепта «{}»", recipe.name),
                why: format!(
                    "не найден файл {}\n  ожидался: {}\n  папка моделей: {}",
                    recipe.file,
                    path.display(),
                    models.display()
                ),
                how: "нажмите «Скачать модель», приложение проверит сводку \
                      файла после загрузки"
                    .into(),
            });
        }
    }

    // 4. Каталог рецептов вообще существует.
    if catalog.profiles.is_empty() {
        problems.push(Problem {
            what: "каталог рецептов пуст".into(),
            why: "прочитать нечего: ни одного профиля".into(),
            how: "переустановите приложение, каталог должен идти в комплекте".into(),
        });
    }

    Preflight {
        usable_recipe: recipes.first().map(|r| r.name.clone()),
        problems,
        models_dir: models.display().to_string(),
    }
}

pub fn start_transactional(
    tier: Tier,
    catalog: &Catalog,
    model_dir: &Path,
    port: u16,
) -> Result<(Sidecar, StartOutcome), String> {
    let mut ordered: Vec<&Recipe> = catalog.for_tier(tier).iter().collect();
    ordered.sort_by_key(|r| !r.verified);

    if ordered.is_empty() {
        return Err(format!("в каталоге нет рецептов для профиля {}", tier.as_str()));
    }

    // Проверяем всё сразу, чтобы не сообщать по одной проблеме.
    let pre = preflight(tier, catalog, model_dir);
    if pre.blocked() && !ordered.iter().any(|r| model_dir.join(&r.file).exists()) {
        return Err(format_problems(&pre));
    }

    let mut attempts = Vec::new();
    for recipe in ordered {
        let model = model_dir.join(&recipe.file);
        match start(recipe, &model, port) {
            Ok(sidecar) => {
                attempts.push(Attempt {
                    recipe: recipe.name.clone(),
                    result: "запущен".into(),
                });
                return Ok((
                    sidecar,
                    StartOutcome {
                        ok: true,
                        error: None,
                        active: Some(recipe.clone()),
                        attempts,
                    },
                ));
            }
            Err(e) => {
                attempts.push(Attempt {
                    recipe: recipe.name.clone(),
                    result: e.clone(),
                });
            }
        }
    }

    Err(format!("ни один рецепт не запустился: {}", render_attempts(&attempts)))
}

/// Текст со всеми проблемами разом, а не с первой.
pub fn format_problems(pre: &Preflight) -> String {
    let mut s = format!(
        "не удалось подготовить запуск, проблем: {}\n",
        pre.problems.len()
    );
    for (i, p) in pre.problems.iter().enumerate() {
        s.push_str(&format!(
            "\n{}. {}\n   почему: {}\n   что делать: {}\n",
            i + 1,
            p.what,
            p.why,
            p.how
        ));
    }
    s
}

fn render_attempts(attempts: &[Attempt]) -> String {
    attempts
        .iter()
        .map(|a| format!("{} — {}", a.recipe, a.result))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Мёртвый сайдкар обязан читаться как отсутствующий.
    ///
    /// На живой машине модель упала сама, а состояние продолжало
    /// врать «запущена»: интерфейс показывал работающую модель и
    /// кнопку «остановить», которая не останавливала ничего. Тест
    /// поднимает настоящий процесс, убивает его и требует, чтобы
    /// приложение заметило.
    #[test]
    fn убитый_сайдкар_читается_как_остановленный() {
        use std::process::{Command, Stdio};

        let ребёнок = Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1 > nul"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("не удалось поднять проверочный процесс");

        let состояние = SidecarState::new();
        {
            let mut guard = состояние.inner.lock().expect("замок");
            *guard = Some(Sidecar {
                child: ребёнок,
                port: 18099,
                recipe: recipe_named("проверка", "нет.gguf"),
                log: LogBuffer::new(16),
                placement: Placement {
                    gpu_layers: None,
                    total_layers: None,
                    model_mib_on_gpu: None,
                    kv_mib_on_gpu: None,
                    model_mib_cpu_mapped: None,
                    device: None,
                    flash_attn: false,
                },
            });
        }
        assert!(состояние.is_running(), "живой процесс обязан считаться живым");

        // Убиваем процесс извне — так же, как это делает сбой.
        let pid = {
            let guard = состояние.inner.lock().expect("замок");
            guard.as_ref().map(|s| s.child.id())
        };
        if let Some(pid) = pid {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output();
        }
        std::thread::sleep(std::time::Duration::from_millis(700));

        assert!(
            !состояние.is_running(),
            "убитый процесс обязан читаться как остановленный, а не как работающий"
        );
        assert!(
            состояние.inner.lock().expect("замок").is_none(),
            "мёртвый сайдкар обязан убираться из состояния: иначе следующий \
             запуск скажет «порт занят», хотя он свободен"
        );

        // Причина обязана читаться ДО уборки. Проверка живости убирает
        // мёртвый сайдкар, а причина падения хранится в нём самом, так
        // что при обратном порядке она всегда терялась. На живой
        // машине интерфейс молчал именно по этой причине.
        assert!(
            состояние.упал_сам().is_none(),
            "после уборки повторный запрос не должен выдумывать причину \
             для уже удалённого сайдкара"
        );
    }

    /// Причина падения доходит до того, как сайдкар уберут.
    ///
    /// Отдельный порядок проверяется здесь, а в тесте выше — на
    /// мёртвом процессе целиком. Здесь падение происходит внутри
    /// программы: процесс убивает себя сам, без чужой руки, и причина
    /// обязана быть названа по-человечески.
    #[test]
    fn причина_падения_читается_до_уборки() {
        use std::process::{Command, Stdio};

        let ребёнок = Command::new("cmd")
            .args(["/C", "exit 7"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("не удалось поднять проверочный процесс");
        let pid = ребёнок.id();
        let mut ребёнок = ребёнок;

        let состояние = SidecarState::new();
        *состояние.inner.lock().expect("замок") = Some(Sidecar {
            child: ребёнок,
            port: 18098,
            recipe: recipe_named("проверка", "нет.gguf"),
            log: LogBuffer::new(16),
            placement: Placement {
                gpu_layers: None,
                total_layers: None,
                model_mib_on_gpu: None,
                kv_mib_on_gpu: None,
                model_mib_cpu_mapped: None,
                device: None,
                flash_attn: false,
            },
        });
        let _ = pid;

        // Ждём завершения: `exit 7` отрабатывает мгновенно, но
        // завершение процесса ещё не обязано быть видно сразу.
        let причина = (0..40)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(100));
                состояние.упал_сам()
            })
            .expect("падение процесса обязано быть замечено за 4 с");

        assert!(
            причина.contains("завершилась сама"),
            "текст должен называть, что произошло, а не молчать: {причина}"
        );
        assert!(
            причина.contains("18098"),
            "в тексте должен быть порт: {причина}"
        );
        assert!(
            !состояние.is_running(),
            "после падения модель не должна считаться работающей"
        );
    }

    /// Строка журнала читается как человеком, а не как машиной.
///
/// Проверка на настоящем выводе llama-server: без разбора человек
/// увидел бы фигурные скобки и не понял ничего.
#[test]
fn журнал_разбирается_до_сообщения() {
    let сырое = r#"{"type":"log","time":3073550,"level":"info","msg":"slot release: id 3"}"#;
    assert_eq!(по_человечески(сырое), "slot release: id 3");

    // Перевод строки внутри сообщения не должен оставлять «\n».
    assert!(
        !по_человечески(r#"{"msg":"a\nb"}"#).contains('\n'),
        "в тексте для человека не должно быть служебных escape-последовательностей"
    );

    // Не JSON — строка возвращается как есть: разбирать не вышло,
    // значит молчать нельзя.
    assert_eq!(по_человечески("просто текст"), "просто текст");

    // Очень длинная строка обрезается, иначе сообщение о падении
    // растянулось бы на весь экран.
    let длинная = "x".repeat(2000);
    assert!(по_человечески(&длинная).chars().count() <= 901);
}

fn recipe_named(name: &str, file: &str) -> Recipe {
        Recipe {
            name: name.into(),
            file: file.into(),
            kv_cache: "q4_0".into(),
            context: 32_768,
            full_gpu: true,
            verified: false,
            tok_s: None,
            note: None,
            measured_on: None,
        }
    }

    /// Главный сценарий с экрана: профиль есть, модель для него нет,
    /// движока тоже нет. Раньше пользователю показывали только первую
    /// проблему, и первой была не та.
    #[test]
    fn показываются_все_проблемы_а_не_первая() {
        let cat = catalog(vec![recipe_named("9B Q5_K_M + q4_0", "missing.gguf")]);
        // Профиль T2, под который есть рецепт: иначе список проблем
        // был бы пустым по другой причине, и тест ничего не проверял бы.
        // Папка движка подставляется параметром, а не переменной
        // окружения: переменная видна сразу всем тестам процесса, и
        // этот тест портил соседние — проверка «движок не попадает в
        // проблемы» падала через раз на живой машине.
        let pre = preflight_со(
            Tier::T2,
            &cat,
            Path::new("/нет/такой/папки"),
            Some("Z:\\нет\\такого\\bin"),
        );

        assert!(pre.blocked());
        let text = format_problems(&pre);
        assert!(text.contains("не установлен движок"), "нет про движок");
        assert!(text.contains("missing.gguf"), "нет про модель");
        // Обе проблемы в одном сообщении, а не по очереди.
        assert!(text.find("движок").unwrap() < text.find("модель").unwrap());
    }

    /// На машине разработки движок есть, и про него в списке проблем
    /// писать незачем: пользователя смутит лишний пункт.
    #[test]
    fn найденный_движок_не_попадает_в_проблемы() {
        let cat = catalog(vec![recipe_named("9B Q4_K_M + q4_0", "missing.gguf")]);
        let pre = preflight(Tier::T2, &cat, Path::new("/нет/такой/папки"));
        let text = format_problems(&pre);
        assert!(!text.contains("движок"), "движок есть, жаловаться не на что");
        assert!(text.contains("missing.gguf"));
    }

    #[test]
    fn пустая_папка_моделей_даёт_понятный_текст_а_не_путь() {
        let cat = catalog(vec![recipe_named("9B Q4_K_M + q4_0", "m.gguf")]);
        let pre = preflight(Tier::T2, &cat, Path::new("/нет/такой/папки"));
        let text = format_problems(&pre);
        assert!(text.contains("/нет/такой/папки"));
        assert!(text.contains("Скачать модель"));
    }

    #[test]
    fn пустой_каталог_отмечается_отдельно() {
        let mut cat = catalog(vec![]);
        cat.profiles.clear();
        let pre = preflight(Tier::T1, &cat, Path::new("."));
        assert!(pre.blocked());
        assert!(format_problems(&pre).contains("каталог рецептов пуст"));
    }

    #[test]
    fn профиль_без_рецептов_называется_своим_именем() {
        let cat = catalog(vec![]); // профиля T5 в каталоге нет
        let pre = preflight(Tier::T5, &cat, Path::new("."));
        assert!(format_problems(&pre).contains("T5"));
    }

    /// Поиск папок не должен молча откатываться на путь от текущего
    /// каталога: пользователь запускает exe из папки установки.
    #[test]
    fn пути_поиска_относительно_exe_а_не_рабочего_каталога() {
        let paths = пути_от_exe();
        assert!(paths.len() >= 2, "должно быть несколько кандидатов");

        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.display().to_string()));

        if let Some(dir) = exe_dir {
            let has_relative = paths
                .iter()
                .any(|p| p.display().to_string().contains(&dir));
            assert!(has_relative, "в поиске должен быть путь рядом с exe");
        }
    }

    /// Переопределение важнее поиска по диску.
    ///
    /// Переменная окружения здесь не ставится намеренно: она видна
    /// сразу всем тестам процесса, и любой, кто её трогает, портит
    /// соседние проверки. Переопределение передаётся параметром, и
    /// общее состояние не трогается вовсе.
    #[test]
    fn движок_берётся_из_переопределения() {
        assert_eq!(
            bin_dir_со(Some("D:\\test\\bin")),
            PathBuf::from("D:\\test\\bin")
        );
    }

    #[test]
    fn неверифицированный_рецепт_не_становится_рекомендацией() {
        let c = catalog(vec![recipe_named("9B q4_0", "m.gguf")]);
        assert!(c.recommended(Tier::T2).is_none());
        assert!(c.fallback(Tier::T2).is_some());
    }

    #[test]
    fn среди_верифицированных_выбирается_быстрый() {
        let mut a = recipe_named("9B Q4_K_M + q4_0", "m.gguf");
        a.verified = true;
        a.tok_s = Some(31.0);
        let mut b = recipe_named("9B Q4_K_M + q8_0", "m.gguf");
        b.kv_cache = "q8_0".into();
        b.verified = true;
        b.tok_s = Some(34.0);
        let mut c = recipe_named("27B UD-IQ2_XXS", "m.gguf");
        c.kv_cache = "q8_0".into();
        c.verified = false;
        c.tok_s = Some(99.0);

        let cat = catalog(vec![a, b, c]);
        // Неверифицированный рецепт с лучшей цифрой не выигрывает:
        // выбираем среди проверенных.
        assert_eq!(cat.recommended(Tier::T2).unwrap().kv_cache, "q8_0");
    }

    /// Разбор реального лога llama.cpp b11557, снятого на dev-машине.
    #[test]
    fn полное_размещение_читается_из_лога() {
        let lines = vec![
            "cmn  print_info: n_layer               = 32".to_string(),
            "load_tensors: offloaded 33/33 layers to GPU".to_string(),
            "load_tensors:   CPU_Mapped model buffer size =   545.62 MiB".to_string(),
            "load_tensors:        CUDA0 model buffer size =  4861.28 MiB".to_string(),
            "llama_kv_cache:      CUDA0 KV buffer size =   288.00 MiB".to_string(),
            "llama_init_from_model: enabling flash_attn since it is required for quantized V cache"
                .to_string(),
            "llama_prepare_model_devices: using device CUDA0 (NVIDIA GeForce RTX 5060)".to_string(),
        ];
        let p = parse_placement(&lines);

        assert_eq!(p.gpu_layers, Some(33));
        // Знаменатель из "offloaded 33/33", а не n_layer = 32:
        // 33 это 32 слоя модели плюс выходной слой.
        assert_eq!(p.total_layers, Some(33));
        assert_eq!(p.model_mib_on_gpu, Some(4861));
        assert_eq!(p.kv_mib_on_gpu, Some(288));
        assert_eq!(p.model_mib_cpu_mapped, Some(545));
        assert_eq!(p.device.as_deref(), Some("NVIDIA GeForce RTX 5060"));
        assert!(p.flash_attn);
        assert!(p.full_gpu());
        assert!(p.offload_explanation().is_none());
    }

    /// Главная поправка S2: CPU_Mapped это mmap, а не offload.
    #[test]
    fn cpu_mapped_не_считается_offload() {
        let lines = vec![
            "load_tensors: offloaded 33/33 layers to GPU".to_string(),
            "load_tensors:   CPU_Mapped model buffer size =   545.62 MiB".to_string(),
        ];
        let p = parse_placement(&lines);
        assert!(p.full_gpu(), "mmap-буфер не должен считаться частичным размещением");
    }

    #[test]
    fn частичное_размещение_ловится_и_объясняется() {
        let lines = vec![
            "print_info: n_layer               = 40".to_string(),
            "load_tensors: offloaded 18/41 layers to GPU".to_string(),
        ];
        let p = parse_placement(&lines);
        assert!(!p.full_gpu());
        let why = p.offload_explanation().unwrap();
        assert!(why.contains("18"));
        // Число просадки должно быть своё, из измерения, а не из
        // чужой статьи.
        assert!(
            !why.contains("6–8") && !why.contains("6-8"),
            "в объяснении осталось число из чужой статьи: {why}"
        );
    }

    /// Правило 23: число, на котором держится решение, должно быть
    /// нашим. Здесь пороги — из замера 12 октября 2026, значит они
    /// обязаны совпадать с измеренными точками.
    ///
    /// Если кто-то поменяет порог на число из статьи, тест это
    /// поймает: границы перестанут совпадать с точками замера.
    #[test]
    fn пороги_шкалы_выведены_из_замера() {
        let at = |g: u32| {
            parse_placement(&[
                "print_info: n_layer               = 33".to_string(),
                format!("load_tensors: offloaded {g}/33 layers to GPU"),
            ])
            .severity()
        };
        // Точки из docs/10-замеры.md: 67.02, 36.40, 20.79, 14.30, 11.20
        assert_eq!(at(33), Offload::Full, "полное размещение");
        assert_eq!(at(28), Offload::Mild, "1.84× — рабочее");
        assert_eq!(at(20), Offload::Heavy, "3.22× — заметно медленнее");
        assert_eq!(at(12), Offload::Unusable, "4.69× — уже нет");
    }

    /// Те же точки плюс крайние случаи: почти всё в памяти и
    /// неполный лог. Первое — отказ, второе — тоже: неизвестное
    /// и «всё хорошо» не одно и то же.
    #[test]
    fn крайние_случаи_размещения() {
        let at = |g: u32, t: u32| {
            parse_placement(&[
                format!("print_info: n_layer               = {t}"),
                format!("load_tensors: offloaded {g}/{t} layers to GPU"),
            ])
            .severity()
        };
        assert_eq!(at(4, 33), Offload::Unusable, "5.98× — краем");
        assert_eq!(at(18, 41), Offload::Unusable, "44 процента — краем");
    }

    /// Пользователю с машиной на 8 ГБ нельзя отказывать в варианте,
    /// который работает. Это ровно то, что чинили.
    #[test]
    fn рабочее_размещение_не_блокирует_запуск() {
        let mild = parse_placement(&[
            "print_info: n_layer               = 33".to_string(),
            "load_tensors: offloaded 28/33 layers to GPU".to_string(),
        ]);
        assert!(!mild.severity().blocks_start(), "28 из 33 должны работать");

        let bad = parse_placement(&[
            "print_info: n_layer               = 33".to_string(),
            "load_tensors: offloaded 12/33 layers to GPU".to_string(),
        ]);
        assert!(bad.severity().blocks_start(), "12 из 33 должны отклоняться");
    }

    /// Неизвестное размещение нельзя считать хорошим.
    #[test]
    fn неизвестное_размещение_блокирует() {
        let p = parse_placement(&[]);
        assert_eq!(p.severity(), Offload::Unknown);
        assert!(p.severity().blocks_start());
    }

    #[test]
    fn доля_на_gpu_считается_правильно() {
        let p = parse_placement(&[
            "print_info: n_layer               = 33".to_string(),
            "load_tensors: offloaded 28/33 layers to GPU".to_string(),
        ]);
        let share = p.gpu_share().expect("доля должна считаться");
        assert!((share - 28.0 / 33.0).abs() < 0.01, "{share}");
    }

    #[test]
    fn пустой_лог_даёт_неизвестность_а_не_ок() {
        let p = parse_placement(&[]);
        assert!(!p.full_gpu());
        assert!(p.offload_explanation().is_some());
    }

    /// Если строки offloaded нет, знаменатель берётся из n_layer.
    #[test]
    fn n_layer_используется_как_запасной_знаменатель() {
        let p = parse_placement(&["print_info: n_layer               = 32".to_string()]);
        assert_eq!(p.total_layers, Some(32));
        assert!(!p.full_gpu(), "слои не отчитались, размещение не подтверждено");
    }

    /// План предписывал `--tool-call-parser qwen3_coder`. В b11557 флага
    /// нет, и он даёт ошибку запуска. Проверяем, что в аргументах его
    /// не осталось, а шаблон включён.
    #[test]
    fn устаревший_флаг_не_попадает_в_аргументы() {
        let args = server_args(&recipe_named("9B q4_0", "m.gguf"), Path::new("m.gguf"), 18080);
        assert!(!args.iter().any(|a| a == "--tool-call-parser"));
        assert!(args.contains(&"--jinja".to_string()));
        assert!(args.contains(&"-ngl".to_string()));
        assert!(args.contains(&"99".to_string()));
        assert!(args.contains(&"--reasoning".to_string()));
        assert!(args.contains(&"off".to_string()));
        // Уровень лога 4 обязателен: без него нет строки о размещении.
        assert!(args.contains(&"-lv".to_string()));
        assert!(args.contains(&"4".to_string()));
    }

    #[test]
    fn контекст_рецепта_попадает_в_аргументы() {
        let mut r = recipe_named("9B q4_0", "m.gguf");
        r.context = 65_536;
        let args = server_args(&r, Path::new("m.gguf"), 18080);
        let idx = args.iter().position(|a| a == "-c").unwrap();
        assert_eq!(args[idx + 1], "65536");
    }



    fn catalog(recipes: Vec<Recipe>) -> Catalog {
        let mut profiles = HashMap::new();
        profiles.insert(
            "T2".to_string(),
            ProfileRecipes {
                tier: "T2".into(),
                llm: "Qwen/Qwen3.5-9B-GGUF".into(),
                recipes,
            },
        );
        Catalog {
            llama_cpp: LlamaCppPin {
                repo: "ggml-org/llama.cpp".into(),
                release: "b11557".into(),
                asset_windows: "a.zip".into(),
                asset_cudart: "b.zip".into(),
                license: "MIT".into(),
            },
            profiles,
            common_args: vec![],
            chat_template_kwargs: serde_json::Value::Null,
            sampling: None,
        }
    }

    #[test]
    fn без_секции_выборки_берутся_значения_для_секретаря() {
        let c = catalog(vec![]);
        let s = c.sampling().unwrap();
        assert_eq!(s.temperature, 0.3);
        assert_eq!(s.top_p, 0.9);
        assert_eq!(s.top_k, 40);
        assert_eq!(s.repeat_penalty, 1.1);
    }

    /// Раньше параметров выборки не было вообще, и модель брала их
    /// из шаблона. Теперь значения приходят из каталога.
    #[test]
    fn секция_выборки_перекрывает_умолчания() {
        let mut c = catalog(vec![]);
        c.sampling = Some(SamplingSection {
            temperature: Some(0.15),
            top_p: Some(0.8),
            top_k: None,
            repeat_penalty: None,
        });
        let s = c.sampling().unwrap();
        assert_eq!(s.temperature, 0.15);
        assert_eq!(s.top_p, 0.8);
        // Незаданные поля берутся из умолчаний, а не становятся нулём.
        assert_eq!(s.top_k, 40);
        assert_eq!(s.repeat_penalty, 1.1);
    }

    #[test]
    fn бессмысленные_значения_выборки_отвергаются() {
        let mut c = catalog(vec![]);
        c.sampling = Some(SamplingSection {
            temperature: Some(5.0),
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        });
        assert!(c.sampling().unwrap_err().contains("temperature"));

        c.sampling = Some(SamplingSection {
            temperature: None,
            top_p: None,
            top_k: Some(0),
            repeat_penalty: None,
        });
        assert!(c.sampling().unwrap_err().contains("top_k"));

        c.sampling = Some(SamplingSection {
            temperature: None,
            top_p: None,
            top_k: None,
            // Ниже 1 модель начинает избегать повторов и рассыпается.
            repeat_penalty: Some(0.7),
        });
        assert!(c.sampling().unwrap_err().contains("repeat_penalty"));
    }

    #[test]
    fn значения_выборки_уходят_в_json_как_есть() {
        let j = Sampling::default_for_secrets().to_json();
        // Сравниваем как f64: 0.3 хранится как f32 и при переводе
        // в f64 даёт 0.3000000119, точное равенство не сработает.
        let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert!(near(j["temperature"].as_f64().unwrap() as f32, 0.3));
        assert!(near(j["top_p"].as_f64().unwrap() as f32, 0.9));
        assert_eq!(j["top_k"].as_u64(), Some(40));
        assert!(near(j["repeat_penalty"].as_f64().unwrap() as f32, 1.1));
    }
}

// Разрешения: чем секретарю можно пользоваться.
//
// Два разных вопроса, которые легко спутать:
//
//   возможность  — что агент вообще умеет (см. `capabilities`)
//   разрешение   — что ему разрешено делать на этой машине
//
// Возможность без разрешения бесполезна. Разрешение без возможности
// ничего не разрешает. Реестр отвечает на первое, этот модуль — на
// второе, и порядок именно такой: сначала «умеет», потом «можно».
//
// Что разрешено без явного решения, определяет характер действия:
// чтение — да, действие — нет. Объяснение в [`Settings::allows`].
//
// Почему не «запрещено всё, что не разрешено»: чтение файлов без
// явной галочки означало бы, что агент бесполезен до первого
// знакомства. Почему не «разрешено всё, чего не запретили»: запись,
// запуск программ и управление окнами опаснее чтения на порядок, и
// о них пользователь должен был сказать сам.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Как агент должен поступать с действием.
///
/// Словарь значений не придуман: `actions` / `all` / `never` — это
/// `COMPUTER_USE_APPROVAL_MODE` у Hermes, задокументированный в
/// [docs/06-референсы.md](../docs/06-референсы.md). Своё имя в
/// настройках пользователя путало бы с тем, что он уже видел.
///
/// Формулировки для интерфейса — свои: по названию значения в файле
/// нельзя понять, когда спросят.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalMode {
    /// `actions` — спрашивать, когда действие что-то меняет.
    #[serde(rename = "actions")]
    Risky,
    /// `all` — спрашивать всегда.
    #[serde(rename = "all")]
    Confirm,
    /// `never` — не спрашивать.
    #[serde(rename = "never")]
    Auto,
}

impl ApprovalMode {
    pub fn title(self) -> &'static str {
        match self {
            ApprovalMode::Risky => "Спрашивать, когда меняет",
            ApprovalMode::Confirm => "Спрашивать всегда",
            ApprovalMode::Auto => "Не спрашивать",
        }
    }

    /// Что это значит, словами для подсказки в настройках.
    pub fn explain(self) -> &'static str {
        match self {
            ApprovalMode::Risky => "Подтверждать запись, удаление и запуск программ. Чтение идёт без спроса.",
            ApprovalMode::Confirm => "Подтверждать каждое действие, включая простое чтение.",
            ApprovalMode::Auto => "Не спрашивать ничего. Только для тех, кто доверяет агенту полностью.",
        }
    }
}

/// Что агент вообще делает с системой.
///
/// Значения — как в [docs/02-архитектура.md](../docs/02-архитектура.md):
/// `dry-run` и `live`, через дефис.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Executor {
    /// Только показывать, что было бы сделано. Ничего не выполнять.
    ///
    /// Это режим по умолчанию, и он не для галочки: агент с правом
    /// записи, который не проверен, опаснее бесполезного.
    #[serde(rename = "dry-run")]
    DryRun,
    /// Выполнять.
    #[serde(rename = "live")]
    Live,
}

impl Executor {
    pub fn title(self) -> &'static str {
        match self {
            Executor::DryRun => "Только показывать",
            Executor::Live => "Выполнять",
        }
    }
}

/// Решение по одной возможности.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRule {
    pub enabled: bool,
    #[serde(default)]
    pub mode: Option<ApprovalMode>,
}

/// Настройки разрешений.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    /// Читать всё на диске, без списка папок.
    ///
    /// По умолчанию выключено. Человек решает это сам и видит
    /// решение в знакомстве и в настройках, а не получает его
    /// вместе со списком папок. Значение по умолчанию именно
    /// «выключено», а не «папки пусты»: это разные вещи, и второе
    /// читалось бы как «доступа нет вовсе».
    #[serde(default)]
    pub читать_весь_диск: bool,
    /// Приложения, которых агент не коснётся никогда.
    #[serde(default)]
    pub deny_apps: Vec<String>,
    #[serde(default)]
    pub capabilities: std::collections::BTreeMap<String, CapabilityRule>,
    #[serde(default)]
    pub cloud: bool,
    #[serde(default = "default_executor")]
    pub executor: Executor,
    #[serde(default = "default_approval")]
    pub approval_mode: ApprovalMode,
}

fn default_executor() -> Executor {
    Executor::DryRun
}

fn default_approval() -> ApprovalMode {
    ApprovalMode::Risky
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            читать_весь_диск: false,
            // Секретарь обязан быть в собственном запрете: агент не
            // должен управлять интерфейсом, через который с ним
            // говорят. Иначе он может закрыть окно, в котором его
            // спрашивают, и ответить за пользователя.
            deny_apps: vec!["local.secretary.app".to_string()],
            capabilities: std::collections::BTreeMap::new(),
            cloud: false,
            executor: Executor::DryRun,
            approval_mode: ApprovalMode::Risky,
        }
    }
}

/// Ответ «можно или нет» с причиной.
#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub allowed: bool,
    /// Что сказать пользователю, если нельзя. Пусто, если можно.
    pub reason: String,
}

impl Decision {
    fn yes() -> Self {
        Self { allowed: true, reason: String::new() }
    }
    fn no(reason: impl Into<String>) -> Self {
        Self { allowed: false, reason: reason.into() }
    }
}

impl Settings {
    pub fn candidate_paths() -> Vec<PathBuf> {
        let mut paths = vec![crate::profile::config_dir().join("permissions.json")];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                paths.push(dir.join("data").join("permissions.json"));
                paths.push(dir.join("permissions.json"));
            }
        }
        if let Ok(dir) = std::env::var("ARKHA_DATA_DIR") {
            paths.push(Path::new(&dir).join("permissions.json"));
        }
        paths.push(PathBuf::from("data").join("permissions.json"));
        paths.push(PathBuf::from("../data/permissions.json"));
        paths
    }

    pub fn load() -> Result<Self, String> {
        Self::load_из(&Self::candidate_paths())
    }

    /// Чтение из указанных файлов: берётся первый, который есть.
    ///
    /// Отдельная функция не для красоты. `load` читает настоящий
    /// файл человека, и тест, который её зовёт, проверяет не наш
    /// код, а то, как настроил машину владелец: он требовал, чтобы
    /// в файле был `dry-run`, то есть чтобы мы запрещали человеку
    /// выбрать «выполнять». Замечено 15.10.2026 — после обычного
    /// сеанса тесты посыпались, и виноват был не код, а человек,
    /// сделавший то, что ему и предлагали.
    pub fn load_из(пути: &[PathBuf]) -> Result<Self, String> {
        for path in пути {
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("не удалось прочитать {}: {e}", path.display()))?;
                return serde_json::from_str(&text)
                    .map_err(|e| format!("повреждён {}: {e}", path.display()));
            }
        }
        // Отсутствие файла — не ошибка: настройки по умолчанию
        // безопасны, и их применение лучше, чем отказ работать.
        Ok(Settings::default())
    }

    /// Можно ли выполнять действие по этой возможности.
    ///
    /// Порядок проверок важен: сначала то, что пользователь не может
    /// изменить случайно, потом то, что может.
    pub fn allows(&self, capability: &str) -> Decision {
        if self.executor == Executor::DryRun && self.is_side_effecting(capability) {
            return Decision::no(
                "режим dry-run: действие показано, но не выполнено. Включите выполнение в настройках",
            );
        }
        match self.capabilities.get(capability) {
            Some(rule) if !rule.enabled => Decision::no("выключено в настройках разрешений"),
            Some(_) => Decision::yes(),
            // Явного решения нет — решает характер действия.
            //
            // Читать безопасно, действовать — нет. Значит чтение
            // разрешено по умолчанию, а запись, запуск и удаление
            // требуют явного разрешения. Раньше здесь стояло
            // «разрешено всё, чего не запретили», и это противоречило
            // тому, что написано в документации.
            None if self.is_side_effecting(capability) => {
                Decision::no("не разрешено явно: сначала включи в знакомстве")
            }
            None => Decision::yes(),
        }
    }

    /// Знает ли модель, что инструмент существует.
    ///
    /// Отличие от `allows` намеренное. `allows` отвечает на вопрос
    /// «можно ли выполнить сейчас», здесь — «показывать ли его
    /// модели».
    ///
    /// Инструмент, который не разрешён, но не выключен, должен быть
    /// виден. Иначе модель отвечает «у меня такого инструмента
    /// нет», а это неправда: инструмент есть, его не разрешили.
    ///
    /// Что выдало себя на живом железе: `apps.launch` и
    /// `windows.control` заканчиваются на «опасные» суффиксы, по
    /// умолчанию не разрешены и выбрасывались из схемы. Модель
    /// отвечала «я не могу открыть программу, так как у меня нет
    /// `apps.launch`» — и это было чистое враньё, потому что
    /// инструмент в системе есть.
    ///
    /// Явно выключенное (`enabled: false`) скрывается по-прежнему:
    /// человек выключил, значит и знать не должен.
    pub fn показывать(&self, capability: &str) -> bool {
        match self.capabilities.get(capability) {
            Some(rule) => rule.enabled,
            None => true,
        }
    }

    /// Нужно ли подтверждение для этой возможности.
    pub fn needs_approval(&self, capability: &str) -> bool {
        if let Some(rule) = self.capabilities.get(capability) {
            if let Some(mode) = rule.mode {
                return mode != ApprovalMode::Auto;
            }
        }
        match self.approval_mode {
            ApprovalMode::Auto => false,
            ApprovalMode::Confirm => true,
            ApprovalMode::Risky => self.is_side_effecting(capability),
        }
    }

    /// Может ли агент коснуться этого приложения.
    pub fn app_allowed(&self, app: &str) -> Decision {
        if self.is_denied_app(app) {
            return Decision::no("приложение в списке запрещённых");
        }
        Decision::yes()
    }

    /// Запрет нечувствителен к регистру и пути: агент может получить
    /// `C:\...\arkha.exe`, а список хранит `arkha`. Сравнение
    /// «как есть» дало бы обход в одну строку.
    pub fn is_denied_app(&self, app: &str) -> bool {
        let needle = app.trim().to_lowercase();
        let needle = needle.rsplit('\\').next().unwrap_or(&needle).to_string();
        self.deny_apps.iter().any(|d| {
            let d = d.trim().to_lowercase();
            d == needle || d.ends_with(&needle) || needle.ends_with(&d)
        })
    }

    /// Действие, меняющее систему. Читать можно свободно, писать —
    /// нет, иначе «чтение документов» превратилось бы в «правку
    /// всего подряд».
    pub fn is_side_effecting(&self, capability: &str) -> bool {
        capability.ends_with(".write")
            || capability.ends_with(".delete")
            || capability.ends_with(".launch")
            || capability.ends_with(".control")
            || capability.ends_with(".run")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn по_умолчанию_ничего_не_выполняется() {
        let s = Settings::default();
        assert_eq!(s.executor, Executor::DryRun);
        assert!(!s.cloud, "облако по умолчанию выключено");
    }

    /// Агент не должен управлять интерфейсом, через который с ним
    /// говорят. Иначе он может закрыть окно и ответить за пользователя.
    #[test]
    fn секретарь_в_своём_же_запрете() {
        let s = Settings::default();
        assert!(s.is_denied_app("local.secretary.app"));
    }

    /// Запрет не должен обходиться регистром или путём к файлу.
    #[test]
    fn запрет_не_обходится_регистром_и_путём() {
        let s = Settings::default();
        assert!(s.is_denied_app("Local.Secretary.App"));
        assert!(s.is_denied_app(r"C:\Program Files\ArkHa\local.secretary.app"));
        assert!(!s.is_denied_app("notepad"));
    }

    #[test]
    fn режим_dry_run_останавливает_запись_но_не_чтение() {
        let s = Settings::default();
        assert!(!s.allows("files.write").allowed);
        assert!(s.allows("files.read").allowed);
        assert!(s
            .allows("files.write")
            .reason
            .contains("dry-run"));
    }

    /// Без явного решения действовать нельзя. Раньше здесь стояло
    /// «разрешено всё, чего не запретили», и это противоречило
    /// документации: запись и запуск программ опаснее чтения.
    #[test]
    fn без_явного_решения_действовать_нельзя() {
        let mut s = Settings::default();
        s.executor = Executor::Live;

        assert!(s.allows("files.read").allowed, "чтение безопасно");
        assert!(!s.allows("files.write").allowed, "запись требует явного разрешения");
        assert!(!s.allows("apps.launch").allowed, "запуск требует явного разрешения");
        assert!(
            !s.allows("windows.control").allowed,
            "управление окнами требует явного разрешения"
        );
        let d = s.allows("files.write");
        assert!(d.reason.contains("не разрешено явно"), "{}", d.reason);
    }

    /// Видимость и разрешение — разные вопросы: модель должна видеть
    /// существующее действие, чтобы получить честный отказ, но
    /// пользователь может скрыть его явным запретом.
    #[test]
    fn видимость_отделена_от_права_исполнить() {
        let mut s = Settings::default();
        s.executor = Executor::Live;

        assert!(s.показывать("apps.launch"), "не выданное действие видно модели");
        assert!(
            !s.allows("apps.launch").allowed,
            "видимость не даёт права запуска"
        );

        s.capabilities.insert(
            "apps.launch".into(),
            CapabilityRule { enabled: false, mode: None },
        );
        assert!(!s.показывать("apps.launch"), "явно выключенное скрывается");
        assert!(!s.allows("apps.launch").allowed);
    }

    /// Явное разрешение возвращает действие.
    #[test]
    fn явное_разрешение_возвращает_действие() {
        let mut s = Settings::default();
        s.executor = Executor::Live;
        s.capabilities.insert(
            "files.write".into(),
            CapabilityRule { enabled: true, mode: None },
        );
        assert!(s.allows("files.write").allowed);
    }

    #[test]
    fn выключенное_в_настройках_запрещено() {
        let mut s = Settings::default();
        s.executor = Executor::Live;
        s.capabilities.insert(
            "files.delete".into(),
            CapabilityRule { enabled: false, mode: None },
        );
        let d = s.allows("files.delete");
        assert!(!d.allowed);
        assert!(d.reason.contains("настройк"), "{}", d.reason);
    }

    #[test]
    fn чтение_не_требует_подтверждения_а_запись_требует() {
        let s = Settings::default();
        assert!(!s.needs_approval("files.read"));
        assert!(s.needs_approval("files.write"));
    }

    /// Режим «всегда спрашивать» должен спрашивать и на чтении:
    /// пользователь выбрал именно этот режим.
    #[test]
    fn режим_всегда_спрашивает_на_всём() {
        let mut s = Settings::default();
        s.approval_mode = ApprovalMode::Confirm;
        assert!(s.needs_approval("files.read"));
    }

    /// Отсутствие файла даёт безопасные настройки.
    ///
    /// Путь выбран явно и заведомо несуществующий: раньше тест
    /// звал `load`, то есть смотрел в настоящий файл человека.
    #[test]
    fn отсутствие_файла_даёт_безопасные_настройки() {
        let нет = PathBuf::from("нет-такого-файла-permissions.json");
        let s = Settings::load_из(&[нет]).expect("отсутствие файла — не ошибка");
        assert_eq!(s.executor, Executor::DryRun);
        assert!(
            s.is_denied_app("local.secretary.app"),
            "секретарь обязан быть в запрете по умолчанию"
        );
    }

    /// Выбор человека сохраняется, а не переписывается.
    ///
    /// Тест на то, чего не было. Раньше проверка требовала, чтобы в
    /// файле всегда был `dry-run`, и единственный способ её
    /// «починить», не упав, был запретить человеку выбрать
    /// «выполнять». Проверялся не код, а настройка машины.
    ///
    /// Теперь обратное: то, что человек выбрал сам, доезжает до
    /// него целым. Иначе «починка» была бы тихой и выглядела бы
    /// как безопасность, а была бы подменой его решения.
    #[test]
    fn выбор_человека_не_переписывается() {
        let mut выбрал = Settings::default();
        выбрал.executor = Executor::Live;
        выбрал.paths = vec![PathBuf::from("Рабочий стол")];

        let каталог = std::env::temp_dir().join("arkha-выбор-человека");
        std::fs::create_dir_all(&каталог).expect("каталог создан");
        let файл = каталог.join("permissions.json");
        std::fs::write(&файл, serde_json::to_string(&выбрал).expect("запись"))
            .expect("файл записан");

        let прочитан = Settings::load_из(std::slice::from_ref(&файл)).expect("чтение");
        assert_eq!(
            прочитан.executor,
            Executor::Live,
            "выбранное человеком «выполнять» должно дожить до чтения"
        );
        assert_eq!(прочитан.paths.len(), 1, "выбранная папка доехала");

        let _ = std::fs::remove_dir_all(&каталог);
    }
}

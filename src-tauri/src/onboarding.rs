// Знакомство: пользователь собирает секретаря в разговоре.
//
// Первый настоящий сценарий продукта. До него были вычисления, а не
// общение.
//
// Три решения, которые здесь приняты и неочевидны.
//
// **Ответы собираются в JSON, а не разбираются из прозы.** Разбор
// естественного языка без модели-парсера даёт мусор на длинных
// ответах, а мусор в разрешениях опаснее, чем его отсутствие. Если
// разобрать не удалось, спрашиваем ещё раз, а не угадываем.
//
// **Пользователь видит собранное до того, как оно применится.**
// Шаг «итог» существует не для красоты: конфигурацию разрешений
// невозможно отменить задним числом, если человек не понял, что
// подтвердил.
//
// **Ничего не применяется молча.** `dry-run` по умолчанию остаётся
// в силе, пока пользователь не выбрал иное явно.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::capabilities::Registry;
use crate::permissions::{ApprovalMode, CapabilityRule, Executor, Settings};

/// Шаги знакомства. Пять: шаг «проверка» перенесён на S6, где
/// появляются первые инструменты.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Step {
    /// Представление: кто я и что умею.
    Introduction,
    /// Инструменты: что включать.
    Tools,
    /// Папки: где искать документы.
    Folders,
    /// Уровни: что можно без подтверждения.
    Levels,
    /// Итог: что понял, поправить или закончить.
    Summary,
}

impl Step {
    pub const ALL: [Step; 5] = [
        Step::Introduction,
        Step::Tools,
        Step::Folders,
        Step::Levels,
        Step::Summary,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Step::Introduction => "introduction",
            Step::Tools => "tools",
            Step::Folders => "folders",
            Step::Levels => "levels",
            Step::Summary => "summary",
        }
    }

    /// Название шага для интерфейса. Живёт здесь, а не в командах:
    /// копия рано или поздно разъехалась бы с этим местом.
    pub fn title(self) -> &'static str {
        match self {
            Step::Introduction => "Знакомство",
            Step::Tools => "Инструменты",
            Step::Folders => "Папки",
            Step::Levels => "Уровни доступа",
            Step::Summary => "Итог",
        }
    }

    /// Что спрашивает агент на этом шаге.
    ///
    /// Текст отдаётся модели вместе с реестром возможностей. Модель
    /// должна знать свои границы из данных, а не из нашего текста:
    /// текст разъедется с реестром при первом же изменении.
    pub fn prompt(self, tier: crate::profile::Tier) -> String {
        let reg = Registry::load().unwrap_or_else(|_| Registry { capabilities: vec![] });
        let view = reg.for_model(tier);

        let can: Vec<String> = view
            .iter()
            .filter(|v| v.available)
            .map(|v| format!("- {}: {}", v.id, v.what))
            .collect();
        let cannot: Vec<String> = view
            .iter()
            .filter(|v| !v.available)
            .map(|v| {
                let why = v.reason.clone().unwrap_or_default();
                format!("- {}: {} ({})", v.id, v.what, why)
            })
            .collect();

        // Строки собираем заранее: временное значение внутри format!
        // не проживает дольше выражения.
        let can_text = if can.is_empty() {
            "- (пока ничего)".to_string()
        } else {
            can.join("\n")
        };
        let cannot_text = if cannot.is_empty() {
            "- (нет)".to_string()
        } else {
            cannot.join("\n")
        };

        let head = format!(
            "Ты локальный ИИ-секретарь на Windows. Всё работает на компьютере \
             пользователя, ничего не отправляется наружу.\n\n\
             Сейчас первый запуск, ты знакомишься с пользователем.\n\n\
             ТВОИ ВОЗМОЖНОСТИ СЕЙЧАС.\nДоступно:\n{can_text}\n\n\
             Недоступно:\n{cannot_text}\n\n\
             Не обещай то, что во второй списке. Если пользователь \
             спросит недоступное, скажи прямо, что не умеешь, и объясни \
             причину.\n"
        );

        // Подсказки для шага «папки»: только имена и пути, без
        // содержимого. Содержимое показывается после разрешения,
        // и никогда раньше.
        let suggestions: Vec<String> = folder_suggestions()
            .iter()
            .map(|(label, path)| format!("- {label} ({})", path.display()))
            .collect();
        let suggestions = if suggestions.is_empty() {
            "- (папок по умолчанию не нашлось)".to_string()
        } else {
            suggestions.join("\n")
        };

        match self {
            Step::Introduction => format!(
                "{head}\nЗАДАЧА. Поздоровайся и представься в двух-трёх \
                 предложениях. Скажи, где работаешь и что всё локально. \
                 Перечисли, что умеешь, одной строкой. Спроси, как к тебе \
                 обращаться. Не задавай больше одного вопроса за раз."
            ),
            Step::Tools => format!(
                "{head}\nЗАДАЧА. Спроси, что пользователь хочет от тебя: \
                 работа с файлами, с программами, интернет или голос. \
                 ОТВЕЧАЙ ТОЛЬКО JSON без пояснений, например:\
                 {{\"enable\": [\"files.read\"], \"disable\": [\"browser.dom\"]}}\
                 В списках только идентификаторы из раздела «Доступно» выше."
            ),
            Step::Folders => format!(
                "{head}\nЗАДАЧА. Спроси, в каких папках искать документы. \
                 Вот папки, которые нашлись на диске:\n{suggestions}\n\
                 ОТВЕЧАЙ ТОЛЬКО JSON без пояснений, например:\
                 {{\"paths\": [\"Рабочий стол\", \"Загрузки\"]}}\
                 Перечисляй только те, что есть в списке выше."
            ),
            Step::Levels => format!(
                "{head}\nЗАДАЧА. Спроси, что можно делать без подтверждения \
                 каждый раз. Варианты: только читать (ничего не меняю); \
                 читать и создавать; полный доступ (перемещать и удалять).\
                 ОТВЕЧАЙ ТОЛЬКО JSON без пояснений, например:\
                 {{\"executor\": \"dry-run\", \"approval_mode\": \"actions\"}}"
            ),
            Step::Summary => format!(
                "{head}\nЗАДАЧА. Перечисли одним абзацем, что ты понял: \
                 как к тебе обращаться, что умеешь, какие папки доступны, \
                 что можешь делать без подтверждения. Спроси, правильно ли, \
                 и предложи что-то добавить. Ничего JSON, это последний шаг."
            ),
        }
    }

    /// Нужен ли на этом шаге разбор ответа в JSON.
    pub fn expects_json(self) -> bool {
        matches!(self, Step::Tools | Step::Folders | Step::Levels)
    }

    fn next(self) -> Option<Step> {
        let i = Step::ALL.iter().position(|s| *s == self)?;
        Step::ALL.get(i + 1).copied()
    }
}

/// Что агент собрал о себе.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Collected {
    /// Как просит себя называть.
    pub name: Option<String>,
    /// Возможности, которые пользователь включил.
    pub enable: Vec<String>,
    /// Возможности, которые пользователь выключил.
    pub disable: Vec<String>,
    /// Папки, разрешённые пользователем. Настоящие пути, не названия.
    pub paths: Vec<PathBuf>,
    /// Читать весь диск вместо списка папок.
    #[serde(default)]
    pub whole_disk: bool,
    /// Режим выполнения.
    pub executor: Option<Executor>,
    /// Когда спрашивать.
    pub approval_mode: Option<ApprovalMode>,
}

/// Состояние знакомства.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub step: Step,
    pub finished: bool,
    #[serde(default)]
    pub collected: Collected,
}

/// Разобранный ответ модели для шагов с JSON.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enable: Vec<String>,
    #[serde(default)]
    pub disable: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub executor: Option<Executor>,
    #[serde(default)]
    pub approval_mode: Option<ApprovalMode>,
}

/// Папки, которые имеет смысл предложить. Только имена и пути:
/// содержимое не читается и не показывается.
pub fn folder_suggestions() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let known: [(&str, &str); 4] = [
        ("Рабочий стол", "Desktop"),
        ("Загрузки", "Downloads"),
        ("Документы", "Documents"),
        ("Изображения", "Pictures"),
    ];
    for (label, sub) in known {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            let p = Path::new(&profile).join(sub);
            if p.is_dir() {
                out.push((label.to_string(), p));
            }
        }
    }
    out
}

/// Название вида «Рабочий стол» приводится к настоящему пути.
///
/// Несуществующие и неизвестные названия отбрасываются. В разрешения
/// попадает только то, что действительно есть на диске: путь, которого
/// нет, всплывёт через месяц и будет выглядеть как поломка.
pub fn resolve_folder(name: &str) -> Option<PathBuf> {
    let wanted = name.trim().trim_matches('"').to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    for (label, path) in folder_suggestions() {
        if label.to_lowercase() == wanted {
            return Some(path);
        }
    }
    // Уже готовый путь, например D:\Работа
    let p = PathBuf::from(name.trim().trim_matches('"'));
    if p.is_dir() {
        return Some(p);
    }
    None
}

/// Разбирает ответ модели.
///
/// Возвращает `None`, если разобрать нельзя. Это не ошибка: модель
/// могла ответить словами, и тогда вопрос повторяется. Угадывать
/// опаснее, чем спросить.
pub fn parse_answer(text: &str) -> Option<Answer> {
    // Модель любит обернуть JSON в блок кода или добавить точку.
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(text[start..=end].trim()).ok()
}

impl State {
    pub fn start() -> Self {
        Self { step: Step::Introduction, finished: false, collected: Collected::default() }
    }

    /// Применяет ответ и переходит к следующему шагу.
    ///
    /// Возвращает `false`, если ответ не разобрался: шаг остаётся
    /// прежним, вопрос повторяется.
    pub fn apply(&mut self, text: &str) -> bool {
        if !self.step.expects_json() {
            // На представлении и итоге JSON нет: берём имя из текста
            // или оставляем как есть. Это единственный шаг, где
            // разбор текста уместен, потому что там ничего не
            // применяется к системе.
            if self.step == Step::Introduction {
                if let Some(a) = parse_answer(text) {
                    self.collected.name = a.name;
                }
            }
            self.advance();
            return true;
        }

        let Some(answer) = parse_answer(text) else {
            return false;
        };
        match self.step {
            Step::Tools => {
                self.collected.enable = answer.enable;
                self.collected.disable = answer.disable;
            }
            Step::Folders => {
                // Только существующие. Молча отбрасываем остальное:
                // путь, которого нет, в разрешениях не нужен.
                self.collected.paths = answer
                    .paths
                    .iter()
                    .filter_map(|n| resolve_folder(n))
                    .collect();
                self.collected.paths.sort();
                self.collected.paths.dedup();
            }
            Step::Levels => {
                self.collected.executor = answer.executor;
                self.collected.approval_mode = answer.approval_mode;
            }
            _ => {}
        }
        self.advance();
        true
    }

    fn advance(&mut self) {
        match self.step.next() {
            Some(next) => self.step = next,
            None => self.finished = true,
        }
    }

        /// Подтверждение итога человеком.
    ///
    /// Итог подтверждается **кнопкой**, а не фразой: человек читает
    /// собранное и жмёт «сохранить».
    ///
    /// Раньше единственным способом закончить знакомство был ответ
    /// текстом «всё верно, закончить», а кнопка «сохранить» звала
    /// применение, которое требует `finished`. То есть кнопка
    /// упиралась в ту же проверку, что и фраза, и не сработать не
    /// могла: знакомство нельзя было закончить. Замечено человеком
    /// 15.10.2026, вместе с пустым списком папок — настройки не
    /// сохранялись, потому что сохранять было нечем.
    pub fn подтвердить_итог(&mut self) -> Result<(), String> {
        if self.step != Step::Summary {
            return Err(format!(
                "подтвердить можно только на итоге, а сейчас шаг «{}»",
                self.step.as_str()
            ));
        }
        self.finished = true;
        Ok(())
    }

    /// Принимает выбор режима и уровня доступа кнопками.
    ///
    /// Шаг сдвигается, когда выбраны оба. Один выбранный — это
    /// половина ответа, и двигать шаг после неё нельзя: человек
    /// окажется на следующем вопросе, не ответив на второй.
    pub fn set_modes(&mut self, executor: Option<Executor>, approval: Option<ApprovalMode>) {
        if let Some(e) = executor {
            self.collected.executor = Some(e);
        }
        if let Some(a) = approval {
            self.collected.approval_mode = Some(a);
        }
        if self.step == Step::Levels
            && self.collected.executor.is_some()
            && self.collected.approval_mode.is_some()
        {
            self.advance();
        }
    }

    /// Принимает выбор папок кнопками.
    ///
    /// Названия приводятся к настоящим путям, несуществующие
    /// отбрасываются. Путь, которого нет, всплывёт через месяц и
    /// будет выглядеть как поломка.
    pub fn set_paths(&mut self, names: Vec<String>) {
        let mut paths: Vec<PathBuf> = names.iter().filter_map(|n| resolve_folder(n)).collect();
        paths.sort();
        paths.dedup();
        self.collected.paths = paths;

        if self.step == Step::Folders {
            self.advance();
        }
    }

    /// Принимает решение «читать весь диск».
    ///
    /// Шаг сдвигается только при включении. Выключение — это отказ от
    /// варианта, а не ответ на вопрос, и двигать шаг после него
    /// нельзя: человек снял галочку, чтобы поставить другую, и
    /// оказался бы дальше, чем собирался.
    ///
    /// Включение сдвигает шаг, потому что это и есть ответ на
    /// вопрос шага: «что секретарю разрешено читать». Выбрать можно
    /// что-то одно — иначе человек не понимает, что именно он дал.
    pub fn set_whole_disk(&mut self, enabled: bool) {
        self.collected.whole_disk = enabled;
        if enabled && self.step == Step::Folders {
            self.advance();
        }
    }

    /// Принимает выбор инструментов кнопками.
    ///
    /// Идентификаторы проверяются по реестру. Кнопки в интерфейсе
    /// подставляют то, что пришло из Rust, но проверить всё равно
    /// надо: значение из интерфейса — это значение извне.
    ///
    /// Неизвестное отбрасывается, а не отвергает весь выбор: один
    /// неверный идентификатор не должен заставлять человека
    /// проходить знакомство заново.
    pub fn set_tools(&mut self, enable: Vec<String>, disable: Vec<String>) {
        let known: Vec<String> = Registry::load()
            .map(|r| r.capabilities.iter().map(|c| c.id.clone()).collect())
            .unwrap_or_default();

        let clean = |v: Vec<String>| -> Vec<String> {
            v.into_iter()
                .filter(|id| known.contains(id))
                .collect()
        };

        // Один идентификатор не может быть и включён, и выключен.
        // Выключение побеждает: это безопасная сторона.
        let dis = clean(disable);
        let en: Vec<String> = clean(enable).into_iter().filter(|id| !dis.contains(id)).collect();

        self.collected.enable = en;
        self.collected.disable = dis;

        if self.step == Step::Tools {
            self.advance();
        }
    }

    /// Собирает разрешения из собранного.
    ///
    /// Показанные, а не применённые. Применение — отдельный вызов с
    /// явным подтверждением.
    pub fn to_settings(&self) -> Settings {
        self.to_settings_из(Settings::load().unwrap_or_default())
    }

    /// Настройки по собранным ответам, начиная с указанной базы.
    ///
    /// Отдельная функция не для красоты. `to_settings` читает
    /// настоящий файл пользователя, и тест, который её зовёт,
    /// зависит от того, что человек успел настроить: после
    /// обычного сеанса тесты знакомства
    /// посыпались, потому что в файле уже стояло `executor: live`,
    /// а тесты ждали чистого старта.
    pub fn to_settings_из(&self, mut s: Settings) -> Settings {

        s.paths = self.collected.paths.clone();
        s.читать_весь_диск = self.collected.whole_disk;

        if let Some(e) = self.collected.executor {
            s.executor = e;
        }
        if let Some(m) = self.collected.approval_mode {
            s.approval_mode = m;
        }

        // Записываем и разрешённое, и запрещённое. Иначе включённое
        // потерялось бы, а решение по умолчанию — «действие запрещено»
        // — осталось бы в силе, и знакомство ничего не меняло.
        //
        // Молча оставлять «как было» нельзя: пользователь прошёл
        // знакомство, отметил нужное и ждёт, что это записалось.
        for id in &self.collected.enable {
            s.capabilities
                .insert(id.clone(), CapabilityRule { enabled: true, mode: None });
        }
        for id in &self.collected.disable {
            s.capabilities
                .insert(id.clone(), CapabilityRule { enabled: false, mode: None });
        }

        s
    }

    /// Текст итога: что агент понял, словами.
    pub fn summary(&self) -> String {
        self.summary_из(Settings::load().unwrap_or_default())
    }

    /// Итог словами, начиная с указанной базы настроек.
    ///
    /// Как и `to_settings_из`: база передаётся параметром, а не
    /// берётся с машины, иначе тест знакомства зависит от того,
    /// что человек успел настроить.
    pub fn summary_из(&self, база: Settings) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(n) = &self.collected.name {
            parts.push(format!("Зовут {n}"));
        }

        let reg = Registry::load().unwrap_or_else(|_| Registry { capabilities: vec![] });
        let mut enabled: Vec<String> = self
            .collected
            .enable
            .iter()
            .filter_map(|id| reg.get(id))
            .filter(|c| !self.collected.disable.contains(&c.id))
            .map(|c| c.name.clone())
            .collect();
        enabled.sort();
        enabled.dedup();

        if enabled.is_empty() {
            parts.push("инструменты выбраны не будут".to_string());
        } else {
            parts.push(format!("умею: {}", enabled.join(", ")));
        }

        if self.collected.paths.is_empty() {
            parts.push("папки не открыты".to_string());
        } else {
            let names: Vec<String> = self
                .collected
                .paths
                .iter()
                .map(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| p.display().to_string())
                })
                .collect();
            parts.push(format!("папки: {}", names.join(", ")));
        }

        let настройки = self.to_settings_из(база);
        parts.push(format!("выполнение: {}", настройки.executor.title()));
        parts.push(format!("подтверждение: {}", настройки.approval_mode.title()));

        format!("Понял так. {}", parts.join(". "))
    }
}

/// Запись настроек. Отдельная команда, потому что это единственное
/// место, где знакомство меняет систему, и оно требует явного
/// подтверждения.
pub fn save(settings: &Settings) -> Result<(), String> {
    let dir = crate::profile::config_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("не удалось создать {}: {e}", dir.display()))?;
    let path = dir.join("permissions.json");
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("не удалось подготовить настройки: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("не удалось записать {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn шагов_пять_а_не_шесть() {
        // Шаг проверки перенесён на S6: инструментов нет, проверять
        // нечем.
        assert_eq!(Step::ALL.len(), 5);
        assert_eq!(Step::ALL[0], Step::Introduction);
        assert_eq!(Step::ALL[4], Step::Summary);
    }

    #[test]
    fn знакомство_проходит_по_шагам_до_итога() {
        let mut s = State::start();
        assert_eq!(s.step, Step::Introduction);

        assert!(s.apply("Зовут Артём"));
        assert_eq!(s.step, Step::Tools);

        assert!(s.apply(r#"{"enable":["chat.local"]}"#));
        assert_eq!(s.step, Step::Folders);

        assert!(s.apply(r#"{"paths":["Загрузки"]}"#));
        assert_eq!(s.step, Step::Levels);

        assert!(s.apply(r#"{"executor":"dry-run","approval_mode":"actions"}"#));
        assert_eq!(s.step, Step::Summary);

        assert!(s.apply("всё понятно"));
        assert!(s.finished, "после итога знакомство закончено");
    }

    /// Неразобранный ответ обязан остановить шаг. Угадывать
    /// разрешения опаснее, чем спросить ещё раз.
    #[test]
    fn неразобранный_ответ_останавливает_шаг() {
        let mut s = State::start();
        s.step = Step::Tools;
        assert!(!s.apply("я бы включил файлы и интернет"));
        assert_eq!(s.step, Step::Tools, "шаг не должен сдвинуться");
    }

    /// Модель часто оборачивает JSON в блок кода или добавляет точку.
    /// Это не повод терять ответ.
    #[test]
    fn json_в_обёртке_разбирается() {
        let a = parse_answer("Вот ответ:\n```json\n{\"enable\": [\"files.read\"]}\n```\nГотово.")
            .expect("должен разобраться");
        assert_eq!(a.enable, vec!["files.read"]);
    }

    /// Путь, которого нет на диске, в разрешения попадать не должен:
    /// он всплывёт через месяц и будет выглядеть как поломка.
    #[test]
    fn несуществующие_папки_отбрасываются() {
        assert!(resolve_folder("Несуществующая папка").is_none());
        assert!(resolve_folder("").is_none());
        assert!(resolve_folder(r#"D:\точно\нет\такой"#).is_none());
    }

    #[test]
    fn известная_папка_приводится_к_настоящему_пути() {
        if let Some(p) = resolve_folder("Загрузки") {
            assert!(p.is_dir());
            assert!(p.ends_with("Downloads"));
        }
        // На машине без «Загрузок» просто нечего проверять.
    }

    /// Названия файлов на шаге папок не показываются. Только папки.
    #[test]
    fn подсказки_содержат_только_папки() {
        for (label, path) in folder_suggestions() {
            assert!(!label.is_empty());
            assert!(path.is_dir());
            // В подсказке не должно быть ничего, что выглядит как
            // содержимое папки.
            assert!(!label.contains('.'), "имя папки не должно содержать точку");
        }
    }

    /// Итог обязан быть понятен человеку, а не только машине.
    #[test]
    fn итог_перечисляет_что_понято() {
        let mut s = State::start();
        s.step = Step::Summary;
        s.collected.enable = vec!["chat.local".into()];
        s.collected.paths = resolve_folder("Загрузки").into_iter().collect();

        let text = s.summary();
        assert!(text.contains("Понял"), "{text}");
        assert!(text.contains("выполнение"), "{text}");
        assert!(text.contains("подтверждение"), "{text}");
    }

    /// Знакомство не имеет права включать то, о чём не спросили.
    #[test]
    fn выключенным_считается_только_то_о_чём_спросили() {
        let mut s = State::start();
        s.collected.disable = vec!["browser.dom".into()];
        let settings = s.to_settings_из(crate::onboarding::Settings::default());
        assert!(!settings.capabilities["browser.dom"].enabled);
        assert!(
            !settings.capabilities.contains_key("files.read"),
            "не спрошенное не включаем и не выключаем"
        );
    }

    /// Выбор кнопками пишется в разрешения, иначе знакомство ничего
    /// бы не меняло: решение по умолчанию — действовать нельзя.
    #[test]
    fn выбор_кнопками_попадает_в_разрешения() {
        let mut s = State::start();
        s.step = Step::Tools;
        s.set_tools(vec!["files.read".into()], vec!["browser.dom".into()]);

        let settings = s.to_settings_из(crate::onboarding::Settings::default());
        assert_eq!(
            settings.capabilities["files.read"].enabled, true,
            "включённое потерялось"
        );
        assert_eq!(settings.capabilities["browser.dom"].enabled, false);
    }

    /// Идентификатор из интерфейса — значение извне, его надо
    /// проверять. Один ошибочный не должен рушить весь выбор.
    #[test]
    fn неизвестные_идентификаторы_отбрасываются() {
        let mut s = State::start();
        s.step = Step::Tools;
        s.set_tools(
            vec!["files.read".into(), "выдуманное.нет".into()],
            vec!["browser.dom".into(), "тоже.нет".into()],
        );
        assert_eq!(s.collected.enable, vec!["files.read"]);
        assert_eq!(s.collected.disable, vec!["browser.dom"]);
    }

    /// Один идентификатор не может быть и включён, и выключен.
    /// Выключение побеждает: это безопасная сторона.
    #[test]
    fn выключение_побеждает_включение() {
        let mut s = State::start();
        s.step = Step::Tools;
        s.set_tools(vec!["files.read".into()], vec!["files.read".into()]);
        assert!(s.collected.enable.is_empty());
        assert_eq!(s.collected.disable, vec!["files.read"]);
    }

    /// Пока пользователь не выбрал, ничего не включается.
    #[test]
    fn без_ответа_остаётся_безопасный_режим() {
        let mut s = State::start();
        s.step = Step::Levels;
        s.apply(r#"{"paths":["Загрузки"]}"#);
        let settings = s.to_settings_из(crate::onboarding::Settings::default());
        assert_eq!(settings.executor, Executor::DryRun);
        assert_eq!(settings.approval_mode, ApprovalMode::Risky);
    }
}

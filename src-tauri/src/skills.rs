// Навыки: повторяемые процедуры, написанные человеком.
//
// Три решения, которые всё определяют.
//
// **Навык пишет человек.** Не агент. Причина в том, что мы измеряли:
// агент выдумывает содержимое файла, которого не читал, — и стоит
// ему доверить ещё и выдумывание процедур, ошибка станет
// повторяемой. Совпадает с правилом из [docs/05](../../docs/05-безопасность.md):
// выдуманные навыки не используются вообще.
//
// **Формат без YAML.** Шапка разбирается вручную: `serde_yaml` ради
// шести строк тянет за собой дерево зависимостей.
//
// Изначально причиной было правило 15 — запрет на непроверенную
// лицензию. Правило изменено 12 октября 2026, запрета больше нет,
// и это осталось выбором в пользу нуля зависимостей, а не
// обязательством. Если формат усложнится до настоящего YAML, решение
// надо пересмотреть.
//
// **Навык не даёт прав.** То, что он перечисляет в шапке, — это
// заявка, а не доступ. Права по-прежнему проверяются на каждом
// вызове инструмента, и подтверждение спрашивается так же. Навык не
// может обойти ни то, ни другое, потому что это единственное место,
// где проверяется.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Навык целиком.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    /// Имя для вызова. Латиницей, как имя файла без расширения.
    pub name: String,
    /// Версия навыка. Семантическая (например, "1.0.0"), чтобы модель
    /// и человек могли отличить старую процедуру от новой.
    pub version: String,
    /// Что навык делает. По нему модель выбирает навык.
    pub description: String,
    /// Какие возможности нужны. Заявка, а не доступ.
    pub tools: Vec<String>,
    /// Ресурсы, к которым навык обращается. Файлы, папки, URL.
    /// Декларация, а не доступ: права проверяются на каждом вызове.
    pub resources: Vec<String>,
    /// Аргументы навыка.
    pub parameters: Value,
    /// Тело: шаги для модели.
    pub steps: String,
    /// Для manifest-based skills тело можно прочитать только при вызове.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps_path: Option<PathBuf>,
    /// Откуда прочитан. Для ошибок и для показа пользователю.
    pub file: PathBuf,
}

impl Skill {
    /// Текст, который уходит модели при вызове навыка.
    ///
    /// Шаги обрамляются явно: иначе модель получает инструкцию и не
    /// понимает, что это инструкция, а не описание.
    ///
    /// Требование «выполняй» стоит в конце, а не в начале, и это не
    /// стилистика. На живом гейте S8 модель читала навык, где
    /// «выполняй шаги» шло первым предложением, и отвечала текстом:
    /// «Начинаю выполнение навыка: ищу окно notepad…» — то есть
    /// пересказывала план вместо того, чтобы его выполнять. Три
    /// прогона подряд, результат одинаковый. Причина в том, что
    /// после длинного списка шагов требование в начале уже не в
    /// начале, а модель к этому месту готова отвечать. В конце —
    /// готова действовать.
    pub fn текст_для_модели(&self, args: &Value) -> String {
        let steps = self
            .steps_path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_else(|| self.steps.clone());
        let дополнительные = self
            .steps_path
            .as_ref()
            .map(|path| дополнительные_ресурсы(path.parent().unwrap_or(path)))
            .unwrap_or_default();
        let mut вызов = String::from("Параметры навыка:\n");
        match args.as_object() {
            Some(м) if !м.is_empty() => {
                let mut ключи: Vec<&String> = м.keys().collect();
                ключи.sort();
                for к in ключи {
                    вызов.push_str(&format!("- {к}: {}\n", Human(&м[к])));
                }
            }
            _ => вызов.push_str("- не заданы\n"),
        }
        format!(
            "Навык «{}» (версия {}) выбран человеком.\n\
             Ресурсы: {}\n{}\
             Требуемые возможности: {}\n\
             Параметры:\n{}             Шаги:\n{}\n\n\
             Теперь выполняй. Следующий твой ответ — вызов инструмента, который \
             делает первый шаг, а не текст: пользователь ждёт результата, а не \
             пересказа плана. И так на каждом шаге — пока не закончите или пока \
             не упрётесь в отказ.\n\
             Если шаг не получается — скажите пользователю, что именно не вышло. \
             Не выдумывайте результат и не продолжайте мимо отказа.",
            self.name,
            self.version,
             если_пусто(&self.resources, "нет"),
             дополнительные,
             self.tools.join(", "),
             вызов,
             steps
        )
    }

    /// Строки для отчёта и журнала.
    pub fn сводка(&self) -> String {
        format!("{} — {}", self.name, self.description)
    }
}

/// Значение для показа модели.
///
/// Строки показываются без кавычек: `папка: C:\Проверка` читается,
/// `папка: "C:\\Проверка"` — нет. Это же объясняет модели, что перед
/// ней готовое значение, а не фрагмент кода.
struct Human<'a>(&'a Value);

impl std::fmt::Display for Human<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Value::String(с) => write!(f, "{с}"),
            other => write!(f, "{other}"),
        }
    }
}

/// Если срез пуст — возвращает строку-заглушку, иначе элементы через запятую.
fn если_пусто(список: &[String], заглушка: &str) -> String {
    if список.is_empty() {
        заглушка.to_string()
    } else {
        список.join(", ")
    }
}

/// Что не так с навыком.
#[derive(Debug, Clone, PartialEq)]
pub struct Ошибка {
    pub file: String,
    pub текст: String,
}

impl std::fmt::Display for Ошибка {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.текст)
    }
}

/// Набор навыков.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Набор {
    pub навыки: Vec<Skill>,
    /// Файлы, которые не удалось прочитать. Не молча: молчащий
    /// сломанный навык выглядит как «навыков нет».
    pub сломанные: Vec<Ошибка>,
}

/// Лежит ли путь внутри папки после разрешения ссылок.
///
/// Навык может быть символической ссылкой. Без этой проверки файл
/// `../../секрет.md` выглядел бы как обычный навык и читался бы при
/// вызове. Сравнение идёт по каноническим путям, а не по строкам:
/// `..`, короткие имена Windows и ссылки иначе обходят проверку.
fn внутри_папки(корень: &Path, путь: &Path) -> bool {
    let Ok(корень) = корень.canonicalize() else {
        return false;
    };
    let Ok(путь) = путь.canonicalize() else {
        return false;
    };
    путь.starts_with(&корень)
}

impl Набор {
    pub fn пусто() -> Self {
        Self::default()
    }

    /// Взять навык по имени.
    pub fn взять(&self, имя: &str) -> Option<&Skill> {
        let ключ = имя.trim().to_lowercase();
        self.навыки.iter().find(|н| н.name.to_lowercase() == ключ)
    }

    /// Добавить навык, не подменяя уже загруженный с тем же именем.
    ///
    /// Имя приходит из шапки файла, а не из папки. Без этой проверки
    /// новый файл молча вытеснял бы старый: модель вызывала бы
    /// «одно и то же» имя, а выполнялись бы разные шаги. Поэтому
    /// конфликт — это ошибка, а не замена.
    pub fn добавить(&mut self, навык: Skill) {
        if let Some(существующий) = self
            .навыки
            .iter()
            .find(|н| н.name.eq_ignore_ascii_case(&навык.name))
        {
            self.сломанные.push(Ошибка {
                file: навык.file.display().to_string(),
                текст: format!(
                    "имя «{}» уже занято навыком {}",
                    навык.name,
                    существующий.file.display()
                ),
            });
            return;
        }
        self.навыки.push(навык);
    }

    /// Навыки, для которых не хватает возможностей.
    ///
    /// Такие показываются в списке, но не вызываются: иначе модель
    /// выберет навык и упрётся в стену изнутри.
    pub fn доступные(&self, разрешено: &dyn Fn(&str) -> bool) -> Vec<&Skill> {        self.навыки
            .iter()
            .filter(|н| н.tools.iter().all(|t| разрешено(t)))
            .collect()
    }

    pub fn сводка(&self) -> Vec<String> {
        self.навыки.iter().map(Skill::сводка).collect()
    }

    /// Оставить только явно включённые skills. Пустой список означает
    /// совместимый режим до миграции: все найденные legacy skills активны.
    pub fn только_включённые(&self, включённые: &[String]) -> Self {
        if включённые.is_empty() {
            return self.clone();
        }
        Self {
            навыки: self
                .навыки
                .iter()
                .filter(|skill| включённые.iter().any(|id| id == &skill.name))
                .cloned()
                .collect(),
            сломанные: self.сломанные.clone(),
        }
    }

    /// Прочитать все навыки из папки.
    ///
    /// Ошибки не поднимаются вверх: один сломанный навык не должен
    /// ломать остальные. Но они собираются и показываются.
    pub fn из_папки(папка: &Path) -> Self {        let mut набор = Self::пусто();
        let Ok(записи) = std::fs::read_dir(папка) else {
            return набор;
        };
        // Порядок чтения с диска произволен, а список навыков уходит
        // в модель. Без сортировки список менялся бы от запуска к
        // запуску, и кэш префиксов — тоже.
        let записи: Vec<PathBuf> = записи.flatten().map(|з| з.path()).collect();
        let mut файлы: Vec<PathBuf> = записи
            .iter()
            .filter(|п| {
                п.extension().map(|р| р == "md").unwrap_or(false)
                    && !путь_другого_профиля(п)
                    && внутри_папки(папка, п)
            })
            .cloned()
            .collect();
        let mut manifest_dirs: Vec<PathBuf> = записи
            .iter()
            .filter(|п| {
                п.is_dir()
                    && п.join("skill.json").is_file()
                    && !путь_другого_профиля(п)
                    && !путь_другого_профиля(&п.join("skill.json"))
                    && внутри_папки(папка, п)
                    && внутри_папки(папка, &п.join("skill.json"))
            })
            .cloned()
            .collect();
        файлы.sort();
        manifest_dirs.sort();

        for файл in файлы {
            match разобрать_файл(&файл) {
                Ok(н) => набор.добавить(н),
                Err(о) => набор.сломанные.push(о),
            }
        }
        for directory in manifest_dirs {
            match разобрать_manifest(&directory) {
                Ok(skill) => набор.добавить(skill),
                Err(error) => набор.сломанные.push(error),
            }
        }
        набор
    }
}

/// Папка внешних навыков OpenCode: `%USERPROFILE%\.agents\skills`.
///
/// Каждый навык там — папка с `SKILL.md`. Формат тот же markdown с
/// шапкой, но шапка богаче нашей: `license`, `allowed-tools` и прочее
/// встречаются постоянно. Для внешних навыков незнакомые ключи
/// игнорируются, иначе почти все они считались бы сломанными.
pub fn папка_агентов() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".agents")
        .join("skills")
}

/// Прочитать внешние навыки OpenCode из подпапок с `SKILL.md`.
pub fn из_агентских(папка: &Path) -> Набор {
    let mut набор = Набор::пусто();
    let Ok(записи) = std::fs::read_dir(папка) else {
        return набор;
    };
    let mut файлы: Vec<PathBuf> = записи
        .flatten()
        .map(|запись| запись.path().join("SKILL.md"))
        .filter(|файл| {
            файл.is_file()
                && !путь_другого_профиля(файл)
                && внутри_папки(папка, файл)
        })
        .collect();
    файлы.sort();
    for файл in файлы {
        match разобрать_внешний(&файл) {
            Ok(навык) => набор.добавить(навык),
            Err(ошибка) => набор.сломанные.push(ошибка),
        }
    }
    набор
}

/// Разобрать `SKILL.md` из папки OpenCode.
///
/// Мягче нашего формата: неизвестные ключи шапки пропускаются, версия
/// по умолчанию, имя берётся из папки, если в шапке его нет. Тело
/// обязательно: навык без шагов — это описание, а не процедура.
pub fn разобрать_внешний(файл: &Path) -> Result<Skill, Ошибка> {
    let текст = std::fs::read_to_string(файл)
        .map_err(|e| Ошибка { file: путь(файл), текст: format!("не читается: {e}") })?;
    let ош = |т: &str| Ошибка { file: путь(файл), текст: т.to_string() };

    let без_бома = текст.trim_start_matches('\u{feff}');
    let (шапка, тело) = match без_бома.strip_prefix("---") {
        Some(остаток) => match остаток.split_once("\n---") {
            Some((шапка, тело)) => (шапка, тело.trim_start_matches(['\n', '\r'])),
            None => return Err(ош("шапка не закрыта: нет второй строки ---")),
        },
        None => ("", без_бома),
    };

    let mut имя = None;
    let mut версия = None;
    let mut описание = None;
    for строка in шапка.lines() {
        let с = строка.trim();
        let Some((ключ, значение)) = с.split_once(':') else {
            continue;
        };
        let значение = значение.trim().trim_matches('"').to_string();
        if значение.is_empty() {
            continue;
        }
        match ключ.trim().to_lowercase().as_str() {
            "name" => имя = Some(значение),
            "version" => версия = Some(значение),
            "description" => описание = Some(значение),
            // license, allowed-tools, metadata и прочее: не наши дела.
            _ => {}
        }
    }

    if тело.trim().is_empty() {
        return Err(ош("тело навыка пустое: там должны быть шаги"));
    }

    let имя = имя
        .or_else(|| {
            файл
                .parent()
                .and_then(|папка| папка.file_name())
                .and_then(|имя| имя.to_str())
                .map(str::to_string)
        })
        .ok_or_else(|| ош("не удалось определить имя навыка"))?;

    // OpenCode допускает в именах больше символов. Приводим к нашему
    // алфавиту, а не отвергаем навык целиком: имя нужно только для
    // вызова и поиска.
    let имя: String = имя
        .to_lowercase()
        .chars()
        .map(|с| {
            if с.is_ascii_lowercase() || с.is_ascii_digit() || с == '-' || с == '_' {
                с
            } else {
                '-'
            }
        })
        .collect();

    let описание = описание
        .or_else(|| {
            тело.lines()
                .map(str::trim)
                .find(|строка| !строка.is_empty() && !строка.starts_with('#'))
                .map(str::to_string)
        })
        .ok_or_else(|| ош("в шапке нет description: по нему модель выбирает навык"))?;

    Ok(Skill {
        name: имя,
        version: версия.unwrap_or_else(|| "1.0.0".to_string()),
        description: описание,
        tools: Vec::new(),
        resources: Vec::new(),
        parameters: схема_аргументов(&[]),
        steps: тело.trim().to_string(),
        steps_path: None,
        file: файл.to_path_buf(),
    })
}

/// Ключи шапки, которые мы понимаем.
///
/// Список закрытый. Неизвестный ключ — ошибка, а не «игнорируем»:
/// опечатка в `desription` сделала бы навык невидимым для модели, и
/// человек искал бы причину не там.
const КЛЮЧИ: &[&str] = &["name", "version", "description", "tools", "resources", "arguments"];

/// Разобрать файл навыка.
pub fn разобрать_файл(файл: &Path) -> Result<Skill, Ошибка> {
    let текст = std::fs::read_to_string(файл)
        .map_err(|e| Ошибка { file: путь(файл), текст: format!("не читается: {e}") })?;
    разобрать(&текст, файл)
}

/// Разобрать текст навыка.
pub fn разобрать(текст: &str, файл: &Path) -> Result<Skill, Ошибка> {
    let ош = |т: &str| Ошибка { file: путь(файл), текст: т.to_string() };

    let без_бома = текст.trim_start_matches('\u{feff}');
    let Остаток = без_бома
        .strip_prefix("---")
        .ok_or_else(|| ош("нет шапки: файл должен начинаться со строки ---"))?;

    let (шапка, тело) = match Остаток.split_once("\n---") {
        Some((ш, т)) => (ш, т.trim_start_matches(['\n', '\r'])),
        None => return Err(ош("шапка не закрыта: нет второй строки ---")),
    };

    let mut имя = None;
    let mut версия = None;
    let mut описание = None;
    let mut инструменты: Vec<String> = Vec::new();
    let mut ресурсы: Vec<String> = Vec::new();
    let mut аргументы: Vec<String> = Vec::new();

    for строка in шапка.lines() {
        let с = строка.trim();
        if с.is_empty() || с.starts_with('#') {
            continue;
        }
        let Some((ключ, значение)) = с.split_once(':') else {
            return Err(ош(&format!("строка шапки без «:» — {с:?}")));
        };
        let ключ = ключ.trim().to_lowercase();
        let значение = значение.trim().trim_matches('"').to_string();
        if !КЛЮЧИ.contains(&ключ.as_str()) {
            return Err(ош(&format!("неизвестный ключ шапки «{ключ}». Понимаются: {}", КЛЮЧИ.join(", "))));
        }
        match ключ.as_str() {
            "name" => имя = Some(значение),
            "version" => версия = Some(значение),
            "description" => описание = Some(значение),
            "tools" => инструменты = список(&значение),
            "resources" => ресурсы = список(&значение),
            "arguments" => аргументы = список(&значение),
            _ => unreachable!("ключ проверен выше"),
        }
    }

    let имя = имя.filter(|с| !с.is_empty()).ok_or_else(|| ош("в шапке нет name"))?;
    let версия = версия.filter(|с| !с.is_empty()).unwrap_or_else(|| "1.0.0".to_string());
    let описание = описание
        .filter(|с| !с.is_empty())
        .ok_or_else(|| ош("в шапке нет description: по нему модель выбирает навык"))?;

    if !имя.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        || !имя.starts_with(|c: char| c.is_ascii_lowercase())
    {
        return Err(ош(&format!(
            "имя «{имя}» должно быть латиницей в нижнем регистре, цифрами, дефисом \
             и подчёркиванием, и начинаться с буквы"
        )));
    }

    if тело.trim().is_empty() {
        return Err(ош("тело навыка пустое: там должны быть шаги"));
    }

    // Имя обязано совпадать с именем файла. Иначе навык вызывается
    // одним именем, а в списке папок видно другим, и поиск человека
    // по файлу перестаёт работать.
    if let Some(имя_файла) = файл.file_stem().and_then(|с| с.to_str()) {
        if имя_файла.to_lowercase() != имя.to_lowercase() {
            return Err(ош(&format!(
                "имя «{имя}» не совпадает с именем файла «{имя_файла}»"
            )));
        }
    }

    Ok(Skill {
        name: имя,
        version: версия,
        description: описание,
        tools: инструменты,
        resources: ресурсы,
        parameters: схема_аргументов(&аргументы),
        steps: тело.trim().to_string(),
        steps_path: None,
        file: файл.to_path_buf(),
    })
}

fn путь(файл: &Path) -> String {
    файл.display().to_string()
}

/// Разобрать список через запятую.
fn список(значение: &str) -> Vec<String> {
    значение
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|с| с.trim().trim_matches('"').to_string())
        .filter(|с| !с.is_empty())
        .collect()
}

/// Схема аргументов из списка.
///
/// Все аргументы — строки. Числа и списки в аргументах навыка не
/// разбираются: их польза не доказана, а лишний разбор в шапке — это
/// лишняя поверхность, где можно ошибиться незаметно.
fn схема_аргументов(аргументы: &[String]) -> Value {
    let свойства: serde_json::Map<String, Value> = аргументы
        .iter()
        .map(|а| {
            (
                а.clone(),
                json!({
                    "type": "string",
                    "description": format!("параметр навыка «{а}»"),
                }),
            )
        })
        .collect();
    json!({
        "type": "object",
        "properties": свойства,
        "required": аргументы,
    })
}

/// Папка навыков по умолчанию.
///
/// Рядом с данными, а не в каталоге пользователя: навыки — часть
/// поставки, и править их должен тот, кто ставил программу.
pub fn папка() -> PathBuf {
    crate::user_data::current_roots()
        .map(|roots| roots.skills)
        .unwrap_or_else(|_| PathBuf::new())
}

fn путь_другого_профиля(path: &Path) -> bool {
    crate::user_data::current_roots()
        .is_ok_and(|roots| crate::user_data::is_other_profile_path(&roots, path))
}

/// Навыки, установленные вместе с программой.
///
/// Ищутся в нескольких местах, а не по одному относительному пути.
/// Один относительный путь работает только при запуске из корня
/// проекта: тесты cargo запускаются из `src-tauri`, а собранная
/// программа — из папки рядом с `.exe`. Навык молча не нашёлся бы
/// и выглядел бы как «навыков нет».
pub fn папка_данных() -> PathBuf {
    let mut кандидаты: Vec<PathBuf> = vec![Path::new("data").join("skills")];

    // Корень проекта: `src-tauri` на уровень выше.
    if let Ok(манифест) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = Path::new(&манифест);
        if let Some(корень) = p.parent() {
            кандидаты.push(корень.join("data").join("skills"));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(папка) = exe.parent() {
            кандидаты.push(папка.join("data").join("skills"));
        }
    }
    кандидаты.push(папка());

    for п in &кандидаты {
        if п.is_dir() {
            return п.clone();
        }
    }
    // Ничего не нашлось: возвращаем первый путь, чтобы ошибка была
    // про ту папку, которую искали, а не про пустоту.
    кандидаты.remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Внешний навык OpenCode: незнакомые ключи шапки не должны его
    /// ломать, а имя приводится к нашему алфавиту.
    #[test]
    fn внешний_навык_переживает_чужие_ключи_шапки() {
        let dir = std::env::temp_dir().join(format!(
            "arkha-open-skill-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("SKILL.md");
        std::fs::write(
            &file,
            "---\nname: \"My Skill\"\ndescription: \"Does things\"\nlicense: MIT\nallowed-tools: bash\nmetadata:\n  owner: someone\n---\n\n# Steps\n\n1. Do it.\n",
        )
        .unwrap();

        let skill = разобрать_внешний(&file).expect("внешний навык должен читаться");
        assert_eq!(skill.name, "my-skill");
        assert_eq!(skill.description, "Does things");
        assert!(skill.steps.contains("Do it"), "{}", skill.steps);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Реальная папка OpenCode: если она есть, навыки должны читаться.
    #[test]
    fn навыки_opencode_читаются_если_папка_есть() {
        let dir = папка_агентов();
        if !dir.is_dir() {
            eprintln!("ПРОПУЩЕНО: нет папки {}", dir.display());
            return;
        }
        let набор = из_агентских(&dir);
        assert!(
            !набор.навыки.is_empty(),
            "папка {} есть, но ни один навык не прочитался; сломано: {}",
            dir.display(),
            набор.сломанные.len()
        );
    }

    fn файл(имя: &str, текст: &str) -> Skill {
        разобрать(текст, Path::new(имя)).expect("навык должен разобраться")
    }

    const ХОРОШИЙ: &str = "\
---
name: pereimenovat
description: Переименовывает файлы по дате из содержимого
tools: files.read, files.write
arguments: папка, шаблон
---
1. Покажи содержимое папки.
2. Прочитай каждый файл.
3. Переименуй по шаблону.
";

    #[test]
    fn разбирается_нормальный_навык() {
        let н = файл("pereimenovat.md", ХОРОШИЙ);
        assert_eq!(н.name, "pereimenovat");
        assert_eq!(н.tools, vec!["files.read", "files.write"]);
        assert!(н.steps.contains("Переименуй"));
        assert_eq!(н.parameters["required"][0], "папка");
    }

    /// Два навыка с одним именем — конфликт, а не замена. Имя из
    /// шапки выбирает модель; молчаливая подмена означала бы, что
    /// вызывается один навык, а выполняется другой.
    #[test]
    fn дубликат_имени_не_подменяет_первый() {
        let mut набор = Набор::пусто();
        набор.добавить(файл(
            "b.md",
            "---\nname: b\ndescription: первый\n---\nшаги\n",
        ));
        набор.добавить(файл(
            "b.md",
            "---\nname: b\ndescription: второй\n---\nшаги\n",
        ));
        assert_eq!(набор.навыки.len(), 1);
        assert_eq!(набор.навыки[0].description, "первый");
        assert_eq!(набор.сломанные.len(), 1);
        assert!(набор.сломанные[0].текст.contains("уже занято"));
    }

    /// Без описания навык невидим для модели: она выбирает по нему.
    /// Молча пропустить такую ошибку нельзя.
    #[test]
    fn без_описания_ошибка() {
        let e = разобрать(
            "---\nname: a\n---\nшаги\n",
            Path::new("a.md"),
        )
        .unwrap_err();
        assert!(e.текст.contains("description"), "{}", e.текст);
    }

    /// Имя в шапке обязано совпадать с именем файла, иначе навык
    /// вызывается одним именем, а ищется по файлу с другим.
    #[test]
    fn имя_не_совпадает_с_файлом_ошибка() {
        let e = разобрать(
            "---\nname: odno\ndescription: что-то\n---\nшаги\n",
            Path::new("drugoe.md"),
        )
        .unwrap_err();
        assert!(e.текст.contains("не совпадает"), "{}", e.текст);
    }

    /// Опечатка в ключе шапки делает навык невидимым. Молчание здесь
    /// хуже ошибки.
    #[test]
    fn неизвестный_ключ_ошибка() {
        let e = разобрать(
            "---\nname: a\ndesription: опечатка\n---\nшаги\n",
            Path::new("a.md"),
        )
        .unwrap_err();
        assert!(e.текст.contains("неизвестный ключ"), "{}", e.текст);
    }

    #[test]
    fn пустое_тело_ошибка() {
        let e = разобрать(
            "---\nname: a\ndescription: что\n---\n\n   \n",
            Path::new("a.md"),
        )
        .unwrap_err();
        assert!(e.текст.contains("тело"), "{}", e.текст);
    }

    #[test]
    fn без_шапки_ошибка() {
        let e = разобрать("просто текст\n", Path::new("a.md")).unwrap_err();
        assert!(e.текст.contains("шапки"), "{}", e.текст);
    }

    #[test]
    fn не_закрытая_шапка_ошибка() {
        let e = разобрать("---\nname: a\n", Path::new("a.md")).unwrap_err();
        assert!(e.текст.contains("не закрыта"), "{}", e.текст);
    }

    #[test]
    fn кириллица_в_имени_ошибка() {
        let e = разобрать(
            "---\nname: переименовать\ndescription: что\n---\nшаги\n",
            Path::new("a.md"),
        )
        .unwrap_err();
        assert!(e.текст.contains("латиницей"), "{}", e.текст);
    }

    /// Порядок чтения не должен зависеть от диска: список уходит в
    /// модель, и кэш префиксов тоже.
    #[test]
    fn набор_читается_в_стабильном_порядке() {
        let dir = std::env::temp_dir().join(format!("arkha-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("папка");
        for n in ["b.md", "a.md", "c.md"] {
            std::fs::write(dir.join(n), "---\nname: x\ndescription: y\n---\nшаги\n")
                .ok();
            // Имя в шапке должно совпадать с именем файла.
            let текст = format!("---\nname: {}\ndescription: что\n---\nшаги\n", &n[..1]);
            std::fs::write(dir.join(n), текст).expect("запись");
        }
        let н1 = Набор::из_папки(&dir);
        let н2 = Набор::из_папки(&dir);
        let имена: Vec<&str> = н1.навыки.iter().map(|н| н.name.as_str()).collect();
        assert_eq!(имена, vec!["a", "b", "c"], "порядок должен быть по именам");
        assert_eq!(н1, н2, "два чтения одной папки должны совпасть");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Навык без нужных прав не должен попадать в список доступных:
    /// иначе модель выберет его и упрётся в стену изнутри.
    #[test]
    fn навык_без_прав_не_доступен() {
        let н = файл("pereimenovat.md", ХОРОШИЙ);
        let набор = Набор { навыки: vec![н], сломанные: vec![] };
        let всё = true;
        assert_eq!(набор.доступные(&|_| всё).len(), 1);
        let только_чтение = |c: &str| c == "files.read";
        assert!(
            набор.доступные(&только_чтение).is_empty(),
            "навык с записью не должен быть доступен при одном чтении"
        );
    }

    /// Текст навыка должен быть явно обрамлён: иначе модель принимает
    /// шаги за описание и не выполняет их.
    ///
    /// Проверяется именно требование действовать, а не прежняя
    /// формулировка: на живой модели текст «Выполняй его шаги» в
    /// начале приводил к пересказу плана вместо выполнения, и
    /// требование переехало в конец. Тест обязан ловить возвращение
    /// этого поведения, а не замену одной формулировки другой.
    #[test]
    fn текст_для_модели_обрамлён() {
        let н = файл("pereimenovat.md", ХОРОШИЙ);
        let т = н.текст_для_модели(&json!({"папка": "C:\\Проверка"}));
        assert!(т.contains("выбран человеком"), "{т}");
        assert!(т.contains("папка: C:\\Проверка"), "{т}");
        assert!(т.contains("Не выдумывай"), "{т}");
        // Требование действовать идёт после шагов, а не до них.
        let требование = т
            .find("Теперь выполняй")
            .unwrap_or_else(|| panic!("в тексте нет требования выполнять шаги: {т}"));
        let конец_шагов = т
            .find("Не выдумывай")
            .unwrap_or_else(|| panic!("в тексте нет запрета выдумывать: {т}"));
        assert!(
            требование < конец_шагов,
            "требование выполнять должно быть после шагов, а не до них: {т}"
        );
    }

    #[test]
    fn без_аргументов_текст_не_ломается() {
        let н = Skill {
            name: "a".into(),
            version: "1.0.0".into(),
            description: "что".into(),
            tools: vec![],
            resources: vec![],
            parameters: json!({}),
            steps: "шаги".into(),
            steps_path: None,
            file: PathBuf::from("a.md"),
        };
        let т = н.текст_для_модели(&json!({}));
        assert!(т.contains("не заданы"), "{т}");
    }
}

fn дополнительные_ресурсы(directory: &Path) -> String {
    let mut paths = Vec::new();
    for folder in ["references", "templates", "examples", "bin"] {
        let path = directory.join(folder);
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                for entry in entries.flatten() {
                    paths.push(entry.path().display().to_string());
                }
            }
        }
    }
    if paths.is_empty() {
        String::new()
    } else {
        format!("\nДополнительные ресурсы, доступные через инструменты:\n- {}\n", paths.join("\n- "))
    }
}

/// Прочитать metadata manifest, оставив тело инструкции ленивым.
pub fn разобрать_manifest(directory: &Path) -> Result<Skill, Ошибка> {
    #[derive(Deserialize)]
    struct Manifest {
        id: Option<String>,
        name: Option<String>,
        version: Option<String>,
        description: String,
        #[serde(default)]
        tools: Vec<String>,
        #[serde(default)]
        resources: Vec<String>,
        #[serde(default)]
        arguments: Vec<String>,
        #[serde(default = "default_entrypoint")]
        entrypoint: String,
    }
    fn default_entrypoint() -> String { "SKILL.md".to_string() }

    let manifest_path = directory.join("skill.json");
    if путь_другого_профиля(&manifest_path) {
        return Err(Ошибка {
            file: путь(&manifest_path),
            текст: "профили других пользователей ArkHa недоступны".into(),
        });
    }
    let text = std::fs::read_to_string(&manifest_path).map_err(|error| Ошибка {
        file: путь(&manifest_path),
        текст: format!("не читается: {error}"),
    })?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(|error| Ошибка {
        file: путь(&manifest_path),
        текст: format!("manifest не разобран: {error}"),
    })?;
    let name = manifest
        .id
        .or(manifest.name)
        .or_else(|| directory.file_name().and_then(|value| value.to_str()).map(str::to_string))
        .ok_or_else(|| Ошибка { file: путь(&manifest_path), текст: "нет id".into() })?;
    let steps_path = directory.join(manifest.entrypoint);
    if !steps_path.is_file() || путь_другого_профиля(&steps_path) {
        return Err(Ошибка { file: путь(&steps_path), текст: "entrypoint не найден".into() });
    }
    Ok(Skill {
        name,
        version: manifest.version.unwrap_or_else(|| "1.0.0".into()),
        description: manifest.description,
        tools: manifest.tools,
        resources: manifest.resources,
        parameters: схема_аргументов(&manifest.arguments),
        steps: String::new(),
        steps_path: Some(steps_path.clone()),
        file: steps_path,
    })
}

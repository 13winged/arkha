// Запуск приложений.
//
// Правило простое: запускается только то, что человек назвал, и
// только то, что нашлось в известных папках. Никакого «ищем в
// интернете», никакого «откроем то, что Model подсказала».
//
// **Запрещённый список проверяется дважды.** До поиска файла — по
// имени, и после, по полному пути. Проверка только по имени обходится
// переименованием файла, а человек рассчитывает, что запрет
// работает.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::{Tool, ToolError, ToolOutput, ToolSpec};
use crate::permissions::Settings;

/// Папки, где ищутся программы.
///
/// Порядок имеет значение: результат зависит от того, какая папка
/// просмотрена первой, и человек должен знать, какая. Первая — та,
/// что ближе к его программам.
pub fn папки_поиска() -> Vec<PathBuf> {
    let mut папки: Vec<PathBuf> = Vec::new();
    let переменная = |имя: &str| std::env::var_os(имя).map(PathBuf::from);

    if let Some(p) = переменная("LOCALAPPDATA") {
        // Обычные места установки без прав администратора.
        папки.push(p.join("Programs"));
        папки.push(p.join("Microsoft/WindowsApps"));
    }
    for имя in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(p) = переменная(имя) {
            папки.push(p);
        }
    }
    if let Some(w) = переменная("WINDIR") {
        папки.push(w.join("System32"));
    }
    папки
}

/// Запрещено ли имя или путь.
///
/// Сравнение нечувствительно к регистру и принимает и имя, и полный
/// путь: человек пишет в запрет то, что видит.
pub fn запрещено(запреты: &[String], имя: &str, полный_путь: Option<&Path>) -> Option<String> {
    let игла = имя.trim().to_lowercase();
    if игла.is_empty() {
        return None;
    }
    let без_расширения = |с: &str| с.trim_end_matches(".exe").to_string();

    for з in запреты {
        let з = з.trim().to_lowercase();
        if з.is_empty() {
            continue;
        }
        let з_без = без_расширения(&з);
        // Имя сравнивается и само, и без `.exe`: человек пишет в
        // запрет то, что видит в меню, а не то, что лежит на диске.
        if з_без == без_расширения(&игла) {
            return Some(з.clone());
        }
        // Путь ловится двумя способами: по имени файла и суффиксом.
        // Суффикс нужен, когда в запрете записан полный путь, а
        // программа лежит в подпапке. Сравнение с `.exe` на конце
        // не сработало бы никогда: путь кончается расширением.
        if let Some(путь) = полный_путь {
            let путь = путь.to_string_lossy().to_lowercase();
            let имя_файла = Path::new(&путь)
                .file_name()
                .map(|с| с.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if без_расширения(&имя_файла) == з_без
                || (!з.is_empty() && путь.ends_with(&з))
                || (!з_без.is_empty() && путь.ends_with(&з_без))
            {
                return Some(з.clone());
            }
        }
    }
    None
}

/// Найти программу в реестре Windows.
/// `App Paths` — штатное место, куда установщики прописывают полный
/// путь: Chrome, Edge, Office и большинство обычных программ лежат не
/// в корне `Program Files`, а в подпапках, и простой перебор папок их
/// не находит. Ключ читается через `reg.exe`: это дешевле и проще,
/// чем тянуть в приложение весь COM-слой ради одного значения.
#[cfg(windows)]
fn из_app_paths(имя_файла: &str) -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let ключи = [
        format!("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{имя_файла}"),
        format!("HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{имя_файла}"),
        format!("HKLM\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{имя_файла}"),
    ];

    for ключ in ключи {
        let вывод = match std::process::Command::new("reg")
            .args(["query", &ключ, "/ve"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(вывод) if вывод.status.success() => вывод,
            _ => continue,
        };
        let текст = String::from_utf8_lossy(&вывод.stdout);
        for строка in текст.lines() {
            let Some(позиция) = строка.find("REG_SZ") else {
                continue;
            };
            let значение = строка[позиция + "REG_SZ".len()..].trim().trim_matches('"');
            let путь = PathBuf::from(значение);
            if путь.is_file() {
                return Some(путь);
            }
        }
    }
    None
}

/// Привести имя к сравнимому виду: без `.exe`, без регистра и без
/// пробелов/дефисов. «Google Chrome», «google-chrome.exe» и «chrome»
/// должны находить одну и ту же программу.
fn нормализовать(значение: &str) -> String {
    let base = значение.trim().trim_matches('"');
    let base = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base);
    base.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect()
}

fn стартовые_меню() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(
            PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    if let Some(program_data) = std::env::var_os("ProgramData") {
        dirs.push(
            PathBuf::from(program_data)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    dirs
}

fn собрать_ярлыки(dir: &Path, глубина: usize, out: &mut Vec<(String, PathBuf)>) {
    if глубина > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            собрать_ярлыки(&path, глубина + 1, out);
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                out.push((нормализовать(stem), path));
            }
        }
    }
}

/// Ярлыки меню «Пуск», прочитанные один раз за процесс.
fn ярлыки() -> &'static Vec<(String, PathBuf)> {
    static ЯРЛЫКИ: OnceLock<Vec<(String, PathBuf)>> = OnceLock::new();
    ЯРЛЫКИ.get_or_init(|| {
        let mut all = Vec::new();
        for dir in стартовые_меню() {
            собрать_ярлыки(&dir, 0, &mut all);
        }
        all.sort_by(|left, right| left.1.cmp(&right.1));
        all
    })
}

fn из_меню_пуск(имя: &str) -> Option<PathBuf> {
    let key = нормализовать(имя);
    if key.is_empty() {
        return None;
    }
    let mut candidates: Vec<(u8, &PathBuf)> = Vec::new();
    for (name, path) in ярлыки() {
        let score = if name == &key {
            0
        } else if name.ends_with(&key) {
            1
        } else if name.contains(&key) {
            2
        } else {
            continue;
        };
        candidates.push((score, path));
    }
    candidates.sort_by_key(|(score, path)| (*score, path.as_os_str().len()));
    candidates.first().map(|(_, path)| (*path).clone())
}

fn обойти(dir: &Path, глубина: usize, index: &mut HashMap<String, Vec<PathBuf>>) {
    if глубина > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if matches!(
                name.as_str(),
                "windowsapps" | "installer" | "temp" | "windows defender" | "$recycle.bin"
            ) {
                continue;
            }
            обойти(&path, глубина + 1, index);
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default();
            let lower = stem.to_lowercase();
            if lower.contains("unins")
                || lower.contains("setup")
                || lower.contains("update")
                || lower.contains("crash")
                || lower.contains("vc_redist")
            {
                continue;
            }
            let key = нормализовать(stem);
            if !key.is_empty() {
                index.entry(key).or_default().push(path);
            }
        }
    }
}

/// Все .exe из папок установки, собранные один раз за процесс.
fn индекс_приложений() -> &'static HashMap<String, Vec<PathBuf>> {
    static ИНДЕКС: OnceLock<HashMap<String, Vec<PathBuf>>> = OnceLock::new();
    ИНДЕКС.get_or_init(|| {
        let mut index = HashMap::new();
        for root in папки_поиска() {
            обойти(&root, 0, &mut index);
        }
        index
    })
}

fn из_индекса(имя: &str) -> Option<PathBuf> {
    let key = нормализовать(имя);
    if key.is_empty() {
        return None;
    }
    let mut candidates: Vec<(u8, &PathBuf)> = Vec::new();
    for (name, paths) in индекс_приложений() {
        let score = if name == &key {
            0
        } else if name.ends_with(&key) {
            1
        } else if name.contains(&key) {
            2
        } else {
            continue;
        };
        for path in paths {
            candidates.push((score, path));
        }
    }
    candidates.sort_by_key(|(score, path)| (*score, path.as_os_str().len()));
    candidates.first().map(|(_, path)| (*path).clone())
}

/// Найти исполняемый файл по имени.
///
/// Только имя плюс `.exe`. Никакого разбора строк с пробелами в
/// аргументы: «notepad» — это программа, а «notepad C:\файл» — уже
/// не запуск программы, а две разные вещи, и смешивать их нельзя.
pub fn найти(имя: &str) -> Result<PathBuf, ToolError> {
    let имя = имя.trim().trim_matches('"');
    if имя.is_empty() {
        return Err(ToolError::BadArgs("не указано имя программы".into()));
    }

    // Явный путь — как есть, но только если это файл.
    let как_путь = Path::new(имя);
    if как_путь.is_absolute() {
        if как_путь.is_file() {
            return Ok(как_путь.to_path_buf());
        }
        return Err(ToolError::Missing(format!("файла нет: {имя}")));
    }

    if имя.contains(['/', '\\']) {
        return Err(ToolError::BadArgs(
            "указан относительный путь. Либо имя программы, либо полный путь к файлу"
                .into(),
        ));
    }

    let имя_файла = if имя.to_lowercase().ends_with(".exe") {
        имя.to_string()
    } else {
        format!("{имя}.exe")
    };

    for папка in папки_поиска() {
        let кандидат = папка.join(&имя_файла);
        if кандидат.is_file() {
            return Ok(кандидат);
        }
    }

    #[cfg(windows)]
    if let Some(путь) = из_app_paths(&имя_файла) {
        return Ok(путь);
    }

    // Ярлык из меню «Пуск» знает, где лежит программа, даже если
    // установщик спрятал её в три подпапки.
    if let Some(путь) = из_меню_пуск(имя) {
        return Ok(путь);
    }

    // Последний шанс: обход папок установки. Дорого, поэтому только
    // после всего остального и с кэшем на процесс.
    if let Some(путь) = из_индекса(имя) {
        return Ok(путь);
    }

    Err(ToolError::Missing(format!(
        "программа «{имя}» не найдена. Искали в: {}, в App Paths реестра, \
         в меню «Пуск» и в папках установки",
        папки_поиска()
            .iter()
            .map(|п| п.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    )))
}

/// Запуск приложения по имени.
pub struct Запустить;

impl Tool for Запустить {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "apps.launch".into(),
            capability: "apps.launch".into(),
            name: "Запустить программу".into(),
            description: "Запустить программу по имени или полному пути к .exe. Ищет \
                          в папках установки программ. Если программа запрещена в \
                          настройках — не запускает. Спроси подтверждение, прежде чем \
                          запускать то, что открывает окно и занимает экран."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "имя программы: notepad, или полный путь к .exe" }
                },
                "required": ["name"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, settings: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(имя) = args.get("name").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля name".into()));
        };

        // Первая проверка — по имени, до поиска файла. Вторая — по
        // найденному пути. Обе нужны: имя можно подделать переименованием.
        if let Some(запрет) = запрещено(&settings.deny_apps, имя, None) {
            return Err(ToolError::Denied(format!(
                "программа «{имя}» в запрещённом списке ({запрет})"
            )));
        }

        let путь = найти(имя)?;
        if let Some(запрет) = запрещено(&settings.deny_apps, имя, Some(&путь)) {
            return Err(ToolError::Denied(format!(
                "программа «{}» в запрещённом списке ({запрет})",
                путь.display()
            )));
        }

        // Ярлык запускается через `start`: двойной щелчок по нему в
        // Проводнике делает ровно это. Прямой запуск .lnk не работает.
        let ярлык = путь
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));

        let запуск = if ярлык {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "start", "", &путь.to_string_lossy()]);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                command.creation_flags(CREATE_NO_WINDOW);
            }
            command.spawn()
        } else {
            std::process::Command::new(&путь).spawn()
        };

        match запуск {
            Ok(_) => Ok(ToolOutput::changed(
                format!("запущено: {}", путь.display()),
                format!("запущено: {}", путь.display()),
            )),
            Err(e) => Err(ToolError::Refused(format!(
                "не удалось запустить «{}»: {e}",
                путь.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «Google Chrome», «google-chrome.exe» и «chrome» должны
    /// приводиться к сравнимому виду.
    #[test]
    fn имена_сравниваются_без_регистра_пробелов_и_расширения() {
        assert_eq!(нормализовать("Google Chrome.exe"), "googlechrome");
        assert_eq!(нормализовать("google-chrome.EXE"), "googlechrome");
        assert_eq!(нормализовать(" chrome "), "chrome");
    }

    /// Запрет по имени и по пути — обе проверки нужны.
    ///
    /// Здесь запрет взят из настоящих настроек по умолчанию, а не
    /// придуман для теста. Проверка идёт именно по имени файла, и
    /// пока в запрете стоял идентификатор Tauri вместо `arkha.exe`,
    /// такой тест был зелёным, а защита не работала.
    #[test]
    fn запрет_ловится_по_имени_и_пути() {
        let запреты = crate::permissions::Settings::default().deny_apps;
        assert!(запрещено(&запреты, "arkha.exe", None).is_some());
        assert!(запрещено(&запреты, "ARKHA.EXE", None).is_some());
        assert!(
            запрещено(&запреты, "arkha", Some(Path::new("C:/Program Files/ArkHa/arkha.exe")))
                .is_some(),
            "секретарь не должен проходить по своему пути"
        );
        assert!(запрещено(&запреты, "notepad", None).is_none());
    }

    /// Пустой запрет не должен срабатывать на всё подряд.
    #[test]
    fn пустой_запрет_не_мешает() {
        let запреты = vec!["".to_string(), "   ".to_string()];
        assert!(запрещено(&запреты, "notepad", None).is_none());
        assert!(запрещено(&запреты, "", None).is_none());
    }

    /// Существующую программу найти обязаны: без этого весь S8 не
    /// работает ни на одной машине.
    #[test]
    fn системная_программа_находится() {
        let путь = найти("notepad").expect("блокнот есть в Windows");
        assert!(путь.is_file(), "{:?} не файл", путь);
        assert!(путь.to_string_lossy().to_lowercase().ends_with(".exe"));
    }

    /// Несуществующее имя обязано называть, где искали: иначе
    /// человек не поймёт, что программа не установлена или называется
    /// иначе.
    #[test]
    fn несуществующая_программа_объясняет() {
        let e = найти("такойпрограммынет").unwrap_err();
        let текст = e.to_string();
        assert!(текст.contains("не найдена"), "{текст}");
        assert!(текст.contains("Program Files") || текст.contains("WindowsApps"), "{текст}");
    }

    /// Путь вместо имени — тоже понятная ошибка, а не запуск
    /// чего-то случайного из текущей папки.
    #[test]
    fn относительный_путь_отклоняется() {
        let e = найти("Program Files/что-то").unwrap_err();
        assert!(e.to_string().contains("относительный"), "{e}");
    }

    #[test]
    fn пустое_имя_отклоняется() {
        assert!(найти("").is_err());
        assert!(найти("  ").is_err());
    }
}

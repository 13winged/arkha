// Файловые инструменты S6: прочитать и посмотреть папку.
//
// Оба инструмента только читают. Запись в S6 не делаем: она меняет
// систему, а проверить её на своей машине безопаснее, когда читающий
// инструмент уже работает.
//
// **Проверка пути решает всё.** Модель не доверенная сторона: она
// может назвать `C:\Windows\System32\config\SAM`, и это не должно
// работать просто потому, что файл существует. Путь нормализуется,
// симлинки разрешаются, и только потом сравнивается с разрешёнными
// папками. Сравнение идёт с разделителем, иначе
// `C:\Users\me\Документы` прошёл бы проверку по префиксу
// `C:\Users\me\Доку`.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::{ensure_allowed, Tool, ToolError, ToolOutput, ToolResult, ToolSpec};
use crate::permissions::Settings;

/// Сколько байт отдаём модели за один раз.
///
/// Число выбрано, а не измерено: 64 КиБ — это примерно двадцать
/// страниц текста. Модель всё равно не держит в голове больше
/// контекста, а лишние мегабайты съедают окно и замедляют ответ. Если
/// файл длиннее — отдаём начало и говорим, что файл обрезан.
const MAX_BYTES: usize = 64 * 1024;

/// Сколько имён показываем в папке.
const MAX_ENTRIES: usize = 200;

/// Проверка и нормализация пути.
///
/// Возвращает путь, который гарантированно лежит внутри одной из
/// разрешённых папок.
fn resolve_inside(raw: &str, settings: &Settings) -> Result<PathBuf, ToolError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ToolError::BadArgs("пустой путь".into()));
    }

    // Путь от модели может быть относительным. Относительный путь
    // без указания, относительно чего, — это неопределённость, а
    // модель зачастую присылает именно его.
    let given = Path::new(raw);
    if given.is_relative() {
        return Err(ToolError::BadArgs(format!(
            "путь «{raw}» относительный. Нужен полный, например C:\\Users\\me\\Документы\\файл.txt"
        )));
    }

    let roots = &settings.paths;
    if roots.is_empty() {
        return Err(ToolError::Denied(
            "не выбрано ни одной папки. Отметь папки в знакомстве".into(),
        ));
    }

    // Нормализация без обращения к диску: убираем `.` и `..`,
    // приводим разделители. Это нужно ДО сравнения, иначе
    // `Документы\..\Документы` прошёл бы как разрешённый путь.
    let normalized = normalize(given);
    let normal_str = normalized.to_string_lossy().to_string();

    // Сначала текстовое сравнение: это ловит `..` без обращения к
    // диску и работает даже для несуществующего файла.
    for root in roots {
        let r = normalize(root);
        let rs = r.to_string_lossy().to_string();
        if within(&normal_str, &rs) {
            return Ok(normalized);
        }
    }

    // Потом настоящий путь: симлинк может увести файл в сторону
    // уже после того, как текст совпал. Симлинк разрешается, если
    // файл есть; если нет — остаётся первый ответ, отказ по тексту.
    if let Ok(real) = normalized.canonicalize() {
        let real_str = real.to_string_lossy().to_string();
        for root in roots {
            let r = normalize(root);
            if let Ok(rr) = r.canonicalize() {
                if within(&real_str, &rr.to_string_lossy()) {
                    return Ok(normalized);
                }
            }
        }
    }

    Err(ToolError::Outside(format!(
        "{raw}. Разрешены папки: {}",
        roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// Убрать `.` и `..`, не обращаясь к диску.
///
/// `Path::components` разбирает на части, но `..` оставляет как есть,
/// и `push` его тоже не разрешает: результат будет
/// `C:\...\подпапка\..\секрет.txt`. Для текстового сравнения это
/// выглядит как файл ВНУТРИ разрешённой папки — то есть проверка
/// проходит, а файл снаружи. Поэтому `..` разрешается здесь вручную,
/// приёмом «снять последнюю часть».
///
/// Настоящий файловой системы эта функция не касается, поэтому
/// диск и сеть не читаются: путь можно нормализовать даже для файла,
/// которого ещё нет.
fn normalize(p: &Path) -> PathBuf {
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    for c in p.components() {
        use std::path::Component as C;
        match c {
            C::CurDir => {}
            C::ParentDir => {
                // Подняться выше корня нельзя. `C:\..\..` — это всё
                // равно `C:\`. Молча игнорируем, как это делает
                // Windows, иначе путь станет относительным и проверка
                // ослабнет.
                out.pop();
            }
            C::Normal(s) => out.push(s.to_os_string()),
            // Префикс диска и корень обязаны остаться: без них путь
            // снова станет относительным.
            other => out.push(other.as_os_str().to_os_string()),
        }
    }
    let mut res = PathBuf::new();
    for s in out {
        res.push(s);
    }
    res
}

/// Лежит ли `path` внутри `root`.
///
/// Сравнение с разделителем обязательно. Без него папка
/// `C:\Users\me\Документы` считала бы своим подкаталогом
/// `C:\Users\me\ДокументыСекретно` — а это другой каталог с другим
/// содержимым.
fn within(path: &str, root: &str) -> bool {
    let path = path.trim_end_matches(['\\', '/']);
    let root = root.trim_end_matches(['\\', '/']);
    if path.eq_ignore_ascii_case(root) {
        return true;
    }
    if path.len() <= root.len() {
        return false;
    }
    let rest = &path[root.len()..];
    rest.starts_with('\\') || rest.starts_with('/')
}

/// Из аргументов — строка. Без значения по умолчанию: если модель не
/// прислала путь, повторять тот же вызов она будет долго.
fn arg_str(args: &Value, key: &str) -> Result<String, ToolError> {
    match args.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        Some(Value::String(_)) => Err(ToolError::BadArgs(format!("{key} пустой"))),
        Some(_) => Err(ToolError::BadArgs(format!("{key} должен быть строкой"))),
        None => Err(ToolError::BadArgs(format!("нет обязательного поля {key}"))),
    }
}

/// Прочитать текстовый файл.
pub struct ReadFile;

impl Tool for ReadFile {
    fn spec(&self, settings: &Settings) -> ToolSpec {
        ToolSpec {
            id: "fs.read".into(),
            capability: "files.read".into(),
            name: "Прочитать файл".into(),
            // Список папок — часть описания, а не системной подсказки.
            // Модель не может угадать, что ей разрешили: на живом гейте
            // S6 она угадывала `C:\Users\me\Документы` и получала отказ
            // двенадцать раз подряд.
            description: format!(
                "Прочитать текстовый файл и получить его содержимое. \
                 Путь должен быть полным, начинаться с одной из папок ниже \
                 и указывать на существующий файл. \
                 Для больших файлов возвращает начало. \
                 Ищи файлы через fs.list, а не по названию. {}",
                super::папки_словами(settings)
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Полный путь к файлу, например C:\\Users\\me\\Документы\\заметка.txt"
                    }
                },
                "required": ["path"]
            }),
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, settings: &Settings) -> ToolResult {
        ensure_allowed(settings, "files.read")?;

        let raw = arg_str(args, "path")?;
        let path = resolve_inside(&raw, settings)?;

        // Папка вместо файла — это не ошибка чтения, а неверное
        // обращение. Своим кодом, чтобы модель поняла разницу.
        if path.is_dir() {
            return Err(ToolError::BadArgs(format!(
                "{raw} это папка, а не файл. Посмотреть содержимое папки: fs.list"
            )));
        }
        if !path.exists() {
            return Err(ToolError::Missing(raw));
        }

        let bytes = std::fs::read(&path).map_err(|e| {
            ToolError::Unreadable(format!("{raw}: {e}. Если файл открыт в другой программе, закрой его"))
        })?;

        let truncated = bytes.len() > MAX_BYTES;
        let slice = if truncated { &bytes[..MAX_BYTES] } else { &bytes[..] };

        let text = match std::str::from_utf8(slice) {
            Ok(t) => t.to_string(),
            Err(_) => {
                // Двоичный файл нельзя превратить в текст честно:
                // попытка с lossiness даст модели мусор, на который она
                // ответит содержательно. Лучше сказать, что это не
                // текст, и не тратить её вывод.
                return Err(ToolError::Unreadable(format!(
                    "{raw} не похож на текст: это, скорее всего, двоичный файл"
                )));
            }
        };

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| raw.clone());

        let summary = if truncated {
            format!(
                "прочитан {}: начало {MAX_BYTES} байт из {}",
                name,
                bytes.len()
            )
        } else {
            format!("прочитан {}: {} байт", name, bytes.len())
        };

        let text = if truncated {
            format!(
                "{text}\n\n[файл обрезан: показаны первые {MAX_BYTES} байт из {}]",
                bytes.len()
            )
        } else {
            text
        };

        Ok(ToolOutput::read(text, summary))
    }
}

/// Посмотреть содержимое папки.
pub struct ListDir;

impl Tool for ListDir {
    fn spec(&self, settings: &Settings) -> ToolSpec {
        ToolSpec {
            id: "fs.list".into(),
            capability: "files.read".into(),
            name: "Показать папку".into(),
            description: format!(
                "Показать, что лежит в папке: имена и тип. \
                 Путь должен быть полным и вести в одну из папок ниже. \
                 Не открывает файлы и не читает их содержимое — \
                 чтобы прочитать, вызови fs.read. {}",
                super::папки_словами(settings)
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Полный путь к папке"
                    }
                },
                "required": ["path"]
            }),
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, settings: &Settings) -> ToolResult {
        ensure_allowed(settings, "files.read")?;

        let raw = arg_str(args, "path")?;
        let path = resolve_inside(&raw, settings)?;

        if !path.is_dir() {
            return Err(ToolError::BadArgs(format!(
                "{raw} не папка. Прочитать файл: fs.read"
            )));
        }

        let mut items: Vec<String> = Vec::new();
        let mut dirs = 0usize;
        let mut files = 0usize;
        for entry in std::fs::read_dir(&path).map_err(|e| ToolError::Unreadable(format!("{raw}: {e}")))? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                dirs += 1;
                items.push(format!("{name}/"));
            } else {
                files += 1;
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                items.push(format!("{name} ({size} байт)"));
            }
        }
        items.sort();
        let shown = items.len().min(MAX_ENTRIES);
        items.truncate(shown);

        let mut text = items.join("\n");
        if items.is_empty() {
            text = "папка пустая".to_string();
        }
        let more = files + dirs - shown;
        if more > 0 {
            text.push_str(&format!("\n[показаны первые {shown}, всего {more} ещё]"));
        } else {
            text.push_str(&format!("\n[всего: папок {dirs}, файлов {files}]"));
        }

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| raw.clone());

        Ok(ToolOutput::read(text, format!("показана папка {name}: {dirs} папок, {files} файлов")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{CapabilityRule, Executor};
    use crate::tools::tempdir::TempRoot;

    /// Настройки с одной разрешённой папкой во временной директории.
    /// Тест не должен зависеть от того, что лежит на диске у того,
    /// кто его запускает.
    fn окружение() -> (TempRoot, Settings) {
        let root = TempRoot::new();
        let mut s = Settings::default();
        s.paths.push(root.path().to_path_buf());
        (root, s)
    }

    fn пишет(root: &TempRoot, имя: &str, содержимое: &[u8]) -> PathBuf {
        root.write(имя, содержимое)
    }

    #[test]
    fn читает_файл_из_разрешённой_папки() {
        let (root, s) = окружение();
        let p = пишет(&root, "заметка.txt", "Встреча в 15:00".as_bytes());
        let out = ReadFile.call(&json!({"path": p.to_string_lossy()}), &s).expect("прочитать");
        assert!(out.text.contains("Встреча в 15:00"), "{}", out.text);
        assert!(!out.changed, "чтение ничего не меняет");
    }

    /// Главный отказ: файл существует, но папка не разрешена.
    #[test]
    fn файл_вне_разрешённых_папок_не_читается() {
        let (root, s) = окружение();
        let чужой = пишет(&root, "секрет.txt", "пароль".as_bytes());
        // Разрешена другая папка.
        let mut s2 = s.clone();
        s2.paths = vec![root.path().join("пусто")];
        let e = ReadFile.call(&json!({"path": чужой.to_string_lossy()}), &s2).unwrap_err();
        assert!(matches!(e, ToolError::Outside(_)), "{e:?}");
    }

    /// `..` не должен обходить проверку. Текстовое сравнение
    /// нормализует путь ДО сравнения, поэтому подъём выше папки
    /// виден.
    #[test]
    fn выход_через_точки_не_проходит() {
        let (root, s) = окружение();
        let secret = пишет(&root, "секрет.txt", "пароль".as_bytes());
        let внутри = root.dir("подпапка");
        // Из разрешённой вложенной папки шаг вверх и наружу.
        let путь = format!("{}\\..\\секрет.txt", внутри.display());
        // Разрешена вложенная папка, а не корень.
        let mut s2 = s.clone();
        s2.paths = vec![внутри.clone()];
        let e = ReadFile.call(&json!({"path": путь}), &s2).unwrap_err();
        assert!(matches!(e, ToolError::Outside(_)), "{e:?}");
        assert!(secret.exists());
    }

    /// Похожие имена не должны проходить друг за друга.
    #[test]
    fn похожая_папка_не_считается_вложенной() {
        assert!(within(r"C:\me\Документы", r"C:\me\Документы"));
        assert!(within(r"C:\me\Документы\a.txt", r"C:\me\Документы"));
        assert!(!within(r"C:\me\ДокументыСекретно\a.txt", r"C:\me\Документы"));
        assert!(!within(r"C:\me\Доку", r"C:\me\Документы"));
        assert!(within(r"c:\me\Документы", r"C:\ME\Документы"), "регистр не должен мешать");
    }

    #[test]
    fn без_выбранных_папок_читать_нельзя() {
        let s = Settings::default();
        assert!(s.paths.is_empty());
        let e = ReadFile.call(&json!({"path": r"C:\любой\файл.txt"}), &s).unwrap_err();
        assert!(matches!(e, ToolError::Denied(_)), "{e:?}");
    }

    #[test]
    fn относительный_путь_отклоняется() {
        let (root, s) = окружение();
        пишет(&root, "файл.txt", b"x");
        let e = ReadFile.call(&json!({"path": "файл.txt"}), &s).unwrap_err();
        match e {
            ToolError::BadArgs(m) => assert!(m.contains("полный"), "{m}"),
            other => panic!("ожидался BadArgs, получено {other:?}"),
        }
    }

    #[test]
    fn пустой_путь_отклоняется() {
        let (_root, s) = окружение();
        let e = ReadFile.call(&json!({"path": "   "}), &s).unwrap_err();
        assert!(matches!(e, ToolError::BadArgs(_)), "{e:?}");
    }

    /// Нет аргумента — отказ, а не значение по умолчанию.
    #[test]
    fn без_аргумента_отказ() {
        let (_root, s) = окружение();
        assert!(matches!(ReadFile.call(&json!({}), &s).unwrap_err(), ToolError::BadArgs(_)));
    }

    #[test]
    fn несуществующий_файл_отличается_от_запрещённого() {
        let (root, s) = окружение();
        let p = root.path().join("нет-такого.txt");
        let e = ReadFile.call(&json!({"path": p.to_string_lossy()}), &s).unwrap_err();
        assert!(matches!(e, ToolError::Missing(_)), "{e:?}");
    }

    /// Двоичный файл не должен превращаться в текст из мусора.
    #[test]
    fn двоичный_файл_не_читается() {
        let (root, s) = окружение();
        let p = пишет(&root, "картинка.png", &[0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF, 0xFE, 0x00]);
        let e = ReadFile.call(&json!({"path": p.to_string_lossy()}), &s).unwrap_err();
        assert!(matches!(e, ToolError::Unreadable(_)), "{e:?}");
    }

    /// Папка для fs.read — это разные инструменты, а не одна ошибка.
    #[test]
    fn папка_для_чтения_отделена_от_файла() {
        let (root, s) = окружение();
        std::fs::create_dir_all(root.path().join("папка")).expect("папка");
        let e = ReadFile
            .call(&json!({"path": root.path().join("папка").to_string_lossy()}), &s)
            .unwrap_err();
        match e {
            ToolError::BadArgs(m) => assert!(m.contains("fs.list"), "{m}"),
            other => panic!("ожидался BadArgs с указанием на fs.list, получено {other:?}"),
        }
    }

    #[test]
    fn показывает_содержимое_папки() {
        let (root, s) = окружение();
        пишет(&root, "а.txt", b"12345");
        пишет(&root, "б.txt", b"123");
        std::fs::create_dir_all(root.path().join("внутри")).expect("папка");
        let out = ListDir
            .call(&json!({"path": root.path().to_string_lossy()}), &s)
            .expect("показать");
        assert!(out.text.contains("а.txt"), "{}", out.text);
        assert!(out.text.contains("внутри/"), "{}", out.text);
        assert!(!out.changed);
    }

    #[test]
    fn файл_для_просмотра_папки_отдельно() {
        let (root, s) = окружение();
        let p = пишет(&root, "а.txt", b"1");
        let e = ListDir.call(&json!({"path": p.to_string_lossy()}), &s).unwrap_err();
        assert!(matches!(e, ToolError::BadArgs(_)), "{e:?}");
    }

    /// Выключенное разрешение должно закрывать оба инструмента, а не
    /// один: возможность у них одна.
    #[test]
    fn выключенное_чтение_закрывает_оба_инструмента() {
        let (root, mut s) = окружение();
        let p = пишет(&root, "а.txt", b"1");
        s.capabilities
            .insert("files.read".into(), CapabilityRule { enabled: false, mode: None });
        assert!(matches!(ReadFile.call(&json!({"path": p.to_string_lossy()}), &s).unwrap_err(), ToolError::Denied(_)));
        assert!(matches!(
            ListDir.call(&json!({"path": root.path().to_string_lossy()}), &s).unwrap_err(),
            ToolError::Denied(_)
        ));
    }

    /// В режиме выполнения чтение всё равно разрешено: оно ничего не
    /// меняет.
    #[test]
    fn чтение_разрешено_в_режиме_выполнения() {
        let (root, mut s) = окружение();
        s.executor = Executor::Live;
        let p = пишет(&root, "а.txt", "данные".as_bytes());
        let out = ReadFile.call(&json!({"path": p.to_string_lossy()}), &s).expect("прочитать");
        assert!(out.text.contains("данные"));
    }
}
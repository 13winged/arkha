// Управление браузером.
//
// Chrome, Edge и Firefox рисуют интерфейс сами, поэтому обычный поиск
// дочерних окон у них пуст. Работаем через дерево доступности:
// `browser.snapshot` перечисляет элементы страницы и выдаёт ref,
// `browser.click` нажимает по ref или по видимому тексту,
// `browser.type` вводит текст в поле.
//
// Почему не координаты. Модель не должна придумывать, куда кликнуть.
// Координата берётся из прямоугольника, который вернула Windows для
// найденного элемента. Если у элемента нет `InvokePattern`, это
// единственный честный запасной путь: настоящий клик мышью по центру
// того самого элемента.
//
// Чего модуль не делает: не читает пароли, не подставляет данные в
// формы сам и не работает со страницей, которую не видит. Всё, что
// происходит, — это тот же клик и тот же ввод, что сделал бы человек.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};
use ::windows::Win32::Foundation::{HWND, RECT};
use ::windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

use crate::permissions::Settings;
use super::{apps, control, uia, windows};
use super::{Tool, ToolError, ToolOutput, ToolSpec};

/// По каким словам узнать окно браузера.
const ПРИЗНАКИ: &[&str] = &[
    "google chrome",
    "microsoft edge",
    "mozilla firefox",
    "chromium",
    "opera",
    "brave",
];

/// Сколько элементов страницы показывать за один снимок.
const ПОКАЗЫВАТЬ: usize = 120;

fn это_браузер(заголовок: &str) -> bool {
    let нижний = заголовок.to_lowercase();
    ПРИЗНАКИ.iter().any(|признак| нижний.contains(признак))
}

fn подсказка(args: &Value) -> Option<&str> {
    args.get("title")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Найти окно браузера.
///
/// Если окон несколько, берём активное: человек обычно говорит о том,
/// которое видит. Если активное не браузер — просим уточнить, а не
/// угадываем.
fn окно(args: &Value) -> Result<windows::Окно, ToolError> {
    let все = windows::видимые();
    let браузерные: Vec<windows::Окно> = все
        .into_iter()
        .filter(|окно| это_браузер(&окно.заголовок))
        .collect();

    if браузерные.is_empty() {
        return Err(ToolError::Missing(
            "не вижу окна браузера. Открой Chrome, Edge или Firefox и повтори. \
             Если окно называется необычно, передай часть заголовка в title"
                .into(),
        ));
    }

    let подходящие: Vec<windows::Окно> = match подсказка(args) {
        Some(часть) => {
            let игла = часть.to_lowercase();
            браузерные
                .into_iter()
                .filter(|окно| окно.заголовок.to_lowercase().contains(&игла))
                .collect()
        }
        None => браузерные,
    };

    match подходящие.len() {
        1 => Ok(подходящие.into_iter().next().expect("длина 1")),
        0 => {
            let список: Vec<String> = windows::видимые()
                .into_iter()
                .filter(|окно| это_браузер(&окно.заголовок))
                .map(|окно| format!("«{}»", окно.заголовок))
                .collect();
            Err(ToolError::Unknown(format!(
                "окна браузера с «{}» в заголовке нет. Открыты: {}",
                подсказка(args).unwrap_or(""),
                if список.is_empty() {
                    "ни одного".to_string()
                } else {
                    список.join("; ")
                }
            )))
        }
        _ => {
            if подсказка(args).is_none()
                && let Some(активное) = windows::активное()
                && let Some(окно) = подходящие.iter().find(|окно| окно.hwnd == активное.hwnd)
            {
                return Ok(окно.clone());
            }
            let список: Vec<String> = подходящие
                .iter()
                .map(|окно| format!("«{}»", окно.заголовок))
                .collect();
            Err(ToolError::Ambiguous(format!(
                "подходит {} окон браузера: {}. Уточни часть заголовка в title",
                подходящие.len(),
                список.join("; ")
            )))
        }
    }
}

fn дескриптор(окно: &windows::Окно) -> HWND {
    HWND(окно.hwnd as *mut _)
}

/// Дерево доступности с коротким ожиданием.
///
/// Chrome включает доступность не мгновенно: первый запрос после
/// открытия страницы может вернуть пустое дерево, а второй — уже
/// заполненное. Без ожидания агент получал бы «элементов нет» на
/// нормальной странице.
fn дерево(hwnd: HWND) -> uia::Найденные {
    let mut найденные = uia::найти(hwnd, "");
    for _ in 0..80 {
        if uia::есть_документ(hwnd) && !найденные.элементы.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
        найденные = uia::найти(hwnd, "");
    }
    найденные
}

/// Выбранный элемент вместе с COM-объектом, который держит дерево.
///
/// `Найденные` обязано жить до конца действия: если отпустить его
/// раньше, COM снимется, а элемент останется — и обращение к нему
/// уронит процесс.
struct Выбор {
    найденные: uia::Найденные,
    индекс: usize,
}

impl Выбор {
    fn элемент(&self) -> &uia::Элемент {
        &self.найденные.элементы[self.индекс]
    }
}

fn выбрать(
    hwnd: HWND,
    текст: Option<&str>,
    ссылка: Option<&str>,
    только_поле: bool,
) -> Result<Выбор, ToolError> {
    if let Some(ссылка) = ссылка {
        let номер: usize = ссылка
            .trim()
            .trim_start_matches('@')
            .trim_start_matches('e')
            .parse()
            .map_err(|_| ToolError::BadArgs(format!("неверный ref «{ссылка}», нужно @e3")))?;
        let найденные = дерево(hwnd);
        if номер == 0 || номер > найденные.элементы.len() {
            return Err(ToolError::Unknown(format!(
                "ref {ссылка} вне диапазона: сейчас в дереве {} элементов. \
                 Сделай browser.snapshot заново",
                найденные.элементы.len()
            )));
        }
        return Ok(Выбор {
            найденные,
            индекс: номер - 1,
        });
    }

    let Some(текст) = текст.map(str::trim).filter(|текст| !текст.is_empty()) else {
        return Err(ToolError::BadArgs(
            "нужен ref из browser.snapshot или text — видимая подпись элемента".into(),
        ));
    };

    let найденные = дерево(hwnd);
    let игла = текст.to_lowercase();
    let индексы: Vec<usize> = (0..найденные.элементы.len())
        .filter(|индекс| {
            let элемент = &найденные.элементы[*индекс];
            элемент.текст.to_lowercase().contains(&игла)
                && (!только_поле || элемент.вид == "поле ввода")
        })
        .collect();

    match индексы.len() {
        1 => Ok(Выбор {
            найденные,
            индекс: индексы[0],
        }),
        0 => {
            let подписи = подписи(&найденные);
            Err(ToolError::Unknown(format!(
                "в браузере нет элемента{} с текстом «{текст}». Видно: {}",
                if только_поле { " — поля ввода" } else { "" },
                если_пусто(&подписи)
            )))
        }
        _ => {
            let список: Vec<String> = индексы
                .iter()
                .map(|индекс| {
                    let элемент = &найденные.элементы[*индекс];
                    format!("{} «{}»", элемент.вид, элемент.текст)
                })
                .collect();
            Err(ToolError::Ambiguous(format!(
                "под «{текст}» подходит {} элементов: {}. Уточни текст или возьми ref из browser.snapshot",
                индексы.len(),
                список.join("; ")
            )))
        }
    }
}

fn подписи(найденные: &uia::Найденные) -> Vec<String> {
    let mut все: Vec<String> = найденные
        .элементы
        .iter()
        .map(|элемент| format!("{} «{}»", элемент.вид, элемент.текст))
        .collect();
    все.sort();
    все.dedup();
    все
}

fn если_пусто(список: &[String]) -> String {
    if список.is_empty() {
        "ничего".to_string()
    } else {
        список.iter().take(20).cloned().collect::<Vec<_>>().join("; ")
    }
}

/// Где лежит браузер: настоящий `.exe`, а не ярлык.
fn браузер_exe() -> Result<PathBuf, ToolError> {
    for имя in ["chrome", "msedge", "firefox"] {
        let Ok(путь) = apps::найти(имя) else {
            continue;
        };
        let exe = путь
            .extension()
            .and_then(|расширение| расширение.to_str())
            .is_some_and(|расширение| расширение.eq_ignore_ascii_case("exe"));
        if exe && путь.is_file() {
            return Ok(путь);
        }
    }
    Err(ToolError::Missing(
        "не нашёл Chrome, Edge или Firefox. Установи браузер или дай полный путь к .exe".into(),
    ))
}

fn проверить_адрес(адрес: &str) -> Result<(), ToolError> {
    let адрес = адрес.trim();
    if адрес.is_empty() {
        return Err(ToolError::BadArgs("пустой адрес".into()));
    }
    if адрес.contains(['\n', '\r', '"']) {
        return Err(ToolError::BadArgs(
            "в адресе есть кавычки или перевод строки".into(),
        ));
    }
    let нижний = адрес.to_lowercase();
    if нижний.starts_with("http://")
        || нижний.starts_with("https://")
        || нижний.starts_with("file://")
    {
        Ok(())
    } else {
        Err(ToolError::BadArgs(
            "адрес должен начинаться с http://, https:// или file://".into(),
        ))
    }
}

/// Открыть адрес в браузере.
pub struct Открыть;

impl Tool for Открыть {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.open".into(),
            capability: "browser.dom".into(),
            name: "Открыть адрес в браузере".into(),
            description: "Открыть http(s)- или file-адрес в Chrome, Edge или Firefox. \
                          Если браузер уже запущен, откроется новая вкладка. После \
                          открытия сделай browser.snapshot, чтобы увидеть страницу."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "адрес: https://..., http://..., file:///..." }
                },
                "required": ["url"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(адрес) = args.get("url").and_then(|value| value.as_str()) else {
            return Err(ToolError::BadArgs("нет поля url".into()));
        };
        проверить_адрес(адрес)?;
        let браузер = браузер_exe()?;
        Command::new(&браузер)
            .arg(адрес.trim())
            .spawn()
            .map_err(|ошибка| {
                ToolError::Refused(format!(
                    "не удалось запустить {}: {ошибка}",
                    браузер.display()
                ))
            })?;
        // Дать браузеру время создать вкладку и окно. Не ошибка, если
        // не успел: следующий browser.snapshot сам подождёт дерево.
        std::thread::sleep(Duration::from_millis(900));
        Ok(ToolOutput::changed(
            format!("открыт адрес {}", адрес.trim()),
            format!("открыт адрес {}", адрес.trim()),
        ))
    }
}

/// Перечислить элементы страницы с ref.
pub struct Снимок;

impl Tool for Снимок {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.snapshot".into(),
            capability: "windows.read".into(),
            name: "Снимок страницы браузера".into(),
            description: "Показать элементы открытой страницы: кнопки, ссылки, поля. \
                          Каждый элемент получает ref (@e1, @e2, ...). Нажимай \
                          browser.click по ref или по видимому тексту, вводи текст \
                          browser.type. Снимок не меняет страницу."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "часть заголовка окна, если открыто несколько" }
                },
                "required": []
            }),
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let окно = окно(args)?;
        let найденные = дерево(дескриптор(&окно));
        if найденные.элементы.is_empty() {
            return Err(ToolError::Refused(format!(
                "браузер «{}» не отдал дерево доступности. Открой обычную страницу \
                 и повтори: некоторые внутренние страницы браузера закрыты от \
                 автоматизации",
                окно.заголовок
            )));
        }

        let mut текст = format!(
            "Окно браузера: «{}»\nЭлементы страницы (жми browser.click, вводи browser.type):\n",
            окно.заголовок
        );
        for (номер, элемент) in найденные.элементы.iter().take(ПОКАЗЫВАТЬ).enumerate() {
            текст.push_str(&format!(
                "  @e{} — {} «{}»\n",
                номер + 1,
                элемент.вид,
                элемент.текст
            ));
        }
        if найденные.элементы.len() > ПОКАЗЫВАТЬ {
            текст.push_str(&format!(
                "\nПоказаны первые {} из {}. Уточни title или работай по видимому тексту.\n",
                ПОКАЗЫВАТЬ,
                найденные.элементы.len()
            ));
        }
        let кратко = format!(
            "элементов: {} в «{}»",
            найденные.элементы.len(),
            окно.заголовок
        );
        Ok(ToolOutput::read(текст, кратко))
    }
}

/// Нажать элемент страницы.
pub struct Нажать;

impl Tool for Нажать {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.click".into(),
            capability: "browser.dom".into(),
            name: "Нажать в браузере".into(),
            description: "Нажать кнопку или ссылку на открытой странице по ref из \
                          browser.snapshot или по видимому тексту. Сначала пробует \
                          штатное действие доступности, затем настоящий клик мышью \
                          по центру найденного элемента."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "часть заголовка окна" },
                    "ref": { "type": "string", "description": "ref из browser.snapshot, например @e3" },
                    "text": { "type": "string", "description": "видимая подпись элемента" }
                },
                "required": []
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let окно = окно(args)?;
        let ссылка = args.get("ref").and_then(|value| value.as_str());
        let текст = args.get("text").and_then(|value| value.as_str());
        let выбор = выбрать(дескриптор(&окно), текст, ссылка, false)?;
        let элемент = выбор.элемент();
        let подпись = элемент.текст.clone();

        match uia::нажать(элемент) {
            Ok(()) => Ok(ToolOutput::changed(
                format!("нажато «{подпись}» в «{}»", окно.заголовок),
                format!("нажато «{подпись}»"),
            )),
            Err(причина) => {
                let Some(точка) = uia::центр(элемент) else {
                    return Err(ToolError::Refused(format!(
                        "{причина}; у элемента нет видимых размеров, кликнуть мышью нельзя"
                    )));
                };
                control::клик_в_точку(точка).map_err(|ошибка| {
                    ToolError::Refused(format!(
                        "{причина}; клик мышью тоже не прошёл: {ошибка}"
                    ))
                })?;
                Ok(ToolOutput::changed(
                    format!("нажато «{подпись}» мышью в «{}»", окно.заголовок),
                    format!("нажато «{подпись}» мышью"),
                ))
            }
        }
    }
}

/// Ввести текст в поле страницы.
pub struct Ввести;

impl Tool for Ввести {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.type".into(),
            capability: "browser.dom".into(),
            name: "Ввести текст в браузере".into(),
            description: "Ввести текст в поле на открытой странице. target — видимая \
                          подпись поля или его ref. replace=true сначала выделяет и \
                          удаляет старое содержимое поля. Без target текст уходит в \
                          то, что сейчас в фокусе."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "часть заголовка окна" },
                    "text": { "type": "string", "description": "что ввести" },
                    "target": { "type": "string", "description": "подпись поля" },
                    "ref": { "type": "string", "description": "ref поля из browser.snapshot" },
                    "replace": { "type": "boolean", "description": "заменить старое содержимое, по умолчанию нет" }
                },
                "required": ["text"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let окно = окно(args)?;
        let Some(текст) = args.get("text").and_then(|value| value.as_str()) else {
            return Err(ToolError::BadArgs("нет поля text".into()));
        };
        let заменить = args
            .get("replace")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let target = args.get("target").and_then(|value| value.as_str());
        let ссылка = args.get("ref").and_then(|value| value.as_str());

        if target.is_some() || ссылка.is_some() {
            let выбор = выбрать(дескриптор(&окно), target, ссылка, true)?;
            let элемент = выбор.элемент();

            // Первый путь: поле принимает значение напрямую. Он не
            // зависит от фокуса и раскладки и заменяет содержимое целиком.
            if uia::установить_значение(элемент, текст).is_ok() {
                return Ok(ToolOutput::changed(
                    format!("в поле «{}» вставлен текст", элемент.текст),
                    format!("введено в «{}»", элемент.текст),
                ));
            }

            // Второй путь: фокус на элемент и настоящий ввод с клавиатуры.
            let _ = uia::сфокусировать(элемент);
            if let Some(точка) = uia::центр(элемент) {
                let _ = control::клик_в_точку(точка);
            }
            std::thread::sleep(Duration::from_millis(120));
            if заменить {
                control::нажать("ctrl+a")?;
                control::нажать("delete")?;
            }
            let символов = control::напечатать(текст)?;
            return Ok(ToolOutput::changed(
                format!("в поле «{}» введено символов: {символов}", элемент.текст),
                format!("введено в «{}»", элемент.текст),
            ));
        }

        // Без target — в активное поле. Окно поднимаем, иначе клавиши
        // уйдут в чужую программу.
        let _ = control::фокус_на(окно.hwnd);
        if заменить {
            control::нажать("ctrl+a")?;
            control::нажать("delete")?;
        }
        let символов = control::напечатать(текст)?;
        Ok(ToolOutput::changed(
            format!("введено символов: {символов} в «{}»", окно.заголовок),
            format!("введено символов: {символов}"),
        ))
    }
}

/// Нажать горячие клавиши в браузере.
pub struct Клавиши;

impl Tool for Клавиши {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.press".into(),
            capability: "browser.dom".into(),
            name: "Нажать клавиши в браузере".into(),
            description: "Нажать клавиши в окне браузера: enter, tab, ctrl+l, \
                          ctrl+w, pagedown. Окно сначала получает фокус."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "часть заголовка окна" },
                    "keys": { "type": "string", "description": "например ctrl+l или enter" }
                },
                "required": ["keys"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let окно = окно(args)?;
        let Some(клавиши) = args.get("keys").and_then(|value| value.as_str()) else {
            return Err(ToolError::BadArgs("нет поля keys".into()));
        };
        let _ = control::фокус_на(окно.hwnd);
        let нажатые = control::нажать(клавиши)?;
        Ok(ToolOutput::changed(
            format!("нажато {} в «{}»", нажатые.join("+"), окно.заголовок),
            format!("нажато {}", нажатые.join("+")),
        ))
    }
}

/// Прокрутить страницу колесом.
pub struct Прокрутить;

impl Tool for Прокрутить {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "browser.scroll".into(),
            capability: "browser.dom".into(),
            name: "Прокрутить страницу".into(),
            description: "Прокрутить открытую страницу вверх или вниз колесом мыши. \
                          Курсор ставится в центр окна браузера."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "часть заголовка окна" },
                    "direction": { "type": "string", "enum": ["down", "up"], "description": "куда крутить" },
                    "amount": { "type": "integer", "minimum": 1, "maximum": 10, "description": "шагов, по умолчанию 3" }
                },
                "required": ["direction"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let окно = окно(args)?;
        let направление = args
            .get("direction")
            .and_then(|value| value.as_str())
            .unwrap_or("down")
            .to_lowercase();
        let вниз = match направление.as_str() {
            "down" | "вниз" => true,
            "up" | "вверх" => false,
            другое => {
                return Err(ToolError::BadArgs(format!(
                    "неизвестное направление «{другое}», нужно down или up"
                )))
            }
        };
        let шагов = args
            .get("amount")
            .and_then(|value| value.as_u64())
            .unwrap_or(3) as u32;

        let mut рамка = RECT::default();
        unsafe { GetWindowRect(дескриптор(&окно), &mut рамка) }.map_err(|ошибка| {
            ToolError::Refused(format!("не удалось прочитать размеры окна: {ошибка}"))
        })?;
        let центр = (
            (рамка.left + рамка.right) / 2,
            (рамка.top + рамка.bottom) / 2,
        );
        control::прокрутить_в(центр, вниз, шагов)?;

        let слово = if вниз { "вниз" } else { "вверх" };
        Ok(ToolOutput::changed(
            format!("прокручено {слово} на {шагов}: «{}»", окно.заголовок),
            format!("прокручено {слово}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn браузер_узнаётся_по_заголовку() {
        assert!(это_браузер("Титан квест — Google Chrome"));
        assert!(это_браузер("Документ - Microsoft Edge"));
        assert!(это_браузер("Новая вкладка — Mozilla Firefox"));
        assert!(!это_браузер("Блокнот — заметка.txt"));
    }

    #[test]
    fn адрес_проверяется() {
        assert!(проверить_адрес("https://example.com").is_ok());
        assert!(проверить_адрес("http://example.com/a?b=1").is_ok());
        assert!(проверить_адрес("file:///C:/tmp/a.html").is_ok());
        assert!(проверить_адрес("javascript:alert(1)").is_err());
        assert!(проверить_адрес("calc.exe").is_err());
        assert!(проверить_адрес("https://example.com/\"onload").is_err());
    }
}

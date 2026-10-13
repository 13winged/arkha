// Действия, которые меняют компьютер: фокус, ввод, клик.
//
// Всё здесь — L1, то есть подтверждение спрашивается каждый раз.
// Причина не в том, что действия необратимы (ввод текста обратим),
// а в том, что результат непредсказуем для человека: он не видит за
//ранее, во что попадёт.
//
// **Клик только по названному элементу.** Ни одного инструмента,
// который кликает по координатам, здесь нет и не планируется. Путь
// такой: агент называет окно и текст элемента, мы ищем дочернее окно
// с таким текстом и кликаем в его центр. Если элемент не найден —
// отказ с перечислением того, что найдено.
//
// Почему без координат: координаты агент выдумывает. Проверка
// выдумки по тексту ответа уже была отвергнута — она даёт ложные
// срабатывания дороже пропусков. Клик по выдуманной координате
// бьёт не туда, и человек видит только последствие.

use serde_json::{json, Value};

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetSystemMetrics, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    IsWindowVisible, SetForegroundWindow, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SYSTEM_METRICS_INDEX,
};

use super::windows::{найти, Окно};
use super::{Tool, ToolError, ToolOutput, ToolSpec};
use crate::permissions::Settings;

/// Заголовок окна по дескриптору.
fn заголовок(hwnd: HWND) -> String {
    let длина = unsafe { GetWindowTextLengthW(hwnd) };
    if длина <= 0 {
        return String::new();
    }
    let mut буфер = vec![0u16; длина as usize + 1];
    let получено = unsafe { GetWindowTextW(hwnd, &mut буфер) };
    if получено <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&буфер[..получено as usize])
        .trim()
        .to_string()
}

/// Найти окно по подстроке заголовка.
///
/// Ровно одно совпадение — ищем; несколько — ошибка с перечислением,
/// чтобы человек выбрал. Невидимые окна сюда не попадают вовсе:
/// `найти` их отсекает, а фокус на скрытом окне выглядит как «ничего
/// не произошло». Свёрнутые окна при этом остаются: они видимы, и
/// вернуть им фокус можно.
pub fn окно_для(подстрока: &str) -> Result<Окно, ToolError> {
    let найденные = найти(подстрока);
    match найденные.len() {
        0 => Err(ToolError::Unknown(format!(
            "видимого окна с «{подстрока}» в названии нет"
        ))),
        1 => Ok(найденные.into_iter().next().expect("список длиной 1")),
        _ => {
            let список: Vec<String> = найденные
                .iter()
                .map(|о| format!("[{}] {}", о.hwnd, о.заголовок))
                .collect();
            Err(ToolError::Ambiguous(format!(
                "под «{подстрока}» подходит {} окон: {}. Уточни, какое",
                найденные.len(),
                список.join("; ")
            )))
        }
    }
}

/// Элемент внутри окна, найденный по своему тексту.
#[derive(Debug, Clone, PartialEq)]
pub struct Элемент {
    pub hwnd: isize,
    pub текст: String,
    /// Центр элемента в экранных координатах. Считается один раз, до
    /// клика: между поиском и кликом элемент может уехать, и клик
    /// пройдёт мимо.
    pub центр: (i32, i32),
}

/// Найти элемент по тексту внутри окна.
///
/// Обход только видимых дочерних окон: невидимый элемент кликнуть
/// нельзя, клик уйдёт в то, что под ним.
pub fn найти_элемент(родитель: HWND, текст: &str) -> Vec<Элемент> {
    let игла = текст.trim().to_lowercase();
    let mut найденные: Vec<Элемент> = Vec::new();
    if игла.is_empty() {
        return найденные;
    }

    unsafe extern "system" fn обход(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let пара = unsafe { &mut *(lparam.0 as *mut (String, Vec<Элемент>)) };
        let (игла, найденные) = (&mut пара.0, &mut пара.1);

        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            let надпись = заголовок(hwnd);
            if !надпись.is_empty() && надпись.to_lowercase().contains(игла.as_str()) {
                let mut прямоугольник = RECT::default();
                if unsafe { GetWindowRect(hwnd, &mut прямоугольник) }.is_ok() {
                    let ширина = прямоугольник.right - прямоугольник.left;
                    let высота = прямоугольник.bottom - прямоугольник.top;
                    // Нулевой размер — значит свёрнуто или это
                    // невидимый разделитель. Кликать некуда.
                    if ширина > 0 && высота > 0 {
                        найденные.push(Элемент {
                            hwnd: hwnd.0 as isize,
                            текст: надпись,
                            центр: (
                                (прямоугольник.left + прямоугольник.right) / 2,
                                (прямоугольник.top + прямоугольник.bottom) / 2,
                            ),
                        });
                    }
                }
            }
        }
        BOOL(1)
    }

    let mut пара = (игла, Vec::<Элемент>::new());
    unsafe {
        let _ = EnumChildWindows(
            Some(родитель),
            Some(обход),
            LPARAM(&mut пара as *mut (String, Vec<Элемент>) as isize),
        );
    }
    пара.1
}

/// Клавиатура: напечатать текст и нажать клавиши.
pub fn напечатать(текст: &str) -> Result<usize, ToolError> {
    // Юникод через SendInput, а не раскладка: текст может быть
    // русским, а текущая раскладка — английской. Переключать
    // раскладку ради ввода означало бы менять настройки человека.
    let единиц: Vec<u16> = текст.encode_utf16().collect();
    let всего = единиц.len();
    if всего == 0 {
        return Ok(0);
    }
    let mut ввод: Vec<INPUT> = Vec::with_capacity(всего * 2);
    for код in единиц {
        for нажатие in [true, false] {
            let флаги = if нажатие {
                KEYEVENTF_UNICODE
            } else {
                KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
            };
            ввод.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: код,
                        dwFlags: флаги,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    let отправлено = unsafe { SendInput(&ввод, std::mem::size_of::<INPUT>() as i32) };
    if отправлено as usize != ввод.len() {
        // SendInput возвращает, сколько событий принято. Несовпадение
        // означает блокировку ввода — например, работает другая
        // программа в повышенных правах.
        return Err(ToolError::Refused(
            "система приняла не все нажатия — ввод заблокирован другой программой".into(),
        ));
    }
    Ok(всего)
}

/// Нажать клавиши: «ctrl+s», «enter», «alt+tab».
///
/// Названия, а не коды: модель не знает 0x11 и не должна знать.
pub fn нажать(комбо: &str) -> Result<Vec<String>, ToolError> {
    let части: Vec<String> = комбо
        .split('+')
        .map(|с| с.trim().to_lowercase())
        .filter(|с| !с.is_empty())
        .collect();
    if части.is_empty() {
        return Err(ToolError::BadArgs("не указана ни одна клавиша".into()));
    }
    let коды: Vec<VIRTUAL_KEY> = части
        .iter()
        .map(|имя| код_клавиши(имя))
        .collect::<Result<Vec<_>, _>>()?;
    let последний = коды.len() - 1;

    // Модификаторы нажимаются и остаются нажатыми, последняя клавиша
    // отпускается — иначе «ctrl+s» превратится в «c».
    let событие = |код: VIRTUAL_KEY, вверх: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: код,
                wScan: 0,
                dwFlags: if вверх { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let mut ввод: Vec<INPUT> = Vec::with_capacity(коды.len() * 2);
    for (н, код) in коды.iter().enumerate() {
        let вверх = н == последний;
        ввод.push(событие(*код, вверх));
        if вверх {
            // Отпускаем модификаторы в обратном порядке.
            for модификатор in коды[..последний].iter().rev() {
                ввод.push(событие(*модификатор, true));
            }
        }
    }
    let отправлено = unsafe { SendInput(&ввод, std::mem::size_of::<INPUT>() as i32) };
    if отправлено as usize != ввод.len() {
        return Err(ToolError::Refused(
            "система приняла не все нажатия — ввод заблокирован".into(),
        ));
    }
    Ok(части)
}

/// Имя клавиши в код.
fn код_клавиши(имя: &str) -> Result<VIRTUAL_KEY, ToolError> {
    // Регистр приводится здесь, а не в вызывающем: иначе прямой
    // вызов «ENTER» отличался бы от «enter» через «нажать», и
    // тест проверял бы не то, чем пользуется модель.
    let имя = &имя.trim().to_lowercase();
    let код = match имя.as_str() {
        "ctrl" | "control" => 0x11,
        "alt" => 0x12,
        "shift" => 0x10,
        "win" | "meta" => 0x5B,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "esc" | "escape" => 0x1B,
        "space" => 0x20,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "f1" => 0x70,
        "f2" => 0x71,
        "f3" => 0x72,
        "f4" => 0x73,
        "f5" => 0x74,
        "f6" => 0x75,
        "f10" => 0x79,
        "f11" => 0x7A,
        "f12" => 0x7B,
        _ => {
            // Одна буква или цифра — её код в Latin-1 совпадает с
            // ASCII, а кириллицу так не напечатать: для неё есть
            // `напечатать`.
            let байты = имя.as_bytes();
            if байты.len() == 1 && байты[0].is_ascii_alphanumeric() {
                байты[0] as u16
            } else {
                return Err(ToolError::BadArgs(format!(
                    "не знаю клавишу «{имя}». Для текста используй «напиши», \
                     здесь только именованные клавиши: ctrl, alt, shift, enter, \
                     tab, esc, space, backspace, delete, f1–f12, стрелки"
                )));
            }
        }
    };
    Ok(VIRTUAL_KEY(код))
}

/// Перевести фокус на окно.
pub struct ВФокус;

impl Tool for ВФокус {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "windows.focus".into(),
            capability: "windows.control".into(),
            name: "Перевести фокус на окно".into(),
            description: "Перевести фокус на окно по части его названия. Если подходит \
                          несколько окон, действие не выполняется и список возвращается."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "title": { "type": "string", "description": "часть заголовка окна" } },
                "required": ["title"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(имя) = args.get("title").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля title".into()));
        };
        let о = окно_для(имя)?;
        let hwnd = HWND(о.hwnd as *mut _);
        // Windows запрещает программе забирать фокус у другой. Метод
        // работает не всегда: если окно свёрнуто или принадлежит
        // процессу повышенных прав, он вернёт «ложь».
        let получилось = unsafe { SetForegroundWindow(hwnd) }.as_bool();
        let теперь = super::windows::активное();
        let совпало = теперь.as_ref().map(|а| а.hwnd) == Some(о.hwnd);
        if !получилось || !совпало {
            return Err(ToolError::Refused(format!(
                "не удалось перевести фокус на «{}»: система не отдала окно другой \
                 программе. Открой его вручную и скажи ещё раз",
                о.заголовок
            )));
        }
        Ok(ToolOutput::changed(
            format!("фокус на «{}»", о.заголовок),
            format!("фокус переведён на «{}»", о.заголовок),
        ))
    }
}

/// Напечатать текст.
pub struct Написать;

impl Tool for Написать {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "windows.type".into(),
            capability: "windows.control".into(),
            name: "Напечатать текст".into(),
            description: "Напечатать текст в окне, где сейчас фокус. Раскладка не важна: \
                          русский и английский печатаются одинаково."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "text": { "type": "string", "description": "текст для ввода" } },
                "required": ["text"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(текст) = args.get("text").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля text".into()));
        };
        if текст.is_empty() {
            return Err(ToolError::BadArgs("текст пустой — печатать нечего".into()));
        }
        let символов = напечатать(текст)?;
        Ok(ToolOutput::changed(
            format!("напечатано {символов} символов"),
            format!("напечатано {символов} символов"),
        ))
    }
}

/// Нажать клавиши.
pub struct Нажать;

impl Tool for Нажать {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "windows.key".into(),
            capability: "windows.control".into(),
            name: "Нажать клавиши".into(),
            description: "Нажать клавиши в окне, где сейчас фокус: «enter», «tab», \
                          «ctrl+s», «alt+tab». Имена латиницей."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": { "keys": { "type": "string", "description": "например ctrl+s или enter" } },
                "required": ["keys"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(комбо) = args.get("keys").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля keys".into()));
        };
        let нажатые = нажать(комбо)?;
        Ok(ToolOutput::changed(
            format!("нажато: {}", нажатые.join("+")),
            format!("нажато: {}", нажатые.join("+")),
        ))
    }
}

/// Кликнуть по элементу окна, найденному по его тексту.
pub struct Кликнуть;

impl Tool for Кликнуть {
    fn spec(&self, _s: &Settings) -> ToolSpec {
        ToolSpec {
            id: "windows.click".into(),
            capability: "windows.control".into(),
            name: "Кликнуть по элементу окна".into(),
            description: "Кликнуть по элементу окна, найденному по его видимому тексту. \
                          По координатам кликать нельзя: секретарь не должен угадывать, \
                          куда попадёт. Если элемент не найден или подходит несколько — \
                          ничего не нажимается, возвращается список найденного."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "часть заголовка окна" },
                    "control": { "type": "string", "description": "текст элемента: «Сохранить», «ОК»" }
                },
                "required": ["title", "control"]
            }),
            side_effecting: true,
        }
    }

    fn call(&self, args: &Value, _s: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(заголовок_окна) = args.get("title").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля title".into()));
        };
        let Some(текст) = args.get("control").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs("нет поля control".into()));
        };
        let о = окно_для(заголовок_окна)?;
        let родитель = HWND(о.hwnd as *mut _);

        let найденные = найти_элемент(родитель, текст);
        match найденные.len() {
            0 => {
                let список: Vec<String> = найти_элемент(родитель, "")
                    .into_iter()
                    .map(|э| э.текст)
                    .collect();
                Err(ToolError::Unknown(format!(
                    "в окне «{}» нет видимого элемента с текстом «{}». \
                     Видимые подписи: {}",
                    о.заголовок,
                    текст,
                    if список.is_empty() {
                        "ни одной".to_string()
                    } else {
                        список.join("; ")
                    }
                )))
            }
            1 => {
                let э = &найденные[0];
                кликнуть(э)?;
                Ok(ToolOutput::changed(
                    format!("нажато «{}» в окне «{}»", э.текст, о.заголовок),
                    format!("нажато «{}» в «{}»", э.текст, о.заголовок),
                ))
            }
            _ => {
                let список: Vec<String> = найденные
                    .iter()
                    .map(|э| format!("[{}] {}", э.hwnd, э.текст))
                    .collect();
                Err(ToolError::Ambiguous(format!(
                    "под «{текст}» подходит {} элементов: {}. Уточни, какой",
                    найденные.len(),
                    список.join("; ")
                )))
            }
        }
    }
}

/// Клик мышью в точку.
fn кликнуть(э: &Элемент) -> Result<(), ToolError> {
    let (абс_x, абс_y) = в_абсолютные(э.центр)?;
    let mut событий: Vec<INPUT> = Vec::with_capacity(2);
    for флаги in [
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        MOUSEEVENTF_LEFTDOWN | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        MOUSEEVENTF_LEFTUP | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    ] {
        событий.push(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: абс_x,
                    dy: абс_y,
                    mouseData: 0,
                    dwFlags: флаги,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }
    let отправлено = unsafe { SendInput(&событий, std::mem::size_of::<INPUT>() as i32) };
    if отправлено as usize != событий.len() {
        return Err(ToolError::Refused(
            "система не приняла нажатие мыши".into(),
        ));
    }
    Ok(())
}

/// Экранные координаты в формате `SendInput`.
///
/// Без этого клик уходит не туда. В `MOUSEINPUT` координаты
/// **относительные**, если не задан `MOUSEEVENTF_ABSOLUTE`: то есть
/// клик по центру кнопки сместил бы мышь на столько пикселей от
/// текущего места, а не нажал бы кнопку. Ошибка выглядит как «секретарь
/// кликает мимо», и виноват был бы не UIA, а арифметика.
///
/// Формат абсолютных координат — 0..65535 по **всему виртуальному**
/// экрану, включая мониторы слева и сверху, а не по основному.
fn в_абсолютные((x, y): (i32, i32)) -> Result<(i32, i32), ToolError> {
    let метрика = |что: SYSTEM_METRICS_INDEX| unsafe { GetSystemMetrics(что) };
    let левый = метрика(SM_XVIRTUALSCREEN);
    let верхний = метрика(SM_YVIRTUALSCREEN);
    let ширина = метрика(SM_CXVIRTUALSCREEN);
    let высота = метрика(SM_CYVIRTUALSCREEN);
    if ширина <= 0 || высота <= 0 {
        return Err(ToolError::Refused(
            "система не сообщила размеры экрана".into(),
        ));
    }
    let норм = |значение: i32, начало: i32, размер: i32| -> i32 {
        ((значение - начало).max(0) * 65535) / размер
    };
    Ok((
        норм(x, левый, ширина).min(65535),
        норм(y, верхний, высота).min(65535),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Клавиши по именам, а не по кодам: модель не знает 0x11.
    #[test]
    fn имена_клавиш_понятны() {
        assert_eq!(код_клавиши("ctrl").unwrap(), VIRTUAL_KEY(0x11));
        assert_eq!(код_клавиши("ENTER").unwrap(), VIRTUAL_KEY(0x0D));
        assert_eq!(код_клавиши("a").unwrap(), VIRTUAL_KEY(0x61));
        assert_eq!(код_клавиши("5").unwrap(), VIRTUAL_KEY(0x35));
    }

    /// Неизвестная клавиша должна называть известные: иначе модель
    /// подберёт наугад и потратит заходы.
    #[test]
    fn неизвестная_клавиша_объясняет() {
        let e = код_клавиши("суперкнопка").unwrap_err();
        assert!(e.to_string().contains("enter"), "{e}");
    }

    /// Кириллица через «клавиши» не набирается осмысленно: для неё
    /// есть отдельный инструмент ввода текста.
    #[test]
    fn кириллица_не_считается_клавишей() {
        assert!(код_клавиши("ы").is_err());
    }

    #[test]
    fn пустое_комбо_отказ() {
        assert!(нажать("").is_err());
        assert!(нажать("   ").is_err());
    }

    /// Поиск элемента не должен находить пустое: иначе первый же
    /// клик попадёт во что попало.
    #[test]
    fn пустой_текст_элемента_ничего_не_находит() {
        let hwnd = HWND(std::ptr::null_mut());
        assert!(найти_элемент(hwnd, "").is_empty());
        assert!(найти_элемент(hwnd, "   ").is_empty());
    }

    /// Действия обязаны быть подтверждаемыми. Если уровень упадёт до
    /// L3, секретарь начнёт печатать и кликать без спроса.
    #[test]
    fn действия_требуют_подтверждения() {
        use crate::agent::approvals::{уровень, Уровень};
        for возможность in ["windows.control", "apps.launch"] {
            assert_eq!(
                уровень(возможность, &json!({})),
                Уровень::L1,
                "{возможность} обязана быть подтверждаемой"
            );
        }
    }

    /// Инструменты управления обязаны быть помечены как меняющие
    /// систему: от этого зависит, спрашивается ли подтверждение.
    #[test]
    fn инструменты_помечены_как_меняющие() {
        let s = Settings::default();
        for инструмент in [
            Box::new(ВФокус) as Box<dyn Tool>,
            Box::new(Написать),
            Box::new(Нажать),
            Box::new(Кликнуть),
        ] {
            assert!(
                инструмент.spec(&s).side_effecting,
                "{} должен быть помечен как меняющий систему",
                инструмент.spec(&s).id
            );
        }
    }

    /// Инструмент, кликающий по координатам, не должен существовать.
    ///
    /// Проверяется намеренно: вернуться к координатам захочется, когда
    /// UIA окажется хрупким, и тогда это будет выглядеть как
    /// «простое улучшение». Простое улучшение, бьющее по файлам
    /// пользователя, — не улучшение.
    #[test]
    fn клика_по_координатам_нет() {
        let s = Settings::default();
        let ids: Vec<String> = crate::tools::Registry::allowed(&s)
            .specs(&s)
            .into_iter()
            .map(|сп| сп.id)
            .collect();
        for подозрительное in ["windows.click_xy", "windows.click_at", "mouse.move"] {
            assert!(
                !ids.contains(&подозрительное.to_string()),
                "инструмент {подозрительное} не должен существовать"
            );
        }
    }
}

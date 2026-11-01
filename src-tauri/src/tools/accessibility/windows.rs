// Windows-адаптер доступности: обёртка над существующим uia.rs.

use std::sync::Mutex;

use windows::Win32::Foundation::HWND;

use super::{
    Accessibility, AccessibilityError, RefEntry, Rect, Role, Snapshot, Target,
};
use crate::tools::uia;

/// Дескриптор окна, безопасный для передачи между потоками.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowHandle(HWND);

unsafe impl Send for WindowHandle {}
unsafe impl Sync for WindowHandle {}

/// Адаптер Windows: создается на каждый вызов.
pub struct WindowsAccessibility;

impl WindowsAccessibility {
    pub fn new() -> Box<dyn super::Accessibility> {
        Box::new(WindowsAccessibility)
    }
}

impl Accessibility for WindowsAccessibility {
    fn snapshot(&self, target: &Target) -> Result<Snapshot, AccessibilityError> {
        let hwnd = match target {
            Target::Window(hwnd) => HWND(*hwnd as *mut _),
            Target::Desktop => {
                return Err(AccessibilityError::Platform(
                    "desktop snapshot не поддерживается на Windows".into(),
                ))
            }
        };

        if !uia::окно_живое(hwnd) {
            return Err(AccessibilityError::TreeUnavailable(
                "окно не отдаёт дерево доступности".into(),
            ));
        }

        // Сохраняем HWND для click_ref (элементы не Send, храним только handle)
        {
            let mut slot = REF_SLOT.lock().unwrap();
            *slot = Some(WindowHandle(hwnd));
        }

        let found = uia::найти(hwnd, "");

        let mut refs = Vec::new();
        let mut counter: u32 = 1;

        for el in &found.элементы {
            let ref_id = format!("@e{}", counter);
            counter += 1;
            refs.push(RefEntry {
                ref_id,
                role: uia_role_to_role(el.код_вида),
                name: Some(el.текст.clone()),
                bounds: Some(rect_from_uia(el.рамка)),
                available_actions: vec!["click".to_string()],
            });
        }

        Ok(Snapshot {
            refs,
            nodes_observed: found.элементы.len(),
            complete: true,
        })
    }

    fn click_ref(&self, ref_id: &str) -> Result<(), AccessibilityError> {
        let n: usize = ref_id
            .trim_start_matches('@')
            .trim_start_matches('e')
            .parse()
            .map_err(|_| AccessibilityError::ElementNotFound(format!("неверный ref: {ref_id}")))?;

        let slot = REF_SLOT.lock().unwrap();
        let Some(WindowHandle(hwnd)) = *slot else {
            drop(slot);
            return Err(AccessibilityError::ElementNotFound(
                "сначала сделайте snapshot, чтобы получить refs".into(),
            ));
        };
        drop(slot);

        // Пересчитываем элементы и берём N-ый
        let found = uia::найти(hwnd, "");

        if n == 0 || n > found.элементы.len() {
            return Err(AccessibilityError::ElementNotFound(format!(
                "ref {ref_id} вне диапазона 1..{}",
                found.элементы.len()
            )));
        }

        let el = &found.элементы[n - 1];
        uia::нажать(el).map_err(|e| AccessibilityError::Platform(e))?;

        Ok(())
    }

    fn list_apps(&self) -> Result<Vec<String>, AccessibilityError> {
        let windows_list = crate::tools::windows::окна();
        let mut apps: Vec<String> = windows_list.into_iter().map(|w| w.заголовок).collect();
        apps.sort();
        apps.dedup();
        Ok(apps)
    }

    fn list_windows(&self) -> Result<Vec<String>, AccessibilityError> {
        let windows_list = crate::tools::windows::окна();
        let mut titles: Vec<String> = windows_list.into_iter().map(|w| w.заголовок).collect();
        titles.sort();
        titles.dedup();
        Ok(titles)
    }
}

/// Состояние между snapshot и click_ref.
static REF_SLOT: Mutex<Option<WindowHandle>> = Mutex::new(None);

fn uia_role_to_role(code: i32) -> Role {
    match code {
        code if code == uia::UIA_ButtonControlTypeId.0 => Role::Button,
        code if code == uia::UIA_CheckBoxControlTypeId.0 => Role::CheckBox,
        code if code == uia::UIA_HyperlinkControlTypeId.0 => Role::Hyperlink,
        code if code == uia::UIA_EditControlTypeId.0 => Role::Edit,
        code if code == uia::UIA_ListItemControlTypeId.0 => Role::ListItem,
        code if code == uia::UIA_MenuItemControlTypeId.0 => Role::MenuItem,
        code if code == uia::UIA_RadioButtonControlTypeId.0 => Role::RadioButton,
        code if code == uia::UIA_TabItemControlTypeId.0 => Role::TabItem,
        code if code == uia::UIA_TextControlTypeId.0 => Role::Text,
        _ => Role::Other(format!("{code}")),
    }
}

fn rect_from_uia(rect: windows::Win32::Foundation::RECT) -> Rect {
    Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}
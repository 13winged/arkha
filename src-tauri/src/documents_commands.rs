// Команды подгрузки CLI документов.
//
// Загрузка вынесена из инструментов наружу по простой причине: 120 МБ,
// скачанные внутри вызова инструмента, выглядят как зависший агент.
// Человек должен видеть список, размер и полосу, а решение «скачать» —
// принимать сам.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::documents::install::{self, App, Pinned, State};

/// Один CLI в том виде, в каком его показывает интерфейс.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliView {
    /// Идентификатор для команды установки: `wordcraft`, `pdfcraft`…
    pub app: &'static str,
    pub title: &'static str,
    pub version: &'static str,
    /// Обещанный размер загрузки, а не занятое место на диске.
    pub download_mb: u64,
    pub installed: bool,
    pub path: Option<String>,
}

fn view(pin: &Pinned) -> CliView {
    let (installed, path) = match install::status(pin.app) {
        Ok(State::Ready(path)) => (true, Some(path.display().to_string())),
        // «Найдена другая версия» — это не «установлено»: показывать
        // зелёную галочку, когда инструменты откажут, нельзя.
        Ok(State::Missing) | Err(_) => (false, None),
    };
    CliView {
        app: pin.app.id(),
        title: pin.app.title(),
        version: pin.version,
        download_mb: pin.archive_bytes.div_ceil(1024 * 1024),
        installed,
        path,
    }
}

fn all_views() -> Vec<CliView> {
    install::all().iter().map(view).collect()
}

#[tauri::command]
pub fn docs_cli_status() -> Result<Vec<CliView>, String> {
    crate::auth::require_authenticated()?;
    Ok(all_views())
}

#[tauri::command]
pub async fn docs_cli_install(app: AppHandle, name: String) -> Result<Vec<CliView>, String> {
    crate::auth::require_authenticated()?;
    let which = App::from_id(&name).ok_or_else(|| format!("неизвестный CLI: {name}"))?;

    let label = name.clone();
    install::install_with_progress(which, &install::root(), move |received, total| {
        let _ = app.emit(
            "docs://cli-progress",
            serde_json::json!({
                "app": label,
                "received": received,
                "total": total,
            }),
        );
    })
    .await?;

    Ok(all_views())
}

#[tauri::command]
pub async fn docs_launcher_download(app: AppHandle) -> Result<String, String> {
    crate::auth::require_authenticated()?;
    let path = crate::documents::launcher::download(move |received, total| {
        let _ = app.emit(
            "docs://cli-progress",
            serde_json::json!({
                "app": "launcher",
                "received": received,
                "total": total,
            }),
        );
    })
    .await?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn docs_launcher_status() -> Result<Option<String>, String> {
    crate::auth::require_authenticated()?;
    Ok(crate::documents::launcher::downloaded().map(|path| path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn идентификатор_переживает_превращение_туда_и_обратно() {
        for expected in App::ALL {
            assert_eq!(App::from_id(expected.id()), Some(expected));
        }
        assert_eq!(App::from_id("office"), None);
    }

    /// В интерфейсе размер обещан в мегабайтах, и он должен совпадать
    /// с тем, что человек увидит в релизе, а не быть нулём.
    #[test]
    fn размер_загрузки_показывается_в_мегабайтах() {
        for pin in install::all() {
            let view = view(pin);
            assert_eq!(view.app, pin.app.id());
            assert!(view.download_mb >= 40, "{}: {}", view.title, view.download_mb);
        }
    }
}

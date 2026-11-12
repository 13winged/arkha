// Официальный ArtCraft Launcher.
//
// Остальные приложения линейки — PhotoCraft, VectorCraft, LightCraft,
// SoundCraft, FilmCraft и прочие — ArkHa не использует. Ставить их
// внутрь себя значило бы завести второй менеджер загрузок: у них свои
// релизы раз в двое суток и свои обновления. Поэтому здесь одна
// кнопка: скачать официальный установщик в «Загрузки». Дальше человек
// запускает его сам, и Launcher ставит и обновляет всё остальное.
//
// Версия и хеш закреплены, как у четырёх CLI: скачанный установщик —
// это код, который человек запустит, и подменять его по дороге нельзя.

use std::path::PathBuf;

use super::install;

pub const VERSION: &str = "0.19.2";
pub const FILE: &str = "artcraft-launcher-0.19.2-windows-x64.msi";
pub const BYTES: u64 = 10_375_168;
pub const SHA256: &str = "fa73b211cf0a7c377f35fc998582eaa7c9f62013a1236ee4fd16204669b5bbfa";

pub fn url() -> String {
    format!(
        "https://github.com/storytold/craft-launcher/releases/download/v{VERSION}/{FILE}"
    )
}

/// Куда кладём установщик. «Загрузки» — потому что дальше его
/// запускает человек, а не мы: искать файл в недрах AppData он не
/// должен.
pub fn downloads_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Downloads")
}

pub fn target() -> PathBuf {
    downloads_dir().join(FILE)
}

/// Уже скачан и не обрезан. Хеш здесь не считаем: 10 МБ на каждый
/// показ панели — лишняя работа, а перед запуском файл всё равно
/// проверяет человек или установщик.
pub fn downloaded() -> Option<PathBuf> {
    let path = target();
    let size = std::fs::metadata(&path).ok()?.len();
    (path.is_file() && size == BYTES).then_some(path)
}

pub async fn download<F>(mut progress: F) -> Result<PathBuf, String>
where
    F: FnMut(u64, u64),
{
    if let Some(path) = downloaded() {
        return Ok(path);
    }
    let dir = downloads_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("не удалось создать {}: {error}", dir.display()))?;

    // Качаем во временное имя: оборванная загрузка не должна выглядеть
    // как готовый установщик под правильным именем.
    let staging = dir.join(format!(".{FILE}.part"));
    let url = url();
    if let Err(error) =
        install::download_with_progress(&url, SHA256, &staging, &mut progress).await
    {
        let _ = std::fs::remove_file(&staging);
        return Err(error);
    }

    let target = target();
    std::fs::rename(&staging, &target).map_err(|error| {
        let _ = std::fs::remove_file(&staging);
        format!("не удалось сохранить {}: {error}", target.display())
    })?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn закреплённый_установщик_согласован() {
        assert!(FILE.ends_with(".msi"), "{FILE}");
        assert!(FILE.contains(VERSION), "{FILE}");
        assert_eq!(SHA256.len(), 64);
        assert!(SHA256.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(BYTES > 1_000_000, "{BYTES}");
        assert!(url().starts_with("https://github.com/storytold/craft-launcher/"));
        assert!(url().ends_with(FILE));
    }

    #[test]
    fn установщик_кладётся_в_загрузки() {
        let path = target();
        assert_eq!(path.file_name().unwrap(), FILE);
        assert_eq!(
            path.parent().unwrap().file_name().unwrap(),
            "Downloads"
        );
    }
}

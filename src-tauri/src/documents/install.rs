// Установка CLI документов: найти, проверить, скачать.
//
// Решение и числа — в docs/31-документы-через-mcp.md. Коротко: четыре
// чужих CLI не кладутся в установщик целиком (286 МБ), кроме GridCraft
// (20.8 МБ). Значит нужен поиск уже установленной копии и загрузка по
// требованию.
//
// Два решения, которые здесь приняты.
//
// **Версия закреплена, а не «последняя».** Релизы выходят раз в двое
// суток; незакреплённая версия означает, что завтра у человека другой
// инструмент с другим набором схем. Поэтому в таблице ниже лежат и
// версия, и SHA-256 архива, и SHA-256 самого exe.
//
// **Проверяется exe, а не только архив.** Для скачанной копии хеш
// архива доказывает, что до нас дошло то, что опубликовано. Хеш exe
// проверяет и установленную человеком копию, которую мы не качали:
// если версия другая, мы это увидим до запуска, а не в странном ответе
// инструмента. Для точных байтов хеш сильнее подписи: подпись говорит
// «собрал издатель», хеш — «это ровно тот файл».

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

use futures_util::StreamExt;

/// Один из четырёх CLI. Имя варианта — наше, заголовок — их.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    Wordcraft,
    Pdfcraft,
    Gridcraft,
    Deckcraft,
}

impl App {
    pub const ALL: [App; 4] = [App::Wordcraft, App::Pdfcraft, App::Gridcraft, App::Deckcraft];

    /// Имя файла, который запускаем.
    pub fn exe_name(self) -> &'static str {
        match self {
            App::Wordcraft => "wordcraft-cli.exe",
            App::Pdfcraft => "pdfcraft-cli.exe",
            App::Gridcraft => "gridcraft-cli.exe",
            App::Deckcraft => "deckcraft-cli.exe",
        }
    }

    /// Имя папки у ArtCraft и в нашем дереве.
    pub fn title(self) -> &'static str {
        match self {
            App::Wordcraft => "WordCraft",
            App::Pdfcraft => "PdfCraft",
            App::Gridcraft => "GridCraft",
            App::Deckcraft => "DeckCraft",
        }
    }

    fn repo(self) -> &'static str {
        match self {
            App::Wordcraft => "wordcraft",
            App::Pdfcraft => "pdfcraft",
            App::Gridcraft => "gridcraft",
            App::Deckcraft => "deckcraft",
        }
    }

    /// Имя для интерфейса и команд. Латиницей и без заголовка: это
    /// идентификатор, а не подпись.
    pub fn id(self) -> &'static str {
        self.repo()
    }

    pub fn from_id(id: &str) -> Option<Self> {
        App::ALL.into_iter().find(|app| app.id() == id)
    }
}

/// Закреплённый выпуск. Все значения обязаны меняться вместе.
pub struct Pinned {
    pub app: App,
    pub version: &'static str,
    /// Имя архива в релизе GitHub.
    pub archive: &'static str,
    /// Размер архива: показывается человеку до скачивания.
    pub archive_bytes: u64,
    /// SHA-256 архива: его публикует GitHub как digest релиза.
    pub archive_sha256: &'static str,
    /// SHA-256 распакованного `*-cli.exe`.
    pub exe_sha256: &'static str,
}

impl Pinned {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/storytold/{}/releases/download/v{}/{}",
            self.app.repo(),
            self.version,
            self.archive
        )
    }

    /// Куда ставим скачанную копию.
    pub fn target_dir(&self) -> PathBuf {
        root().join(self.app.title()).join(self.version)
    }

    pub fn target_exe(&self) -> PathBuf {
        self.target_dir().join(self.app.exe_name())
    }
}

static PINNED: [Pinned; 4] = [
    Pinned {
        app: App::Wordcraft,
        version: "0.17.0",
        archive: "wordcraft-0.17.0-windows-x64-portable.zip",
        archive_bytes: 117_678_204,
        archive_sha256: "ad05af39d62504ddb382f93f9b7e2e7e9ce92938d132c18c4fb43e10822108ce",
        exe_sha256: "d95ccc0708fd7000e590f8046fd8c87eef9ee3ece31e53aca1d0cde83d729071",
    },
    Pinned {
        app: App::Pdfcraft,
        version: "0.20.0",
        archive: "pdfcraft-0.20.0-windows-x64-portable.zip",
        archive_bytes: 120_804_409,
        archive_sha256: "efb636a8c5c4f8041e6dbe39f56f40992c9a1b199a0b8a27cd0fc4d6037715d2",
        exe_sha256: "cd89da141252bc63c899eed2953bc9c9c7d5bc2e13c35919916636b8ed488d66",
    },
    Pinned {
        app: App::Gridcraft,
        version: "0.18.0",
        archive: "gridcraft-0.18.0-windows-x64-portable.zip",
        archive_bytes: 51_755_358,
        archive_sha256: "ee3a5249c3282840b5b99fa5a202f4356508772eeed9b41344751698dfbe364f",
        exe_sha256: "f3bf239a275e7b27232b25049dca79456f0035c37dba00dcf2d05744711d50bc",
    },
    Pinned {
        app: App::Deckcraft,
        version: "0.17.0",
        archive: "deckcraft-0.17.0-windows-x64-portable.zip",
        archive_bytes: 104_292_946,
        archive_sha256: "53f1fd1bd669c9f3fc6ecb704a749b679325b5ea27f7e86e7a51bafe9ff768d3",
        exe_sha256: "ac01133a31e0e619f0b079b8b6b995757ba8ce68d7080e35011cb3c63a4b6f52",
    },
];

pub fn all() -> &'static [Pinned] {
    &PINNED
}

pub fn pinned(app: App) -> &'static Pinned {
    all()
        .iter()
        .find(|pin| pin.app == app)
        .expect("для каждого App есть закреплённый выпуск")
}

/// Корень наших копий. Переменная окружения нужна тестам и разработке:
/// без неё тест писал бы в настоящий `%APPDATA%`.
pub fn root() -> PathBuf {
    std::env::var_os("ARKHA_DOCS_CLI_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::profile::config_dir().join("docs-cli"))
}

/// Что делать с CLI: использовать или скачивать.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Ready(PathBuf),
    Missing,
}

/// Найти рабочую копию. Уже установленная копия проверяется по хешу:
/// «файл есть» и «файл тот» — разные утверждения.
pub fn status(app: App) -> Result<State, String> {
    locate_in(app, &root())
}

/// Найти или скачать. Возвращает путь к проверенному exe.
pub fn ensure(app: App) -> Result<PathBuf, String> {
    match status(app)? {
        State::Ready(path) => Ok(path),
        State::Missing => install(app),
    }
}

/// Поиск в явно заданном корне. Отдельно от `status`, чтобы тест не
/// зависел от переменной окружения и не трогал настоящий профиль.
fn locate_in(app: App, root: &Path) -> Result<State, String> {
    let pin = pinned(app);
    let mut broken: Vec<String> = Vec::new();

    for candidate in candidates(app, root) {
        if !candidate.is_file() {
            continue;
        }
        match verify(&candidate, pin) {
            Ok(()) => return Ok(State::Ready(candidate)),
            Err(error) => broken.push(format!("{}: {error}", candidate.display())),
        }
    }

    if broken.is_empty() {
        Ok(State::Missing)
    } else {
        // Найденный, но не тот файл — это не «нет CLI». Скачивать поверх
        // чужой версии молча нельзя: человек мог поставить её сам.
        Err(format!(
            "найдена другая версия {} (нужна {}): {}",
            app.exe_name(),
            pin.version,
            broken.join("; ")
        ))
    }
}

/// Где искать. Наши копии — первыми: они проверены по хешу.
fn candidates(app: App, root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![
        root.join(app.title())
            .join(pinned(app).version)
            .join(app.exe_name()),
    ];

    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(
            PathBuf::from(local)
                .join("Programs")
                .join("ArtCraft")
                .join(app.title())
                .join(app.exe_name()),
        );
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        let base = PathBuf::from(program_files);
        paths.push(base.join("ArtCraft").join(app.title()).join(app.exe_name()));
        paths.push(base.join(app.title()).join(app.exe_name()));
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles(x86)") {
        paths.push(
            PathBuf::from(program_files)
                .join("ArtCraft")
                .join(app.title())
                .join(app.exe_name()),
        );
    }
    paths
}

/// Сверить файл с закреплённым выпуском.
pub fn verify(path: &Path, pin: &Pinned) -> Result<(), String> {
    verify_hash(path, pin.exe_sha256)
}

fn verify_hash(path: &Path, expected: &str) -> Result<(), String> {
    let actual = crate::downloader::sha256_file(path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!("SHA-256 не совпал: ожидался {expected}, получен {actual}"))
    }
}

/// Скачать закреплённый выпуск и положить рядом с нашими данными.
pub fn install(app: App) -> Result<PathBuf, String> {
    install_into(app, &root())
}

fn target_paths(app: App, root: &Path) -> (PathBuf, PathBuf) {
    let target = root.join(app.title()).join(pinned(app).version);
    let exe = target.join(app.exe_name());
    (target, exe)
}

/// Уже установленная копия — или «можно ставить».
fn already_there(app: App, root: &Path) -> Result<Option<PathBuf>, String> {
    let (target, exe) = target_paths(app, root);
    if exe.is_file() {
        verify(&exe, pinned(app))?;
        return Ok(Some(exe));
    }
    if target.exists() {
        return Err(format!(
            "папка {} уже есть, но {} в ней нет или он не прошёл проверку; \
             удалите её и повторите",
            target.display(),
            app.exe_name()
        ));
    }
    Ok(None)
}

fn temp_archive(app: App) -> PathBuf {
    std::env::temp_dir().join(format!(
        "arkha-docs-{}-{}.zip",
        app.title(),
        std::process::id()
    ))
}

fn checked_url(app: App) -> Result<String, String> {
    let url = pinned(app).url();
    if !url.starts_with("https://") {
        return Err("CLI можно загружать только по HTTPS".into());
    }
    Ok(url)
}

/// То же, но в явно заданный корень. Нужно гейту: он не должен
/// трогать настоящий профиль и не должен менять переменные окружения.
pub fn install_into(app: App, root: &Path) -> Result<PathBuf, String> {
    if let Some(exe) = already_there(app, root)? {
        return Ok(exe);
    }
    let url = checked_url(app)?;
    let temp = temp_archive(app);
    if let Err(error) = download(&url, pinned(app).archive_sha256, &temp) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    let result = finish(app, root, &temp);
    let _ = std::fs::remove_file(&temp);
    result
}

/// Скачивание по кнопке: то же, но с отчётом о прогрессе.
///
/// Отчёт нужен не для красоты. 120 МБ без обратной связи выглядят как
/// зависшее приложение, и человек либо закрывает его, либо начинает
/// искать сломанный интернет.
pub async fn install_with_progress<F>(
    app: App,
    root: &Path,
    mut progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(u64, u64),
{
    if let Some(exe) = already_there(app, root)? {
        return Ok(exe);
    }
    let url = checked_url(app)?;
    let temp = temp_archive(app);
    if let Err(error) =
        download_with_progress(&url, pinned(app).archive_sha256, &temp, &mut progress).await
    {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    let result = finish(app, root, &temp);
    let _ = std::fs::remove_file(&temp);
    result
}

/// Распаковать, проверить и поставить на место.
fn finish(app: App, root: &Path, archive: &Path) -> Result<PathBuf, String> {
    let pin = pinned(app);
    let (target, _) = target_paths(app, root);
    let staging = target
        .parent()
        .expect("у целевой папки есть родитель")
        .join(format!(
            ".staging-{}-{}-{}",
            app.title(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
    let extracted = extract(archive, pin, &staging);
    let staged_exe = match extracted {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if let Err(error) = verify(&staged_exe, pin) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!(
            "распакованный {} не прошёл проверку: {error}",
            app.exe_name()
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&staging, &target).map_err(|error| {
        let _ = std::fs::remove_dir_all(&staging);
        format!("не удалось переименовать {}: {error}", target.display())
    })?;
    Ok(target.join(app.exe_name()))
}

/// Скачать в файл, считая хеш по дороге. Хеш проверяется до распаковки:
/// распаковывать непроверенный архив — то же самое, что запускать его.
fn download(url: &str, expected: &str, destination: &Path) -> Result<(), String> {
    let mut response = Client::builder()
        .user_agent(concat!("arkha/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("не удалось создать HTTP-клиент: {error}"))?
        .get(url)
        .send()
        .map_err(|error| format!("не удалось начать загрузку: {error}"))?
        .error_for_status()
        .map_err(|error| format!("сервер ответил ошибкой: {error}"))?;

    let mut file = std::fs::File::create(destination)
        .map_err(|error| format!("не удалось создать {}: {error}", destination.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| format!("загрузка прервана: {error}"))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|error| format!("не удалось записать файл: {error}"))?;
    }
    file.flush().map_err(|error| error.to_string())?;

    let actual = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected {
        return Err(format!("SHA-256 архива не совпал: ожидался {expected}, получен {actual}"));
    }
    Ok(())
}

/// Асинхронная загрузка с отчётом. Хеш считается по дороге и
/// проверяется до распаковки — как и в синхронной версии.
async fn download_with_progress<F>(
    url: &str,
    expected: &str,
    destination: &Path,
    progress: &mut F,
) -> Result<(), String>
where
    F: FnMut(u64, u64),
{
    let response = reqwest::Client::builder()
        .user_agent(concat!("arkha/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("не удалось создать HTTP-клиент: {error}"))?
        .get(url)
        .send()
        .await
        .map_err(|error| format!("не удалось начать загрузку: {error}"))?
        .error_for_status()
        .map_err(|error| format!("сервер ответил ошибкой: {error}"))?;

    let total = response.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(destination)
        .map_err(|error| format!("не удалось создать {}: {error}", destination.display()))?;
    let mut hash = Sha256::new();
    let mut received: u64 = 0;
    let mut reported: u64 = 0;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("загрузка прервана: {error}"))?;
        hash.update(&chunk);
        file.write_all(&chunk)
            .map_err(|error| format!("не удалось записать файл: {error}"))?;
        received += chunk.len() as u64;
        // Событие на каждый мегабайт — достаточно часто, чтобы полоса
        // двигалась, и достаточно редко, чтобы не забить канал.
        if received - reported >= 1024 * 1024 {
            reported = received;
            progress(received, total);
        }
    }
    file.flush().map_err(|error| error.to_string())?;
    progress(received, total);

    let actual = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected {
        return Err(format!("SHA-256 архива не совпал: ожидался {expected}, получен {actual}"));
    }
    Ok(())
}

/// Достать из архива только exe и лицензии.
///
/// Распаковывать весь архив нельзя: там 140 МБ приложения, шрифты и
/// чужие файлы. Нам нужен один бинарник и тексты лицензий, которые
/// обязаны ехать вместе с ним.
fn extract(archive: &Path, pin: &Pinned, staging: &Path) -> Result<PathBuf, String> {
    let file = std::fs::File::open(archive)
        .map_err(|error| format!("не удалось открыть архив: {error}"))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| format!("архив не разобрался: {error}"))?;

    std::fs::create_dir_all(staging).map_err(|error| error.to_string())?;
    let mut executable: Option<PathBuf> = None;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| format!("архив не читается: {error}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = Path::new(entry.name())
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let keep = name == pin.app.exe_name() || is_license(&name);
        if !keep {
            continue;
        }

        let destination = staging.join(&name);
        let mut output = std::fs::File::create(&destination)
            .map_err(|error| format!("не удалось создать {}: {error}", destination.display()))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("не удалось распаковать {name}: {error}"))?;
        if name == pin.app.exe_name() {
            executable = Some(destination);
        }
    }

    executable.ok_or_else(|| format!("в архиве нет {}", pin.app.exe_name()))
}

fn is_license(name: &str) -> bool {
    name.starts_with("LICENSE") || name == "NOTICE" || name == "ATTRIBUTION.md"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "arkha-docs-install-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Таблица — единственное место, где версия и оба хеша должны
    /// сходиться. Опечатка здесь означает запуск «того же» CLI, но
    /// другого, поэтому проверяем форму, а не только наличие.
    #[test]
    fn закреплённые_выпуски_согласованы() {
        assert_eq!(all().len(), App::ALL.len());
        for app in App::ALL {
            let pin = pinned(app);
            assert_eq!(pin.app, app);
            assert!(pin.version.split('.').count() == 3, "версия {}", pin.version);
            assert!(pin.archive.ends_with(".zip"), "{}", pin.archive);
            assert!(pin.archive.contains(pin.version), "{}", pin.archive);
            assert!(pin.archive_bytes > 10_000_000, "размер архива {}", pin.archive_bytes);
            assert_eq!(pin.archive_sha256.len(), 64);
            assert_eq!(pin.exe_sha256.len(), 64);
            assert!(pin.archive_sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(pin.exe_sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(pin.url().starts_with("https://github.com/storytold/"));
            assert!(pin.url().contains(pin.version));
        }
    }

    #[test]
    fn хеш_файла_сверяется_и_расхождение_видно() {
        let dir = temp_dir("hash");
        let path = dir.join("wordcraft-cli.exe");
        std::fs::write(&path, b"not really a cli").unwrap();
        let real = crate::downloader::sha256_file(&path).unwrap();

        verify_hash(&path, &real).expect("свой хеш должен подойти");

        let wrong = "0".repeat(64);
        let error = verify_hash(&path, &wrong).expect_err("чужой хеш должен отказать");
        assert!(error.contains("SHA-256"), "{error}");
        assert!(error.contains(&wrong), "{error}");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Пустой корень — это «не установлено», а не ошибка.
    #[test]
    fn в_пустом_корне_ничего_не_находится() {
        let dir = temp_dir("empty");
        assert_eq!(locate_in(App::Gridcraft, &dir).unwrap(), State::Missing);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Найденный, но не тот файл — отказ с путём и версией, а не
    /// молчаливая попытка скачать поверх чужой установки.
    #[test]
    fn чужая_версия_в_нашем_дереве_называется_ошибкой() {
        let dir = temp_dir("wrong");
        let pin = pinned(App::Deckcraft);
        let folder = dir.join(pin.app.title()).join(pin.version);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(pin.app.exe_name()), b"other build").unwrap();

        let error = locate_in(App::Deckcraft, &dir).expect_err("должна быть ошибка");
        assert!(error.contains(pin.version), "{error}");
        assert!(error.contains("DeckCraft"), "{error}");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Из архива берём exe и лицензии, остальное не распаковываем.
    /// Проверка важна не ради места: распакованный `README.md` — это
    /// файл, которого мы не проверяли и не обещали.
    #[test]
    fn из_архива_берутся_только_exe_и_лицензии() {
        let dir = temp_dir("extract");
        let archive = dir.join("test.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("wordcraft-0.17.0/wordcraft-cli.exe", "cli"),
                ("wordcraft-0.17.0/README.md", "docs"),
                ("wordcraft-0.17.0/LICENSE-APACHE", "license"),
                ("wordcraft-0.17.0/NOTICE", "notice"),
                ("../evil.txt", "escape"),
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }

        let pin = pinned(App::Wordcraft);
        let staging = dir.join("staging");
        let exe = extract(&archive, pin, &staging).expect("exe должен найтись");

        assert_eq!(exe.file_name().unwrap(), "wordcraft-cli.exe");
        assert!(staging.join("LICENSE-APACHE").is_file());
        assert!(staging.join("NOTICE").is_file());
        assert!(!staging.join("README.md").exists(), "README не нужен");
        assert!(!staging.join("evil.txt").exists(), "путь из архива не должен убегать");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn архив_без_exe_это_отказ() {
        let dir = temp_dir("noexe");
        let archive = dir.join("test.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("LICENSE", zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"license").unwrap();
            zip.finish().unwrap();
        }
        let error = extract(&archive, pinned(App::Pdfcraft), &dir.join("staging"))
            .expect_err("без exe распаковка не считается успешной");
        assert!(error.contains("pdfcraft-cli.exe"), "{error}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

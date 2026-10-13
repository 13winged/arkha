//! Гейт S2 для загрузчика: работа против настоящего Hugging Face.
//!
//! Тяжёлый тест, качать гигабайты не будем. Проверяем то, что
//! ломается тихо: разбор ответа дерева, поиск файла, отказ качать
//! без сводки, отмена, докачка с середины файла.
//!
//! Запуск:
//!   cargo test --test downloader_gate -- --nocapture --ignored

use archotic_lib::downloader::{self, RemoteFile, RemoteRepo};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const REPO: &str = "unsloth/Qwen3.5-4B-GGUF";
const FILE: &str = "Qwen3.5-4B-Q4_K_M.gguf";

#[tokio::test]
#[ignore = "ходит в сеть, запускается вручную"]
async fn дерево_разбирается_и_файл_находится() {
    let repo = downloader::fetch_repo(REPO)
        .await
        .expect("репозиторий должен прочитаться");

    eprintln!("репозиторий: {}", repo.repo);
    eprintln!("лицензия: {}", repo.license);
    eprintln!("файлов: {}", repo.files.len());

    assert!(!repo.files.is_empty());
    assert!(repo.license.contains("apache"), "лицензия {}", repo.license);

    let file = downloader::find_file(&repo, FILE).expect("файл должен найтись");
    eprintln!("{FILE}: {} байт, sha256 {}", file.size, file.sha256);

    assert!(file.size > 100_000_000, "файл подозрительно мал");
    assert_eq!(
        file.sha256.len(),
        64,
        "сводка должна быть sha256 в 64 символа"
    );
    assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
#[ignore = "ходит в сеть, запускается вручную"]
async fn список_доменов_доступен() {
    let hosts = downloader::required_hosts()
        .await
        .expect("список доменов должен читаться");

    eprintln!("доменов: {}", hosts.len());
    for h in &hosts {
        eprintln!("  {h}");
    }

    assert!(hosts.iter().any(|h| h == "huggingface.co"));
    assert!(
        hosts.iter().any(|h| h.contains("cdn.hf.co")),
        "в списке нет доменов раздачи, через них качается тело файла"
    );
    assert!(
        hosts.iter().any(|h| h.contains("lfs.hf.co")),
        "в списке нет доменов крупных файлов"
    );
    // Мы не используем протокол Xet, поэтому его серверы в списке
    // для нас лишние и показывать их пользователю не надо.
    assert!(!hosts.iter().any(|h| h.starts_with("cas-server")));
}

/// Полный `.part` обязан идти на сверку, а не упираться в 416.
///
/// Случай найден на живой загрузке 9B: сеть оборвала соединение, но
/// `.part` успел дописаться целиком. Следующий запуск отправлял
/// `Range: bytes={size}-`, сервер отвечал 416 — и пользователь оказывался
/// заблокирован: файл полон, скачать нельзя, удалить надо вручную.
///
/// Тест без сети и без гигабайтов. Сводка указана, но заведомо
/// неверная: значит сверка обязана провалиться. Различаем два исхода:
/// «сводка не совпала» — загрузчик пошёл по правильной ветке;
/// «416» — полез в сеть с запросом за пределами файла.
///
/// С пустой сводкой тест был бы пустым: отказ по ней стоит в самом
/// начале функции, до сетевого кода, и прошёл бы и со старым багом.
#[tokio::test]
async fn полный_part_идёт_на_сверку_а_не_в_сеть() {
    let content = vec![7u8; 2048];
    let fake = RemoteFile {
        path: "модель.gguf".into(),
        size: content.len() as u64,
        // Заведомо не та сводка: сверка обязана её отвергнуть.
        sha256: "0".repeat(64),
    };

    let dest = std::env::temp_dir().join("archotic-test-complete");
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).unwrap();

    // `.part` полного размера — ровно то состояние, в котором
    // загрузчик раньше упирался в 416.
    let part = dest.join("модель.gguf.part");
    std::fs::write(&part, &content).unwrap();
    assert_eq!(std::fs::metadata(&part).unwrap().len(), content.len() as u64);

    let err = downloader::download(
        // Репозиторий заведомо не существует: если загрузчик дойдёт
        // до сети, тест упадёт с сетевой ошибкой, а не пройдёт.
        "archotic-нет-такого/репозитория",
        &fake,
        &dest,
        Arc::new(AtomicBool::new(false)),
        Box::new(|_| {}),
    )
    .await
    .unwrap_err();

    eprintln!("ошибка: {err}");
    assert!(
        !err.contains("416") && !err.contains("Range Not Satisfiable"),
        "загрузчик пошёл в сеть вместо сверки: {err}"
    );
    assert!(
        err.contains("сводка не совпала"),
        "ожидался отказ по сводке, получено: {err}"
    );
    // Битый файл удаляется, иначе следующий запуск повторит то же.
    assert!(!part.exists(), "файл с неверной сводкой должен быть удалён");
    assert!(!dest.join("модель.gguf").exists(), "финальный файл не создан");
}

#[tokio::test]
#[ignore = "ходит в сеть, запускается вручную"]
async fn файла_без_сводки_качать_нельзя() {
    // README не в LFS, у него нет lfs.oid.
    let fake = RemoteFile {
        path: "README.md".into(),
        size: 1234,
        sha256: String::new(),
    };
    let dest = std::env::temp_dir().join("archotic-test-nosha");
    let cancel = Arc::new(AtomicBool::new(false));

    let err = downloader::download(
        "x/y",
        &fake,
        &dest,
        cancel,
        Box::new(|_| {}),
    )
    .await
    .unwrap_err();

    eprintln!("ошибка: {err}");
    assert!(err.contains("нет сводки"));
    assert!(!dest.exists() || std::fs::read_dir(&dest).unwrap().count() == 0);
}

/// Докачка: файл `.part` уже наполовину полон, загрузчик обязан
/// продолжить, а не начать заново.
#[tokio::test]
#[ignore = "ходит в сеть, качает настоящий файл"]
async fn докачка_идёт_с_места_остановки() {
    let repo = downloader::fetch_repo(REPO).await.expect("репозиторий");
    let file = downloader::find_file(&repo, FILE).expect("файл");

    let dest = std::env::temp_dir().join("archotic-test-resume");
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).unwrap();

    let part = dest.join(format!(
        "{}.part",
        file.path.rsplit('/').next().unwrap()
    ));

    // Имитируем половину скачанного.
    let fake_half = (file.size / 2) as usize;
    let blob: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
    let mut f = std::fs::File::create(&part).unwrap();
    use std::io::Write;
    while f.metadata().unwrap().len() < fake_half as u64 {
        f.write_all(&blob).unwrap();
    }
    f.sync_all().unwrap();
    drop(f);

    let before = part.metadata().unwrap().len();
    eprintln!("подготовлено {} байт из {}", before, file.size);

    // Отменяем сразу: интересует только то, что запрос ушёл с Range.
    let cancel = Arc::new(AtomicBool::new(true));
    let saw_downloading = Arc::new(AtomicBool::new(false));

    let watcher = Arc::clone(&saw_downloading);
    let _ = downloader::download(
        REPO,
        file,
        &dest,
        cancel,
        Box::new(move |p| {
            if matches!(p, downloader::Progress::Downloading { .. }) {
                watcher.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }),
    )
    .await;

    // Отмена не должна была стереть докачанное.
    let after = part.metadata().map(|m| m.len()).unwrap_or(0);
    eprintln!("после отмены: {after} байт");

    // Проверка ниже имеет смысл, только если запись вообще началась.
    // Иначе тест проверяет не отмену, а что-то другое, и будет зелёным
    // при сломанной отмене.
    assert!(
        saw_downloading.load(std::sync::atomic::Ordering::SeqCst),
        "запись не началась, отмена не проверена"
    );

    assert!(
        after >= before,
        "отмена стёрла уже скачанное: было {before}, стало {after}"
    );
    assert_eq!(
        after, before,
        "отмена должна остановить запись, а не продолжать её"
    );

    let _ = std::fs::remove_dir_all(&dest);
    let _ = saw_downloading;
}

/// Полная проверка на настоящей модели: скачать, сверить сводку,
/// положить в папку моделей.
///
/// Это и есть гейт S2 для загрузчика: приложение должно само
/// получить то, что ему нужно. Доказан только тем, что файл реально
/// скачан и сверен.
///
/// Путь к папке задаётся переменной ARCHOTIC_MODELS_DIR.
/// Запуск:
///   ARCHOTIC_MODELS_DIR=%APPDATA%\Archotic\models \
///     cargo test --test downloader_gate скачать_модель_целиком -- --ignored
#[tokio::test]
#[ignore = "качает гигабайты, запускается вручную"]
async fn скачать_модель_целиком() {
    // Модель выбирается переменными окружения, иначе каждый замер
    // требовал бы правки кода.
    let repo_id = std::env::var("ARCHOTIC_TEST_REPO").unwrap_or_else(|_| REPO.to_string());
    let file_id = std::env::var("ARCHOTIC_TEST_FILE").unwrap_or_else(|_| FILE.to_string());

    let repo = downloader::fetch_repo(&repo_id).await.expect("репозиторий");
    let file = downloader::find_file(&repo, &file_id).expect("файл");

    let dest = std::env::var("ARCHOTIC_MODELS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("archotic-models"));

    eprintln!("качаем {file_id} из {repo_id}");
    eprintln!("  {} байт в {}", file.size, dest.display());
    eprintln!("  сводка репозитория: {}", file.sha256);
    let started = std::time::Instant::now();

    let path = downloader::download(
        &repo_id,
        file,
        &dest,
        Arc::new(AtomicBool::new(false)),
        Box::new(|p| {
            if let downloader::Progress::Downloading { downloaded, total, mib_per_s } = &p {
                if downloaded % (128 * 1024 * 1024) < 1024 * 1024 {
                    eprintln!("  {downloaded}/{total} байт, {mib_per_s:.1} МБ/с");
                }
            }
        }),
    )
    .await
    .expect("скачивание должно пройти");

    let secs = started.elapsed().as_secs_f32();
    eprintln!(
        "готово за {:.0} сек, {:.1} МБ/с",
        secs,
        (file.size as f32 / secs) / (1024.0 * 1024.0)
    );

    assert!(path.exists());
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        file.size,
        "размер на диске должен совпасть с заявленным"
    );

    // Сверяем повторно, независимым вызовом: файл мог повредиться
    // при переносе, и заметить это должен пользователь при запуске.
    let digest = downloader::sha256_file(&path).unwrap();
    assert_eq!(
        digest, file.sha256,
        "сводка скачанного файла разошлась с репозиторием"
    );
    eprintln!("сводка сошлась: {}", &digest[..16]);
}

/// Без сети и без файлов: проверяем решения, которые не требуют сети.
#[test]
fn сводка_считается_по_файлу() {
    let dir = std::env::temp_dir().join("archotic-test-sha");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("f.bin");
    std::fs::write(&p, b"archotic").unwrap();

    let got = downloader::sha256_file(&p).unwrap();
    eprintln!("sha256(archotic) = {got}");

    // Эталон посчитан независимо, средствами Windows, не нашим кодом:
    // certutil и Get-FileHash дали одно и то же значение.
    assert_eq!(
        got,
        "b5e26e99969ba821499d0c99e3eed1ad113b724b31d9fb1c208288c760141d22"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn файл_без_сводки_определяется_по_размеру() {
    let r = RemoteRepo {
        repo: "x/y".into(),
        license: "apache-2.0".into(),
        files: vec![
            RemoteFile { path: "a.gguf".into(), size: 10, sha256: "aa".into() },
            RemoteFile { path: "b.gguf".into(), size: 20, sha256: String::new() },
        ],
    };
    let f = downloader::find_file(&r, "b.gguf").unwrap();
    assert!(f.sha256.is_empty());
    assert_eq!(f.size, 20);
}

#[tokio::test]
async fn отказ_до_создания_папок() {
    let base = std::env::temp_dir().join("archotic-test-mkdir");
    let _ = std::fs::remove_dir_all(&base);
    let nested = base.join("a").join("b");

    // Файл без сводки качать нельзя. Проверка идёт до создания папок,
    // и это правильно: отказавший запрос не должен оставлять следов.
    let f = RemoteFile { path: "x.gguf".into(), size: 1, sha256: String::new() };
    let err = downloader::download(
        "r/r",
        &f,
        &nested,
        Arc::new(AtomicBool::new(false)),
        Box::new(|_| {}),
    )
    .await
    .unwrap_err();

    assert!(err.contains("нет сводки"));
    assert!(
        !nested.exists(),
        "папка не должна создаваться, если качать отказались"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn путь_сборки_совпадает_с_ожиданием() {
    let url = downloader::FILE_URL
        .replace("{repo}", "unsloth/Qwen3.5-4B-GGUF")
        .replace("{path}", FILE);
    assert_eq!(
        url,
        "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf"
    );

    // Адрес должен быть https: файл на 5-6 ГБ не должен уходить
    // по открытому каналу.
    assert!(url.starts_with("https://"));

    let tree = downloader::TREE_URL.replace("{repo}", "unsloth/Qwen3.5-4B-GGUF");
    assert!(tree.starts_with("https://"));
    assert!(tree.ends_with("/tree/main?recursive=true"));
}
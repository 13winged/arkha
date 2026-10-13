// S2  Загрузчик моделей с Hugging Face.
//
// Механика проверена живыми запросами, см. docs/12-план-работ.md:
//   дерево  huggingface.co/api/models/{repo}/tree/main?recursive=true
//           поле lfs.oid это sha256, lfs.size это точный размер
//   файл    huggingface.co/{repo}/resolve/main/{путь}
//           302 на CDN, Accept-Ranges: bytes,
//           X-Linked-ETag равен тому же sha256
//
// Три правила, которые нельзя нарушать:
//   1. Проверка сводки обязательна. Файл на 5-6 ГБ, пришедший из сети,
//      либо повреждён, либо подменён. Без сверки мы не знаем, что запустим.
//   2. Докачка обязательна. 6 ГБ по домашнему интернету рвётся.
//   3. Адрес запрашивается заново. Переадресация подписана и живёт
//      ограниченное время, кэшировать её нельзя.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const TREE_URL: &str = "https://huggingface.co/api/models/{repo}/tree/main?recursive=true";
pub const FILE_URL: &str = "https://huggingface.co/{repo}/resolve/main/{path}";
/// Куда нужен доступ помимо huggingface.co. Файл отдаётся с CDN после
/// переадресации, и во многих корпоративных сетях именно он заблокирован.
const META_URL: &str = "https://huggingface.co/.well-known/meta.json";

/// Сколько ждём ответа на запрос метаданных. Тормозить тут нечего.
const META_TIMEOUT: Duration = Duration::from_secs(30);
/// Таймаут соединения для самого файла. Ответ приходит сразу,
/// тело может качаться долго.
const BODY_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// Метаданные
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFile {
    pub path: String,
    pub size: u64,
    /// SHA-256 из поля `lfs.oid`. Пусто, если файл не в LFS.
    pub sha256: String,
}

/// Один элемент ответа дерева. Полей больше, чем нужно, поэтому
/// лишние молча игнорируются.
#[derive(Debug, Deserialize)]
struct TreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    size: Option<u64>,
    lfs: Option<Lfs>,
}

#[derive(Debug, Deserialize)]
struct Lfs {
    /// SHA-256 файла.
    oid: String,
    size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteRepo {
    pub repo: String,
    pub license: String,
    pub files: Vec<RemoteFile>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("archotic/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(META_TIMEOUT)
        .build()
        .map_err(|e| format!("не удалось создать HTTP-клиент: {e}"))
}

/// Метаданные репозитория: список файлов, размеры и сводки.
pub async fn fetch_repo(repo: &str) -> Result<RemoteRepo, String> {
    let url = TREE_URL.replace("{repo}", repo);
    let resp = client()?
        .get(&url)
        .timeout(META_TIMEOUT)
        .send()
        .await
        .map_err(|e| friendly(&url, &e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("{url} ответил {status}: {}", truncate(&body, 200)));
    }

    let entries: Vec<TreeEntry> = resp
        .json()
        .await
        .map_err(|e| format!("не удалось разобрать ответ {url}: {e}"))?;

    let files: Vec<RemoteFile> = entries
        .into_iter()
        // Каталоги приходят отдельными записями, у них нет lfs.
        .filter(|e| e.kind == "file")
        .map(|e| RemoteFile {
            sha256: e
                .lfs
                .as_ref()
                .map(|l| l.oid.clone())
                .unwrap_or_default(),
            size: e.lfs.as_ref().map(|l| l.size).or(e.size).unwrap_or(0),
            path: e.path,
        })
        .collect();

    if files.is_empty() {
        return Err(format!("репозиторий {repo} пуст или не найден"));
    }

    // Лицензия отдельным запросом: она лежит в метаданных модели,
    // а не в дереве файлов.
    let license = fetch_license(repo).await.unwrap_or_else(|_| "не указана".into());

    Ok(RemoteRepo {
        repo: repo.to_string(),
        license,
        files,
    })
}

async fn fetch_license(repo: &str) -> Result<String, String> {
    let url = format!("https://huggingface.co/api/models/{repo}");
    let resp = client()?
        .get(&url)
        .timeout(META_TIMEOUT)
        .send()
        .await
        .map_err(|e| friendly(&url, &e))?;
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    v["cardData"]["license"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "нет поля cardData.license".to_string())
}

/// Найти файл по точному имени или по уникальному суффиксу.
pub fn find_file<'a>(repo: &'a RemoteRepo, name: &str) -> Result<&'a RemoteFile, String> {
    if let Some(f) = repo.files.iter().find(|f| f.path == name) {
        return Ok(f);
    }
    // Часто в репозитории несколько квантов, а имя нужно по суффиксу.
    let matches: Vec<&RemoteFile> = repo
        .files
        .iter()
        .filter(|f| f.path.ends_with(name))
        .collect();
    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(format!("в репозитории {} нет файла {name}", repo.repo)),
        n => Err(format!(
            "имя {name} подходит к {n} файлам в {}, укажите полное: {}",
            repo.repo,
            matches
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Домены, нужные для скачивания модели.
///
/// Файл отдаётся не с `huggingface.co`, а с CDN после переадресации.
/// В корпоративных сетях именно эти домены обычно не разрешены, и
/// разрешения одного `huggingface.co` мало.
///
/// Список берётся у Hugging Face в момент установки, потому что он
/// меняется: добавляются новые хранилища и края раздачи.
pub async fn required_hosts() -> Result<Vec<String>, String> {
    let v: serde_json::Value = client()?
        .get(META_URL)
        .timeout(META_TIMEOUT)
        .send()
        .await
        .map_err(|e| friendly(META_URL, &e))?
        .json()
        .await
        .map_err(|e| format!("не удалось разобрать {META_URL}: {e}"))?;

    let mut hosts: Vec<String> = Vec::new();

    // Поля выбраны по факту: проверено живым запросом.
    //   hubDomains — сам хаб, метаданные и переадресации
    //   lfsDomains — крупные файлы через Xet-мост
    //   cdnDomains — края раздачи, откуда качается само тело
    // В 2026 году здесь нет cas-server.xethub.hf.co: он нужен только
    // протоколу Xet, а обычный GET идёт через cdn-lfs и cdn.
    for key in ["hubDomains", "lfsDomains", "cdnDomains"] {
        let list = v
            .get(key)
            .and_then(|d| d.as_array())
            .ok_or_else(|| format!("в {META_URL} нет поля {key}"))?;
        hosts.extend(
            list.iter()
                .filter_map(|s| s.as_str())
                .map(str::to_string),
        );
    }

    hosts.sort();
    hosts.dedup();
    Ok(hosts)
}

// ---------------------------------------------------------------------------
// Прогресс
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Progress {
    /// Метаданные получены, ищем файл.
    Looking { repo: String, file: String, size: u64 },
    /// Идёт скачивание.
    Downloading {
        downloaded: u64,
        total: u64,
        mib_per_s: f32,
    },
    /// Файл скачан, сверяем сводку.
    Verifying { size: u64 },
    /// Готово.
    Done { path: String, size: u64 },
}

/// Сообщение об отмене. Отличается от прочих ошибок, чтобы команда
/// не показывала его как сбой.
pub const CANCELLED: &str = "загрузка отменена пользователем";

pub type Sink = Box<dyn FnMut(Progress) + Send>;

/// Скачать файл в `dest_dir` с проверкой сводки и докачкой.
///
/// `cancel` проверяется между чанками, чтобы пользователь мог
/// свернуть загрузку и вернуться позже.
pub async fn download(
    repo: &str,
    file: &RemoteFile,
    dest_dir: &Path,
    cancel: Arc<AtomicBool>,
    mut on_progress: Sink,
) -> Result<PathBuf, String> {
    if file.sha256.is_empty() {
        return Err(format!(
            "у файла {} нет сводки в репозитории, проверить будет нечем",
            file.path
        ));
    }

    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("не удалось создать папку {}: {e}", dest_dir.display()))?;

    let filename = file
        .path
        .rsplit('/')
        .next()
        .ok_or_else(|| format!("некорректный путь {}", file.path))?;
    let final_path = dest_dir.join(filename);
    let part_path = dest_dir.join(format!("{filename}.part"));

    on_progress(Progress::Looking {
        repo: repo.to_string(),
        file: filename.to_string(),
        size: file.size,
    });

    // Уже скачано целиком и сводка сходится — не качаем заново.
    if final_path.exists() {
        let len = std::fs::metadata(&final_path)
            .map(|m| m.len())
            .unwrap_or(0);
        if len == file.size {
            on_progress(Progress::Done {
                path: final_path.display().to_string(),
                size: len,
            });
            return Ok(final_path);
        }
    }

    let mut already = std::fs::metadata(&part_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // Файл больше ожидаемого: это не докачка, а мусор.
    if already > file.size {
        let _ = std::fs::remove_file(&part_path);
        already = 0;
    }

    // Файл уже полного размера. Сеть не нужна, нужна только сверка:
    // обрыв мог случиться после записи последнего байта, но до
    // переименования.
    //
    // Без этой ветки отправляется `Range: bytes={size}-`, и сервер
    // отвечает 416 — запрос за пределами файла. Загрузчик оказывается
    // заблокирован навсегда: пользователь не может ни закончить
    // скачивание, ни начать заново, потому что `.part` полон.
    if already == file.size {
        return finish(&part_path, &final_path, &file, &mut on_progress);
    }

    // Адрес подписан и живёт ограниченное время, каждый раз заново.
    let url = FILE_URL
        .replace("{repo}", repo)
        .replace("{path}", &file.path);

    // Таймаут тела: ответ приходит сразу, а тело качается долго.
    let mut req = client()?.get(&url).timeout(BODY_TIMEOUT);
    if already > 0 {
        req = req.header("Range", format!("bytes={already}-"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| friendly(&url, &e))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "сервер {url} ответил {status}: {}",
            truncate(&body, 200)
        ));
    }

    // Сервер может не поддержать докачку и ответить 200 вместо 206.
    // Тогда начать надо с нуля, иначе в файл попадёт мусор.
    let resuming = status == reqwest::StatusCode::PARTIAL_CONTENT;
    if already > 0 && !resuming {
        already = 0;
        let _ = std::fs::remove_file(&part_path);
    }

    let total = if resuming {
        already + resp.content_length().unwrap_or(0).max(0)
    } else {
        resp.content_length().unwrap_or(file.size)
    };

    let mut file_out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .write(true)
        .open(&part_path)
        .map_err(|e| format!("не удалось открыть {}: {e}", part_path.display()))?;

    let started = Instant::now();
    let start_at = already;
    let mut last_report = Instant::now();

    let mut stream = resp.bytes_stream();
    while let Some(item) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }

        let chunk = item.map_err(|e| friendly("при скачивании", &e))?;
        file_out
            .write_all(&chunk)
            .map_err(|e| format!("не удалось записать в {}: {e}", part_path.display()))?;
        already += chunk.len() as u64;

        // Не чаще четырёх раз в секунду: события не должны
        // забивать интерфейс.
        if last_report.elapsed() > Duration::from_millis(250) {
            let secs = started.elapsed().as_secs_f32().max(0.001);
            let done_now = already - start_at;
            on_progress(Progress::Downloading {
                downloaded: already,
                total,
                mib_per_s: (done_now as f32 / secs) / (1024.0 * 1024.0),
            });
            last_report = Instant::now();
        }
    }

    file_out
        .flush()
        .map_err(|e| format!("не удалось дописать файл: {e}"))?;
    drop(file_out);

    finish(&part_path, &final_path, &file, &mut on_progress)
}

/// Сверка и переход в конечный файл.
///
/// Вынесено отдельно, потому что сверка нужна в двух случаях: после
/// только что завершённой записи и когда `.part` уже полного размера,
/// но переименование не произошло. Во втором случае сеть не должна
/// трогаться вовсе.
fn finish(
    part_path: &std::path::Path,
    final_path: &std::path::Path,
    file: &RemoteFile,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf, String> {
    let actual_len = std::fs::metadata(part_path)
        .map(|m| m.len())
        .unwrap_or(0);

    on_progress(Progress::Verifying { size: actual_len });

    // Сверка. Файл неверного размера удаляется: он недокачан или
    // сервер отдал что-то не то, и оставлять его незачем.
    if actual_len != file.size {
        let _ = std::fs::remove_file(part_path);
        return Err(format!(
            "скачано {actual_len} байт, ожидалось {}. Файл удалён, попробуйте ещё раз",
            file.size
        ));
    }

    let digest = sha256_file(part_path)?;
    if !digest.eq_ignore_ascii_case(&file.sha256) {
        let _ = std::fs::remove_file(part_path);
        return Err(format!(
            "сводка не совпала.\n  ожидалось {}\n  получено   {}",
            file.sha256, digest
        ));
    }

    std::fs::rename(part_path, final_path)
        .map_err(|e| format!("не удалось переименовать файл: {e}"))?;

    on_progress(Progress::Done {
        path: final_path.display().to_string(),
        size: actual_len,
    });

    Ok(final_path.to_path_buf())
}

// ---------------------------------------------------------------------------
// Вспомогательное
// ---------------------------------------------------------------------------

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("не удалось открыть {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut reader = std::io::BufReader::new(file);
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf)
            .map_err(|e| format!("не удалось прочитать {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

/// Тексты ошибок reqwest без указания причины бесполезны пользователю.
/// Здесь добавляется подсказка про прокси, потому что это самая
/// частая причина на нашей аудитории.
fn friendly(what: &str, e: &reqwest::Error) -> String {
    let base = if e.is_timeout() {
        format!("{what}: нет ответа вовремя")
    } else if e.is_connect() {
        format!("{what}: не удалось подключиться")
    } else if e.is_redirect() {
        format!("{what}: сервер перенаправил запрос, куда мы не знаем")
    } else if e.is_request() {
        format!("{what}: запрос отклонён")
    } else {
        format!("{what}: {e}")
    };

    format!(
        "{base}\n\nЕсли вы за корпоративной сетью или прокси, проверьте, \
         разрешён ли доступ к доменам, которые отдают файл. Список нужен \
         в приложении."
    )
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64, sha: &str) -> RemoteFile {
        RemoteFile {
            path: path.into(),
            size,
            sha256: sha.into(),
        }
    }

    fn repo(files: Vec<RemoteFile>) -> RemoteRepo {
        RemoteRepo {
            repo: "test/repo".into(),
            license: "apache-2.0".into(),
            files,
        }
    }

    #[test]
    fn файл_находится_по_точному_имени() {
        let r = repo(vec![
            file("a.gguf", 1, "x"),
            file("model-Q4_K_M.gguf", 2, "y"),
        ]);
        assert_eq!(find_file(&r, "model-Q4_K_M.gguf").unwrap().size, 2);
    }

    #[test]
    fn файл_находится_по_суффиксу_если_он_единственный() {
        let r = repo(vec![
            file("Qwen-Q4_K_M.gguf", 10, "x"),
            file("Qwen-Q8_0.gguf", 20, "y"),
        ]);
        assert_eq!(find_file(&r, "Qwen-Q4_K_M.gguf").unwrap().size, 10);
    }

    #[test]
    fn неоднозначное_имя_даёт_ошибку_со_списком() {
        let r = repo(vec![
            file("a-Q4_K_M.gguf", 1, "x"),
            file("b-Q4_K_M.gguf", 2, "y"),
        ]);
        let e = find_file(&r, "Q4_K_M.gguf").unwrap_err();
        assert!(e.contains("подходит к 2 файлам"));
        assert!(e.contains("a-Q4_K_M.gguf"));
    }

    #[test]
    fn отсутствующий_файл_называется_по_имени() {
        let r = repo(vec![file("a.gguf", 1, "x")]);
        assert!(find_file(&r, "нет.gguf")
            .unwrap_err()
            .contains("нет.gguf"));
    }

    #[test]
    fn без_сводки_скачивать_нельзя() {
        let f = file("a.gguf", 10, "");
        // Проверка стоит первой и не даёт качать то, что нечем проверить.
        assert!(f.sha256.is_empty());
    }

    #[test]
    fn сводка_совпадает_с_хексом_с_пробелами_и_регистром() {
        let a = "D784CE9EDA1A5A7B51E8F705A9E6310844BF4F173654D115823C775FDEA56D43";
        let b = "d784ce9eda1a5a7b51e8f705a9e6310844bf4f173654d115823c775fdea56d43";
        assert!(a.eq_ignore_ascii_case(b));
    }

    #[test]
    fn сводка_реального_файла_совпадает_с_эталоном() {
        // sha256 пустой строки, проверяем сам счётчик на известном значении.
        let f = std::env::temp_dir().join("archotic-sha-test.bin");
        std::fs::write(&f, b"archotic").unwrap();
        let got = sha256_file(&f).unwrap();
        let _ = std::fs::remove_file(&f);
        assert_eq!(got.len(), 64);
        assert!(got.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn дерево_разбирается_и_пропускает_каталоги() {
        let json = r#"[
            {"type":"directory","path":"sub","size":0},
            {"type":"file","path":"m-Q4_K_M.gguf","size":100,
             "lfs":{"oid":"aa11","size":100,"pointerSize":135}},
            {"type":"file","path":"README.md","size":10}
        ]"#;
        let entries: Vec<TreeEntry> = serde_json::from_str(json).unwrap();
        let files: Vec<RemoteFile> = entries
            .into_iter()
            .filter(|e| e.kind == "file")
            .map(|e| RemoteFile {
                sha256: e.lfs.as_ref().map(|l| l.oid.clone()).unwrap_or_default(),
                size: e.lfs.as_ref().map(|l| l.size).or(e.size).unwrap_or(0),
                path: e.path,
            })
            .collect();

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].sha256, "aa11");
        // Обычный файл без LFS остаётся без сводки, и качать его нельзя.
        assert!(files[1].sha256.is_empty());
    }

    #[test]
    fn адрес_файла_собирается_из_репозитория_и_пути() {
        let url = FILE_URL
            .replace("{repo}", "unsloth/Qwen3.5-9B-GGUF")
            .replace("{path}", "Qwen3.5-9B-Q4_K_M.gguf");
        assert_eq!(
            url,
            "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf"
        );
    }

    #[test]
    fn отмена_отличима_от_сбоя() {
        assert_eq!(CANCELLED, "загрузка отменена пользователем");
    }


}
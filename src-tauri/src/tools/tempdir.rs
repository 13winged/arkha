// Временная папка для тестов.
//
// Своя, а не библиотечная: `tempfile` в зависимостях нет.
//
// Изначально причиной была правило 15: новая зависимость с
// непроверенной лицензией считалась запретом. Правило изменено 12
// октября 2026 и больше не запрещает, так что это больше не
// обязательство, а выбор в пользу нуля зависимостей на одну
// вспомогательную функцию. Выбор оставлен: `tempfile` тянет за собой
// дерево, а здесь тридцать строк.
//
// Имя уникально не по счётчику, а по времени и номеру потока. Если
// два теста стартуют одновременно, счётчик в пределах процесса у них
// общий, а вот номер потока разный.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static СЧЁТЧИК: AtomicU64 = AtomicU64::new(0);

pub struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    pub fn new() -> Self {
        let n = СЧЁТЧИК.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("arsha-test-{nanos}-{n}"));
        std::fs::create_dir_all(&path).expect("создать временную папку");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Записать файл, создав промежуточные папки.
    pub fn write(&self, имя: &str, содержимое: &[u8]) -> PathBuf {
        let p = self.path.join(имя);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("создать вложенную папку");
        }
        std::fs::write(&p, содержимое).expect("записать файл");
        p
    }

    pub fn dir(&self, имя: &str) -> PathBuf {
        let p = self.path.join(имя);
        std::fs::create_dir_all(&p).expect("создать папку");
        p
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        // Ошибка уборки не должна ронять тест: тест проверял логику
        // инструмента, а не умение удалять папку.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
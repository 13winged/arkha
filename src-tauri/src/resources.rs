// Что сейчас занимает память: оперативная и видеопамять.
//
// Зачем это интерфейсу. Человек не должен гадать, сколько свободно,
// и тем более не должен узнавать об этом из отказа модели. Полоса
// памяти отвечает на два вопроса сразу: сколько занято и сколько
// осталось, — и по ней видно, что место кончилось, до того как
// секретарь начнёт падать.
//
// Два источника, и они не равноправны.
//
// Оперативная память читается из Windows напрямую: `GlobalMemoryStatusEx`
// отдаёт и занятое, и доступное без единого внешнего процесса.
//
// Видеопамять читается через `nvidia-smi`. Это внешний процесс, и на
// машине без NVIDIA его нет — а профиль T1 это встроенная графика
// или 3060 на 4 ГиБ, и там видеопамяти в этом смысле нет вовсе.
// Поэтому «видеопамять неизвестна» это нормальный честный ответ, а
// не ошибка: интерфейс в этом случае показывает только оперативную
// память и не рисует пустую полосу.
//
// Чего тут не делается. Не рисуется полоса видеопамяти, если она не
// измерена: лучше не показать, чем показать нули и получить вопрос
// «почему у меня 0 ГиБ на карте с 8 ГиБ».

use serde::Serialize;
use windows::Win32::System::SystemInformation::{
    GlobalMemoryStatusEx, MEMORYSTATUSEX,
};

/// Снимок памяти. Всё в мегабайтах, без единиц в полях: единицу
/// знает только интерфейс, и если она поменяется, менять придётся
/// одно место.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Память {
    /// Занято оперативной памяти.
    pub ram_used_mb: u64,
    /// Всего оперативной памяти.
    pub ram_total_mb: u64,
    /// Занято видеопамяти, если её удалось измерить.
    pub vram_used_mb: Option<u64>,
    /// Всего видеопамяти, если её удалось измерить.
    pub vram_total_mb: Option<u64>,
    /// Видеопамять не измерилась: машина не NVIDIA либо счётчик не
    /// ответил. Интерфейс по этому флагу не рисует полосу видеопамяти.
    pub vram_неизвестна: bool,
}

impl Память {
    /// Доля занятой оперативной памяти, проценты.
    ///
    /// Ноль при нулевом итоге: деление на ноль в интерфейсе дало
    /// бы `NaN` и полосу длиной во весь экран либо пустую — в
    /// зависимости от того, как именно считать.
    pub fn ram_проценты(&self) -> u32 {
        if self.ram_total_mb == 0 {
            return 0;
        }
        ((self.ram_used_mb * 100) / self.ram_total_mb).min(100) as u32
    }

    /// Доля занятой видеопамяти, проценты. `None`, если не измерена.
    pub fn vram_проценты(&self) -> Option<u32> {
        let всего = self.vram_total_mb?;
        if всего == 0 {
            return None;
        }
        Some((((self.vram_used_mb.unwrap_or(0)) * 100) / всего).min(100) as u32)
    }
}

/// Прочитать память прямо сейчас.
pub fn снимок() -> Память {
    let (ram_used, ram_total) = оперативная();
    let (vram_used, vram_total) = видеопамять();
    Память {
        ram_used_mb: ram_used,
        ram_total_mb: ram_total,
        vram_неизвестна: vram_total.is_none(),
        vram_used_mb: vram_used,
        vram_total_mb: vram_total,
    }
}

/// Занято и всего оперативной памяти.
///
/// `GlobalMemoryStatusEx` — единственный путь без внешних процессов,
/// и он есть на любой Windows. Ошибку проглатываем и отдаём нули:
/// отсутствие снимка памяти не должно ломать интерфейс, который
/// просто хотел нарисовать полосу.
fn оперативная() -> (u64, u64) {
    let mut буфер = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    let прошло = unsafe { GlobalMemoryStatusEx(&mut буфер) }.is_ok();
    if !прошло {
        return (0, 0);
    }
    // «Занято» — это всего минус доступно. Само поле `ullTotalPhys`
    // минус `ullAvailPhys` означает то же, но так в нём сразу видно,
    // что обе величины пришли из одной структуры.
    let всего = буфер.ullTotalPhys / (1024 * 1024);
    let доступно = буфер.ullAvailPhys / (1024 * 1024);
    (всего.saturating_sub(доступно), всего)
}

/// Занято и всего видеопамяти через `nvidia-smi`.
///
/// Отдельная функция, а не часть `оперативная`, потому что это
/// единственное место, где разрешено звать внешний процесс, и
/// единственное место, где может ничего не получиться.
fn видеопамять() -> (Option<u64>, Option<u64>) {
    let mut command = std::process::Command::new("nvidia-smi");
    command.args([
        "--query-gpu=memory.used,memory.total",
        "--format=csv,noheader,nounits",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let вывод = command.output();
    // Нет `nvidia-smi` в PATH или он не ответил — это не ошибка
    // программы, а отсутствие NVIDIA. Отвечаем «неизвестно».
    let вывод = match вывод {
        Ok(в) => в,
        Err(_) => return (None, None),
    };
    let текст = String::from_utf8_lossy(&вывод.stdout);
    let Some(первая) = текст.lines().next() else {
        return (None, None);
    };
    let числа: Vec<u64> = первая
        .split(',')
        .filter_map(|кусок| кусок.trim().parse::<u64>().ok())
        .collect();
    if числа.len() < 2 {
        return (None, None);
    }
    (Some(числа[0]), Some(числа[1]))
}

/// Снимок загрузки CPU.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuLoad {
    /// Загрузка CPU в процентах (0-100).
    pub load_percent: u32,
    /// Количество логических ядер.
    pub logical_cores: u16,
}

/// Последний замер CPU. Считается в фоне: команда интерфейса не должна
/// спать двести миллисекунд на каждом опросе.
static CPU_LOAD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static CPU_CORES: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
static CPU_SAMPLER: std::sync::Once = std::sync::Once::new();

fn cpu_snapshot(system: &mut sysinfo::System) -> (u32, u16) {
    system.refresh_cpu_usage();
    let cores = system.cpus().len();
    let load = if cores == 0 {
        0.0
    } else {
        system.cpus().iter().map(|cpu| cpu.cpu_usage()).sum::<f32>() / cores as f32
    };
    ((load as u32).min(100), cores.max(1) as u16)
}

/// Прочитать загрузку CPU через sysinfo.
///
/// Между двумя замерами нужна пауза: sysinfo считает загрузку как
/// разницу между снимками, и первый же вызов после старта всегда даёт
/// ноль. Поэтому первый замер делается один раз с паузой, а дальше
/// фоновый поток обновляет значение раз в секунду. Команда только
/// читает атомики и не блокирует интерфейс.
pub fn cpu_load() -> CpuLoad {
    use std::sync::atomic::Ordering;

    CPU_SAMPLER.call_once(|| {
        let mut system = sysinfo::System::new_all();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        let (load, cores) = cpu_snapshot(&mut system);
        CPU_LOAD.store(load, Ordering::Relaxed);
        CPU_CORES.store(cores, Ordering::Relaxed);

        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let (load, cores) = cpu_snapshot(&mut system);
            CPU_LOAD.store(load, Ordering::Relaxed);
            CPU_CORES.store(cores, Ordering::Relaxed);
        });
    });

    CpuLoad {
        load_percent: CPU_LOAD.load(Ordering::Relaxed),
        logical_cores: CPU_CORES.load(Ordering::Relaxed).max(1),
    }
}

/// Команда для интерфейса.
#[tauri::command]
pub fn cpu_load_сейчас() -> CpuLoad {
    cpu_load()
}

/// Команда для интерфейса.
#[tauri::command]
pub fn память_сейчас() -> Память {
    снимок()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ноль в знаменателе обязан давать ноль, а не падать.
    /// Интерфейс делит на это число каждый кадр обновления.
    #[test]
    fn проценты_не_взрываются_на_нуле() {
        let п = Память {
            ram_used_mb: 0,
            ram_total_mb: 0,
            vram_used_mb: None,
            vram_total_mb: None,
            vram_неизвестна: true,
        };
        assert_eq!(п.ram_проценты(), 0);
        assert_eq!(п.vram_проценты(), None);
    }

    /// Процент выше сотни не показываем: полоса иначе вылезет за
    /// границы и будет врать.
    #[test]
    fn проценты_не_бывают_больше_сотни() {
        let п = Память {
            ram_used_mb: 900,
            ram_total_mb: 100,
            vram_used_mb: Some(900),
            vram_total_mb: Some(100),
            vram_неизвестна: false,
        };
        assert_eq!(п.ram_проценты(), 100);
        assert_eq!(п.vram_проценты(), Some(100));
    }

    /// Видеопамять без счётчика остаётся «неизвестна», а не нулём:
    /// это разные вещи, и интерфейс обязан их различать.
    #[test]
    fn неизвестная_видеопамять_не_превращается_в_ноль() {
        let п = Память {
            ram_used_mb: 1,
            ram_total_mb: 2,
            vram_used_mb: None,
            vram_total_mb: None,
            vram_неизвестна: true,
        };
        assert!(п.vram_неизвестна);
        assert_eq!(п.vram_проценты(), None);
    }

    /// На живой машине снимок обязан получиться: если нет, полоса
    /// в интерфейсе всегда будет пустой и никто не поймёт почему.
    #[test]
    fn снимок_читается_на_этой_машине() {
        let п = снимок();
        assert!(
            п.ram_total_mb > 1024,
            "оперативная память прочиталась как {} МиБ — это не число памяти",
            п.ram_total_mb
        );
        assert!(п.ram_used_mb <= п.ram_total_mb, "занято больше, чем всего");
    }
}

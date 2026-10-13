//! S1  Определение профиля железа T1-T5 по VRAM и RAM.
//!
//! Правило выбора — docs/01-контекст.md, раздел «Правило выбора».
//! Всё дальше опирается на этот модуль, поэтому ошибка здесь дороже
//! всего остального в приложении.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Профиль
// ---------------------------------------------------------------------------

/// Профиль железа. Порядок вариантов значим: `T1` — самый слабый,
/// `T5` — самый сильный, и `tier < self.min_tier` должно читаться
/// как «железа не хватает».
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tier {
    T1,
    T2,
    T3,
    T4,
    T5,
}

impl Tier {
    /// Правило выбора из docs/01. `vram_mb` — выделенная видеопамять
    /// адаптера в МБ, `ram_mb` — физическая память в МБ.
    ///
    /// ```text
    /// VRAM >= 24 ГБ  -> T5
    /// VRAM >= 12 ГБ  -> T4
    /// VRAM >= 8 ГБ   -> T3, если RAM >= 24 ГБ, иначе T2
    /// VRAM >= 4 ГБ   -> T2
    /// VRAM < 4 ГБ    -> T1 (CPU-режим)
    /// RAM < 8 ГБ     -> не поддерживается
    /// ```
    ///
    /// Видеопамять округляется вверх до класса карты, см. [`vram_class_mb`].
    pub fn detect(vram_mb: u64, ram_mb: u64) -> Tier {
        let vram = vram_class_mb(vram_mb);
        match vram {
            v if v >= 24_576 => Tier::T5,
            v if v >= 12_288 => Tier::T4,
            v if v >= 8_192 => {
                if ram_mb >= 24_576 {
                    Tier::T3
                } else {
                    Tier::T2
                }
            }
            v if v >= 4_096 => Tier::T2,
            _ => Tier::T1,
        }
    }

    /// Порог ниже которого машина не поддерживается вообще.
    pub const MIN_RAM_MB: u64 = 8_192;

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::T1 => "T1",
            Tier::T2 => "T2",
            Tier::T3 => "T3",
            Tier::T4 => "T4",
            Tier::T5 => "T5",
        }
    }

    /// Рекомендация из docs/01, таблица профилей.
    pub fn spec(self) -> ModelSpec {
        match self {
            Tier::T1 => ModelSpec {
                name: "Qwen3.5 4B".into(),
                quant: "Q4_K_M".into(),
                context: 16_384,
                kv_cache_type: "q8_0".into(),
                model_gb: 2.5,
            },
            Tier::T2 => ModelSpec {
                name: "Qwen3.5 9B".into(),
                quant: "Q4_K_M".into(),
                context: 32_768,
                kv_cache_type: "q4_0".into(),
                model_gb: 5.06,
            },
            Tier::T3 => ModelSpec {
                name: "Qwen3.5 9B".into(),
                quant: "Q5_K_M".into(),
                context: 65_536,
                kv_cache_type: "q4_0".into(),
                model_gb: 6.5,
            },
            Tier::T4 => ModelSpec {
                name: "Qwen3.5 27B".into(),
                quant: "Q4_K_M".into(),
                context: 131_072,
                kv_cache_type: "q4_0".into(),
                model_gb: 16.0,
            },
            Tier::T5 => ModelSpec {
                name: "Qwen3.5 27B".into(),
                quant: "Q4_K_M".into(),
                context: 262_144,
                kv_cache_type: "q4_0".into(),
                model_gb: 16.0,
            },
        }
    }

    /// Человеческое описание для экрана настроек и для объяснений агента.
    pub fn describe(self) -> String {
        let s = self.spec();
        format!(
            "{} — {}, контекст {}K",
            self.as_str(),
            s.name,
            s.context / 1024
        )
    }
}

// ---------------------------------------------------------------------------
// Модель
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    pub name: String,
    pub quant: String,
    pub context: u32,
    pub kv_cache_type: String,
    pub model_gb: f32,
}

// ---------------------------------------------------------------------------
// Железо
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetectionSource {
    /// DXGI, основной путь. Работает без внешних процессов.
    Dxgi,
    /// nvidia-smi, запасной путь. Нужен, если DXGI не дал выделенную память.
    NvidiaSmi,
    /// Ничего не сработало. Профиль T1, причина в unsupported_reason.
    Unavailable,
}

/// Что нашли на машине. Не профиль — факты.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hardware {
    pub gpu_name: String,
    /// Выделенная видеопамять, МБ. 0 означает встройку.
    pub vram_mb: u64,
    /// То же за вычетом запаса под рабочий буфер и оверлей.
    pub vram_usable_mb: u64,
    /// true — дискретная карта, false — встроенная.
    pub is_discrete: bool,
    /// true — встройка и дискретная карта одновременно, как на ноутбуках.
    pub hybrid: bool,
    pub ram_mb: u64,
    pub logical_cores: u16,
    pub cpu_brand: String,
    pub source: DetectionSource,
}

impl Hardware {
    fn empty() -> Self {
        Self {
            gpu_name: "не определена".into(),
            vram_mb: 0,
            vram_usable_mb: 0,
            is_discrete: false,
            hybrid: false,
            ram_mb: 0,
            logical_cores: 0,
            cpu_brand: "не определён".into(),
            source: DetectionSource::Unavailable,
        }
    }

    /// Сколько видеопамяти реально достанется модели.
    ///
    /// Считаем не всю VRAM, а VRAM минус то, что занимает рабочая
    /// система. Основание — замер 12 октября 2026: полное размещение
    /// заметно быстрее, а сильный offload стоит почти в 5 раз.
    pub fn usable_vram_mb(&self) -> u64 {
        self.vram_usable_mb
    }
}

// ---------------------------------------------------------------------------
// Профиль целиком
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Профиль по факту железа, без учёта ручного переопределения.
    pub detected: Tier,
    /// Ручное переопределение из настроек. None — автоопределение.
    #[serde(default)]
    pub forced: Option<Tier>,
    pub hardware: Hardware,
    /// Заполнено, если машина не поддерживается.
    #[serde(default)]
    pub unsupported_reason: Option<String>,
}

impl Profile {
    /// Профиль, по которому реально работаем.
    pub fn effective(&self) -> Tier {
        self.forced.unwrap_or(self.detected)
    }

    pub fn model(&self) -> ModelSpec {
        self.effective().spec()
    }

    /// Собирает профиль из уже известного железа.
    pub fn from_hardware(hw: Hardware) -> Self {
        let mut p = Self {
            detected: Tier::T1,
            forced: None,
            unsupported_reason: None,
            hardware: hw,
        };
        p.recompute();
        p
    }

    /// Пересчитывает detected и unsupported_reason. Вызывается после
    /// изменения железа в настройках или после ручного форсирования.
    pub fn recompute(&mut self) {
        if self.hardware.ram_mb > 0 && self.hardware.ram_mb < Tier::MIN_RAM_MB {
            self.unsupported_reason = Some(format!(
                "{} ГБ оперативной памяти — минимум {} ГБ",
                self.hardware.ram_mb / 1024,
                Tier::MIN_RAM_MB / 1024
            ));
        } else if self.hardware.source == DetectionSource::Unavailable {
            self.unsupported_reason =
                Some("не удалось определить видеопамять".into());
        } else {
            self.unsupported_reason = None;
        }

        // Профиль определяется по ПОЛНОЙ видеопамяти, а не по доступной.
        //
        // Так написано в docs/01, и это же подтверждено замером: RTX 5060
        // с 8 ГБ — это T3. Если считать по доступной памяти, то после
        // вычета запаса под систему карта уходит ниже порога 8 ГБ и T3
        // становится недостижимым ни на одной 8-гиговой карте.
        //
        // Доступная память нужна не для выбора профиля, а для выбора
        // рецепта: кванта и KV-кэша. Это решение этапа S2, и оно
        // принимается замером, а не арифметикой.
        self.detected = Tier::detect(self.hardware.vram_mb, self.hardware.ram_mb);
    }

    /// Строка для экрана настроек: «Ваш профиль: T2 — Qwen3.5 9B, 32K».
    pub fn summary(&self) -> String {
        let tier = self.effective();
        let base = format!("Ваш профиль: {}", tier.describe());
        if self.forced.is_some() {
            format!(
                "{base} (выбран вручную, определён был {})",
                self.detected.as_str()
            )
        } else {
            base
        }
    }
}

// ---------------------------------------------------------------------------
// Определение железа
// ---------------------------------------------------------------------------

const MIB: u64 = 1024 * 1024;

/// Видеопамять, которую не отдаём модели: рабочий буфер, оверлей,
/// рабочий стол, видеодрайвер.
const VRAM_RESERVE_MB: u64 = 512;

/// Округляет видеопамять вверх до класса карты.
///
/// Зачем: правило в docs/01 написано в гигабайтах, а видеопамять так не
/// отдаётся. Измерение на dev-машине, 11 октября 2026:
///
/// | Источник | RTX 5060 8 ГБ |
/// |---|---|
/// | `nvidia-smi --query-gpu=memory.total` | 8151 МиБ |
/// | DXGI `DXGI_ADAPTER_DESC1.DedicatedVideoMemory` | 7895 МБ |
/// | Порог «8 ГБ» в docs/01 | 8192 МБ |
///
/// Ни один из двух реальных значений не достигает 8192. Сравнение с
/// 8192 отправляет любую 8-гиговую карту в T2, и T3 недостижим на той
/// самой машине, для которой он и написан. Это найдено измерением на
/// S1, а не чтением документации.
///
/// Округление вверх до целого гигабайта возвращает карту в свой класс:
/// 7895 -> 8192 -> T3. Проверено на всех классах в тестах.
fn vram_class_mb(vram_mb: u64) -> u64 {
    if vram_mb == 0 {
        return 0;
    }
    let step = 1024;
    vram_mb.div_ceil(step) * step
}

/// Определяет железо. DXGI, при неудаче nvidia-smi, при неудаче T1.
pub fn detect() -> Hardware {
    let (mut hw, mut found) = detect_dxgi();

    if !found && detect_nvidia_smi(&mut hw) {
        found = true;
    }

    if !found {
        hw = Hardware::empty();
    }

    hw.ram_mb = ram_mb();
    hw.logical_cores = logical_cores();
    hw.cpu_brand = cpu_brand();

    // Запас под систему. На встройке он больше: там же живёт рабочий стол.
    let reserve = if hw.is_discrete {
        VRAM_RESERVE_MB
    } else {
        VRAM_RESERVE_MB * 2
    };
    hw.vram_usable_mb = hw.vram_mb.saturating_sub(reserve);

    hw
}

/// Основной путь: DXGI перечисляет адаптеры и отдаёт выделенную память.
/// Работает без запуска внешних процессов.
fn detect_dxgi() -> (Hardware, bool) {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE,
    };

    let mut hw = Hardware::empty();
    let mut best: Option<DXGI_ADAPTER_DESC1> = None;
    let mut integrated_found = false;

    unsafe {
        let factory: IDXGIFactory1 = match CreateDXGIFactory1() {
            Ok(f) => f,
            Err(_) => return (hw, false),
        };

        for index in 0.. {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(a) => a,
                Err(_) => break,
            };

            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };

            // Флаг программного адаптера (WARP, Basic Render Driver).
            // Реальной видеопамяти у него нет, пропускаем.
            if desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }

            let name = decode_wide(&desc.Description);
            let vram = desc.DedicatedVideoMemory as u64 / MIB;

            if desc.DedicatedVideoMemory == 0 {
                integrated_found = true;
                if hw.gpu_name == "не определена" {
                    hw.gpu_name = name.clone();
                }
                continue;
            }

            match &best {
                Some(b) if b.DedicatedVideoMemory >= desc.DedicatedVideoMemory => {}
                _ => {
                    hw.gpu_name = name;
                    hw.vram_mb = vram;
                    best = Some(desc);
                }
            }
        }
    }

    if best.is_none() && integrated_found {
        // Только встройка. Дискретной карты нет.
        hw.vram_mb = 0;
        hw.is_discrete = false;
        hw.source = DetectionSource::Dxgi;
        return (hw, true);
    }

    if best.is_none() {
        return (hw, false);
    }

    hw.is_discrete = true;
    hw.hybrid = integrated_found;
    hw.source = DetectionSource::Dxgi;
    (hw, true)
}

/// Запасной путь: nvidia-smi. Нужен на редких конфигурациях, где DXGI
/// отдаёт 0. Это единственное место в приложении, где запускается
/// внешний процесс, и модель к нему доступа не имеет.
fn detect_nvidia_smi(hw: &mut Hardware) -> bool {
    let out = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output();

    let Ok(out) = out else { return false };
    if !out.status.success() {
        return false;
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let Some(first) = text.lines().next() else {
        return false;
    };
    let Some((name, mem)) = first.trim().split_once(',') else {
        return false;
    };
    let Ok(mem) = mem.trim().parse::<u64>() else {
        return false;
    };

    if mem == 0 {
        return false;
    }

    hw.gpu_name = name.trim().to_string();
    hw.vram_mb = mem;
    hw.is_discrete = true;
    hw.source = DetectionSource::NvidiaSmi;
    true
}

/// Физическая память через GlobalMemoryStatusEx.
fn ram_mb() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    unsafe {
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if GlobalMemoryStatusEx(&mut status).is_ok() {
            status.ullTotalPhys / MIB
        } else {
            0
        }
    }
}

fn logical_cores() -> u16 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u16)
        .unwrap_or(0)
}

/// Бренд процессора из CPUID. Читаем только на x86_64, на других
/// архитектурах возвращаем пустое имя.
fn cpu_brand() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::__cpuid;

        // Максимальный расширенный лист: AMD кладёт его в EAX, Intel — в EDX.
// Проверяем оба, иначе на Ryzen читается мусор и бренд не определяется.
        let leaf0 = __cpuid(0x8000_0000);
        if leaf0.eax < 0x8000_0004 && leaf0.edx < 0x8000_0004 {
            return String::new();
        }

        let mut brand = [0u8; 48];
        for chunk in 0..3u32 {
            let leaf = 0x8000_0002 + chunk;
            let regs = __cpuid(leaf);
            for (i, reg) in [regs.eax, regs.ebx, regs.ecx, regs.edx].iter().enumerate() {
                for byte in 0..4 {
                    brand[(chunk as usize * 16) + (i * 4) + byte] = (reg >> (byte * 8)) as u8;
                }
            }
        }

        let text = String::from_utf8_lossy(&brand);
        text.trim().trim_end_matches('\0').trim().to_string()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        String::new()
    }
}

fn decode_wide(buf: &[u16]) -> String {
    let s: String = buf
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8 as char)
        .collect();
    s.trim().to_string()
}

// ---------------------------------------------------------------------------
// Сохранение
// ---------------------------------------------------------------------------

/// Конфиг и данные живут рядом с приложением, не в Program Files.
pub fn config_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("Archotic")
}

pub fn profile_path() -> PathBuf {
    config_dir().join("profile.json")
}

pub fn save(p: &Profile) -> std::io::Result<()> {
    let path = profile_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(p)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

pub fn load() -> Option<Profile> {
    let json = std::fs::read_to_string(profile_path()).ok()?;
    serde_json::from_str(&json).ok()
}

/// Определяет заново, но сохраняет ручное переопределение: пользователь
/// выбрал продел вручную, железо с тех пор не менялось.
pub fn detect_and_persist() -> Profile {
    let hw = detect();
    let mut p = match load() {
        Some(prev) => Profile {
            detected: Tier::T1,
            forced: prev.forced,
            hardware: hw,
            unsupported_reason: None,
        },
        None => Profile {
            detected: Tier::T1,
            forced: None,
            hardware: hw,
            unsupported_reason: None,
        },
    };
    p.recompute();
    let _ = save(&p);
    p
}

// ---------------------------------------------------------------------------
// Тесты
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(vram_mb: u64, ram_mb: u64) -> Hardware {
        Hardware {
            gpu_name: "test".into(),
            vram_mb,
            vram_usable_mb: vram_mb,
            is_discrete: vram_mb > 0,
            hybrid: false,
            ram_mb,
            logical_cores: 8,
            cpu_brand: "test".into(),
            source: DetectionSource::Dxgi,
        }
    }

    #[test]
    fn границы_профилей() {
        // Гейт S1: 8 ГБ VRAM + 32 ГБ RAM -> T3
        assert_eq!(Tier::detect(8_192, 32_768), Tier::T3);
        // 8 ГБ VRAM + 16 ГБ RAM -> T2
        assert_eq!(Tier::detect(8_192, 16_384), Tier::T2);
        // 8 ГБ VRAM + 20 ГБ RAM -> T2, порог 24 ГБ не пройден
        assert_eq!(Tier::detect(8_192, 20_480), Tier::T2);
        // Встройка -> T1
        assert_eq!(Tier::detect(0, 8_192), Tier::T1);
        assert_eq!(Tier::detect(2_048, 8_192), Tier::T1);
        // 6 ГБ -> T2
        assert_eq!(Tier::detect(6_144, 16_384), Tier::T2);
        // 16 ГБ -> T4
        assert_eq!(Tier::detect(16_384, 32_768), Tier::T4);
        // 24 ГБ -> T5
        assert_eq!(Tier::detect(24_576, 65_536), Tier::T5);
    }

    /// Измерение на dev-машине, 11 октября 2026. Без округления
    /// вверх обе реальные величины дают T2, и гейт S1 не проходит.
    #[test]
    fn измеренная_восьмигиговая_карта_даёт_t3() {
        let ram = 32_376; // как показывает GlobalMemoryStatusEx на 32 ГБ
        assert_eq!(Tier::detect(7_895, ram), Tier::T3, "значение DXGI");
        assert_eq!(Tier::detect(8_151, ram), Tier::T3, "значение nvidia-smi");
        assert_eq!(Tier::detect(8_192, ram), Tier::T3, "ровно 8 ГБ");
    }

    #[test]
    fn округление_возвращает_карту_в_свой_класс() {
        assert_eq!(vram_class_mb(0), 0);
        assert_eq!(vram_class_mb(1), 1_024);
        assert_eq!(vram_class_mb(1_024), 1_024);
        assert_eq!(vram_class_mb(3_900), 4_096); // карта 4 ГБ
        assert_eq!(vram_class_mb(7_895), 8_192); // карта 8 ГБ, значение DXGI
        assert_eq!(vram_class_mb(8_151), 8_192); // карта 8 ГБ, значение smi
        assert_eq!(vram_class_mb(11_500), 12_288); // карта 12 ГБ
        assert_eq!(vram_class_mb(15_400), 16_384); // карта 16 ГБ
        // Реальные величины: 4090 показывает 24564 МиБ.
        assert_eq!(vram_class_mb(24_564), 24_576);
    }

    /// Класс карты не должен прыгать между профилями из-за мелких
    /// колебаний занятой памяти.
    #[test]
    fn соседние_значения_одной_карты_дают_один_профиль() {
        for vram in 7_700..8_200u64 {
            assert_eq!(
                Tier::detect(vram, 32_768),
                Tier::T3,
                "VRAM {vram} МБ должен давать T3"
            );
        }
        for vram in 11_300..12_300u64 {
            assert_eq!(
                Tier::detect(vram, 32_768),
                Tier::T4,
                "VRAM {vram} МБ должен давать T4"
            );
        }
    }

    /// Границы проверяются по классам карт, потому что после округления
    /// вверх промежуточные килобайты в один класс не попадают.
    #[test]
    fn ровно_на_границе_идёт_вверх() {
        assert_eq!(Tier::detect(24_576, 65_536), Tier::T5); // карта 24 ГБ
        assert_eq!(Tier::detect(23_552, 65_536), Tier::T4); // класс 23 ГБ
        assert_eq!(Tier::detect(12_288, 32_768), Tier::T4); // карта 12 ГБ
        assert_eq!(Tier::detect(11_264, 32_768), Tier::T3); // класс 11 ГБ
        assert_eq!(Tier::detect(8_192, 32_768), Tier::T3); // карта 8 ГБ
        assert_eq!(Tier::detect(7_168, 32_768), Tier::T2); // класс 7 ГБ
        assert_eq!(Tier::detect(4_096, 16_384), Tier::T2); // карта 4 ГБ
        assert_eq!(Tier::detect(3_072, 16_384), Tier::T1); // класс 3 ГБ
    }

    #[test]
    fn маловато_памяти_помечается_неподдерживаемым() {
        let p = Profile::from_hardware(hw(8_192, 4_096));
        assert!(p.unsupported_reason.is_some());
    }

    #[test]
    fn ручное_переопределение_побеждает_определение() {
        let mut p = Profile::from_hardware(hw(8_192, 32_768));
        assert_eq!(p.effective(), Tier::T3);
        p.forced = Some(Tier::T2);
        assert_eq!(p.effective(), Tier::T2);
        assert_eq!(p.detected, Tier::T3);
        assert!(p.summary().contains("вручную"));
    }

    #[test]
    fn определяется_реальное_железо() {
        // Не проверяем значение, проверяем что определение вообще
        // отработало на текущей машине.
        let p = detect_and_persist();
        assert!(p.hardware.logical_cores > 0);
        assert!(p.hardware.ram_mb > 0);
        assert!(!p.hardware.cpu_brand.is_empty());
    }
}
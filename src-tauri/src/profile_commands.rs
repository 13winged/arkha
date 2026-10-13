// S1  Команды профиля для интерфейса.
//
// Модель к этим данным отношения не имеет: это основа, на которой
// строятся выбор модели (S2) и реестр возможностей (S3).

use serde::Serialize;

use crate::profile::{self, DetectionSource, Profile, Tier};

/// То, что уходит в интерфейс.
#[derive(Serialize)]
pub struct ProfileView {
    pub summary: String,
    pub detected: String,
    pub effective: String,
    pub forced: Option<String>,
    pub model_name: String,
    pub model_quant: String,
    pub context: u32,
    pub kv_cache_type: String,
    pub gpu_name: String,
    pub vram_mb: u64,
    pub vram_usable_mb: u64,
    pub is_discrete: bool,
    pub hybrid: bool,
    pub ram_mb: u64,
    pub logical_cores: u16,
    pub cpu_brand: String,
    pub detection_source: String,
    pub unsupported_reason: Option<String>,
    /// Все профили для ручного выбора, по порядку.
    pub available_tiers: Vec<TierNote>,
}

/// Что пользователю стоит знать о своём железе.
#[derive(Serialize)]
pub struct TierNote {
    pub tier: String,
    pub describe: String,
    /// Почему профиль недоступен на этой машине. Пусто — доступен.
    pub blocked_because: Option<String>,
}

/// Что приложение знает о машине, до всякой модели.
pub fn to_view(p: &Profile) -> ProfileView {
    let model = p.model();
    let hw = &p.hardware;

    ProfileView {
        summary: p.summary(),
        detected: p.detected.as_str().to_string(),
        effective: p.effective().as_str().to_string(),
        forced: p.forced.map(|t| t.as_str().to_string()),
        model_name: model.name,
        model_quant: model.quant,
        context: model.context,
        kv_cache_type: model.kv_cache_type,
        gpu_name: hw.gpu_name.clone(),
        vram_mb: hw.vram_mb,
        vram_usable_mb: hw.vram_usable_mb,
        is_discrete: hw.is_discrete,
        hybrid: hw.hybrid,
        ram_mb: hw.ram_mb,
        logical_cores: hw.logical_cores,
        cpu_brand: hw.cpu_brand.clone(),
        detection_source: match hw.source {
            DetectionSource::Dxgi => "dxgi",
            DetectionSource::NvidiaSmi => "nvidia-smi",
            DetectionSource::Unavailable => "не определён",
        }
        .to_string(),
        unsupported_reason: p.unsupported_reason.clone(),
        available_tiers: [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5]
            .iter()
            .map(|t| tier_note(p, *t))
            .collect(),
    }
}

fn tier_note(p: &Profile, tier: Tier) -> TierNote {
    let hw = &p.hardware;
    let vram_gb = hw.vram_usable_mb / 1024;
    let ram_gb = hw.ram_mb / 1024;

    // Причины проверяем от сильных требований к слабым, чтобы
    // сообщение называло настоящее препятствие, а не первое попавшееся.
    let blocked = match tier {
        Tier::T5 if hw.vram_usable_mb < 24_576 => {
            Some(format!("нужно 24 ГБ видеопамяти, доступно {vram_gb} ГБ"))
        }
        Tier::T5 if hw.ram_mb < 65_536 => {
            Some(format!("нужно 64 ГБ оперативной памяти, доступно {ram_gb} ГБ"))
        }
        Tier::T4 if hw.vram_usable_mb < 12_288 => {
            Some(format!("нужно 12 ГБ видеопамяти, доступно {vram_gb} ГБ"))
        }
        // T3 отличается от T2 квантом, но требует 24 ГБ RAM.
        Tier::T3 if hw.ram_mb < 24_576 => {
            Some(format!("нужно 24 ГБ оперативной памяти, доступно {ram_gb} ГБ"))
        }
        Tier::T2 if hw.vram_usable_mb < 4_096 => {
            Some(format!("нужно 4 ГБ видеопамяти, доступно {vram_gb} ГБ"))
        }
        _ => None,
    };

    TierNote {
        tier: tier.as_str().to_string(),
        describe: tier.describe(),
        blocked_because: blocked,
    }
}

/// Определяет железо, сохраняет профиль, отдаёт в интерфейс.
#[tauri::command]
pub fn get_profile() -> ProfileView {
    to_view(&profile::detect_and_persist())
}

/// Ручное переопределение. `tier` из UI — строка "T1".."T5".
/// Пустая строка возвращает автоопределение.
#[tauri::command]
pub fn set_profile_tier(tier: Option<String>) -> Result<ProfileView, String> {
    let forced = match tier.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(other) => Some(parse_tier(other).ok_or_else(|| {
            format!("неизвестный профиль «{other}», ожидается T1…T5")
        })?),
    };

    let mut p = profile::load().unwrap_or_else(|| profile::detect_and_persist());
    p.forced = forced;
    p.recompute();
    profile::save(&p).map_err(|e| format!("не удалось сохранить профиль: {e}"))?;
    Ok(to_view(&p))
}

/// Ручной выбор контекста. Автоматика ошибается редко, но 32K против
/// 16K на одном железе — это разница между «работает» и «не работает».
#[tauri::command]
pub fn set_context(context: u32) -> Result<ProfileView, String> {
    const MIN: u32 = 4_096;
    const MAX: u32 = 262_144;

    if !(MIN..=MAX).contains(&context) {
        return Err(format!("контекст {context} вне диапазона {MIN}…{MAX}"));
    }

    let path = profile::config_dir().join("context");
    std::fs::create_dir_all(profile::config_dir())
        .map_err(|e| format!("не удалось создать папку настроек: {e}"))?;
    std::fs::write(&path, context.to_string())
        .map_err(|e| format!("не удалось сохранить контекст: {e}"))?;

    Ok(to_view(&profile::load().unwrap_or_else(|| profile::detect_and_persist())))
}

/// Контекст с ручным переопределением, если оно есть.
pub fn effective_context(p: &Profile) -> u32 {
    std::fs::read_to_string(profile::config_dir().join("context"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| p.model().context)
}

fn parse_tier(s: &str) -> Option<Tier> {
    match s.trim().to_ascii_uppercase().as_str() {
        "T1" => Some(Tier::T1),
        "T2" => Some(Tier::T2),
        "T3" => Some(Tier::T3),
        "T4" => Some(Tier::T4),
        "T5" => Some(Tier::T5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Hardware;

    fn machine(vram: u64, ram: u64) -> Profile {
        Profile::from_hardware(Hardware {
            gpu_name: "RTX 5060".into(),
            vram_mb: vram,
            vram_usable_mb: vram.saturating_sub(512),
            is_discrete: vram > 0,
            hybrid: false,
            ram_mb: ram,
            logical_cores: 8,
            cpu_brand: "AMD Ryzen 7 8700F".into(),
            source: DetectionSource::Dxgi,
        })
    }

    #[test]
    fn гейт_девел_ма_шина_определяется_как_t3() {
        let v = to_view(&machine(8_192, 32_768));
        assert_eq!(v.effective, "T3");
        assert_eq!(v.model_quant, "Q5_K_M");
        assert_eq!(v.context, 65_536);
    }

    #[test]
    fn гейт_целевая_ма_шина_определяется_как_t2() {
        let v = to_view(&machine(8_192, 16_384));
        assert_eq!(v.effective, "T2");
        assert_eq!(v.model_quant, "Q4_K_M");
        assert_eq!(v.context, 32_768);
    }

    #[test]
    fn гейт_встройка_определяется_как_t1() {
        let v = to_view(&machine(0, 8_192));
        assert_eq!(v.effective, "T1");
        assert_eq!(v.context, 16_384);
        assert!(!v.is_discrete);
    }

    #[test]
    fn недоступные_профили_объясняют_настоящую_причину() {
        let v = to_view(&machine(8_192, 16_384));

        let t5 = v.available_tiers.iter().find(|n| n.tier == "T5").unwrap();
        assert!(t5.blocked_because.as_deref().unwrap().contains("видеопамяти"));

        // T3 на этой машине заблокирован памятью, а не видеопамятью.
        let t3 = v.available_tiers.iter().find(|n| n.tier == "T3").unwrap();
        assert!(t3
            .blocked_because
            .as_deref()
            .unwrap()
            .contains("оперативной памяти"));

        // T2 доступен.
        let t2 = v.available_tiers.iter().find(|n| n.tier == "T2").unwrap();
        assert!(t2.blocked_because.is_none());
    }

    #[test]
    fn парсинг_имени_профиля() {
        assert_eq!(parse_tier("T3"), Some(Tier::T3));
        assert_eq!(parse_tier("t3"), Some(Tier::T3));
        assert_eq!(parse_tier(" T5 "), Some(Tier::T5));
        assert_eq!(parse_tier("T9"), None);
        assert_eq!(parse_tier("мусор"), None);
    }

    #[test]
    fn все_пять_профилей_присутствуют_по_порядку() {
        let v = to_view(&machine(8_192, 32_768));
        let list: Vec<&str> = v.available_tiers.iter().map(|n| n.tier.as_str()).collect();
        assert_eq!(list, ["T1", "T2", "T3", "T4", "T5"]);
    }
}
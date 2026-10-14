// Реестр возможностей агента.
//
// Смысл модуля один: агент должен уметь честно сказать «у меня этого
// нет», и говорить **почему**. Не из вежливости модели — из данных.
//
// Пока модель не знает своих границ, она обещает то, чего не
// умеет. Пользователь верит, потом ищет то, чего нет. Для секретаря
// это худший отказ: он хуже медленного.
//
// Три свойства, которые здесь зафиксированы:
//
//   `verified` — проверено гейтом на живом железе, а не предположено
//   `refusal`  — что именно сказать, когда возможности нет
//   `evidence`  — чем проверено или почему нельзя
//
// Правило: отсутствие свойства означает «не проверено», а не
// «работает». Молчаливое «да» хуже явного «нет».

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::profile::Tier;

/// Почему возможность недоступна. Разные причины требуют разного
/// ответа пользователю, поэтому они не сводятся в одно «нельзя».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocker {
    /// Не хватает ресурсов: видеопамяти или оперативной памяти.
    Hardware,
    /// Модель этого не умеет: нет tool calling или нет мультимодальности.
    Model,
    /// Пользователь не дал разрешение.
    Permission,
    /// Не настроено: не задан адрес поиска, не скачана модель.
    NotConfigured,
    /// Написано, но не сделато.
    NotBuilt,
}

impl Blocker {
    /// Заголовок для интерфейса. Короткий, чтобы поместиться в строку.
    pub fn title(self) -> &'static str {
        match self {
            Blocker::Hardware => "Не хватает ресурсов",
            Blocker::Model => "Модель так не умеет",
            Blocker::Permission => "Запрещено настройками",
            Blocker::NotConfigured => "Не настроено",
            Blocker::NotBuilt => "Ещё не сделано",
        }
    }

    /// Что сказать пользователю. Не «извините, я так не могу» —
    /// это ложь, скрывающая причину.
    pub fn title_ru(self) -> &'static str {
        self.title()
    }

    /// Заголовок на английском.
    ///
    /// Те же пять вариантов, тот же смысл. Перевод придумывать на
    /// лету нельзя: если причину недоступности перепишет интерфейс,
    /// она неизбежно разойдётся с русской и начнёт врать о том, что
    /// мешает. А её читают буквально — когда инструмент не сработал.
    pub fn title_en(self) -> &'static str {
        match self {
            Blocker::Hardware => "Not enough resources",
            Blocker::Model => "The model cannot do this",
            Blocker::Permission => "Forbidden by settings",
            Blocker::NotConfigured => "Not configured",
            Blocker::NotBuilt => "Not built yet",
        }
    }

    /// Заголовок на выбранном языке.
    pub fn title_for(self, en: bool) -> &'static str {
        if en {
            self.title_en()
        } else {
            self.title()
        }
    }
}

/// Одна возможность агента.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Идентификатор: `files.read`, `web.search`.
    pub id: String,
    /// Название для интерфейса.
    pub name: String,
    /// Что возможность делает, одной строкой.
    pub what: String,
    /// Минимальный профиль железа.
    pub min_tier: Tier,
    /// Требуется ли модель с tool calling.
    #[serde(default)]
    pub needs_tool_calling: bool,
    /// Идентификатор разрешения, которым управляется.
    pub permission: String,
    /// Проверено гейтом на живом железе.
    #[serde(default)]
    pub verified: bool,
    /// Чем проверено или почему не проверено.
    #[serde(default)]
    pub evidence: String,
    /// Что сказать, когда возможности нет. Пишет человек, не модель.
    #[serde(default)]
    pub refusal: String,

    // Английские тексты того же человека. Русский остаётся
    // обязательным, английский — дополнение: интерфейс на любом языке
    // показывает русский, если перевода нет, а не пустоту.
    #[serde(default)]
    pub name_en: String,
    #[serde(default)]
    pub what_en: String,
    #[serde(default)]
    pub evidence_en: String,
    #[serde(default)]
    pub refusal_en: String,
}

impl Capability {
    /// Текст на выбранном языке. Пустой перевод берёт русский:
    /// пустая ячейка на экране хуже, чем текст на другом языке.
    fn по_языку(ru: &str, en: &str, en_выбран: bool) -> String {
        if en_выбран && !en.is_empty() {
            en.to_string()
        } else {
            ru.to_string()
        }
    }

    pub fn name_for(&self, en: bool) -> String {
        Self::по_языку(&self.name, &self.name_en, en)
    }

    pub fn what_for(&self, en: bool) -> String {
        Self::по_языку(&self.what, &self.what_en, en)
    }

    pub fn refusal_for_lang(&self, blocker: Blocker, en: bool) -> String {
        let отказ = Self::по_языку(&self.refusal, &self.refusal_en, en);
        if отказ.is_empty() {
            return format!(
                "{} {}: {}",
                self.name_for(en),
                if en {
                    "is not available"
                } else {
                    "недоступно"
                },
                blocker.title_for(en)
            );
        }
        отказ
    }
}

impl Capability {
    /// Сможет ли агент выполнить действие на этом профиле.
    ///
    /// Возвращает `None`, если может, иначе — причину.
    ///
    /// Порядок проверок не случаен. Железо проверяется **первым**,
    /// хотя проверка «не сделано» выглядит строже.
    ///
    /// Причина в том, что «ещё не сделано» обещает: подожди, и
    /// заработает. На машине, которой не хватает ресурсов, не
    /// заработает никогда. Обещание, которое не сбудется, хуже
    /// честного «у тебя не получится».
    pub fn blocked_for(&self, tier: Tier) -> Option<Blocker> {
        if tier < self.min_tier {
            return Some(Blocker::Hardware);
        }
        if !self.verified {
            // Непроверенное — значит «ещё не сделано». Иначе мы
            // врали бы пользователю о работоспособности.
            return Some(Blocker::NotBuilt);
        }
        None
    }

    /// Текст отказа. Это то, что агент говорит пользователю, и
    /// поэтому пишется человеком, а не собирается из строк.
    pub fn refusal_for(&self, blocker: Blocker) -> String {
        if self.refusal.is_empty() {
            return format!("{} недоступно: {}", self.name, blocker.title());
        }
        self.refusal.clone()
    }
}

/// Загруженный реестр.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub capabilities: Vec<Capability>,
}

impl Registry {
    /// Каталог лежит рядом с приложением, в установленной сборке — в
    /// `%APPDATA%\ArkHa\capabilities.json`. Порядок тот же, что у
    /// каталога рецептов: иначе один файл находился бы, а второй нет.
    pub fn candidate_paths() -> Vec<PathBuf> {
        let mut paths = vec![crate::profile::config_dir().join("capabilities.json")];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                paths.push(dir.join("data").join("capabilities.json"));
                paths.push(dir.join("capabilities.json"));
            }
        }
        if let Ok(dir) = std::env::var("ARKHA_DATA_DIR") {
            paths.push(Path::new(&dir).join("capabilities.json"));
        }
        paths.push(PathBuf::from("data").join("capabilities.json"));
        paths.push(PathBuf::from("../data/capabilities.json"));
        paths
    }

    pub fn load() -> Result<Self, String> {
        for path in Self::candidate_paths() {
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("не удалось прочитать {}: {e}", path.display()))?;
                let reg: Registry = serde_json::from_str(&text)
                    .map_err(|e| format!("повреждён {}: {e}", path.display()))?;
                return Ok(reg);
            }
        }
        Err(format!(
            "реестр возможностей не найден, искали в: {}",
            Self::candidate_paths()
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    pub fn get(&self, id: &str) -> Option<&Capability> {
        self.capabilities.iter().find(|c| c.id == id)
    }

    /// Что доступно на этом профиле.
    pub fn available(&self, tier: Tier) -> Vec<&Capability> {
        self.capabilities
            .iter()
            .filter(|c| c.blocked_for(tier).is_none())
            .collect()
    }

    /// Что недоступно, с причинами.
    pub fn unavailable(&self, tier: Tier) -> Vec<(&Capability, Blocker)> {
        self.capabilities
            .iter()
            .filter_map(|c| c.blocked_for(tier).map(|b| (c, b)))
            .collect()
    }

    /// Реестр в виде, который читает модель.
    ///
    /// Не только доступные: недоступные тоже нужны, иначе модель не
    /// знает, что они существуют, и не сможет предложить замену.
    pub fn for_model(&self, tier: Tier) -> Vec<ModelView> {
        self.capabilities
            .iter()
            .map(|c| {
                let blocked = c.blocked_for(tier);
                ModelView {
                    id: c.id.clone(),
                    what: c.what.clone(),
                    available: blocked.is_none(),
                    reason: blocked.map(|b| b.title().to_string()),
                }
            })
            .collect()
    }

    /// Проверка целостности: идентификаторы не должны повторяться.
    ///
    /// Дубль означает, что две записи спорят о возможности, и
    /// непонятно, какая победит. Молчаливый выбор был бы хуже
    /// отказа.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &self.capabilities {
            *seen.entry(c.id.as_str()).or_insert(0) += 1;
        }
        if let Some((id, n)) = seen.iter().find(|(_, n)| **n > 1) {
            return Err(format!("возможность {id} описана {n} раз"));
        }
        for c in &self.capabilities {
            if c.id.is_empty() {
                return Err("возможность без идентификатора".into());
            }
            // Проверенная возможность обязана объяснять, чем проверена.
            // Иначе `verified: true` — это чужое мнение.
            if c.verified && c.evidence.is_empty() {
                return Err(format!(
                    "возможность {} помечена проверенной, но чем проверено не указано",
                    c.id
                ));
            }
        }
        Ok(())
    }
}

/// То, что уходит в промпт модели.
#[derive(Debug, Clone, Serialize)]
pub struct ModelView {
    pub id: String,
    pub what: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(id: &str, tier: Tier, verified: bool) -> Capability {
        Capability {
            id: id.into(),
            name: id.into(),
            what: "что-то".into(),
            min_tier: tier,
            needs_tool_calling: true,
            permission: "files".into(),
            verified,
            evidence: if verified {
                String::from("гейт S3")
            } else {
                String::new()
            },
            refusal: String::new(),
            name_en: String::new(),
            what_en: String::new(),
            evidence_en: String::new(),
            refusal_en: String::new(),
        }
    }

    fn reg(caps: Vec<Capability>) -> Registry {
        Registry { capabilities: caps }
    }

    #[test]
    fn пустой_реестр_проходит_проверку() {
        assert!(reg(vec![]).validate().is_ok());
    }

    /// Непроверенное не значит «работает». Иначе мы врали бы.
    #[test]
    fn непроверенная_возможность_недоступна_даже_на_сверхмощном_железе() {
        let r = reg(vec![cap("files.read", Tier::T1, false)]);
        assert_eq!(
            r.get("files.read").unwrap().blocked_for(Tier::T5),
            Some(Blocker::NotBuilt)
        );
    }

    #[test]
    fn слабое_железо_даёт_причину_ресурсов() {
        let r = reg(vec![cap("vision.read", Tier::T4, true)]);
        assert_eq!(
            r.get("vision.read").unwrap().blocked_for(Tier::T3),
            Some(Blocker::Hardware)
        );
        assert_eq!(r.get("vision.read").unwrap().blocked_for(Tier::T4), None);
    }

    /// «Ещё не сделано» обещает, что подождёшь и заработает. На
    /// машине без нужного железа не заработает никогда, поэтому
    /// причина ресурсов должна побеждать.
    #[test]
    fn нехватка_железа_важнее_недоделки() {
        let mut c = cap("vision.read", Tier::T4, false);
        c.refusal = "нужен файл mmproj".into();
        assert_eq!(c.blocked_for(Tier::T1), Some(Blocker::Hardware));
    }

    /// Причина обязана быть названа. Молчаливый отказ хуже отказа
    /// с объяснением: пользователь не знает, что чинить.
    /// Причина обязана быть названа коротко: её читают в списке
    /// возможностей, где важна длина, а не подробность.
    #[test]
    fn у_каждой_причины_есть_короткий_заголовок() {
        for b in [
            Blocker::Hardware,
            Blocker::Model,
            Blocker::Permission,
            Blocker::NotConfigured,
            Blocker::NotBuilt,
        ] {
            let t = b.title();
            assert!(!t.is_empty());
            assert!(t.chars().count() <= 30, "слишком длинно для списка: {t}");
            assert!(
                !t.ends_with('.'),
                "заголовок причины не ставит точку: {t}"
            );
        }
    }

    /// Разные причины должны называться по-разному, иначе список
    /// возможностей врёт: «не хватает ресурсов» и «ещё не сделано»
    /// требуют от пользователя разного.
    #[test]
    fn причины_различаются_между_собой() {
        let all = [
            Blocker::Hardware,
            Blocker::Model,
            Blocker::Permission,
            Blocker::NotConfigured,
            Blocker::NotBuilt,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.title(), b.title(), "причины слились");
            }
        }
    }

    #[test]
    fn отказ_называет_причину_а_не_извиняется() {
        let mut c = cap("web.search", Tier::T1, true);
        c.refusal = "не задан адрес поискового инстанса".into();
        let r = reg(vec![c]);
        let text = r.get("web.search").unwrap().refusal_for(Blocker::NotConfigured);
        assert!(text.contains("не задан адрес"), "{text}");
        assert!(!text.contains("извините"), "извинение скрывает причину");
    }

    /// Модель должна знать и о недоступных: иначе не сможет предложить
    /// замену и просто промолчит.
    #[test]
    fn в_представление_для_модели_попадают_и_недоступные() {
        let r = reg(vec![
            cap("files.read", Tier::T1, true),
            cap("vision.read", Tier::T4, true),
        ]);
        let view = r.for_model(Tier::T2);
        assert_eq!(view.len(), 2);
        assert!(view[0].available);
        assert!(!view[1].available);
        assert!(view[1].reason.is_some());
    }

    #[test]
    fn дубль_идентификатора_ловится() {
        let r = reg(vec![cap("files.read", Tier::T1, true), cap("files.read", Tier::T2, true)]);
        assert!(r.validate().unwrap_err().contains("files.read"));
    }

    /// `verified: true` без доказательства — это чужое мнение,
    /// записанное в наш файл.
    #[test]
    fn проверенное_без_доказательства_ловится() {
        let mut c = cap("files.read", Tier::T1, true);
        c.evidence = String::new();
        assert!(reg(vec![c]).validate().unwrap_err().contains("чем проверено"));
    }

    #[test]
    fn настоящий_реестр_с_диска_проходит_проверку() {
        let r = Registry::load().expect("реестр должен лежать в data");
        r.validate().expect("реестр должен быть корректным");
        assert!(
            !r.capabilities.is_empty(),
            "пустой реестр бесполезен: агент не знает, что умеет"
        );
    }
}

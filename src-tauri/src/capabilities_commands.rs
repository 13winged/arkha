// Команды реестра возможностей для интерфейса.
//
// Форма ответа выбрана так, чтобы интерфейс не принимал решений сам.
// Причина недоступности приходит готовой: если дать интерфейсу
// решить самому, он рано или поздно покажет «недоступно» без причины,
// и это будет наша ложь, а не ограничение железа.

use serde::Serialize;

use crate::capabilities::{Blocker, Registry};
use crate::profile::{detect_and_persist, Tier};

/// Одна строка в отчёте для интерфейса.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub what: String,
    pub available: bool,
    /// Заголовок причины, если недоступно.
    pub blocked_by: Option<String>,
    /// Полный текст отказа, готовый к показать пользователю.
    pub refusal: Option<String>,
    /// Чем проверено. Пусто у непроверенных — и это видно.
    pub evidence: Option<String>,
    pub min_tier: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub tier: String,
    pub total: usize,
    pub available: usize,
    pub blocked: usize,
    /// Сколько возможностей проверено гейтом.
    pub verified: usize,
    pub rows: Vec<Row>,
}

/// Полный отчёт: что умеем, что нет и почему.
///
/// Язык приходит параметром, а не берётся из настроек: человек
/// переключает язык кнопкой и ждёт отклика немедленно, а читать
/// отчёт, собранный на прошлом языке, он не станет. Неизвестное
/// значение считается русским — молча угадывать язык нельзя.
#[tauri::command]
pub fn capabilities_report(lang: Option<String>) -> Result<Report, String> {
    let tier = detect_and_persist().effective();
    build_lang(tier, lang.as_deref() == Some("en"))
}

/// Отчёт на русском. Отдельная обёртка, чтобы вызывающие не знали
/// про языки: тесты и внутренний код не переводят.
pub fn build(tier: Tier) -> Result<Report, String> {
    build_lang(tier, false)
}

pub fn build_lang(tier: Tier, en: bool) -> Result<Report, String> {
    let reg = Registry::load()?;
    // Повреждённый реестр — повод остановиться, а не показать
    // пустой список: пустой список читается как «я ничего не умею».
    reg.validate()?;

    let mut rows = Vec::with_capacity(reg.capabilities.len());
    let mut available = 0usize;
    let mut verified = 0usize;

    for c in &reg.capabilities {
        if c.verified {
            verified += 1;
        }
        let blocker = c.blocked_for(tier);
        if blocker.is_none() {
            available += 1;
        }
        rows.push(Row {
            id: c.id.clone(),
            name: c.name_for(en),
            what: c.what_for(en),
            available: blocker.is_none(),
            blocked_by: blocker.map(|b: Blocker| b.title_for(en).to_string()),
            refusal: blocker.map(|b| c.refusal_for_lang(b, en)),
            evidence: if c.evidence.is_empty() {
                None
            } else {
                let текст = if en && !c.evidence_en.is_empty() {
                    c.evidence_en.clone()
                } else {
                    c.evidence.clone()
                };
                Some(текст)
            },
            min_tier: c.min_tier.as_str().to_string(),
        });
    }

    Ok(Report {
        tier: tier.as_str().to_string(),
        total: rows.len(),
        available,
        blocked: rows.len() - available,
        verified,
        rows,
    })
}

/// То, что уходит в промпт модели.
///
/// Отдельная команда, а не поле в отчёте: модель получает иное, чем
/// человек. Ей не нужны доказательства и названия профилей, ей
/// нужен список того, что можно, и того, что нельзя.
#[tauri::command]
pub fn capabilities_for_model() -> Result<Vec<crate::capabilities::ModelView>, String> {
    let tier = detect_and_persist().effective();
    let reg = Registry::load()?;
    reg.validate()?;
    Ok(reg.for_model(tier))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Отчёт без реестра должен падать, а не показывать пустоту.
    /// Пустой список в интерфейсе читается как «ничего не умею»,
    /// и это ложь, а не отсутствие данных.
    #[test]
    fn отчёт_считает_правдиво() {
        let r = build(Tier::T3).expect("реестр лежит в data");
        assert_eq!(r.total, r.available + r.blocked);
        assert!(r.total > 0);
        // Проверено гейтом три возможности: разговор (S2), чтение файлов и
        // навыки (S6). Число написано руками: новая гейтовая проверка
        // должна уронить тест и заставить подумать, а не молча
        // перестать проверяться.
        assert_eq!(r.verified, 6, "проверено гейтом: разговор, чтение, навыки, чтение окон, запуск, управление");
        assert_eq!(
            r.available, 6,
            "работают разговор, чтение, навыки и управление ПК, остальное написано, но не сделано"
        );
    }

    /// У каждой недоступной возможности должна быть названа причина.
    /// Иначе пользователь не знает, что чинить.
    #[test]
    fn у_недоступной_возможности_есть_причина_и_отказ() {
        let r = build(Tier::T3).unwrap();
        for row in r.rows.iter().filter(|x| !x.available) {
            assert!(row.blocked_by.is_some(), "{}: нет причины", row.id);
            assert!(
                row.refusal.as_ref().is_some_and(|t| !t.is_empty()),
                "{}: пустой отказ",
                row.id
            );
        }
    }

    /// Проверенная возможность обязана объяснять, чем проверена.
    #[test]
    fn у_проверенной_возможности_есть_доказательство() {
        let r = build(Tier::T3).unwrap();
        for row in r.rows.iter().filter(|x| x.available) {
            assert!(
                row.evidence.as_ref().is_some_and(|t| !t.is_empty()),
                "{}: проверено, но чем — не сказано",
                row.id
            );
        }
    }

    /// Модель должна видеть и недоступные, иначе не сможет предложить
    /// замену и просто промолчит.
    #[test]
    fn модель_видит_и_недоступные() {
        let r = build(Tier::T3).unwrap();
        let view = Registry::load().unwrap().for_model(Tier::T3);
        assert_eq!(view.len(), r.total);
        assert!(view.iter().any(|v| !v.available));
    }
}

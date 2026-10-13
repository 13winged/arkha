// Запуск навыка как инструмента.
//
// Навык вызывается моделью так же, как любой другой инструмент: по
// имени и с аргументами. Отдельный механизм «подставить текст в
// системную строку» сознательно не делался — он обошёл бы выбор
// инструментов и дал бы модели инструкцию без записи в журнал.
//
// **Навык не даёт прав.** Он только возвращает текст шагов. Дальше
// модель вызывает обычные инструменты, и каждый из них по-прежнему
// проверяет разрешения и спрашивает подтверждение. Проверить это
// стоит отдельно, тест внизу это делает.

use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::RwLock;

use super::{Tool, ToolError, ToolOutput, ToolSpec};
use crate::permissions::Settings;
use crate::skills::Набор;

/// Набор навыков, доступный инструменту.
///
/// RwLock, а не просто набор: навыки можно перечитать при изменении
/// папки, и делается это из интерфейса. Копия набора в инструменте
/// разъехалась бы с тем, что видит пользователь.
pub type Хранилище = Arc<RwLock<Набор>>;

/// Запуск навыка по имени.
pub struct RunSkill {
    pub наборы: Хранилище,
}

impl Tool for RunSkill {
    fn spec(&self, settings: &Settings) -> ToolSpec {
        let набор = self.наборы.read().map(|н| н.clone()).unwrap_or_default();
        // Навык должен быть виден модели, даже если его действия пока
        // не разрешены: иначе она скажет «навыка такого нет», хотя он
        // лежит на диске. Явно выключенные возможности — другое дело.
        let можно = |т: &str| settings.показывать(т);
        let доступны = набор.доступные(&можно);
        // Список доступных навыков идёт в описание: модель выбирает
        // по нему. Недоступные не упоминаются — иначе она выберет
        // то, что упрётся в стену.
        let описание = if доступны.is_empty() {
            "Запустить навык. Сейчас навыков нет.".to_string()
        } else {
            let строки: Vec<String> = доступны
                .iter()
                .map(|н| format!("- {}: {}", н.name, н.description))
                .collect();
            format!(
                "Запустить навык — заранее описанную процедуру, написанную человеком. \
                 Если пользователь попросил выполнить навык по имени — вызови этот \
                 инструмент с этим именем и только потом выполняй шаги. Не выполняй \
                 шаги наугад: они могут отличаться от того, что ты сам предположишь. \
                 Навыки:\n{}",
                строки.join("\n")
            )
        };
        // Аргументы навыков объединяются в одну схему: имена
        // аргументов у разных навыков разные, а схема запроса одна.
        let параметры = схема_объединённая(&доступны);
        ToolSpec {
            id: "skill.run".into(),
            capability: "skill.execute".into(),
            name: "Запустить навык".into(),
            description: описание,
            parameters: параметры,
            // Сам запуск ничего не меняет: он возвращает текст
            // шагов. Меняет систему то, что модель вызовет дальше, и
            // для этого у каждого инструмента своя проверка.
            side_effecting: false,
        }
    }

    fn call(&self, args: &Value, settings: &Settings) -> Result<ToolOutput, ToolError> {
        let Some(имя) = args.get("name").and_then(|v| v.as_str()) else {
            return Err(ToolError::BadArgs(
                "нет обязательного поля name — имя навыка".into(),
            ));
        };
        let Ok(набор) = self.наборы.read() else {
            return Err(ToolError::Unreadable("список навыков занят".into()));
        };

        let Some(навык) = набор.взять(имя) else {
            // Список доступных показываем: иначе модель будет
            // угадывать имена по нескольку заходов.
            let есть: Vec<String> = набор.навыки.iter().map(|н| н.name.clone()).collect();
            return Err(ToolError::Unknown(format!(
                "навыка «{имя}» нет. Есть: {}",
                if есть.is_empty() { "ни одного".to_string() } else { есть.join(", ") }
            )));
        };

        // Навык запускается, только если всё, что он объявил, разрешено.
// Проверка здесь, а не в цикле: навык не выполняет действия сам, он
// отдаёт шаги, и запрет должен быть виден сразу, а не на середине
// процедуры.
        let запрещено: Vec<String> = навык
            .tools
            .iter()
            .filter(|т| !settings.allows(т).allowed)
            .cloned()
            .collect();
        if !запрещено.is_empty() {
            return Err(ToolError::Denied(format!(
                "навык «{}» требует то, что не разрешено: {}. Включи это в знакомстве",
                навык.name,
                запрещено.join(", ")
            )));
        }

        Ok(ToolOutput::read(
            навык.текст_для_модели(args),
            format!("запущен навык «{}»", навык.name),
        ))
    }
}

/// Схема аргументов навыков: имя плюс строки.
///
/// Все аргументы объявлены строками и необязательными: у разных
/// навыков они разные, а схема запроса у инструмента одна.
fn схема_объединённая(доступны: &[&crate::skills::Skill]) -> Value {
    let mut свойства = serde_json::Map::new();
    свойства.insert(
        "name".into(),
        json!({
            "type": "string",
            "description": "имя навыка из списка выше",
        }),
    );
    let mut собраны: Vec<String> = Vec::new();
    for н in доступны {
        if let Some(м) = н.parameters.get("properties").and_then(|v| v.as_object()) {
            for (к, знач) in м {
                if !собраны.contains(к) {
                    собраны.push(к.clone());
                    свойства.insert(к.clone(), знач.clone());
                }
            }
        }
    }
    json!({
        "type": "object",
        "properties": свойства,
        "required": ["name"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{Skill, Набор};
    use std::path::PathBuf;

    fn навык(имя: &str, описание: &str, инструменты: &[&str]) -> Skill {
        Skill {
            name: имя.into(),
            description: описание.into(),
            tools: инструменты.iter().map(|с| с.to_string()).collect(),
            parameters: json!({"type": "object", "properties": {}, "required": []}),
            steps: "1. Сделай то-то.".into(),
            file: PathBuf::from(format!("{имя}.md")),
        }
    }

    fn инструмент(навыки: Vec<Skill>) -> RunSkill {
        RunSkill {
            наборы: Arc::new(RwLock::new(Набор { навыки, сломанные: vec![] })),
        }
    }

    #[test]
    fn навык_запускается_и_возвращает_шаги() {
        let т = инструмент(vec![навык("a", "Делает дело", &["files.read"])]);
        let s = Settings::default();
        let out = т.call(&json!({"name": "a"}), &s).expect("навык должен запуститься");
        assert!(out.text.contains("Выполняй его шаги"), "{}", out.text);
        assert!(out.text.contains("Сделай то-то"), "{}", out.text);
        assert!(!out.changed, "сам запуск навыка ничего не меняет");
    }

    #[test]
    fn неизвестный_навык_показывает_список() {
        let т = инструмент(vec![навык("a", "Делает дело", &["files.read"])]);
        let s = Settings::default();
        let e = т.call(&json!({"name": "b"}), &s).unwrap_err();
        assert!(matches!(e, ToolError::Unknown(_)), "{e:?}");
        assert!(e.to_string().contains("a"), "{e}");
    }

    #[test]
    fn без_имени_отказ() {
        let т = инструмент(vec![]);
        let s = Settings::default();
        assert!(matches!(
            т.call(&json!({}), &s).unwrap_err(),
            ToolError::BadArgs(_)
        ));
    }

    /// Навык скрывается только если человек **выключил** нужную
    /// возможность.
    ///
    /// Не выданное по умолчанию — не то же, что выключенное. Навык на
    /// запись виден модели и без разрешения: иначе она ответит
    /// «навыка нет», хотя он лежит на диске и ждёт включения. Именно
    /// это выяснилось на живом железе с `apps.launch`.
    #[test]
    fn в_описании_скрыт_только_выключенное() {
        let т = инструмент(vec![
            навык("читай", "Читает", &["files.read"]),
            навык("пиши", "Пишет", &["files.write"]),
        ]);
        let mut s = Settings::default();
        // Пока не выключено — видны оба.
        assert!(т.spec(&s).description.contains("пиши"));

        // Выключили запись — навык пропадает.
        s.capabilities.insert(
            "files.write".into(),
            crate::permissions::CapabilityRule { enabled: false, mode: None },
        );
        let описание = т.spec(&s).description;
        assert!(описание.contains("читай"), "{описание}");
        assert!(
            !описание.contains("пиши"),
            "навык с выключенной возможностью не должен попадать в описание: {описание}"
        );
    }

    /// Навык с записью не должен запускаться без разрешения на запись,
/// даже если сам инструмент-запуск безобиден.
    #[test]
    fn навык_с_записью_не_запускается_без_разрешения() {
        let т = инструмент(vec![навык("пиши", "Пишет", &["files.write"])]);
        let s = Settings::default();
        let e = т.call(&json!({"name": "пиши"}), &s).unwrap_err();
        assert!(matches!(e, ToolError::Denied(_)), "{e:?}");
        assert!(e.to_string().contains("files.write"), "{e}");
    }

    /// Навык только на чтение запускается при любом режиме, включая
    /// «только показывать»: он и не меняет ничего.
    #[test]
    fn навык_только_на_чтение_запускается_всегда() {
        let т = инструмент(vec![навык("читай", "Читает", &["files.read"])]);
        let mut s = Settings::default();
        s.executor = crate::permissions::Executor::DryRun;
        assert!(т.call(&json!({"name": "читай"}), &s).is_ok());
        s.executor = crate::permissions::Executor::Live;
        assert!(т.call(&json!({"name": "читай"}), &s).is_ok());
    }

    /// Запуск ничего не меняет сам: меняют инструменты шагов.
    #[test]
    fn запуск_навыка_сам_ничего_не_меняет() {
        let т = инструмент(vec![навык("читай", "Читает", &["files.read"])]);
        let spec = т.spec(&Settings::default());
        assert!(!spec.side_effecting);
        assert_eq!(spec.capability, "skill.execute");
    }

    /// Навык не должен обходить разрешения: он только возвращает текст,
    /// а работа идёт обычными инструментами.
    #[test]
    fn навык_не_обходит_разрешения() {
        // Навык заявляет files.write, а разрешено только чтение.
        let т = инструмент(vec![навык("пиши", "Пишет", &["files.write"])]);
        let mut s = Settings::default();
        s.executor = crate::permissions::Executor::Live;
        s.capabilities.insert(
            "files.write".into(),
            crate::permissions::CapabilityRule { enabled: false, mode: None },
        );
        // Ни запуск навыка, ни инструмент записи не должны быть
        // доступны.
        assert!(
            т.call(&json!({"name": "пиши"}), &s).is_err(),
            "навык с записью не должен запускаться при выключенной записи"
        );
        let реестр = crate::tools::Registry::allowed(&s);
        assert!(
            !реестр.ids().iter().any(|и| и.starts_with("fs.write")),
            "запись не должна появляться в реестре без разрешения"
        );
    }
}
// Гейт S3: реестр возможностей и разрешения на живом железе.
//
// Смысл гейта не в том, что код компилируется. В том, чтобы
// заявленное и работающее совпадало: агент должен знать свои границы,
// и пользователь должен видеть, где они проходят.
//
// Пропуск этого гейта не считается успехом. Если файла реестра нет,
// тест падает — так же, как падает гейт S2 без модели.

use archotic_lib::capabilities::Registry;
use archotic_lib::permissions::{Executor, Settings};
use archotic_lib::profile::{detect_and_persist, Tier};

/// Реестр лежит в `data`, путь — как в приложении.
fn registry() -> Registry {
    let r = Registry::load().expect("реестр возможностей должен лежать в data");
    r.validate().expect("реестр должен быть корректным");
    r
}

/// На этой машине работает ровно то, что проверено гейтом, и
/// ничего сверх того. Лишнее «работает» — тоже ложь.
#[test]
fn гейт_s3_доступно_только_проверенное() {
    let tier = detect_and_persist().effective();
    eprintln!("профиль: {}", tier.as_str());

    let reg = registry();
    let available = reg.available(tier);
    eprintln!("доступно {} из {}", available.len(), reg.capabilities.len());

    for c in &available {
        eprintln!("  работает: {}", c.id);
    }

    assert!(
        !available.is_empty(),
        "ничего не работает: проверьте, что профиль определён верно"
    );

    // Каждая доступная возможность обязана быть проверена и объяснять,
    // чем. Непроверенное не должно попадать в «работает».
    for c in &available {
        assert!(
            c.verified,
            "{} помечено доступным, но не проверено",
            c.id
        );
        assert!(
            !c.evidence.is_empty(),
            "{} доступно без указания, чем проверено",
            c.id
        );
    }

    // Разговор проверен гейтом S2 — он обязан быть доступен везде,
    // где есть модель.
    assert!(
        reg.get("chat.local").is_some_and(|c| c.blocked_for(tier).is_none()),
        "разговор проверен гейтом S2 и должен быть доступен"
    );
}

/// Каждая недоступная возможность обязана объяснять, почему.
/// Молчаливый отказ хуже явного: пользователь не знает, что чинить.
#[test]
fn гейт_s3_у_каждого_отказа_есть_причина() {
    let tier = detect_and_persist().effective();
    let reg = registry();

    let blocked = reg.unavailable(tier);
    eprintln!("недоступно {}", blocked.len());

    assert!(
        !blocked.is_empty(),
        "если всё доступно, список причин пустеет и перестаёт проверяться"
    );

    for (c, blocker) in &blocked {
        let text = c.refusal_for(*blocker);
        eprintln!("  {} — {}", c.id, text);
        assert!(!text.is_empty(), "{}: пустой отказ", c.id);
        assert!(
            text.len() > blocker.title().len(),
            "{}: отказ не объясняет причину, только повторяет её",
            c.id
        );
    }
}

/// Причина недоступности должна зависеть от железа.
///
/// Количество доступного проверять бессмысленно: пока проверена одна
/// возможность, она доступна на всех профилях, и счёт будет один и
/// тот же. А вот причина — разная, и именно она полезна.
/// На слабом железе «не хватает ресурсов» вместо «ещё не сделано».
#[test]
fn гейт_s3_причина_зависит_от_железа() {
    use archotic_lib::capabilities::Blocker;

    let reg = registry();
    let vision = reg.get("vision.read").expect("vision.read описан в реестре");

    assert_eq!(
        vision.blocked_for(Tier::T1),
        Some(Blocker::Hardware),
        "на T1 должно быть «не хватает ресурсов», а не «ещё не сделано»"
    );
    assert_eq!(
        vision.blocked_for(Tier::T5),
        Some(Blocker::NotBuilt),
        "на T5 resources хватает, остаётся «ещё не сделано»"
    );

    // Разговор доступен везде, где есть модель.
    let chat = reg.get("chat.local").expect("chat.local описан в реестре");
    for tier in [Tier::T1, Tier::T2, Tier::T3, Tier::T4, Tier::T5] {
        assert_eq!(
            chat.blocked_for(tier),
            None,
            "разговор должен быть доступен на {tier:?}"
        );
    }
}

/// Модель должна видеть недоступные возможности, иначе не сможет
/// предложить замену и просто промолчит.
#[test]
fn гейт_s3_модель_видит_границы() {
    let tier = detect_and_persist().effective();
    let view = registry().for_model(tier);

    assert!(!view.is_empty());
    assert!(
        view.iter().any(|v| !v.available),
        "модель не видит недоступных возможностей и не сможет о них сказать"
    );
    for v in view.iter().filter(|v| !v.available) {
        assert!(
            v.reason.is_some(),
            "{}: недоступна без причины",
            v.id
        );
    }
}

/// Настройки по умолчанию обязаны быть безопасными, а на диске —
///
/// не опаснее. Режим выполнения в файле означает, что кто-то
/// включил его руками, и это надо видеть.
#[test]
fn гейт_s3_разрешения_безопасны() {
    let s = Settings::load().expect("настройки должны читаться");

    eprintln!("executor: {:?}", s.executor);
    eprintln!("запрещено: {}", s.deny_apps.join(", "));

    assert!(!s.cloud, "облако включено в настройках");

    // Секретарь обязан быть в собственном запрете: агент не должен
    // управлять интерфейсом, через который с ним говорят.
    assert!(
        s.is_denied_app("local.secretary.app"),
        "секретаря нет в запрете: агент может закрыть окно и ответить за пользователя"
    );

    // Если в файле live, это должно быть заметно, а не молчаливо.
    if s.executor == Executor::Live {
        eprintln!("ВНИМАНИЕ: в настройках включено выполнение действий");
    }
}

/// Запись и чтение — разные вещи. Пока агент умеет только читать,
/// путать их опасно: подтверждение на чтение раздражает, а его
/// отсутствие на записи опасно.
#[test]
fn гейт_s3_чтение_и_запись_различаются() {
    let s = Settings::load().expect("настройки должны читаться");

    assert!(!s.needs_approval("files.read"), "чтение не должно требовать подтверждения");
    assert!(s.needs_approval("files.write"), "запись обязана требовать подтверждения");

    let dry = Settings::default();
    assert!(!dry.allows("files.write").allowed, "в dry-run запись запрещена");
    assert!(dry.allows("files.read").allowed, "в dry-run чтение разрешено");
}

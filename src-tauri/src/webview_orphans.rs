// Уборка осиротевших обработчиков сбоев WebView2.
//
// Каждое окно Tauri — это отдельный браузерный процесс msedgewebview2.exe
// и его обработчик сбоев (`--type=crashpad-handler`). В диспетчере задач
// Windows такой обработчик подписан как «Сбойная панель»: около 3 МБ,
// состояние «Приостановлено», и по одному на каждое окно.
//
// Пока браузер жив, обработчик нужен. Когда браузер завершается, он
// должен уйти вместе с ним — но если браузерный процесс погиб аварийно
// (перезапуск после падения отрисовщика, снятие задачи, закрытие панели),
// обработчик остаётся висеть. За час работы их набирается три десятка,
// и это выглядит как «сбойные панели», хотя к интерфейсу отношения нет.
//
// Поэтому раз в полминуты смотрим на msedgewebview2.exe и снимаем те,
// чей родитель уже мёртв: работать им всё равно не для кого.

use std::time::Duration;

/// Как часто убирать сирот.
const ЧАСТОТА: Duration = Duration::from_secs(30);

/// Имя браузерного процесса WebView2.
#[cfg(target_os = "windows")]
const БРАУЗЕР: &str = "msedgewebview2.exe";

/// Убрать процессы WebView2, чей родитель уже завершился.
#[cfg(target_os = "windows")]
pub fn reap_orphans() {
    use std::collections::HashSet;

    use sysinfo::{ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);

    let alive: HashSet<u32> = system.processes().keys().map(|p| p.as_u32()).collect();

    for process in system.processes().values() {
        if !process.name().to_string_lossy().eq_ignore_ascii_case(БРАУЗЕР) {
            continue;
        }

        // Родитель жив — это рабочий процесс открытого окна.
        if process
            .parent()
            .map(|p| alive.contains(&p.as_u32()))
            .unwrap_or(false)
        {
            continue;
        }

        let _ = process.kill();
        tracing::info!("уборка: снят осиротевший процесс webview2");
    }
}

#[cfg(not(target_os = "windows"))]
pub fn reap_orphans() {}

/// Фоновая уборка. Запускается один раз за всё время жизни приложения.
#[cfg(target_os = "windows")]
pub fn start_sweeper() {
    use std::sync::Once;

    static START: Once = Once::new();
    START.call_once(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(ЧАСТОТА);
            reap_orphans();
        });
    });
}

#[cfg(not(target_os = "windows"))]
pub fn start_sweeper() {}

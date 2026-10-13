// Синтез речи через `llama-tts.exe`.
//
// Отдельный процесс, а не эндпоинт сайдкара: в llama.cpp b11583
// эндпоинта `/v1/audio/speech` нет вообще, и это проверялось на
// самой сборке, а не по документации. Синтез запускается на один
// запрос и уходит — держать его постоянно незачем: модель 0.96 ГиБ
// на карте это место, которого на 8 ГиБ не хватает агенту.
//
// Числа сняты на этой машине и лежат в `docs/10-замеры.md`:
// 3.60 с звука за 0.93 с вычислений, то есть 3.87× реального времени.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{ошибка, Голос};

/// Синтезировать текст в звуковой файл.
///
/// Возвращает путь к файлу и его длительность в миллисекундах.
/// Возвращает, а не играет: воспроизведение — дело интерфейса, и
/// держать синтез ради «а вдруг» не нужно.
pub fn синтез(г: &Голос, текст: &str) -> Result<(PathBuf, u64), String> {
    if текст.trim().is_empty() {
        return Err(ошибка("нечего произносить: текст пустой"));
    }
    let exe = г.путь_к("llama-tts.exe")?;
    let модель = г.модель("Qwen3-TTS-1.7B-Q4_K_M.gguf")?;
    let проектор = г.модель("Qwen3-TTS-mmproj-Q8_0.gguf")?;
    let куда = std::env::temp_dir().join(format!(
        "archotic-голос-{}-{}.wav",
        std::process::id(),
        г.счётчик_следующий()
    ));
    let _ = std::fs::remove_file(&куда);

    let вывод = Command::new(&exe)
        .args([
            "-m",
            &модель.to_string_lossy(),
            "--mmproj",
            &проектор.to_string_lossy(),
            "-ngl",
            "99",
            "-p",
            текст,
            "--tts-lang",
            "ru",
            "-o",
            &куда.to_string_lossy(),
        ])
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .env("PATH", crate::model_manager::prepend_path(&crate::model_manager::bin_dir().join("cudart")))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("не удалось запустить синтез: {e}"))?;

    if !куда.exists() {
        // Причина — в логе `llama-tts`, а не в нашем сообщении:
        // без него человек увидит «что-то сломалось» вместо «модель
        // голоса не скачана», и чинить будет нечего.
        let хвост = String::from_utf8_lossy(&вывод.stderr);
        let последние: Vec<&str> = хвост.lines().rev().take(3).collect();
        return Err(format!(
            "синтез не дал звука{}{}",
            if последние.is_empty() { "" } else { ": " },
            последние.join(" | ")
        ));
    }
    let размер = std::fs::metadata(&куда).map(|м| м.len()).unwrap_or(0);
    Ok((куда, размер))
}
//! Бенчмарк расчета среднеквадратического значения (RMS) аудио.
//!
//! Сравнивает последовательный и параллельный (rayon) подсчет.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rayon::iter::{IndexedParallelIterator, ParallelIterator};

/// Генерирует тестовый аудиобуфер: белый шум в диапазоне [-1, 1].
fn шум(размер: usize) -> Vec<f32> {
    (0..размер).map(|i| {
        let x = (i as f64 * 0.1).sin() as f32 * 0.5;
        x
    }).collect()
}

fn rms_последовательно(б: &mut Criterion) {
    let звук = шум(1_000_000); // ~62 секунды при 16 кГц
    б.bench_function("rms последовательно", |б| {
        б.iter(|| {
            let сумма: f64 = звук
                .iter()
                .map(|с| (*с as f64) * (*с as f64))
                .sum();
            let _ = (сумма / звук.len() as f64).sqrt();
        })
    });
}

fn rms_параллельно(б: &mut Criterion) {
    let звук = шум(1_000_000);
    б.bench_function("rms параллельно (rayon)", |б| {
        б.iter(|| {
            let сумма: f64 = звук
                .par_chunks(4096)
                .map(|часть| часть.iter().map(|с| (*с as f64) * (*с as f64)).sum::<f64>())
                .sum();
            let _ = (сумма / звук.len() as f64).sqrt();
        })
    });
}

criterion_group!(rms, rms_последовательно, rms_параллельно);
criterion_main!(rms);

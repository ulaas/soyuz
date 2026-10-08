//! Natural sort of 1 000 000 file names (`cargo bench -p soyuz-core --bench sort`).

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use soyuz_core::naming::natural_cmp;

/// Deterministic, realistic-looking names: mixed case, digit runs, extensions.
fn names(n: usize) -> Vec<String> {
    let stems = ["IMG_", "report v", "Track ", "photo-", "Backup ", "invoice_2024-", "Chapter", "x"];
    let exts = ["jpg", "pdf", "mp3", "txt", "RS", "tar.gz"];
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    (0..n)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let stem = stems[(state % stems.len() as u64) as usize];
            let ext = exts[((state >> 8) % exts.len() as u64) as usize];
            format!("{stem}{}.{}{ext}", (state >> 16) % 100_000, i % 7)
        })
        .collect()
}

fn sort(c: &mut Criterion) {
    let n = 1_000_000;
    let input = names(n);
    let mut g = c.benchmark_group("sort");
    g.throughput(Throughput::Elements(n as u64));
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(15));
    g.bench_function("natural_ci/1M", |b| {
        b.iter_batched(
            || input.clone(),
            |mut v| {
                v.sort_by(|a, b| natural_cmp(a, b, true));
                black_box(v)
            },
            BatchSize::LargeInput,
        )
    });
    g.bench_function("natural_ci_unstable/1M", |b| {
        b.iter_batched(
            || input.clone(),
            |mut v| {
                v.sort_unstable_by(|a, b| natural_cmp(a, b, true));
                black_box(v)
            },
            BatchSize::LargeInput,
        )
    });
    g.finish();
}

criterion_group!(benches, sort);
criterion_main!(benches);

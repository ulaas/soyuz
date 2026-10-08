//! Listing throughput: one folder with `SOYUZ_BENCH_LIST_N` (default 100 000) empty files.
//!
//! `cargo bench -p soyuz-fs --bench listing`. On Windows `read_dir` uses
//! `NtQueryDirectoryFile` with large buffers; `find_first_file` is the previous
//! `FindFirstFileExW(LARGE_FETCH)` path, kept for comparison.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use soyuz_core::{Control, OpControl};
use soyuz_fs::{ListOptions, read_dir, read_dir_streaming};

fn entries() -> usize {
    std::env::var("SOYUZ_BENCH_LIST_N").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000)
}

fn listing(c: &mut Criterion) {
    let n = entries();
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..n {
        std::fs::File::create(dir.path().join(format!("file-{i:07}.dat"))).expect("create");
    }
    let path = dir.path();
    let opts = ListOptions::default();

    let mut g = c.benchmark_group("listing");
    g.throughput(Throughput::Elements(n as u64));
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(10));
    g.bench_function(format!("read_dir/{n}"), |b| {
        b.iter(|| {
            let v = read_dir(black_box(path), &opts).expect("read_dir");
            assert_eq!(v.len(), n);
            v
        })
    });
    g.bench_function(format!("read_dir_streaming/{n}"), |b| {
        let ctl = OpControl::new();
        b.iter(|| {
            let mut count = 0;
            read_dir_streaming(path, &opts, &ctl, &mut |chunk| {
                count += chunk.len();
                Control::Continue
            })
            .expect("stream");
            assert_eq!(count, n);
        })
    });
    g.bench_function(format!("find_first_file/{n}"), |b| {
        b.iter(|| {
            let got = soyuz_fs::list::bench_support::read_dir_find_first_file(black_box(path)).expect("list");
            assert_eq!(got, n);
        })
    });
    for (label, nt) in [("raw_ntquerydirectoryfile", true), ("raw_findfirstfileex", false)] {
        g.bench_function(format!("{label}/{n}"), |b| {
            b.iter(|| assert_eq!(soyuz_fs::list::bench_support::count_raw(black_box(path), nt).expect("raw"), n))
        });
    }
    g.finish();
}

criterion_group!(benches, listing);
criterion_main!(benches);

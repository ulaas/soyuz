//! Search throughput over a generated tree (`cargo bench -p soyuz-search --bench search`):
//! 200 folders × 100 files (20 000 entries), each file a few lines of text.

use std::sync::Arc;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use soyuz_core::{NamePatternSpec, PatternKind};
use soyuz_search::{ContentPatternSpec, SearchEvent, SearchQuery, SearchStats, start};

const DIRS: usize = 200;
const FILES: usize = 100;

fn build_tree(root: &std::path::Path) {
    for d in 0..DIRS {
        let dir = root.join(format!("dir-{d:03}"));
        std::fs::create_dir(&dir).expect("mkdir");
        for f in 0..FILES {
            let body = format!("line one of file {f}\nsome filler text {d}\nneedle-{}\nend\n", (d * FILES + f) % 997);
            std::fs::write(dir.join(format!("note-{f:03}.txt")), body).expect("write");
        }
    }
}

fn run(q: &SearchQuery) -> SearchStats {
    let h = start(q.clone(), 0, Arc::new(|| {})).expect("pattern");
    loop {
        match h.rx.recv().expect("search ended without Done") {
            SearchEvent::Done(stats) => return stats,
            _ => continue,
        }
    }
}

fn search(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("tempdir");
    build_tree(dir.path());
    let base = SearchQuery { roots: vec![dir.path().to_path_buf()], max_results: 0, ..SearchQuery::default() };
    let total = (DIRS * FILES + DIRS) as u64;

    let mut g = c.benchmark_group("search");
    g.throughput(Throughput::Elements(total));
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(10));

    let by_name = SearchQuery {
        name: Some(NamePatternSpec { kind: PatternKind::Substring, text: "note-04".into(), case_sensitive: false }),
        ..base.clone()
    };
    g.bench_function("name_substring/20k", |b| b.iter(|| assert_eq!(run(&by_name).hits, DIRS as u64 * 10)));

    let fuzzy = SearchQuery {
        name: Some(NamePatternSpec { kind: PatternKind::Fuzzy, text: "nt99".into(), case_sensitive: false }),
        ..base.clone()
    };
    g.bench_function("name_fuzzy/20k", |b| b.iter(|| run(&fuzzy)));

    let content = SearchQuery {
        content: Some(ContentPatternSpec { regex: true, text: "^needle-42$".into(), case_sensitive: true }),
        ..base.clone()
    };
    g.bench_function("content_literal/20k", |b| b.iter(|| run(&content)));
    g.finish();
}

criterion_group!(benches, search);
criterion_main!(benches);

//! Fixture-tree tests for the streaming search.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use soyuz_core::{EntryKindSet, NamePatternSpec, PatternKind};
use soyuz_search::{ContentMatch, ContentPatternSpec, SearchEvent, SearchHandle, SearchQuery, SearchStats, start};

fn write(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

/// ```text
/// src/main.rs            "fn main() {\n    println!(\"needle\");\n}\n"
/// src/lib.rs             "pub fn Needle() {}\n"
/// src/util/helpers.rs    "// nothing here\n"
/// docs/report.txt        "alpha\nbeta needle gamma\n"
/// docs/Report-2024.md    4096 bytes of 'x'
/// data/blob.bin          "needle\0binary"
/// data/big.log           20 KiB with "needle" at the end
/// .hidden/secret.rs      "needle"
/// target/build.rs        "needle"   (git-ignored)
/// .gitignore             "target/\n"
/// ```
fn fixture() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    write(r, "src/main.rs", b"fn main() {\n    println!(\"needle\");\n}\n");
    write(r, "src/lib.rs", b"pub fn Needle() {}\n");
    write(r, "src/util/helpers.rs", b"// nothing here\n");
    write(r, "docs/report.txt", b"alpha\nbeta needle gamma\n");
    write(r, "docs/Report-2024.md", &[b'x'; 4096]);
    write(r, "data/blob.bin", b"needle\0binary");
    let mut big = vec![b'a'; 20 * 1024];
    big.extend_from_slice(b"\nneedle\n");
    write(r, "data/big.log", &big);
    write(r, ".hidden/secret.rs", b"needle");
    write(r, "target/build.rs", b"needle");
    write(r, ".gitignore", b"target/\n");
    t
}

struct Outcome {
    hits: Vec<(PathBuf, Vec<ContentMatch>)>,
    stats: SearchStats,
    first_hit: Option<Duration>,
}

impl Outcome {
    fn rels(&self, root: &Path) -> BTreeSet<String> {
        self.hits.iter().map(|(p, _)| p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/")).collect()
    }
}

fn drain(h: &SearchHandle) -> Outcome {
    let t0 = Instant::now();
    let mut hits = Vec::new();
    let mut first_hit = None;
    loop {
        match h.rx.recv_timeout(Duration::from_secs(20)).expect("search stalled") {
            SearchEvent::Hits(v) => {
                first_hit.get_or_insert(t0.elapsed());
                hits.extend(v.into_iter().map(|(e, m)| (e.path, m)));
            }
            SearchEvent::Done(stats) => return Outcome { hits, stats, first_hit },
            SearchEvent::Progress { .. } | SearchEvent::Error(..) => {}
        }
    }
}

fn run(q: SearchQuery) -> Outcome {
    let h = start(q, 0, Arc::new(|| {})).unwrap();
    let out = drain(&h);
    // The worker flags completion right after sending `Done`.
    let t0 = Instant::now();
    while !h.is_done() && t0.elapsed() < Duration::from_secs(5) {
        std::thread::yield_now();
    }
    assert!(h.is_done());
    out
}

fn q(root: &Path) -> SearchQuery {
    SearchQuery { roots: vec![root.to_path_buf()], ..Default::default() }
}

fn name(kind: PatternKind, text: &str) -> Option<NamePatternSpec> {
    Some(NamePatternSpec::new(kind, text))
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn glob_hits() {
    let t = fixture();
    let out = run(SearchQuery { name: name(PatternKind::Glob, "*.rs"), ..q(t.path()) });
    // Hidden dirs are skipped by default; .gitignore is not respected by default.
    assert_eq!(out.rels(t.path()), set(&["src/main.rs", "src/lib.rs", "src/util/helpers.rs", "target/build.rs"]));
    assert_eq!(out.stats.hits, 4);
    assert!(!out.stats.truncated);
    assert!(out.stats.scanned >= 10);
    assert!(out.first_hit.unwrap() < Duration::from_secs(2));
}

#[test]
fn hidden_and_gitignore_options() {
    let t = fixture();
    let base = SearchQuery { name: name(PatternKind::Glob, "*.rs"), ..q(t.path()) };
    let out = run(SearchQuery { include_hidden: true, respect_gitignore: true, ..base.clone() });
    assert_eq!(out.rels(t.path()), set(&["src/main.rs", "src/lib.rs", "src/util/helpers.rs", ".hidden/secret.rs"]));
    let out = run(SearchQuery { include_hidden: true, ..base });
    assert_eq!(out.stats.hits, 5);
}

#[test]
fn regex_substring_and_fuzzy_names() {
    let t = fixture();
    let out = run(SearchQuery { name: name(PatternKind::Regex, r"^report.*\.(txt|md)$"), ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["docs/report.txt", "docs/Report-2024.md"]));

    let cs = NamePatternSpec { case_sensitive: true, ..NamePatternSpec::new(PatternKind::Substring, "Report") };
    let out = run(SearchQuery { name: Some(cs), ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["docs/Report-2024.md"]));

    let out = run(SearchQuery { name: name(PatternKind::Fuzzy, "hlprs"), ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["src/util/helpers.rs"]));
    assert!(soyuz_search::fuzzy_score("hlprs", "helpers.rs").is_some());

    // Directories match names too.
    let out = run(SearchQuery { name: name(PatternKind::Substring, "util"), ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["src/util"]));
}

#[test]
fn content_literal_regex_binary_and_size_cap() {
    let t = fixture();
    let content =
        |regex: bool, text: &str, cs: bool| Some(ContentPatternSpec { regex, text: text.into(), case_sensitive: cs });
    let out = run(SearchQuery { content: content(false, "needle", true), ..q(t.path()) });
    // blob.bin is binary, Needle in lib.rs differs in case, hidden skipped.
    assert_eq!(out.rels(t.path()), set(&["src/main.rs", "docs/report.txt", "data/big.log", "target/build.rs"]));
    let (_, m) = out.hits.iter().find(|(p, _)| p.ends_with("report.txt")).unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!((m[0].line, m[0].col, m[0].text.as_str()), (2, 5, "beta needle gamma"));
    let (_, m) = out.hits.iter().find(|(p, _)| p.ends_with("main.rs")).unwrap();
    assert_eq!((m[0].line, m[0].col), (2, 14));

    let out = run(SearchQuery { content: content(false, "needle", false), content_max_size: 1024, ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["src/main.rs", "src/lib.rs", "docs/report.txt", "target/build.rs"]));

    let out = run(SearchQuery {
        content: content(true, r"fn \w+\(\)", true),
        name: name(PatternKind::Glob, "*.rs"),
        ..q(t.path())
    });
    assert_eq!(out.rels(t.path()), set(&["src/main.rs", "src/lib.rs"]));
}

#[test]
fn size_date_kind_ext_filters() {
    let t = fixture();
    let out = run(SearchQuery { size: Some((Some(4096), Some(0))), ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["docs/Report-2024.md", "data/big.log"]));
    let out = run(SearchQuery { size: Some((Some(4096), Some(4096))), ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["docs/Report-2024.md"]));

    let out = run(SearchQuery { kinds: Some(EntryKindSet::DIR), ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["src", "src/util", "docs", "data", "target"]));

    let out = run(SearchQuery { extensions: vec!["MD".into(), ".log".into()], ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["docs/Report-2024.md", "data/big.log"]));

    let old = SystemTime::now() - Duration::from_secs(10 * 86_400);
    fs::File::options().write(true).open(t.path().join("docs/report.txt")).unwrap().set_modified(old).unwrap();
    let cutoff = SystemTime::now() - Duration::from_secs(86_400);
    let out = run(SearchQuery { modified: Some((None, Some(cutoff))), ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["docs/report.txt"]));
    let out = run(SearchQuery {
        modified: Some((Some(cutoff), None)),
        name: name(PatternKind::Glob, "*.txt"),
        ..q(t.path())
    });
    assert!(out.hits.is_empty());
}

#[test]
fn max_depth_and_max_results() {
    let t = fixture();
    let out = run(SearchQuery { max_depth: Some(1), ..q(t.path()) });
    assert_eq!(out.rels(t.path()), set(&["src", "docs", "data", "target"]));

    let out = run(SearchQuery { max_results: 3, ..q(t.path()) });
    assert_eq!(out.stats.hits, 3);
    assert_eq!(out.hits.len(), 3);
    assert!(out.stats.truncated);
}

#[test]
fn multiple_roots() {
    let t = fixture();
    let out = run(SearchQuery {
        roots: vec![t.path().join("src"), t.path().join("docs")],
        name: name(PatternKind::Glob, "*.*"),
        ..Default::default()
    });
    assert_eq!(out.stats.hits, 5);
}

fn big_tree(files: usize) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    for d in 0..files / 50 {
        let dir = t.path().join(format!("d{:03}/sub{}", d, d % 7));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..50 {
            fs::write(dir.join(format!("f{f}.txt")), b"x").unwrap();
        }
    }
    t
}

#[test]
fn cancel_mid_walk() {
    let t = big_tree(800);
    let h = start(q(t.path()), 2, Arc::new(|| {})).unwrap();
    // Wait for the first batch, then cancel.
    loop {
        match h.rx.recv_timeout(Duration::from_secs(20)).unwrap() {
            SearchEvent::Hits(_) => break,
            SearchEvent::Done(_) => panic!("finished before the first hit batch"),
            _ => {}
        }
    }
    let t0 = Instant::now();
    h.cancel();
    let out = drain(&h);
    assert!(out.stats.truncated);
    assert!(t0.elapsed() < Duration::from_secs(2), "cancel took {:?}", t0.elapsed());
    assert!(h.control().is_cancelled());
}

#[test]
fn backpressure_without_reader_then_drop() {
    // Nobody drains: the bounded channel fills, workers must exit after the handle drops.
    let t = big_tree(300);
    let h = start(SearchQuery { max_results: 0, ..q(t.path()) }, 2, Arc::new(|| {})).unwrap();
    let ctl = h.control();
    std::thread::sleep(Duration::from_millis(100));
    drop(h);
    assert!(ctl.is_cancelled());
}

#[test]
fn streams_all_hits_of_big_tree() {
    let t = big_tree(600);
    let wakes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let w = wakes.clone();
    let h = start(
        SearchQuery { name: name(PatternKind::Glob, "*.txt"), ..q(t.path()) },
        0,
        Arc::new(move || {
            w.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }),
    )
    .unwrap();
    let out = drain(&h);
    assert_eq!(out.hits.len(), 600);
    assert_eq!(out.stats.hits, 600);
    assert!(wakes.load(std::sync::atomic::Ordering::Relaxed) > 0);
    let unique: BTreeSet<_> = out.hits.iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(unique.len(), 600);
}

#[test]
fn invalid_content_regex_fails_fast() {
    let q = SearchQuery {
        content: Some(ContentPatternSpec { regex: true, text: "(".into(), case_sensitive: false }),
        ..Default::default()
    };
    assert!(start(q, 0, Arc::new(|| {})).is_err());
}

#[test]
fn missing_root_reports_error_and_finishes() {
    let t = tempfile::tempdir().unwrap();
    let h = start(q(&t.path().join("nope")), 0, Arc::new(|| {})).unwrap();
    let mut errors = 0;
    loop {
        match h.rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            SearchEvent::Error(..) => errors += 1,
            SearchEvent::Done(s) => {
                assert_eq!(s.hits, 0);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(errors, 1);
}

#[test]
fn documents_are_searched_in_their_text() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    // Binary on disk (NUL bytes): only the registered extractor's text can match.
    write(r, "a.fakedoc", b"\0\0binary needle\0");
    write(r, "locked.fakedoc", b"\0");
    write(r, "huge.fakedoc", &vec![0u8; 4096]);
    write(r, "plain.txt", b"no match here\n");
    soyuz_core::doc_text::set_extractor(
        &["fakedoc"],
        1024,
        Arc::new(|p| match p.file_name().and_then(|n| n.to_str()) {
            Some("locked.fakedoc") => Err("encrypted".into()),
            _ => Ok("first page\n\nthe Needle is on page two\n".into()),
        }),
    );
    let content = Some(ContentPatternSpec { regex: false, text: "needle".into(), case_sensitive: false });
    let out = run(SearchQuery { content, ..q(r) });
    soyuz_core::doc_text::clear_extractor();
    assert_eq!(out.rels(r), set(&["a.fakedoc"]), "the locked one is skipped, the huge one is over the cap");
    let (_, m) = &out.hits[0];
    assert_eq!((m[0].line, m[0].col, m[0].text.as_str()), (3, 4, "the Needle is on page two"));
}

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use soyuz_core::config::{Automation, AutomationRule, RuleAction, RuleKind};
use soyuz_core::{EntryInfo, EntryKind};
use soyuz_jobs::{EngineConfig, JobEngine, JobKind};

use super::worker::{Cmd, Event, LogKind, Worker};
use super::*;

fn rule(watch: &Path) -> AutomationRule {
    AutomationRule {
        name: "PDFs".into(),
        watch: watch.to_path_buf(),
        glob: "*.pdf;*.epub".into(),
        ..Default::default()
    }
}

fn file(path: &Path, size: u64, age_min: u64) -> EntryInfo {
    let mut e = EntryInfo::minimal(path.to_path_buf(), EntryKind::File);
    e.size = size;
    e.modified = Some(SystemTime::now() - Duration::from_secs(age_min * 60));
    e
}

#[test]
fn matching_filters() {
    let w = std::env::temp_dir().join("watch");
    let mut r = rule(&w);
    let c = CompiledRule::compile(&r).unwrap();
    let now = SystemTime::now();
    assert!(c.matches(&file(&w.join("a.PDF"), 10, 0), now), "globs are case-insensitive");
    assert!(c.matches(&file(&w.join("b.epub"), 10, 0), now));
    assert!(!c.matches(&file(&w.join("a.txt"), 10, 0), now));
    assert!(!c.matches(&file(&w.join("sub").join("a.pdf"), 10, 0), now), "not recursive");
    assert!(!c.matches(&file(&w.join("a.pdf.crdownload"), 10, 0), now));
    assert!(!c.matches(&EntryInfo::minimal(w.join("d.pdf"), EntryKind::Dir), now), "files only");
    assert!(!c.matches(&file(&std::env::temp_dir().join("a.pdf"), 10, 0), now), "outside the folder");
    r.recursive = true;
    r.regex = r"^inv-\d+".into();
    r.min_size_kib = 1;
    r.max_size_kib = 100;
    r.min_age_minutes = 5;
    let c = CompiledRule::compile(&r).unwrap();
    assert!(c.matches(&file(&w.join("sub").join("INV-12.pdf"), 2048, 10), now));
    assert!(!c.matches(&file(&w.join("inv-12.pdf"), 100, 10), now), "too small");
    assert!(!c.matches(&file(&w.join("inv-12.pdf"), 200 * 1024, 10), now), "too large");
    assert!(!c.matches(&file(&w.join("inv-12.pdf"), 2048, 1), now), "too new");
    assert!(!c.matches(&file(&w.join("x-12.pdf"), 2048, 10), now), "regex");
    r.kind = RuleKind::Folders;
    r.glob.clear();
    r.regex.clear();
    r.min_size_kib = 0;
    r.max_size_kib = 0;
    r.min_age_minutes = 0;
    let c = CompiledRule::compile(&r).unwrap();
    assert!(c.matches(&EntryInfo::minimal(w.join("d"), EntryKind::Dir), now));
    assert!(!c.matches(&file(&w.join("f"), 1, 0), now));
}

#[test]
fn compile_errors() {
    let w = std::env::temp_dir();
    assert!(CompiledRule::compile(&AutomationRule::default()).is_err());
    assert!(CompiledRule::compile(&AutomationRule { watch: "rel".into(), ..Default::default() }).is_err());
    assert!(CompiledRule::compile(&AutomationRule { regex: "(".into(), ..rule(&w) }).is_err());
    assert!(CompiledRule::compile(&AutomationRule { glob: "[".into(), ..rule(&w) }).is_err());
    let bad = AutomationRule { actions: vec![RuleAction::Move { to: "rel".into() }], ..rule(&w) };
    assert!(CompiledRule::compile(&bad).unwrap_err().contains("absolute"));
}

#[test]
fn rename_patterns() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let p = Path::new("/w/In/report.final.pdf");
    let d = local_date(now, "%Y-%m-%d");
    assert_eq!(expand_pattern("{mdate}-{name}", p, Some(now), now).unwrap(), format!("{d}-report.final.pdf"));
    assert_eq!(expand_pattern("{stem}_{parent}.{ext}", p, None, now).unwrap(), "report.final_In.pdf");
    assert_eq!(expand_pattern("{{x}} {name}", p, None, now).unwrap(), "{x} report.final.pdf");
    assert!(expand_pattern("{nope}", p, None, now).is_err());
    assert!(expand_pattern("{name", p, None, now).is_err());
    assert!(expand_pattern("a/{name}", p, None, now).is_err(), "separators are not names");
}

#[test]
fn step_planning() {
    let now = SystemTime::now();
    let cur = Path::new("/w/a.pdf");
    let to = Path::new("/docs/PDF");
    let s = plan_step(&RuleAction::Move { to: to.into() }, cur, None, now, &|p| p == Path::new("/")).unwrap();
    assert_eq!(s.len(), 3, "missing folders are created first: {s:?}");
    assert!(matches!(&s[0], Step::Job { kind: JobKind::Create { path, .. }, .. } if path == Path::new("/docs")));
    assert!(matches!(&s[1], Step::Job { kind: JobKind::Create { path, .. }, .. } if path == to));
    assert!(matches!(&s[2], Step::Job { then, .. } if then == &to.join("a.pdf")));
    let s = plan_step(&RuleAction::Copy { to: to.into() }, cur, None, now, &|_| true).unwrap();
    assert!(matches!(&s[..], [Step::Job { then, .. }] if then == cur), "copy keeps acting on the original");
    let s = plan_step(&RuleAction::Rename { pattern: "x-{name}".into() }, cur, None, now, &|_| true).unwrap();
    assert!(matches!(&s[..], [Step::Job { then, .. }] if then == Path::new("/w/x-a.pdf")));
    let s = plan_step(&RuleAction::Rename { pattern: "{name}".into() }, cur, None, now, &|_| true).unwrap();
    assert!(s.is_empty(), "same name: nothing to do");
    let r = AutomationRule {
        actions: vec![
            RuleAction::Move { to: to.into() },
            RuleAction::Tag { tag: "blue".into() },
            RuleAction::Trash,
            RuleAction::Tag { tag: "red".into() },
        ],
        ..rule(Path::new("/w"))
    };
    let d = describe(&r, &file(cur, 1, 0), now);
    assert_eq!(d.len(), 3, "{d:?}");
    assert!(d[1].contains("Tag “a.pdf” blue"), "{d:?}");
    assert!(d[2].starts_with("Move “a.pdf” to trash"), "{d:?}");
}

#[test]
fn scheduler_debounces_and_suppresses() {
    let mut s = Scheduler::default();
    let t0 = Instant::now();
    let p = Path::new("/w/a.pdf");
    s.changed(0, p, Duration::from_secs(3), t0);
    s.changed(0, p, Duration::from_secs(3), t0 + Duration::from_secs(2));
    assert!(s.due(t0 + Duration::from_secs(4)).is_empty(), "the second change restarted the delay");
    assert_eq!(s.due(t0 + Duration::from_secs(5)), vec![(0, p.to_path_buf())]);
    assert!(s.due(t0 + Duration::from_secs(6)).is_empty(), "fired once");
    s.suppress(p, t0);
    s.changed(0, p, Duration::ZERO, t0 + Duration::from_secs(1));
    assert_eq!(s.pending_len(), 0, "our own output is ignored");
    s.changed(0, p, Duration::ZERO, t0 + SUPPRESS_FOR + Duration::from_secs(1));
    assert_eq!(s.pending_len(), 1, "until the suppression expires");
}

#[test]
fn live_preview_lists_matches() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.pdf"), b"x").unwrap();
    std::fs::write(d.path().join("b.txt"), b"x").unwrap();
    std::fs::create_dir(d.path().join("sub")).unwrap();
    std::fs::write(d.path().join("sub").join("c.pdf"), b"x").unwrap();
    let mut r = rule(d.path());
    r.actions = vec![RuleAction::Tag { tag: "red".into() }];
    let (items, total) = preview(&r, 10).unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].path, d.path().join("a.pdf"));
    assert_eq!(items[0].steps.len(), 1);
    r.recursive = true;
    assert_eq!(preview(&r, 10).unwrap().1, 2);
    assert_eq!(preview(&r, 1).unwrap().0.len(), 1, "limit");
}

fn engine(dir: &Path) -> Arc<JobEngine> {
    let cfg = EngineConfig { history_path: Some(dir.join("history.jsonl")), ..EngineConfig::default() };
    JobEngine::new(cfg, Arc::new(soyuz_fs::RealProbe), Arc::new(|| {}))
}

fn drain(rx: &crossbeam_channel::Receiver<Event>) -> Vec<Event> {
    rx.try_iter().collect()
}

#[test]
fn worker_runs_steps_as_jobs_in_order() {
    let d = tempfile::tempdir().unwrap();
    let watch = d.path().join("in");
    let dest = d.path().join("out").join("PDF");
    std::fs::create_dir(&watch).unwrap();
    let f = watch.join("report.pdf");
    std::fs::write(&f, b"pdf").unwrap();
    let eng = engine(d.path());
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut w = Worker::new(eng.clone(), tx, Arc::new(|| {}), Duration::from_millis(50));
    let mut r = rule(&watch);
    r.actions = vec![
        RuleAction::Move { to: dest.clone() },
        RuleAction::Rename { pattern: "done-{name}".into() },
        RuleAction::Tag { tag: "blue".into() },
    ];
    let automation = Automation { rules: vec![r], ..Automation::default() };
    w.command(Cmd::Configure { automation, copy: Default::default() });
    w.schedule_now(&f);
    let t = Instant::now();
    w.tick(Instant::now());
    while !w.idle() && t.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(30));
        w.tick(Instant::now());
    }
    assert!(w.idle(), "pipeline finished");
    let done = dest.join("done-report.pdf");
    assert!(done.exists(), "moved and renamed");
    assert!(!f.exists());
    let events = drain(&rx);
    let submitted = events.iter().filter(|e| matches!(e, Event::Submitted { .. })).count();
    assert_eq!(submitted, 4, "mkdir ×2, move, rename: {events:#?}");
    assert!(
        events.iter().any(|e| matches!(e, Event::Tag { paths, tag } if paths == &vec![done.clone()] && tag == "blue"))
    );
    assert!(events.iter().any(|e| matches!(e, Event::Log { entry, .. } if entry.kind == LogKind::Done)));
    // Every job is in the undo history with the rule in its label.
    let h = eng.history();
    assert_eq!(h.len(), 4);
    assert!(h.iter().all(|e| e.options.as_ref().and_then(|o| o.label.clone()).unwrap_or_default().ends_with("· PDFs")));
    eng.shutdown(Duration::from_secs(2));
}

#[test]
fn worker_dry_run_changes_nothing() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.pdf");
    std::fs::write(&f, b"x").unwrap();
    let eng = engine(d.path());
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut w = Worker::new(eng.clone(), tx, Arc::new(|| {}), Duration::from_millis(50));
    let mut r = rule(d.path());
    r.dry_run = true;
    r.actions = vec![RuleAction::Trash];
    w.command(Cmd::Configure {
        automation: Automation { rules: vec![r], ..Default::default() },
        copy: Default::default(),
    });
    w.schedule_now(&f);
    w.tick(Instant::now());
    assert!(w.idle());
    assert!(f.exists());
    let events = drain(&rx);
    assert!(!events.iter().any(|e| matches!(e, Event::Submitted { .. })));
    assert!(events.iter().any(|e| matches!(e, Event::Log { entry, .. }
        if entry.kind == LogKind::DryRun && entry.message.contains("Would: Move “a.pdf” to trash"))));
    eng.shutdown(Duration::from_secs(1));
}

#[test]
fn worker_reacts_to_the_watcher() {
    let d = tempfile::tempdir().unwrap();
    let watch = d.path().join("in");
    let dest = d.path().join("out");
    std::fs::create_dir_all(&watch).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    let eng = engine(d.path());
    let h = super::worker::spawn(eng.clone(), Arc::new(|| {}), Duration::from_millis(50));
    let mut r = rule(&watch);
    r.delay_secs = 0;
    r.actions = vec![RuleAction::Copy { to: dest.clone() }];
    h.send(Cmd::Configure {
        automation: Automation { rules: vec![r], ..Default::default() },
        copy: Default::default(),
    });
    // Wait for the watch to be registered.
    let t = Instant::now();
    loop {
        if let Ok(Event::Status(s)) = h.events.recv_timeout(Duration::from_secs(5)) {
            assert!(s.iter().all(|(_, r)| r.is_ok()), "{s:?}");
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(10));
    }
    std::fs::write(watch.join("new.pdf"), b"hello").unwrap();
    let t = Instant::now();
    while !dest.join("new.pdf").exists() && t.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(dest.join("new.pdf").exists(), "the rule copied the new file");
    drop(h);
    eng.shutdown(Duration::from_secs(2));
}

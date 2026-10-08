//! Undo of every record kind, redo, history persistence.

use std::time::{Duration, SystemTime};

use soyuz_core::{Attributes, EntryKind};
use soyuz_fs::{AttrSet, TimesSet, TrashItem};

use super::*;
use crate::{CreateKind, EngineConfig, UndoRecord};

fn undo_ok(h: &H, idx: usize) {
    let hid = h.engine.history()[idx].id;
    let u = h.engine.undo(hid).expect("undoable");
    assert_eq!(h.snap(u).kind, crate::JobKindBrief::Undo);
    // The test volume has no trash: confirm the permanent deletes.
    let s = settle(h, u, true);
    assert_eq!(s.items_failed, 0, "{s:?}");
    assert!(h.engine.history()[idx].undone);
    assert!(h.engine.undo(hid).is_none(), "already undone");
}

#[test]
fn undo_copied() {
    let h = H::new();
    write(&h.p("s/a/f"), b"1");
    write(&h.p("s/g"), b"2");
    write(&h.p("d/keep"), b"k");
    let id = h.submit(copy_kind(&[&h.p("s/a"), &h.p("s/g")], &h.p("d")));
    h.ok(id);
    assert!(h.p("d/a/f").exists() && h.p("d/g").exists());
    undo_ok(&h, 0);
    assert!(!h.p("d/a").exists() && !h.p("d/g").exists());
    assert!(h.p("d/keep").exists() && h.p("s/a/f").exists());
}

#[test]
fn undo_moved_same_and_cross_device() {
    let h = H::new();
    write(&h.p("s/a/f"), b"1");
    write(&h.p("s/g"), b"2");
    let id = h.submit(JobKind::Move { sources: vec![h.p("s/a"), h.p("s/g")], dest_dir: h.p("d"), rename_to: None });
    h.ok(id);
    assert!(!h.p("s/a").exists());
    undo_ok(&h, 0);
    assert_eq!(read(&h.p("s/a/f")), b"1");
    assert_eq!(read(&h.p("s/g")), b"2");
    assert!(!h.p("d/a").exists() && !h.p("d/g").exists());

    let mut spec = JobSpec::new(JobKind::Move { sources: vec![h.p("s/a")], dest_dir: h.p("x"), rename_to: None });
    spec.options.mv.force_cross_device = true;
    let id = h.engine.submit(spec);
    h.ok(id);
    assert!(!h.p("s/a").exists() && h.p("x/a/f").exists());
    undo_ok(&h, 1);
    assert_eq!(read(&h.p("s/a/f")), b"1");
    assert!(!h.p("x/a").exists());
}

#[test]
fn undo_moved_back_into_deleted_folder() {
    let h = H::new();
    write(&h.p("s/sub/f"), b"1");
    let id = h.submit(JobKind::Move { sources: vec![h.p("s/sub/f")], dest_dir: h.p("d"), rename_to: None });
    h.ok(id);
    std::fs::remove_dir_all(h.p("s")).unwrap();
    undo_ok(&h, 0);
    assert_eq!(read(&h.p("s/sub/f")), b"1");
}

#[test]
fn undo_renamed_and_redo() {
    let h = H::new();
    write(&h.p("a"), b"A");
    let id = h.submit(JobKind::Rename { pairs: vec![(h.p("a"), h.p("b"))] });
    h.ok(id);
    assert_eq!(h.engine.history()[0].records, vec![UndoRecord::Renamed { from: h.p("a"), to: h.p("b") }]);
    undo_ok(&h, 0);
    assert_eq!(read(&h.p("a")), b"A");
    assert!(!h.p("b").exists());
    let hid = h.engine.history()[0].id;
    let r = h.engine.redo(hid).expect("redo of an undone entry");
    h.ok(r);
    assert_eq!(read(&h.p("b")), b"A");
    let hist = h.engine.history();
    assert_eq!(hist.len(), 1, "redo updates the entry in place");
    assert!(!hist[0].undone && hist[0].undoable);
    assert!(h.engine.redo(hid).is_none(), "not undone any more");
    undo_ok(&h, 0);
    assert_eq!(read(&h.p("a")), b"A");
}

#[test]
fn undo_created_dir_and_file() {
    let h = H::new();
    let d = h.submit(JobKind::Create { path: h.p("nd"), kind: CreateKind::Dir });
    h.ok(d);
    let f = h.submit(JobKind::Create { path: h.p("nf"), kind: CreateKind::File { template: None } });
    h.ok(f);
    undo_ok(&h, 1);
    assert!(!h.p("nf").exists());
    // a folder that gained content is never deleted permanently by undo
    write(&h.p("nd/user-file"), b"precious");
    let hid = h.engine.history()[0].id;
    let u = h.engine.undo(hid).unwrap();
    match h.prompt(u) {
        JobPrompt::Error { path, error, .. } => {
            assert_eq!(path, h.p("nd"));
            assert_eq!(error.kind, soyuz_core::FsErrorKind::NotEmpty);
        }
        other => panic!("{other:?}"),
    }
    h.engine.respond(u, crate::PromptResponse::Error { action: soyuz_core::ErrorAction::Skip, apply_to_all: false });
    let s = h.ok(u);
    assert_eq!(s.items_failed, 1);
    assert!(h.p("nd/user-file").exists());
    assert!(!h.engine.history()[0].undone, "a failed undo leaves the entry undoable");
    std::fs::remove_file(h.p("nd/user-file")).unwrap();
    undo_ok(&h, 0);
    assert!(!h.p("nd").exists());
}

#[test]
fn undo_meta_changed() {
    let h = H::new();
    let f = h.p("m.txt");
    write(&f, b"m");
    let before = std::fs::metadata(&f).unwrap().modified().unwrap();
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_234_567_890);
    let id = h.submit(JobKind::SetMeta {
        paths: vec![f.clone()],
        times: Some(TimesSet { modified: Some(t), created: None, accessed: None }),
        attrs: Some(AttrSet { set: Attributes::READONLY, clear: Attributes::empty() }),
        mode: None,
    });
    h.ok(id);
    assert!(std::fs::metadata(&f).unwrap().permissions().readonly());
    undo_ok(&h, 0);
    let md = std::fs::metadata(&f).unwrap();
    assert!(!md.permissions().readonly());
    assert_eq!(md.modified().unwrap(), before);
}

#[test]
fn undo_extracted_record() {
    let h = H::new();
    write(&h.p("out/top/a"), b"1");
    write(&h.p("out/other"), b"keep");
    let u = h.engine.submit_undo_records(vec![UndoRecord::Extracted { dst: h.p("out/top") }]);
    assert_eq!(settle(&h, u, true).items_ok, 1);
    assert!(!h.p("out/top").exists());
    assert!(h.p("out/other").exists());
}

#[test]
fn undo_trashed_record_restores_or_reports() {
    let h = H::new();
    let item = TrashItem {
        id: "x".into(),
        original: h.p("gone.txt"),
        deleted_at: SystemTime::UNIX_EPOCH,
        size: 1,
        kind: EntryKind::File,
        trash_root: h.p("bin"),
    };
    let mut spec_errors = h.engine.submit_undo_records(vec![UndoRecord::Trashed(item)]);
    // Baseline fs has no trash: restore fails → error prompt → skip.
    if let Some(JobPrompt::Error { path, .. }) = wait_for(Duration::from_secs(10), || {
        let s = h.snap(spec_errors);
        if s.state.is_terminal() { Some(None) } else { s.pending_prompt.map(Some) }
    }) {
        assert_eq!(path, h.p("gone.txt"));
        h.engine.respond(
            spec_errors,
            crate::PromptResponse::Error { action: soyuz_core::ErrorAction::Skip, apply_to_all: false },
        );
        assert_eq!(h.ok(spec_errors).items_failed, 1);
    }
    spec_errors = h.engine.submit_undo_records(vec![UndoRecord::Irreversible(h.p("z"))]);
    assert_eq!(h.ok(spec_errors).items_skipped, 1);
}

#[test]
fn history_persistence_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let hist_path = dir.path().join("state").join("history.jsonl");
    let mk = || EngineConfig {
        sample_interval: Duration::from_millis(50),
        history_path: Some(hist_path.clone()),
        ..EngineConfig::default()
    };
    write(&dir.path().join("a"), b"A");
    {
        let e = JobEngine::new(mk(), Arc::new(FakeProbe::default()), Arc::new(|| {}));
        let id = e.submit(JobSpec::new(JobKind::Rename { pairs: vec![(dir.path().join("a"), dir.path().join("b"))] }));
        wait_for(Duration::from_secs(10), || e.snapshot(id).filter(|s| s.state.is_terminal()));
        let id2 = e.submit(JobSpec::new(JobKind::Create { path: dir.path().join("d"), kind: CreateKind::Dir }));
        wait_for(Duration::from_secs(10), || e.snapshot(id2).filter(|s| s.state.is_terminal()));
        let u = e.undo(e.history()[1].id).unwrap();
        wait_for(Duration::from_secs(10), || e.snapshot(u).filter(|s| s.state.is_terminal()));
        e.shutdown(Duration::from_secs(1));
    }
    let text = std::fs::read_to_string(&hist_path).unwrap();
    assert_eq!(text.lines().count(), 2);
    let e = JobEngine::new(mk(), Arc::new(FakeProbe::default()), Arc::new(|| {}));
    let h = e.history();
    assert_eq!(h.len(), 2);
    assert!(h.iter().all(|x| !x.this_session), "entries from a previous session");
    assert_eq!(h[0].records, vec![UndoRecord::Renamed { from: dir.path().join("a"), to: dir.path().join("b") }]);
    assert!(!h[0].undone && h[1].undone);
    assert!(matches!(h[1].job, JobKind::Create { .. }));
    // ids keep increasing after a reload; the restored entry is undoable
    let u = e.undo(h[0].id).unwrap();
    wait_for(Duration::from_secs(10), || e.snapshot(u).filter(|s| s.state.is_terminal()));
    assert_eq!(read(&dir.path().join("a")), b"A");
    let id = e.submit(JobSpec::new(JobKind::Create { path: dir.path().join("d2"), kind: CreateKind::Dir }));
    wait_for(Duration::from_secs(10), || e.snapshot(id).filter(|s| s.state.is_terminal()));
    let h = e.history();
    assert_eq!(h[2].id.0, 3);
    e.shutdown(Duration::from_secs(1));
}

#[test]
fn history_limit_trims_oldest() {
    let h = H::with(|c| c.history_limit = 2);
    for n in ["a", "b", "c"] {
        let id = h.submit(JobKind::Create { path: h.p(n), kind: CreateKind::Dir });
        h.ok(id);
    }
    let hist = h.engine.history();
    assert_eq!(hist.iter().map(|e| e.id.0).collect::<Vec<_>>(), vec![2, 3]);
}

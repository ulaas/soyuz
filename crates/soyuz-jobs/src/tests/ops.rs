//! Job kinds end to end.

use std::time::{Duration, SystemTime};

use soyuz_core::{Attributes, ConflictResolution, ErrorAction, FsErrorKind, HashAlgo};
use soyuz_fs::{AttrSet, TimesSet, TrashSupport};

use super::*;
use crate::{CreateKind, DeleteMode, LogLevel, PromptResponse, UndoRecord};

#[test]
fn copy_tree_totals_progress_and_history() {
    let h = H::new();
    let src = h.p("src");
    write(&src.join("a.txt"), b"0123456789");
    write(&src.join("sub").join("b.bin"), &[7u8; 1000]);
    let dst = h.p("dst");
    std::fs::create_dir(&dst).unwrap();
    let id = h.submit(copy_kind(&[&src], &dst));
    let s = h.ok(id);
    assert_eq!(read(&dst.join("src").join("a.txt")), b"0123456789");
    assert_eq!(read(&dst.join("src").join("sub").join("b.bin")), vec![7u8; 1000]);
    assert_eq!((s.items_ok, s.items_failed, s.items_skipped, s.bytes), (4, 0, 0, 1010));
    let snap = h.snap(id);
    assert!(snap.progress.totals_known);
    assert_eq!((snap.progress.bytes_done, snap.progress.bytes_total), (1010, 1010));
    assert_eq!((snap.progress.items_done, snap.progress.items_total), (4, 4));
    assert_eq!(snap.queue_pos, None);
    let hist = h.engine.history();
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].records, vec![UndoRecord::Copied { dst: dst.join("src") }]);
    assert!(hist[0].undoable && !hist[0].undone);
    let log = h.engine.log(id, 0);
    assert!(log.iter().any(|l| l.msg == "started"), "{log:?}");
    assert!(log.iter().any(|l| l.msg == "finished"), "{log:?}");
    assert_eq!(h.engine.log(id, snap.log_len), vec![]);
    let events = h.events();
    assert!(matches!(events.first(), Some(JobEvent::Added(x)) if *x == id));
    assert!(events.iter().any(|e| matches!(e, JobEvent::Finished(x, _) if *x == id)));
    assert!(events.iter().any(|e| matches!(e, JobEvent::HistoryChanged)));
    assert!(!h.engine.has_active());
}

#[test]
fn duplicate_and_rename_to() {
    let h = H::new();
    let f = h.p("f.txt");
    write(&f, b"x");
    let mut spec = JobSpec::new(copy_kind(&[&f], h.dir.path()));
    spec.options.conflict = Some(ConflictResolution::KeepBoth);
    let id = h.engine.submit(spec);
    h.ok(id);
    assert_eq!(read(&h.p("f (2).txt")), b"x");
    let id =
        h.submit(JobKind::Copy { sources: vec![f.clone()], dest_dir: h.p("out"), rename_to: Some("g.txt".into()) });
    h.ok(id);
    assert_eq!(read(&h.p("out").join("g.txt")), b"x");
    // rename_to with several sources is rejected by the planner
    let id = h.submit(JobKind::Copy { sources: vec![f.clone(), f], dest_dir: h.p("o2"), rename_to: Some("z".into()) });
    assert!(matches!(h.done(id), JobState::Failed(e) if e.kind == FsErrorKind::InvalidName));
}

#[test]
fn move_same_device_and_merge() {
    let h = H::new();
    write(&h.p("a/f.txt"), b"f");
    write(&h.p("a/d/g.txt"), b"g");
    write(&h.p("dst/d/existing.txt"), b"e");
    let id =
        h.submit(JobKind::Move { sources: vec![h.p("a/f.txt"), h.p("a/d")], dest_dir: h.p("dst"), rename_to: None });
    let s = h.ok(id);
    assert_eq!(s.items_failed, 0);
    assert_eq!(read(&h.p("dst/f.txt")), b"f");
    assert_eq!(read(&h.p("dst/d/g.txt")), b"g");
    assert_eq!(read(&h.p("dst/d/existing.txt")), b"e");
    assert!(!h.p("a/f.txt").exists() && !h.p("a/d").exists());
    let rec = &h.engine.history()[0].records;
    assert!(rec.contains(&UndoRecord::Moved { from: h.p("a/f.txt"), to: h.p("dst/f.txt") }));
    assert!(rec.contains(&UndoRecord::Moved { from: h.p("a/d/g.txt"), to: h.p("dst/d/g.txt") }));
}

#[test]
fn move_cross_device_copies_then_deletes() {
    let h = H::new();
    write(&h.p("a/t/x.bin"), &[1u8; 4096]);
    write(&h.p("a/t/y/z.txt"), b"z");
    let mut spec = JobSpec::new(JobKind::Move { sources: vec![h.p("a/t")], dest_dir: h.p("b"), rename_to: None });
    spec.options.mv.force_cross_device = true;
    let id = h.engine.submit(spec);
    let s = h.ok(id);
    assert_eq!(s.bytes, 4097);
    assert_eq!(read(&h.p("b/t/x.bin")), vec![1u8; 4096]);
    assert_eq!(read(&h.p("b/t/y/z.txt")), b"z");
    assert!(!h.p("a/t").exists(), "source removed after a complete copy");
    assert_eq!(h.engine.history()[0].records, vec![UndoRecord::Moved { from: h.p("a/t"), to: h.p("b/t") }]);
    let p = h.snap(id).progress;
    assert_eq!((p.bytes_done, p.bytes_total), (4097, 4097));
}

#[test]
fn move_and_copy_into_self_rejected() {
    let h = H::new();
    write(&h.p("a/sub/f"), b"1");
    for kind in [
        JobKind::Move { sources: vec![h.p("a")], dest_dir: h.p("a/sub"), rename_to: None },
        JobKind::Copy { sources: vec![h.p("a")], dest_dir: h.p("a"), rename_to: None },
    ] {
        let id = h.submit(kind);
        match h.done(id) {
            JobState::Failed(e) => assert_eq!(e.kind, FsErrorKind::InvalidName, "{e}"),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(read(&h.p("a/sub/f")), b"1");
    assert!(!h.p("a/sub/a").exists());
}

#[test]
fn delete_permanent_and_wipe() {
    let h = H::new();
    write(&h.p("t/a/b.txt"), b"b");
    write(&h.p("t/c.txt"), b"c");
    write(&h.p("w.bin"), &[9u8; 3000]);
    let id = h.submit(JobKind::Delete { paths: vec![h.p("t")], mode: DeleteMode::Permanent });
    let s = h.ok(id);
    assert_eq!(s.items_ok, 4);
    assert!(!h.p("t").exists());
    let hist = h.engine.history();
    assert!(!hist[0].undoable, "permanent delete is irreversible");
    assert!(h.engine.undo(hist[0].id).is_none());
    let id = h.submit(JobKind::Delete { paths: vec![h.p("w.bin")], mode: DeleteMode::Wipe { passes: 2 } });
    h.ok(id);
    assert!(!h.p("w.bin").exists());
    let p = h.snap(id).progress;
    assert_eq!((p.bytes_done, p.bytes_total), (6000, 6000));
}

#[test]
fn trash_unsupported_prompt_apply_to_all() {
    let h = H::new();
    write(&h.p("x"), b"1");
    write(&h.p("y/z"), b"2");
    let id = h.submit(JobKind::Delete { paths: vec![h.p("x"), h.p("y")], mode: DeleteMode::Trash });
    match h.prompt(id) {
        JobPrompt::TrashUnsupported { path, reason } => {
            assert_eq!(path, h.p("x"));
            assert!(reason.contains("no trash"));
        }
        other => panic!("{other:?}"),
    }
    h.engine.respond(id, PromptResponse::TrashUnsupported { permanent: true, apply_to_all: true });
    let s = h.ok(id);
    assert_eq!(s.items_ok, 2);
    assert!(!h.p("x").exists() && !h.p("y").exists());
    let prompts = h.events().iter().filter(|e| matches!(e, JobEvent::Prompt(_))).count();
    assert_eq!(prompts, 1);
    assert!(!h.engine.history()[0].undoable);
}

#[test]
fn trash_unsupported_skip_and_native_failure() {
    let h = H::new();
    write(&h.p("x"), b"1");
    let id = h.submit(JobKind::Delete { paths: vec![h.p("x")], mode: DeleteMode::Trash });
    h.prompt(id);
    h.engine.respond(id, PromptResponse::TrashUnsupported { permanent: false, apply_to_all: false });
    assert_eq!(h.ok(id).items_skipped, 1);
    assert!(h.p("x").exists());
    // Probe claims native support but the bin is unavailable (baseline fs): same flow.
    // Ask the real file system *before* submitting (the job may trash `x` right away),
    // and never put anything in a real trash can unless SOYUZ_TRASH_TESTS=1.
    let native = soyuz_fs::trash::trash_support(&h.p("x")) == TrashSupport::Native;
    if native && std::env::var_os("SOYUZ_TRASH_TESTS").is_none() {
        return;
    }
    *h.probe.trash.lock() = TrashSupport::Native;
    let id = h.submit(JobKind::Delete { paths: vec![h.p("x")], mode: DeleteMode::Trash });
    if native {
        h.ok(id);
        assert!(!h.p("x").exists());
    } else {
        assert!(matches!(h.prompt(id), JobPrompt::TrashUnsupported { .. }));
        h.engine.respond(id, PromptResponse::Cancelled);
        assert_eq!(h.done(id), JobState::Cancelled);
        assert!(h.p("x").exists());
    }
}

#[test]
fn create_kinds_and_conflict() {
    let h = H::new();
    let id = h.submit(JobKind::Create { path: h.p("newdir"), kind: CreateKind::Dir });
    h.ok(id);
    assert!(h.p("newdir").is_dir());
    let id = h.submit(JobKind::Create { path: h.p("new.txt"), kind: CreateKind::File { template: None } });
    h.ok(id);
    assert_eq!(read(&h.p("new.txt")), b"");
    write(&h.p("tpl.txt"), b"template");
    let id =
        h.submit(JobKind::Create { path: h.p("new.txt"), kind: CreateKind::File { template: Some(h.p("tpl.txt")) } });
    match h.prompt(id) {
        JobPrompt::Conflict(c) => {
            assert_eq!(c.kind, crate::ConflictKind::FileExists);
            assert_eq!(c.suggested_name, "new (2).txt");
        }
        other => panic!("{other:?}"),
    }
    h.engine.respond(id, PromptResponse::Conflict { resolution: ConflictResolution::KeepBoth, apply_to_all: false });
    h.ok(id);
    assert_eq!(read(&h.p("new (2).txt")), b"template");
    let recs: Vec<_> = h.engine.history().into_iter().flat_map(|e| e.records).collect();
    assert_eq!(
        recs,
        vec![
            UndoRecord::Created(h.p("newdir")),
            UndoRecord::Created(h.p("new.txt")),
            UndoRecord::Created(h.p("new (2).txt"))
        ]
    );
}

#[test]
fn set_meta_records_snapshot() {
    let h = H::new();
    let f = h.p("m.txt");
    write(&f, b"m");
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let id = h.submit(JobKind::SetMeta {
        paths: vec![f.clone()],
        times: Some(TimesSet { modified: Some(t), created: None, accessed: None }),
        attrs: Some(AttrSet { set: Attributes::READONLY, clear: Attributes::empty() }),
        mode: None,
    });
    h.ok(id);
    let md = std::fs::metadata(&f).unwrap();
    assert!(md.permissions().readonly());
    assert_eq!(md.modified().unwrap(), t);
    assert!(
        matches!(&h.engine.history()[0].records[0], UndoRecord::MetaChanged { path, before } if *path == f && !before.attrs.contains(Attributes::READONLY))
    );
    // clean up so the temp dir can be removed
    let mut p = md.permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    p.set_readonly(false);
    std::fs::set_permissions(&f, p).unwrap();
}

#[test]
fn hash_known_vector_and_folder() {
    let h = H::new();
    write(&h.p("h/abc.txt"), b"abc");
    write(&h.p("h/sub/abc2.txt"), b"abc");
    let id = h.submit(JobKind::Hash { paths: vec![h.p("h")], algo: HashAlgo::Sha256 });
    let s = h.ok(id);
    let want = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(s.items_ok, 2);
    assert_eq!(s.notes.iter().filter(|n| n.starts_with(want)).count(), 2, "{:?}", s.notes);
    let log = h.engine.log(id, 0);
    assert!(log.iter().any(|l| l.level == LogLevel::Info && l.msg == want && l.path == Some(h.p("h/abc.txt"))));
    let p = h.snap(id).progress;
    assert_eq!((p.bytes_done, p.bytes_total, p.items_total), (6, 6, 2));
}

#[test]
fn not_enough_space_prompt() {
    let h = H::new();
    write(&h.p("big"), &[0u8; 5000]);
    *h.probe.space.lock() = Some((10_000, 100));
    let id = h.submit(copy_kind(&[&h.p("big")], &h.p("out")));
    match h.prompt(id) {
        JobPrompt::NotEnoughSpace { needed, free, .. } => assert_eq!((needed, free), (5000, 100)),
        other => panic!("{other:?}"),
    }
    h.engine.respond(id, PromptResponse::Space { proceed: false });
    assert_eq!(h.done(id), JobState::Cancelled);
    assert!(!h.p("out").join("big").exists());
    let id = h.submit(copy_kind(&[&h.p("big")], &h.p("out")));
    h.prompt(id);
    h.engine.respond(id, PromptResponse::Space { proceed: true });
    h.ok(id);
    assert_eq!(read(&h.p("out/big")).len(), 5000);
    // Unsupported space (baseline fs) = unknown: no prompt.
    *h.probe.space.lock() = None;
    let id = h.submit(copy_kind(&[&h.p("big")], &h.p("out2")));
    h.ok(id);
}

#[test]
fn rename_batch_cycles_and_chains() {
    let h = H::new();
    for (n, c) in [("a", "A"), ("b", "B"), ("c", "C"), ("x", "X"), ("y", "Y"), ("p", "P"), ("q", "Q")] {
        write(&h.p(n), c.as_bytes());
    }
    let pairs = vec![
        // 3-cycle a→b→c→a
        (h.p("a"), h.p("b")),
        (h.p("b"), h.p("c")),
        (h.p("c"), h.p("a")),
        // 2-cycle (swap)
        (h.p("x"), h.p("y")),
        (h.p("y"), h.p("x")),
        // chain q→r, p→q (p must wait for q)
        (h.p("p"), h.p("q")),
        (h.p("q"), h.p("r")),
    ];
    let id = h.submit(JobKind::Rename { pairs });
    let s = h.ok(id);
    assert_eq!((s.items_ok, s.items_failed), (7, 0));
    assert_eq!(read(&h.p("b")), b"A");
    assert_eq!(read(&h.p("c")), b"B");
    assert_eq!(read(&h.p("a")), b"C");
    assert_eq!(read(&h.p("y")), b"X");
    assert_eq!(read(&h.p("x")), b"Y");
    assert_eq!(read(&h.p("q")), b"P");
    assert_eq!(read(&h.p("r")), b"Q");
    assert!(!h.p("p").exists());
    let leftovers: Vec<_> = std::fs::read_dir(h.dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".soyuz"))
        .collect();
    assert!(leftovers.is_empty(), "temp names left: {leftovers:?}");
    // Undo restores every original name (reverse pairs form the same cycles).
    let hid = h.engine.history()[0].id;
    let u = h.engine.undo(hid).unwrap();
    h.ok(u);
    for (n, c) in [("a", "A"), ("b", "B"), ("c", "C"), ("x", "X"), ("y", "Y"), ("p", "P"), ("q", "Q")] {
        assert_eq!(read(&h.p(n)), c.as_bytes(), "{n}");
    }
    assert!(!h.p("r").exists());
    assert!(h.engine.history()[0].undone);
}

#[test]
fn rename_conflict_with_outside_file() {
    let h = H::new();
    write(&h.p("a"), b"A");
    write(&h.p("b"), b"B");
    let id = h.submit(JobKind::Rename { pairs: vec![(h.p("a"), h.p("b"))] });
    assert!(matches!(h.prompt(id), JobPrompt::Conflict(c) if c.kind == crate::ConflictKind::FileExists));
    h.engine.respond(
        id,
        PromptResponse::Conflict { resolution: ConflictResolution::Rename("c".into()), apply_to_all: false },
    );
    h.ok(id);
    assert_eq!(read(&h.p("c")), b"A");
    assert_eq!(read(&h.p("b")), b"B");
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn rename_case_only() {
    let h = H::new();
    write(&h.p("readme.md"), b"r");
    let id = h.submit(JobKind::Rename { pairs: vec![(h.p("readme.md"), h.p("README.md"))] });
    h.ok(id);
    let names: Vec<String> = std::fs::read_dir(h.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["README.md".to_string()]);
}

#[test]
fn retry_failed_from_history() {
    let h = H::new();
    write(&h.p("s/ok.txt"), b"ok");
    let missing = h.p("s/missing.txt");
    let mut spec = JobSpec::new(copy_kind(&[&h.p("s/ok.txt"), &missing], &h.p("d")));
    spec.options.errors = Some(ErrorAction::Skip);
    let id = h.engine.submit(spec);
    let s = h.ok(id);
    assert_eq!((s.items_ok, s.items_failed), (1, 1));
    assert_eq!(s.errors[0].0, missing);
    let hid = h.engine.history()[0].id;
    write(&missing, b"now here");
    let r = h.engine.retry_failed(hid).unwrap();
    let s2 = h.ok(r);
    assert_eq!((s2.items_ok, s2.items_failed), (1, 0));
    assert_eq!(read(&h.p("d/missing.txt")), b"now here");
    // nothing failed in the retry → nothing to retry
    assert!(h.engine.retry_failed(h.engine.history()[1].id).is_none());
}

#[test]
fn clear_finished_and_remove() {
    let h = H::new();
    let a = h.submit(JobKind::Create { path: h.p("d1"), kind: CreateKind::Dir });
    let b = h.submit(JobKind::Create { path: h.p("d2"), kind: CreateKind::Dir });
    h.ok(a);
    h.ok(b);
    h.events();
    h.engine.remove(a);
    assert!(h.engine.snapshot(a).is_none());
    h.engine.clear_finished();
    assert!(h.engine.snapshots().is_empty());
    let ev = h.events();
    assert!(ev.iter().any(|e| matches!(e, JobEvent::Removed(x) if *x == a)));
    assert!(ev.iter().any(|e| matches!(e, JobEvent::Removed(x) if *x == b)));
    assert_eq!(h.engine.history().len(), 2, "history is independent of the job list");
}

#[test]
#[ignore = "needs the soyuz-archive implementation; bodies are todo!/Unsupported without it"]
fn compress_then_extract_round_trip() {
    let h = H::new();
    write(&h.p("in/a.txt"), b"hello");
    let id = h.submit(JobKind::Compress {
        sources: vec![h.p("in")],
        dest: h.p("x.zip"),
        format: soyuz_archive::ArchiveFormat::Zip,
        level: 6,
        password: None,
    });
    h.ok(id);
    let id = h.submit(JobKind::Extract {
        archive: h.p("x.zip"),
        entries: None,
        dest_dir: h.p("out"),
        password: None,
        strip_components: 0,
    });
    h.ok(id);
    assert_eq!(read(&h.p("out/in/a.txt")), b"hello");
    assert_eq!(h.engine.history()[1].records, vec![UndoRecord::Extracted { dst: h.p("out/in") }]);
}

#[test]
fn extract_unsupported_archive_fails_cleanly() {
    let h = H::new();
    write(&h.p("x.zip"), b"not really a zip");
    let id = h.submit(JobKind::Extract {
        archive: h.p("x.zip"),
        entries: None,
        dest_dir: h.p("out"),
        password: None,
        strip_components: 0,
    });
    // Archive stub → Unsupported; a real implementation → Corrupt. Either way: Failed, no panic.
    assert!(matches!(h.done(id), JobState::Failed(_)));
}

#[test]
fn extract_bomb_limit_asks_continue_or_abort() {
    let h = H::new();
    write(&h.p("in/zeros.bin"), &vec![0u8; 512 * 1024]);
    let id = h.submit(JobKind::Compress {
        sources: vec![h.p("in")],
        dest: h.p("x.zip"),
        format: soyuz_archive::ArchiveFormat::Zip,
        level: 6,
        password: None,
    });
    h.ok(id);
    let kind = |out: &str| JobKind::Extract {
        archive: h.p("x.zip"),
        entries: None,
        dest_dir: h.p(out),
        password: None,
        strip_components: 0,
    };
    let mut spec = JobSpec::new(kind("abort"));
    spec.options.extract_limits =
        soyuz_archive::ExtractLimits { max_ratio: 2, min_total_bytes: 1024, ..Default::default() };
    // A remembered "skip all" must not silently lift a safety limit.
    spec.options.errors = Some(ErrorAction::SkipAll);
    let limits = spec.options.extract_limits.clone();
    let id = h.engine.submit(spec);
    match h.prompt(id) {
        JobPrompt::Error { error, .. } => assert_eq!(error.op, crate::kinds::extract::ARCHIVE_LIMIT_OP),
        other => panic!("unexpected prompt {other:?}"),
    }
    h.engine.respond(id, PromptResponse::Error { action: ErrorAction::Abort, apply_to_all: false });
    assert!(matches!(h.done(id), JobState::Failed(_)));
    assert!(!h.p("abort/in/zeros.bin").exists());

    let mut spec = JobSpec::new(kind("continue"));
    spec.options.extract_limits = limits;
    let id = h.engine.submit(spec);
    assert!(matches!(h.prompt(id), JobPrompt::Error { .. }));
    h.engine.respond(id, PromptResponse::Error { action: ErrorAction::Skip, apply_to_all: false });
    h.ok(id);
    assert_eq!(read(&h.p("continue/in/zeros.bin")).len(), 512 * 1024);
}

#[test]
fn sync_copy_as_keeps_both_versions_and_snapshot_reports_verify() {
    let h = H::new();
    let (l, r) = (h.p("L"), h.p("R"));
    std::fs::create_dir_all(&l).unwrap();
    std::fs::create_dir_all(r.join("sub")).unwrap();
    std::fs::create_dir_all(l.join("sub")).unwrap();
    std::fs::write(l.join("sub/f.txt"), b"left").unwrap();
    std::fs::write(r.join("sub/f.txt"), b"right!").unwrap();
    // Keep both: the right version is kept as "f (conflict).txt" on both sides, then the
    // left version wins the original name.
    let alt = PathBuf::from("sub/f (conflict).txt");
    let actions = vec![
        crate::SyncAction::CopyAs { src_left: false, rel: "sub/f.txt".into(), dst_left: false, to: alt.clone() },
        crate::SyncAction::CopyAs { src_left: false, rel: "sub/f.txt".into(), dst_left: true, to: alt.clone() },
        crate::SyncAction::CopyLR("sub/f.txt".into()),
    ];
    let plan = crate::SyncPlan { left: l.clone(), right: r.clone(), actions };
    let mut spec = JobSpec::new(JobKind::Sync { plan });
    spec.options.copy.verify = Some(HashAlgo::Blake3);
    let id = h.engine.submit(spec);
    assert_eq!(h.snap(id).verify, Some(HashAlgo::Blake3));
    h.ok(id);
    assert_eq!(std::fs::read(l.join(&alt)).unwrap(), b"right!");
    assert_eq!(std::fs::read(r.join(&alt)).unwrap(), b"right!");
    assert_eq!(std::fs::read(r.join("sub/f.txt")).unwrap(), b"left");
    assert_eq!(std::fs::read(l.join("sub/f.txt")).unwrap(), b"left");
    let hist = h.engine.history();
    let last = hist.last().expect("history entry");
    assert!(last.options.as_ref().is_some_and(|o| o.copy.verify.is_some()), "verify recorded in history");
}

#[test]
fn copy_links_duplicates_to_the_first_copy() {
    let h = H::new();
    let src = h.p("src");
    let big = vec![5u8; 100 * 1024];
    let mut other = big.clone();
    other[0] = 6;
    write(&src.join("a.bin"), &big);
    write(&src.join("sub").join("b.bin"), &big);
    write(&src.join("c.bin"), &other);
    write(&src.join("s1.txt"), b"small");
    write(&src.join("s2.txt"), b"small");
    let dst = h.p("dst");
    std::fs::create_dir(&dst).unwrap();
    let mut spec = JobSpec::new(copy_kind(&[&src], &dst));
    spec.options.copy.duplicates = soyuz_fs::Duplicates::Link;
    let s = h.ok(h.engine.submit(spec));
    let out = dst.join("src");
    assert_eq!((s.items_ok, s.items_failed, s.bytes), (7, 0, 3 * 100 * 1024 + 10));
    assert_eq!(read(&out.join("sub").join("b.bin")), big);
    assert_eq!(read(&out.join("c.bin")), other);
    let linked = |a: &str, b: &str| soyuz_fs::same_file(&out.join(a), &out.join(b)).unwrap();
    assert!(linked("a.bin", "sub/b.bin"), "same content: one file, two names");
    assert!(!linked("a.bin", "c.bin"), "same size, other content");
    assert!(!linked("s1.txt", "s2.txt"), "small files are copied");
    assert!(s.notes.iter().any(|n| n.contains("1 duplicate files hard-linked")), "{:?}", s.notes);
    // Undo removes the copy, links included.
    let hid = h.engine.history()[0].id;
    let u = h.engine.undo(hid).expect("undoable");
    assert_eq!(settle(&h, u, true).items_failed, 0);
    assert!(!out.exists());
    // Clone on a volume that can't clone (NTFS, ext4): plain copies, nothing hashed.
    let dst2 = h.p("dst2");
    std::fs::create_dir(&dst2).unwrap();
    let mut spec = JobSpec::new(copy_kind(&[&src], &dst2));
    spec.options.copy.duplicates = soyuz_fs::Duplicates::Clone;
    let s = h.ok(h.engine.submit(spec));
    let out = dst2.join("src");
    assert_eq!(read(&out.join("sub").join("b.bin")), big);
    let reflinks = soyuz_fs::volumes()
        .iter()
        .any(|v| v.supports.contains(soyuz_fs::FsCaps::REFLINK) && dst2.starts_with(&v.mount));
    if !reflinks {
        assert!(!soyuz_fs::same_file(&out.join("a.bin"), &out.join("sub").join("b.bin")).unwrap());
        assert!(s.notes.iter().all(|n| !n.contains("duplicate")), "{:?}", s.notes);
    }
}

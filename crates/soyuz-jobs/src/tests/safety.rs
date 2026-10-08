//! Regression tests for the data-safety review (docs/REVIEW-1.md).

use soyuz_core::{ConflictResolution, FsErrorKind};

use super::*;
use crate::{PromptResponse, UndoRecord};

/// #1: undo never removes something that changed after the operation.
#[test]
fn undo_refuses_when_the_copy_changed() {
    let h = H::new();
    write(&h.p("s/f.txt"), b"original");
    let id = h.submit(copy_kind(&[&h.p("s/f.txt")], &h.p("d")));
    h.ok(id);
    let hist = h.engine.history();
    assert!(hist[0].this_session);
    assert_eq!(hist[0].stamps.len(), 1, "the copy was stamped");
    // The user edits the copy afterwards.
    std::fs::write(h.p("d/f.txt"), b"edited by the user, longer").unwrap();
    let u = h.engine.undo(hist[0].id).unwrap();
    let s = settle(&h, u, true);
    assert_eq!(s.items_failed, 1, "{s:?}");
    assert!(s.errors[0].1.message.contains("changed"), "{:?}", s.errors);
    assert_eq!(read(&h.p("d/f.txt")), b"edited by the user, longer");
    assert!(!h.engine.history()[0].undone);

    // Replaced by a different file with the same size: also refused (identity differs).
    let id = h.submit(copy_kind(&[&h.p("s/f.txt")], &h.p("e")));
    h.ok(id);
    std::fs::remove_file(h.p("e/f.txt")).unwrap();
    std::fs::write(h.p("e/f.txt"), b"ORIGINAL").unwrap();
    let u = h.engine.undo(h.engine.history()[1].id).unwrap();
    assert_eq!(settle(&h, u, true).items_failed, 1);
    assert_eq!(read(&h.p("e/f.txt")), b"ORIGINAL");
}

/// #1: without a trash, undo asks before deleting permanently; declining keeps the file
/// and the entry stays undoable.
#[test]
fn undo_asks_before_permanent_delete() {
    let h = H::new();
    write(&h.p("s/f.txt"), b"data");
    let id = h.submit(copy_kind(&[&h.p("s/f.txt")], &h.p("d")));
    h.ok(id);
    let hid = h.engine.history()[0].id;
    let u = h.engine.undo(hid).unwrap();
    match h.prompt(u) {
        JobPrompt::TrashUnsupported { path, reason } => {
            assert_eq!(path, h.p("d/f.txt"));
            assert!(reason.contains("permanently"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    h.engine.respond(u, PromptResponse::TrashUnsupported { permanent: false, apply_to_all: false });
    let s = h.ok(u);
    assert_eq!((s.items_ok, s.items_skipped), (0, 1));
    assert_eq!(read(&h.p("d/f.txt")), b"data");
    assert!(!h.engine.history()[0].undone && h.engine.history()[0].undoable);
    let u = h.engine.undo(hid).unwrap();
    assert_eq!(settle(&h, u, true).items_ok, 1);
    assert!(!h.p("d/f.txt").exists());
    assert!(h.engine.history()[0].undone);
}

/// #7: a partially failed undo drops the records it reversed; the next undo does not
/// replay them (here: does not delete a new file the user put at the same path).
#[test]
fn partial_undo_is_not_replayed() {
    let h = H::new();
    write(&h.p("s/a.txt"), b"a");
    write(&h.p("s/b.txt"), b"b");
    let id = h.submit(copy_kind(&[&h.p("s/a.txt"), &h.p("s/b.txt")], &h.p("d")));
    h.ok(id);
    std::fs::write(h.p("d/b.txt"), b"b, edited").unwrap();
    let hid = h.engine.history()[0].id;
    let u = h.engine.undo(hid).unwrap();
    let s = settle(&h, u, true);
    assert_eq!((s.items_ok, s.items_failed), (1, 1));
    assert!(!h.p("d/a.txt").exists());
    let he = &h.engine.history()[0];
    assert!(!he.undone && he.undoable);
    assert_eq!(he.records, vec![UndoRecord::Copied { dst: h.p("d/b.txt") }], "a.txt is no longer journaled");
    // The user creates a new d/a.txt; undoing the rest must not touch it.
    std::fs::write(h.p("d/a.txt"), b"new and precious").unwrap();
    let u = h.engine.undo(hid).unwrap();
    settle(&h, u, true);
    assert_eq!(read(&h.p("d/a.txt")), b"new and precious");
    assert_eq!(read(&h.p("d/b.txt")), b"b, edited");
}

/// #2: `a→b, b→c` where `b→c` fails: with Overwrite pre-answered, `a→b` must not replace
/// the `b` that stayed.
#[test]
fn rename_dependency_failure_keeps_original() {
    let h = H::new();
    write(&h.p("a"), b"A");
    write(&h.p("b"), b"B");
    std::fs::create_dir(h.p("c")).unwrap(); // b→c fails: a file cannot replace a folder
    let mut spec = JobSpec::new(JobKind::Rename { pairs: vec![(h.p("a"), h.p("b")), (h.p("b"), h.p("c"))] });
    spec.options.conflict = Some(ConflictResolution::Overwrite);
    let id = h.engine.submit(spec);
    let s = h.ok(id);
    assert_eq!((s.items_ok, s.items_failed, s.items_skipped), (0, 1, 1), "{s:?}");
    assert_eq!(read(&h.p("b")), b"B", "the original b is intact");
    assert_eq!(read(&h.p("a")), b"A");
    assert!(h.p("c").is_dir());
}

/// #2: the dependency rule does not break plain cycles (parking still unblocks them).
#[test]
fn rename_swap_of_file_and_folder_still_works() {
    let h = H::new();
    write(&h.p("a"), b"A");
    std::fs::create_dir(h.p("b")).unwrap();
    write(&h.p("b/inner"), b"I");
    let mut spec = JobSpec::new(JobKind::Rename { pairs: vec![(h.p("a"), h.p("b")), (h.p("b"), h.p("a"))] });
    spec.options.conflict = Some(ConflictResolution::Overwrite);
    let id = h.engine.submit(spec);
    let s = h.ok(id);
    assert_eq!(s.items_ok, 2, "a plain swap still works: {s:?}");
    assert_eq!(read(&h.p("b")), b"A");
    assert_eq!(read(&h.p("a/inner")), b"I");
}

/// #6: copying a folder into a junction / symlink that points inside it is rejected.
#[test]
fn copy_into_link_pointing_inside_source_is_rejected() {
    let h = H::new();
    write(&h.p("src/inner/f"), b"1");
    let link = h.p("link");
    #[cfg(windows)]
    let made = soyuz_fs::junction(&h.p("src/inner"), &link).is_ok();
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(h.p("src/inner"), &link).is_ok();
    if !made {
        return;
    }
    for kind in [
        JobKind::Copy { sources: vec![h.p("src")], dest_dir: link.clone(), rename_to: None },
        JobKind::Copy { sources: vec![h.p("src")], dest_dir: link.join("deeper"), rename_to: None },
        JobKind::Move { sources: vec![h.p("src")], dest_dir: link.clone(), rename_to: None },
    ] {
        let id = h.submit(kind);
        match h.done(id) {
            JobState::Failed(e) => assert_eq!(e.kind, FsErrorKind::InvalidName, "{e}"),
            other => panic!("{other:?}"),
        }
    }
    assert!(!h.p("src/inner/src").exists());
    assert_eq!(read(&h.p("src/inner/f")), b"1");
}

/// #9: a cross-device move with `follow_symlinks` never deletes a link target's contents.
#[test]
fn cross_device_move_does_not_follow_links() {
    let h = H::new();
    write(&h.p("target/keep.txt"), b"keep");
    write(&h.p("s/own.txt"), b"own");
    let link = h.p("s/link");
    #[cfg(windows)]
    let made = soyuz_fs::symlink(&h.p("target"), &link, soyuz_fs::LinkKind::Dir).is_ok();
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(h.p("target"), &link).is_ok();
    if !made {
        return; // no symlink privilege
    }
    let mut spec = JobSpec::new(JobKind::Move { sources: vec![h.p("s")], dest_dir: h.p("d"), rename_to: None });
    spec.options.mv.force_cross_device = true;
    spec.options.copy.follow_symlinks = true;
    let id = h.engine.submit(spec);
    h.ok(id);
    assert_eq!(read(&h.p("target/keep.txt")), b"keep", "the link target is untouched");
    assert_eq!(read(&h.p("d/s/own.txt")), b"own");
    assert!(std::fs::symlink_metadata(h.p("d/s/link")).unwrap().file_type().is_symlink());
}

/// #10: redo and retry reuse the original options (here: a pre-answered conflict).
#[test]
fn redo_keeps_original_options() {
    let h = H::new();
    write(&h.p("s/f.txt"), b"new");
    write(&h.p("d/f.txt"), b"old");
    let mut spec = JobSpec::new(copy_kind(&[&h.p("s/f.txt")], &h.p("d")));
    spec.options.conflict = Some(ConflictResolution::KeepBoth);
    let id = h.engine.submit(spec);
    h.ok(id);
    let he = h.engine.history()[0].clone();
    assert_eq!(he.options.as_ref().and_then(|o| o.conflict.clone()), Some(ConflictResolution::KeepBoth));
    let u = h.engine.undo(he.id).unwrap();
    settle(&h, u, true);
    // Redo meets the same conflict: answered by the stored option, no prompt.
    let r = h.engine.redo(he.id).unwrap();
    h.ok(r);
    assert_eq!(read(&h.p("d/f.txt")), b"old");
    let copies = std::fs::read_dir(h.p("d")).unwrap().count();
    assert_eq!(copies, 2, "kept both");
}

/// #8: per-item purge runs as a job and reports failures per item.
#[test]
fn purge_trash_runs_as_a_job() {
    let h = H::new();
    let item = soyuz_fs::TrashItem {
        id: "$Rnothere".into(),
        original: h.p("gone.txt"),
        deleted_at: std::time::SystemTime::UNIX_EPOCH,
        size: 1,
        kind: soyuz_core::EntryKind::File,
        trash_root: h.p("bin"),
    };
    let mut spec = JobSpec::new(JobKind::PurgeTrash { items: vec![item] });
    spec.options.errors = Some(soyuz_core::ErrorAction::Skip);
    let id = h.engine.submit(spec);
    assert_eq!(h.snap(id).kind, crate::JobKindBrief::EmptyTrash);
    let s = h.ok(id);
    assert_eq!(s.items_failed, 1);
}

//! Conflict and error prompts, apply-to-all, error policies.

use std::time::{Duration, SystemTime};

use soyuz_core::{ConflictResolution, ErrorAction};

use super::*;
use crate::{ConflictKind, PromptResponse};

fn conflict(r: ConflictResolution, all: bool) -> PromptResponse {
    PromptResponse::Conflict { resolution: r, apply_to_all: all }
}

/// src/f.txt ("new", 3 bytes) copied onto dst/f.txt ("older-and-longer").
fn setup(h: &H) -> (PathBuf, PathBuf) {
    write(&h.p("src/f.txt"), b"new");
    write(&h.p("dst/f.txt"), b"older-and-longer");
    (h.p("src/f.txt"), h.p("dst"))
}

#[test]
fn conflict_overwrite() {
    let h = H::new();
    let (src, dst) = setup(&h);
    let id = h.submit(copy_kind(&[&src], &dst));
    match h.prompt(id) {
        JobPrompt::Conflict(c) => {
            assert_eq!(c.kind, ConflictKind::FileExists);
            assert_eq!(c.src.path, src);
            assert_eq!(c.dst.path, dst.join("f.txt"));
            assert_eq!(c.dst.size, 16);
            assert_eq!(c.suggested_name, "f (2).txt");
        }
        other => panic!("{other:?}"),
    }
    assert!(h.events().iter().any(|e| matches!(e, JobEvent::Prompt(x) if *x == id)));
    h.engine.respond(id, conflict(ConflictResolution::Overwrite, false));
    h.ok(id);
    assert_eq!(read(&dst.join("f.txt")), b"new");
    assert!(!h.engine.history()[0].undoable, "overwrite is irreversible");
}

#[test]
fn conflict_skip_keep_both_rename() {
    let h = H::new();
    let (src, dst) = setup(&h);
    let id = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id);
    h.engine.respond(id, conflict(ConflictResolution::Skip, false));
    assert_eq!(h.ok(id).items_skipped, 1);
    assert_eq!(read(&dst.join("f.txt")), b"older-and-longer");

    let id = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id);
    h.engine.respond(id, conflict(ConflictResolution::KeepBoth, false));
    h.ok(id);
    assert_eq!(read(&dst.join("f (2).txt")), b"new");

    let id = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id);
    h.engine.respond(id, conflict(ConflictResolution::Rename("f (2).txt".into()), false));
    // the chosen name exists too → asked again
    h.wait_state(id, |s| *s == JobState::WaitingForUser);
    h.engine.respond(id, conflict(ConflictResolution::Rename("renamed.txt".into()), false));
    h.ok(id);
    assert_eq!(read(&dst.join("renamed.txt")), b"new");
}

#[test]
fn conflict_if_newer_and_if_larger() {
    let h = H::new();
    let (src, dst) = setup(&h);
    // dst is larger → IfLarger skips
    let id = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id);
    h.engine.respond(id, conflict(ConflictResolution::OverwriteIfLarger, false));
    assert_eq!(h.ok(id).items_skipped, 1);
    assert_eq!(read(&dst.join("f.txt")), b"older-and-longer");
    // src is newer → IfNewer overwrites
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    soyuz_fs::set_times(&dst.join("f.txt"), &soyuz_fs::TimesSet { modified: Some(old), ..Default::default() }, true)
        .unwrap();
    let id = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id);
    h.engine.respond(id, conflict(ConflictResolution::OverwriteIfNewer, false));
    assert_eq!(h.ok(id).items_ok, 1);
    assert_eq!(read(&dst.join("f.txt")), b"new");
}

#[test]
fn apply_to_all_answers_once() {
    let h = H::new();
    for n in ["a", "b", "c"] {
        write(&h.p(&format!("src/{n}")), b"new");
        write(&h.p(&format!("dst/src/{n}")), b"old");
    }
    let id = h.submit(copy_kind(&[&h.p("src")], &h.p("dst")));
    h.prompt(id);
    h.engine.respond(id, conflict(ConflictResolution::Overwrite, true));
    let s = h.ok(id);
    assert_eq!(s.items_ok, 4);
    for n in ["a", "b", "c"] {
        assert_eq!(read(&h.p(&format!("dst/src/{n}"))), b"new");
    }
    let prompts = h.events().iter().filter(|e| matches!(e, JobEvent::Prompt(_))).count();
    assert_eq!(prompts, 1);
}

#[test]
fn apply_to_all_memory_shared_when_not_reset() {
    let h = H::with(|c| c.reset_apply_all_between_jobs = false);
    let (src, dst) = setup(&h);
    let id = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id);
    h.engine.respond(id, conflict(ConflictResolution::Skip, true));
    h.ok(id);
    let id2 = h.submit(copy_kind(&[&src], &dst));
    assert_eq!(h.ok(id2).items_skipped, 1, "second job reused the remembered answer");

    // default config resets between jobs
    let h = H::new();
    let (src, dst) = setup(&h);
    let id = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id);
    h.engine.respond(id, conflict(ConflictResolution::Skip, true));
    h.ok(id);
    let id2 = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id2);
    h.engine.cancel(id2);
    assert_eq!(h.done(id2), JobState::Cancelled);
}

#[test]
fn error_retry_then_success() {
    let h = H::new();
    let missing = h.p("late.txt");
    let id = h.submit(copy_kind(&[&missing], &h.p("out")));
    match h.prompt(id) {
        JobPrompt::Error { path, error, .. } => {
            assert_eq!(path, missing);
            assert_eq!(error.kind, soyuz_core::FsErrorKind::NotFound);
        }
        other => panic!("{other:?}"),
    }
    // Retry while still missing → asked again.
    h.engine.respond(id, PromptResponse::Error { action: ErrorAction::Retry, apply_to_all: false });
    h.wait_state(id, |s| *s == JobState::WaitingForUser);
    write(&missing, b"arrived");
    h.engine.respond(id, PromptResponse::Error { action: ErrorAction::Retry, apply_to_all: false });
    let s = h.ok(id);
    assert_eq!((s.items_ok, s.items_failed), (1, 0));
    assert_eq!(read(&h.p("out/late.txt")), b"arrived");
}

#[test]
fn error_skip_all_and_abort() {
    let h = H::new();
    write(&h.p("good"), b"g");
    let kinds = || copy_kind(&[&h.p("m1"), &h.p("m2"), &h.p("good"), &h.p("m3")], &h.p("out"));
    let id = h.submit(kinds());
    h.prompt(id);
    h.engine.respond(id, PromptResponse::Error { action: ErrorAction::SkipAll, apply_to_all: false });
    let s = h.ok(id);
    assert_eq!((s.items_ok, s.items_failed), (1, 3));
    assert_eq!(s.errors.len(), 3);
    assert_eq!(h.events().iter().filter(|e| matches!(e, JobEvent::Prompt(_))).count(), 1);

    let id = h.submit(kinds());
    h.prompt(id);
    h.engine.respond(id, PromptResponse::Error { action: ErrorAction::Abort, apply_to_all: false });
    match h.done(id) {
        JobState::Failed(e) => assert_eq!(e.kind, soyuz_core::FsErrorKind::NotFound),
        other => panic!("{other:?}"),
    }
}

#[test]
fn preset_error_policy_and_respond_without_prompt() {
    let h = H::new();
    let mut spec = JobSpec::new(copy_kind(&[&h.p("nope")], &h.p("out")));
    spec.options.errors = Some(ErrorAction::Abort);
    let id = h.engine.submit(spec);
    assert!(matches!(h.done(id), JobState::Failed(_)));
    // stray answers are ignored
    h.engine.respond(id, PromptResponse::Space { proceed: true });
    h.engine.respond(JobId(9999), PromptResponse::Cancelled);
    write(&h.p("a"), b"a");
    let id = h.submit(copy_kind(&[&h.p("a")], &h.p("out")));
    h.engine.respond(id, PromptResponse::Conflict { resolution: ConflictResolution::Cancel, apply_to_all: true });
    h.ok(id);
}

#[test]
fn pause_while_waiting_applies_after_answer() {
    let h = H::new();
    let (src, dst) = setup(&h);
    let id = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id);
    h.engine.pause(id);
    assert_eq!(h.snap(id).state, JobState::WaitingForUser, "prompt stays visible");
    h.engine.respond(id, conflict(ConflictResolution::Skip, false));
    assert_eq!(h.snap(id).state, JobState::Paused);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(h.snap(id).state, JobState::Paused);
    h.engine.resume(id);
    h.ok(id);
}

#[test]
fn conflict_cancel_resolution_cancels_job() {
    let h = H::new();
    let (src, dst) = setup(&h);
    let id = h.submit(copy_kind(&[&src], &dst));
    h.prompt(id);
    h.engine.respond(id, conflict(ConflictResolution::Cancel, false));
    assert_eq!(h.done(id), JobState::Cancelled);
}

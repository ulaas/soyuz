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

/// A command prompt that waits on its input, working inside `dir`.
#[cfg(windows)]
fn prompt_inside(dir: &std::path::Path) -> std::process::Child {
    use std::process::{Command, Stdio};
    Command::new("cmd.exe")
        .arg("/k")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// Wait for an error prompt of `id` that `pred` accepts and return who keeps the item in use.
#[cfg(windows)]
fn in_use_prompt(h: &H, id: JobId, pred: impl Fn(&Option<crate::InUse>) -> bool) -> Option<crate::InUse> {
    wait_for(Duration::from_secs(20), || match h.snap(id) {
        JobSnapshot {
            state: JobState::WaitingForUser,
            pending_prompt: Some(JobPrompt::Error { in_use, retryable, .. }),
            ..
        } if pred(&in_use) => {
            assert!(retryable || in_use.is_none());
            Some(in_use)
        }
        s => {
            assert!(!s.state.is_terminal(), "job ended: {:?}", s.state);
            None
        }
    })
}

#[cfg(windows)]
#[test]
fn folder_in_use_names_the_program_and_ending_it_retries() {
    let h = H::new();
    std::fs::create_dir_all(h.p("a/inner")).unwrap();
    let mut child = prompt_inside(&h.p("a/inner"));
    let id = h.submit(JobKind::Rename { pairs: vec![(h.p("a"), h.p("b"))] });
    let u = in_use_prompt(&h, id, |u| u.is_some()).unwrap();
    assert!(u.may_end && !u.still_open && u.complete);
    let b = u.blockers.iter().find(|b| b.pid == child.id()).expect("the program inside is named");
    assert!(b.closable());
    assert_eq!(b.paths[0].file_name().unwrap(), "inner");

    h.engine.respond(id, PromptResponse::CloseBlockers { force: true });
    let s = h.ok(id);
    assert_eq!((s.items_ok, s.items_failed), (1, 0));
    assert!(h.p("b/inner").is_dir() && !h.p("a").exists());
    assert!(child.wait().is_ok());
    let log = h.engine.log(id, 0);
    assert!(log.iter().any(|l| l.msg.starts_with("ended ")), "{log:#?}");
}

#[cfg(windows)]
#[test]
fn ending_a_program_can_be_switched_off() {
    let h = H::with(|c| c.end_blockers = false);
    std::fs::create_dir_all(h.p("a/inner")).unwrap();
    let mut child = prompt_inside(&h.p("a/inner"));
    let id = h.submit(JobKind::Rename { pairs: vec![(h.p("a"), h.p("b"))] });
    let u = in_use_prompt(&h, id, |u| u.is_some()).unwrap();
    assert!(!u.may_end);
    // A forced close is only a request to close then; a program without a window cannot
    // take one, so it keeps running and the prompt says so.
    h.engine.respond(id, PromptResponse::CloseBlockers { force: true });
    let u = in_use_prompt(&h, id, |u| u.as_ref().is_some_and(|u| u.still_open)).unwrap();
    assert!(u.blockers.iter().any(|b| b.pid == child.id()));
    assert!(child.try_wait().unwrap().is_none(), "the program still runs");

    child.kill().unwrap();
    child.wait().unwrap();
    // Asked again until the system has closed the ended program's handles.
    loop {
        h.engine.respond(id, PromptResponse::Error { action: ErrorAction::Retry, apply_to_all: false });
        let s = h.wait_state(id, |s| s.is_terminal() || *s == JobState::WaitingForUser);
        if s.state.is_terminal() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(h.ok(id).items_ok, 1);
}

#[cfg(windows)]
#[test]
fn looking_for_blocking_programs_can_be_switched_off() {
    let h = H::with(|c| c.find_blockers = false);
    std::fs::create_dir_all(h.p("a/inner")).unwrap();
    let mut child = prompt_inside(&h.p("a/inner"));
    let id = h.submit(JobKind::Rename { pairs: vec![(h.p("a"), h.p("b"))] });
    assert_eq!(in_use_prompt(&h, id, |_| true), None);
    h.engine.respond(id, PromptResponse::Error { action: ErrorAction::Skip, apply_to_all: false });
    assert_eq!(h.ok(id).items_failed, 1);
    child.kill().unwrap();
    child.wait().unwrap();
}

/// A job that waits with a conflict about `src/f.txt`, polling quickly for a vanished item.
fn waiting_conflict(policy: crate::GonePolicy) -> (H, JobId, PathBuf) {
    let h = H::with(|c| {
        c.item_gone = policy;
        c.gone_poll = Duration::from_millis(20);
    });
    write(&h.p("src/f.txt"), b"new");
    write(&h.p("src/g.txt"), b"gee");
    write(&h.p("dst/f.txt"), b"older-and-longer");
    let id = h.submit(copy_kind(&[&h.p("src/f.txt"), &h.p("src/g.txt")], &h.p("dst")));
    assert!(matches!(h.prompt(id), JobPrompt::Conflict(_)));
    let src = h.p("src/f.txt");
    (h, id, src)
}

#[test]
fn a_waiting_job_cancels_itself_when_its_item_is_gone() {
    let (h, id, src) = waiting_conflict(crate::GonePolicy::Cancel);
    // Still there: it keeps waiting.
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(h.snap(id).state, JobState::WaitingForUser);
    std::fs::remove_file(&src).unwrap();
    assert_eq!(h.done(id), JobState::Cancelled);
    assert!(!h.p("dst/g.txt").exists(), "nothing after the question ran");
    assert_eq!(read(&h.p("dst/f.txt")), b"older-and-longer");
    let log = h.engine.log(id, 0);
    assert!(log.iter().any(|l| l.msg.contains("no longer exists")), "{log:#?}");
}

#[test]
fn a_waiting_job_can_skip_a_gone_item_or_keep_waiting() {
    let (h, id, src) = waiting_conflict(crate::GonePolicy::Skip);
    std::fs::remove_file(&src).unwrap();
    let s = h.ok(id);
    assert_eq!((s.items_ok, s.items_skipped), (1, 1));
    assert_eq!(read(&h.p("dst/g.txt")), b"gee");
    assert_eq!(read(&h.p("dst/f.txt")), b"older-and-longer");

    let (h, id, src) = waiting_conflict(crate::GonePolicy::Wait);
    std::fs::remove_file(&src).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(h.snap(id).state, JobState::WaitingForUser);
    h.engine.cancel(id);
    assert_eq!(h.done(id), JobState::Cancelled);
}

#[test]
fn a_question_about_a_missing_item_or_a_vanished_folder_keeps_waiting() {
    // The item was never there: the question is about that, and waits for its answer.
    let h = H::with(|c| c.gone_poll = Duration::from_millis(20));
    let id = h.submit(copy_kind(&[&h.p("late.txt")], &h.p("out")));
    assert!(matches!(h.prompt(id), JobPrompt::Error { .. }));
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(h.snap(id).state, JobState::WaitingForUser);
    write(&h.p("late.txt"), b"arrived");
    h.engine.respond(id, PromptResponse::Error { action: ErrorAction::Retry, apply_to_all: false });
    assert_eq!(h.ok(id).items_ok, 1);

    // Its whole folder went away (an unplugged drive looks like this): not "gone".
    let (h, id, src) = waiting_conflict(crate::GonePolicy::Cancel);
    let parked = h.p("parked");
    std::fs::rename(src.parent().unwrap(), &parked).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(h.snap(id).state, JobState::WaitingForUser);
    std::fs::rename(&parked, src.parent().unwrap()).unwrap();
    h.engine.respond(id, conflict(ConflictResolution::Skip, true));
    assert_eq!(h.ok(id).items_ok, 1);
}

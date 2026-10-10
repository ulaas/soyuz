//! Pause/resume/cancel latency, prompt starvation, root overlap, shutdown.

use std::time::{Duration, Instant};

use soyuz_core::ConflictResolution;

use super::*;
use crate::{CreateKind, PromptResponse, QueueMode};

#[test]
fn pause_resume_running_copy() {
    let h = H::new();
    let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    write(&h.p("big"), &data);
    let id = h.engine.submit(throttled(copy_kind(&[&h.p("big")], &h.p("out")), 1_000_000));
    wait_for(Duration::from_secs(10), || (h.snap(id).progress.bytes_done > 0).then_some(()));
    let t = Instant::now();
    h.engine.pause(id);
    assert_eq!(h.snap(id).state, JobState::Paused);
    // Within 100 ms the worker reaches a checkpoint and stops moving bytes.
    std::thread::sleep(Duration::from_millis(100));
    let b1 = h.snap(id).progress.bytes_done;
    std::thread::sleep(Duration::from_millis(300));
    let b2 = h.snap(id).progress.bytes_done;
    assert_eq!(b1, b2, "no progress while paused");
    assert!(b2 < 1_000_000);
    let paused_elapsed = h.snap(id).elapsed;
    assert!(paused_elapsed < t.elapsed() + Duration::from_millis(400));
    h.engine.resume(id);
    h.wait_state(id, |s| *s == JobState::Running);
    h.ok(id);
    assert_eq!(read(&h.p("out/big")), data);
    let snap = h.snap(id);
    assert!(!snap.samples.is_empty(), "speed samples recorded");
    assert_eq!(snap.speed_bps, 0.0, "finished jobs report no speed");
}

#[test]
fn speed_and_eta_while_running() {
    let h = H::new();
    write(&h.p("big"), &[5u8; 600_000]);
    let id = h.engine.submit(throttled(copy_kind(&[&h.p("big")], &h.p("out")), 400_000));
    let s = wait_for(Duration::from_secs(10), || {
        let s = h.snap(id);
        (s.speed_bps > 0.0 && s.eta.is_some() && s.samples.len() >= 2).then_some(s)
    });
    assert!(s.speed_bps > 50_000.0 && s.speed_bps < 2_000_000.0, "{}", s.speed_bps);
    assert!(s.eta.unwrap() < Duration::from_secs(10));
    assert!(s.progress.current.is_some());
    h.ok(id);
}

#[test]
fn cancel_running_copy_removes_partial() {
    let h = H::new();
    write(&h.p("big"), &[1u8; 2_000_000]);
    let id = h.engine.submit(throttled(copy_kind(&[&h.p("big")], &h.p("out")), 1_000_000));
    wait_for(Duration::from_secs(10), || (h.snap(id).progress.bytes_done > 0).then_some(()));
    let t = Instant::now();
    h.engine.cancel(id);
    assert_eq!(h.done(id), JobState::Cancelled);
    assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
    assert!(!h.p("out/big").exists(), "partial destination removed");
    assert!(h.engine.history().is_empty(), "only Done jobs enter history");
}

#[test]
fn cancel_paused_job() {
    let h = H::new();
    write(&h.p("big"), &[1u8; 1_000_000]);
    let id = h.engine.submit(throttled(copy_kind(&[&h.p("big")], &h.p("out")), 500_000));
    wait_for(Duration::from_secs(10), || (h.snap(id).progress.bytes_done > 0).then_some(()));
    h.engine.pause(id);
    std::thread::sleep(Duration::from_millis(150));
    let t = Instant::now();
    h.engine.cancel(id);
    assert_eq!(h.done(id), JobState::Cancelled);
    assert!(t.elapsed() < Duration::from_millis(100), "{:?}", t.elapsed());
}

#[test]
fn cancel_while_waiting_for_user_under_100ms() {
    let h = H::new();
    write(&h.p("src/f"), b"new");
    write(&h.p("dst/f"), b"old");
    let id = h.submit(copy_kind(&[&h.p("src/f")], &h.p("dst")));
    h.prompt(id);
    let t = Instant::now();
    h.engine.cancel(id);
    assert!(h.snap(id).pending_prompt.is_none(), "prompt closes immediately");
    let st = h.done(id);
    let took = t.elapsed();
    assert_eq!(st, JobState::Cancelled);
    assert!(took < Duration::from_millis(100), "cancel took {took:?}");
    assert_eq!(read(&h.p("dst/f")), b"old");
}

#[test]
fn waiting_jobs_do_not_starve_the_queue() {
    let h = H::with(|c| c.max_parallel = 2);
    let mut waiting = Vec::new();
    for n in ["w1", "w2"] {
        write(&h.p(&format!("{n}/src/f")), b"new");
        write(&h.p(&format!("{n}/dst/f")), b"old");
        let mut spec = JobSpec::new(copy_kind(&[&h.p(&format!("{n}/src/f"))], &h.p(&format!("{n}/dst"))));
        spec.options.queue = QueueMode::Parallel;
        let id = h.engine.submit(spec);
        h.prompt(id);
        waiting.push(id);
    }
    let mut third = JobSpec::new(JobKind::Create { path: h.p("third"), kind: CreateKind::Dir });
    third.options.queue = QueueMode::Parallel;
    let third = h.engine.submit(third);
    h.ok(third);
    assert!(h.p("third").is_dir());
    for id in waiting {
        assert_eq!(h.snap(id).state, JobState::WaitingForUser);
        h.engine.respond(id, PromptResponse::Conflict { resolution: ConflictResolution::Skip, apply_to_all: false });
        h.ok(id);
    }
}

#[test]
fn overlapping_roots_never_run_concurrently() {
    let h = H::with(|c| c.max_parallel = 4);
    write(&h.p("a/x"), &[1u8; 150_000]);
    write(&h.p("b/y"), &[2u8; 150_000]);
    let spec = |src: &str| {
        let mut s = throttled(copy_kind(&[&h.p(src)], &h.p("shared")), 1_000_000);
        s.options.queue = QueueMode::Parallel;
        s
    };
    let j1 = h.engine.submit(spec("a/x"));
    let j2 = h.engine.submit(spec("b/y"));
    // a third job on an unrelated tree is not blocked
    let mut other = JobSpec::new(JobKind::Create { path: h.p("elsewhere"), kind: CreateKind::Dir });
    other.options.queue = QueueMode::Parallel;
    let j3 = h.engine.submit(other);
    let mut overlap = false;
    wait_for(Duration::from_secs(20), || {
        let started = |id| {
            let s = h.snap(id);
            s.state != JobState::Queued && !s.state.is_terminal()
        };
        overlap |= started(j1) && started(j2);
        (h.snap(j1).state.is_terminal() && h.snap(j2).state.is_terminal()).then_some(())
    });
    assert!(!overlap, "jobs touching the same destination ran concurrently");
    h.ok(j1);
    h.ok(j2);
    h.ok(j3);
    assert_eq!(read(&h.p("shared/x")).len(), 150_000);
    assert_eq!(read(&h.p("shared/y")).len(), 150_000);
}

#[test]
fn shutdown_joins_all_threads() {
    let h = H::new();
    write(&h.p("big"), &[1u8; 5_000_000]);
    let mut running = throttled(copy_kind(&[&h.p("big")], &h.p("out")), 1_000_000);
    running.options.queue = QueueMode::Parallel;
    let running = h.engine.submit(running);
    write(&h.p("src/f"), b"new");
    write(&h.p("dst/f"), b"old");
    let mut waiting = JobSpec::new(copy_kind(&[&h.p("src/f")], &h.p("dst")));
    waiting.options.queue = QueueMode::Parallel;
    let waiting = h.engine.submit(waiting);
    h.prompt(waiting);
    let mut paused = JobSpec::new(JobKind::Create { path: h.p("never"), kind: CreateKind::Dir });
    paused.options.start_paused = true;
    let paused = h.engine.submit(paused);
    h.wait_state(running, |s| *s == JobState::Running);
    let t = Instant::now();
    h.engine.shutdown(Duration::from_secs(1));
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(h.engine.live_workers(), 0);
    for id in [running, waiting, paused] {
        assert_eq!(h.snap(id).state, JobState::Cancelled);
    }
    assert!(!h.engine.has_active());
    // jobs submitted after shutdown are cancelled immediately
    let late = h.submit(JobKind::Create { path: h.p("late"), kind: CreateKind::Dir });
    assert_eq!(h.snap(late).state, JobState::Cancelled);
    assert!(!h.p("late").exists() && !h.p("never").exists());
}

#[test]
fn remove_running_job_cancels_then_removes() {
    let h = H::new();
    write(&h.p("big"), &[1u8; 2_000_000]);
    let id = h.engine.submit(throttled(copy_kind(&[&h.p("big")], &h.p("out")), 1_000_000));
    h.wait_state(id, |s| *s == JobState::Running);
    h.engine.remove(id);
    wait_for(Duration::from_secs(5), || h.engine.snapshot(id).is_none().then_some(()));
    assert!(h.events().iter().any(|e| matches!(e, JobEvent::Removed(x) if *x == id)));
}

#[test]
fn many_jobs_respect_max_parallel() {
    let h = H::with(|c| c.max_parallel = 2);
    let mut ids = Vec::new();
    for i in 0..6 {
        write(&h.p(&format!("s{i}/f")), &[0u8; 60_000]);
        let mut s = throttled(copy_kind(&[&h.p(&format!("s{i}/f"))], &h.p(&format!("d{i}"))), 600_000);
        s.options.queue = QueueMode::Parallel;
        ids.push(h.engine.submit(s));
    }
    let mut max_active = 0;
    wait_for(Duration::from_secs(20), || {
        let snaps = h.engine.snapshots();
        max_active = max_active.max(snaps.iter().filter(|s| s.state.is_active()).count());
        snaps.iter().all(|s| s.state.is_terminal()).then_some(())
    });
    assert!(max_active <= 2, "{max_active}");
    for id in ids {
        h.ok(id);
    }
}

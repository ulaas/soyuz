//! Queue ordering, priorities, lanes, start_now.

use std::time::Duration;

use super::*;
use crate::{CreateKind, Priority, QueueMode};

fn create_spec(h: &H, name: &str, p: Priority) -> JobSpec {
    let mut spec = JobSpec::new(JobKind::Create { path: h.p(name), kind: CreateKind::Dir });
    spec.options.priority = p;
    spec.options.start_paused = true;
    spec.options.queue = QueueMode::Serial;
    spec
}

/// Order in which jobs entered `Planning`.
fn start_order(h: &H, n: usize) -> Vec<JobId> {
    let mut order = Vec::new();
    wait_for(Duration::from_secs(10), || {
        for e in h.engine.events().try_iter() {
            if let JobEvent::StateChanged(id, JobState::Planning) = e {
                order.push(id);
            }
        }
        (order.len() >= n).then_some(())
    });
    order
}

#[test]
fn priority_order_in_serial_lane() {
    let h = H::new();
    let low = h.engine.submit(create_spec(&h, "low", Priority::Low));
    let n1 = h.engine.submit(create_spec(&h, "n1", Priority::Normal));
    let high = h.engine.submit(create_spec(&h, "high", Priority::High));
    let n2 = h.engine.submit(create_spec(&h, "n2", Priority::Normal));
    assert_eq!(h.snap(high).queue_pos, Some(0));
    assert_eq!(h.snap(n1).queue_pos, Some(1));
    assert_eq!(h.snap(n2).queue_pos, Some(2));
    assert_eq!(h.snap(low).queue_pos, Some(3));
    assert!(h.snapshots_all_paused());
    h.events();
    h.engine.resume_all();
    assert_eq!(start_order(&h, 4), vec![high, n1, n2, low]);
    for id in [low, n1, high, n2] {
        h.ok(id);
    }
}

impl H {
    fn snapshots_all_paused(&self) -> bool {
        self.engine.snapshots().iter().all(|s| s.state == JobState::Paused)
    }
}

#[test]
fn move_in_queue_and_set_priority() {
    let h = H::new();
    let a = h.engine.submit(create_spec(&h, "a", Priority::Normal));
    let b = h.engine.submit(create_spec(&h, "b", Priority::Normal));
    let c = h.engine.submit(create_spec(&h, "c", Priority::Normal));
    h.engine.move_in_queue(c, -5); // clamped to the front
    assert_eq!(h.snap(c).queue_pos, Some(0));
    assert_eq!(h.snap(a).queue_pos, Some(1));
    h.engine.move_in_queue(a, 1);
    assert_eq!(h.snap(a).queue_pos, Some(2));
    assert_eq!(h.snap(b).queue_pos, Some(1));
    h.engine.set_priority(b, Priority::Low);
    assert_eq!(h.snap(b).queue_pos, Some(2));
    assert_eq!(h.snap(b).priority, Priority::Low);
    h.events();
    h.engine.resume_all();
    assert_eq!(start_order(&h, 3), vec![c, a, b]);
    h.ok(b);
    // running/finished jobs ignore priority changes
    h.engine.set_priority(b, Priority::High);
    assert_eq!(h.snap(b).priority, Priority::Low);
}

#[test]
fn start_now_bypasses_max_parallel() {
    let h = H::with(|c| c.max_parallel = 1);
    write(&h.p("big"), &[0u8; 300_000]);
    let mut slow = throttled(copy_kind(&[&h.p("big")], &h.p("out")), 300_000);
    slow.options.queue = QueueMode::Parallel;
    let slow = h.engine.submit(slow);
    h.wait_state(slow, |s| *s == JobState::Running);
    let mut quick = JobSpec::new(JobKind::Create { path: h.p("q"), kind: CreateKind::Dir });
    quick.options.queue = QueueMode::Parallel;
    let quick = h.engine.submit(quick);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(h.snap(quick).state, JobState::Queued, "max_parallel = 1 holds it back");
    h.engine.start_now(quick);
    h.ok(quick);
    assert!(!h.snap(slow).state.is_terminal(), "started while the slow job still runs");
    h.ok(slow);
}

#[test]
fn per_device_lanes() {
    let h = H::with(|c| c.max_parallel = 8);
    let d1 = h.p("dev1");
    let d2 = h.p("dev2");
    h.probe.devices.lock().extend([(d1.clone(), 11), (d2.clone(), 22)]);
    write(&d1.join("a"), &[1u8; 200_000]);
    write(&d1.join("b"), &[2u8; 200_000]);
    write(&d2.join("c"), &[3u8; 200_000]);
    let j1 = h.engine.submit(throttled(copy_kind(&[&d1.join("a")], &d1.join("o1")), 800_000));
    let j2 = h.engine.submit(throttled(copy_kind(&[&d1.join("b")], &d1.join("o2")), 800_000));
    let j3 = h.engine.submit(throttled(copy_kind(&[&d2.join("c")], &d2.join("o3")), 800_000));
    let mut saw_j1_j3 = false;
    let mut saw_j1_j2 = false;
    wait_for(Duration::from_secs(20), || {
        let started = |id| {
            let s = h.snap(id);
            s.state != JobState::Queued && !s.state.is_terminal()
        };
        let (s1, s2, s3) = (started(j1), started(j2), started(j3));
        saw_j1_j3 |= s1 && s3;
        saw_j1_j2 |= s1 && s2;
        [j1, j2, j3].iter().all(|id| h.snap(*id).state.is_terminal()).then_some(())
    });
    assert!(!saw_j1_j2, "same (dev1, dev1) lane must serialize");
    assert!(saw_j1_j3, "different lanes run concurrently");
    for id in [j1, j2, j3] {
        h.ok(id);
    }
}

#[test]
fn queued_job_pause_resume_cancel() {
    let h = H::new();
    let mut spec = create_spec(&h, "x", Priority::Normal);
    spec.options.start_paused = true;
    let id = h.engine.submit(spec);
    assert_eq!(h.snap(id).state, JobState::Paused);
    assert!(h.engine.has_active());
    h.engine.cancel(id);
    assert_eq!(h.snap(id).state, JobState::Cancelled);
    assert!(!h.p("x").exists());
    let id = h.engine.submit(create_spec(&h, "y", Priority::Normal));
    h.engine.resume(id);
    h.ok(id);
    assert!(h.p("y").is_dir());
}

#[test]
fn raising_max_parallel_live_starts_waiting_jobs() {
    let h = H::with(|c| c.max_parallel = 1);
    write(&h.p("big"), &[0u8; 300_000]);
    let mut slow = throttled(copy_kind(&[&h.p("big")], &h.p("out")), 300_000);
    slow.options.queue = QueueMode::Parallel;
    let slow = h.engine.submit(slow);
    h.wait_state(slow, |s| *s == JobState::Running);
    let mut quick = JobSpec::new(JobKind::Create { path: h.p("q"), kind: CreateKind::Dir });
    quick.options.queue = QueueMode::Parallel;
    let quick = h.engine.submit(quick);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(h.snap(quick).state, JobState::Queued, "max_parallel = 1 holds it back");
    let q = crate::QueueSettings { max_parallel: 2, ..h.engine.queue() };
    h.engine.set_queue(q);
    h.ok(quick);
    assert!(!h.snap(slow).state.is_terminal(), "started while the slow job still runs");
    h.ok(slow);
}

#[test]
fn lowering_history_limit_live_drops_the_oldest() {
    let h = H::with(|c| c.history_limit = 10);
    for n in ["a", "b", "c"] {
        let id = h.submit(JobKind::Create { path: h.p(n), kind: CreateKind::Dir });
        h.ok(id);
    }
    h.engine.set_queue(crate::QueueSettings { history_limit: 1, ..h.engine.queue() });
    assert_eq!(h.engine.history().iter().map(|e| e.id.0).collect::<Vec<_>>(), vec![3]);
}

#[test]
fn io_stats_say_where_the_time_went_and_who_blocks() {
    let h = H::with(|c| c.max_parallel = 1);
    let d1 = h.p("dev1");
    h.probe.devices.lock().push((d1.clone(), 11));
    write(&d1.join("a"), &[1u8; 400_000]);
    write(&d1.join("b"), &[2u8; 1000]);
    let slow = h.engine.submit(throttled(copy_kind(&[&d1.join("a")], &d1.join("o1")), 400_000));
    h.wait_state(slow, |s| *s == JobState::Running);
    let next = h.engine.submit(JobSpec::new(copy_kind(&[&d1.join("b")], &d1.join("o2"))));
    let blocked = wait_for(Duration::from_secs(5), || h.snap(next).io.blocked);
    assert_eq!(blocked, crate::Blocked::SameDevice(Some(slow)), "queued behind the job on the same disk");
    let mut quick = JobSpec::new(JobKind::Create { path: h.p("q"), kind: CreateKind::Dir });
    quick.options.queue = QueueMode::Parallel;
    let quick = h.engine.submit(quick);
    let blocked = wait_for(Duration::from_secs(5), || h.snap(quick).io.blocked);
    assert_eq!(blocked, crate::Blocked::Parallel(1));
    h.ok(slow);
    let s = h.snap(slow);
    assert_eq!(s.io.same_device, Some(true));
    assert!(s.io.moving > Duration::ZERO, "{:?}", s.io);
    assert!(s.io.avg_bps > 0.0 && s.io.peak_bps >= s.io.avg_bps * 0.5, "{:?}", s.io);
    assert_eq!(s.io.blocked, None);
    h.ok(next);
    let n = h.snap(next);
    assert!(n.io.queued >= Duration::from_millis(100), "{:?}", n.io);
    h.ok(quick);
}

//! "Chaos copy" (ROADMAP R2): many small files plus a few large ones, copied or moved
//! across "devices" while faults (disk full, I/O errors, access denied, files in use,
//! sources vanishing, slow I/O) fire at random points and the job is paused, resumed and
//! cancelled at random times. Every scenario is derived from a seed, printed on failure.
//!
//! Invariants checked after every run:
//! * no source is lost (copy: all sources intact; move: every file is at its source or at
//!   its destination with identical content, never neither);
//! * no file under a final name is partial (the copy engine stages through temp files);
//! * no `.soyuz-part-` / other temp leftovers remain;
//! * the history is consistent: a `Done` job has exactly one entry whose `Copied`/`Moved`
//!   records exist, failed items are accounted for, a cancelled job has no entry.
//!
//! The default suite runs in well under a minute; `chaos_heavy` (50k files, large files,
//! many seeds) is `#[ignore]`d: `cargo test -p soyuz-jobs chaos_heavy -- --ignored`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use soyuz_core::ErrorAction;
use soyuz_fs::fault::{FaultKind, FaultOp, FaultPlan, install};

use super::*;
use crate::journal::is_leftover_name;

/// xorshift64*: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

#[derive(Clone, Copy, Debug)]
struct Shape {
    small: usize,
    large: usize,
    large_len: usize,
}

const DEFAULT: Shape = Shape { small: 600, large: 2, large_len: 3 << 20 };
const HEAVY: Shape = Shape { small: 50_000, large: 3, large_len: 64 << 20 };

/// Build the source tree; returns relative path → content.
fn build(root: &Path, shape: Shape, rng: &mut Rng) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for i in 0..shape.small {
        let rel = PathBuf::from(format!("d{}/s{}/f{i:05}.txt", i % 7, i % 13));
        let len = rng.below(3000) as usize;
        let data: Vec<u8> = (0..len).map(|k| (k as u64 ^ i as u64) as u8).collect();
        files.insert(rel, data);
    }
    for i in 0..shape.large {
        let rel = PathBuf::from(format!("big{i}.bin"));
        let data: Vec<u8> = (0..shape.large_len).map(|k| ((k / 4096) as u64 * 31 + i as u64) as u8).collect();
        files.insert(rel, data);
    }
    for (rel, data) in &files {
        write(&root.join(rel), data);
    }
    // A large, mostly empty file (logical size only; sparse where the filesystem allows).
    let holes = root.join("holes.img");
    let f = std::fs::File::create(&holes).unwrap();
    f.set_len(shape.large_len as u64 * 2).unwrap();
    drop(f);
    files.insert(PathBuf::from("holes.img"), vec![0u8; shape.large_len * 2]);
    files
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stack.push(p.clone());
            }
            out.push(p);
        }
    }
    out
}

fn rand_fault(rng: &mut Rng) -> FaultKind {
    match rng.below(5) {
        0 => FaultKind::no_space(),
        1 => FaultKind::io_error(),
        2 => FaultKind::access_denied(),
        3 => FaultKind::in_use(),
        _ => FaultKind::Slow(Duration::from_millis(rng.below(20))),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Copy,
    /// Forced cross-device move (copy + delete sources).
    Move,
}

struct Outcome {
    state: JobState,
}

fn scenario(seed: u64, mode: Mode, shape: Shape) -> Outcome {
    let mut rng = Rng::new(seed);
    let h = H::new();
    let src = h.p("src");
    let files = build(&src, shape, &mut rng);
    let dst_dir = h.p("dst");
    let n = files.len() as u64;

    // Fault plan: a few random failures somewhere in the run.
    let mut plan = FaultPlan::new(h.dir.path());
    let mut vanished: Option<PathBuf> = None;
    for _ in 0..rng.below(4) {
        let op = match rng.below(4) {
            0 => FaultOp::Write,
            1 => FaultOp::Create,
            2 => FaultOp::Rename,
            _ => FaultOp::Delete,
        };
        plan = plan.fail(op, 1 + rng.below(n), rand_fault(&mut rng));
    }
    if rng.chance(40) {
        let victim = files.keys().nth(rng.below(n) as usize).unwrap().clone();
        let name = victim.file_name().unwrap().to_string_lossy().into_owned();
        plan = plan.on_name(FaultOp::Open, &name, FaultKind::Vanish);
        vanished = Some(victim);
    }
    let guard = install(plan);

    let mut spec = JobSpec::new(match mode {
        Mode::Copy => JobKind::Copy { sources: vec![src.clone()], dest_dir: dst_dir.clone(), rename_to: None },
        Mode::Move => JobKind::Move { sources: vec![src.clone()], dest_dir: dst_dir.clone(), rename_to: None },
    });
    spec.options.errors = Some(if rng.chance(80) { ErrorAction::Skip } else { ErrorAction::Abort });
    spec.options.mv.force_cross_device = mode == Mode::Move;
    spec.options.copy.chunk = 1 << 20;
    let id = h.engine.submit(spec);

    // Random pause / resume cycles and maybe a cancel.
    let cancel_at = rng.chance(30).then(|| Duration::from_millis(rng.below(1500)));
    let pauses = rng.below(3);
    let start = std::time::Instant::now();
    for _ in 0..pauses {
        std::thread::sleep(Duration::from_millis(rng.below(60)));
        h.engine.pause(id);
        std::thread::sleep(Duration::from_millis(rng.below(30)));
        h.engine.resume(id);
    }
    if let Some(at) = cancel_at {
        std::thread::sleep(at.saturating_sub(start.elapsed()));
        h.engine.cancel(id);
    }
    let state = wait_for(Duration::from_secs(120), || {
        let s = h.engine.snapshot(id)?;
        assert!(
            s.state != JobState::WaitingForUser,
            "seed {seed}: unexpected prompt {:?}; log {:#?}",
            s.pending_prompt,
            h.engine.log(id, 0)
        );
        s.state.is_terminal().then_some(s.state)
    });
    drop(guard);

    let ctx = format!("seed {seed} {mode:?} -> {state:?}");
    let dst_root = dst_dir.join("src");

    // 1. Nothing lost, nothing partial under a final name.
    for (rel, data) in &files {
        let s = src.join(rel);
        let d = dst_root.join(rel);
        let at_src = std::fs::read(&s).ok();
        let at_dst = std::fs::read(&d).ok();
        if let Some(b) = &at_dst {
            assert_eq!(b.len(), data.len(), "{ctx}: partial file at destination {}", d.display());
            assert!(b == data, "{ctx}: corrupt destination {}", d.display());
        }
        match mode {
            Mode::Copy => {
                if vanished.as_ref() != Some(rel) {
                    assert!(at_src.as_ref() == Some(data), "{ctx}: source damaged {}", s.display());
                }
            }
            Mode::Move => {
                if let Some(b) = &at_src {
                    assert!(b == data, "{ctx}: source damaged {}", s.display());
                }
                if vanished.as_ref() != Some(rel) {
                    assert!(at_src.is_some() || at_dst.is_some(), "{ctx}: {} LOST", rel.display());
                }
            }
        }
    }
    // 2. No temp leftovers anywhere.
    let left: Vec<PathBuf> = all_files(h.dir.path())
        .into_iter()
        .filter(|p| is_leftover_name(&p.file_name().unwrap().to_string_lossy()))
        .collect();
    assert!(left.is_empty(), "{ctx}: leftovers {left:?}");

    // 3. History consistent with the outcome.
    let hist = h.engine.history();
    match &state {
        JobState::Done(sum) => {
            assert_eq!(hist.len(), 1, "{ctx}");
            let e = &hist[0];
            assert_eq!(e.summary.items_failed, sum.items_failed, "{ctx}");
            assert_eq!(sum.errors.len() as u64, sum.items_failed.min(crate::job::MAX_SUMMARY_ERRORS as u64), "{ctx}");
            for r in &e.records {
                match r {
                    crate::UndoRecord::Copied { dst } | crate::UndoRecord::Moved { to: dst, .. } => {
                        assert!(dst.exists(), "{ctx}: recorded {} missing", dst.display())
                    }
                    other => panic!("{ctx}: unexpected record {other:?}"),
                }
            }
            if vanished.is_none() && sum.items_failed == 0 && mode == Mode::Copy {
                assert_eq!(sum.items_skipped, 0, "{ctx}");
            }
        }
        JobState::Cancelled | JobState::Failed(_) => assert!(hist.is_empty(), "{ctx}: {hist:?}"),
        other => panic!("{ctx}: not terminal {other:?}"),
    }
    Outcome { state }
}

fn run_seeds(seeds: std::ops::Range<u64>, mode: Mode, shape: Shape) {
    let mut done = 0;
    for seed in seeds {
        if let JobState::Done(_) = scenario(seed, mode, shape).state {
            done += 1;
        }
    }
    assert!(done > 0, "at least one scenario completed");
}

#[test]
fn chaos_copy() {
    run_seeds(1..7, Mode::Copy, DEFAULT);
}

#[test]
fn chaos_cross_device_move() {
    run_seeds(100..105, Mode::Move, DEFAULT);
}

#[test]
fn clean_run_copies_everything() {
    // Control: no faults, no cancel (seed chosen so the plan is empty is not guaranteed,
    // so run without the scenario machinery).
    let h = H::new();
    let mut rng = Rng::new(7);
    let files = build(&h.p("src"), Shape { small: 200, large: 1, large_len: 1 << 20 }, &mut rng);
    let id = h.submit(copy_kind(&[&h.p("src")], &h.p("dst")));
    let sum = h.ok(id);
    assert_eq!(sum.items_failed, 0);
    for (rel, data) in &files {
        assert_eq!(&read(&h.p("dst/src").join(rel)), data);
    }
}

#[test]
#[ignore = "heavy: 50k files, 64 MiB files, many seeds (run with --ignored)"]
fn chaos_heavy() {
    run_seeds(1000..1012, Mode::Copy, HEAVY);
    run_seeds(2000..2006, Mode::Move, HEAVY);
}

//! Crash journal: what is written while jobs run, and resume / roll back from it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::journal::{self, JournalJob};

struct J {
    h: H,
    _jdir: tempfile::TempDir,
    path: PathBuf,
}

fn harness() -> J {
    let jdir = tempfile::tempdir().unwrap();
    let path = jdir.path().join(journal::JOURNAL_FILE);
    let p = path.clone();
    let h = H::with(move |c| c.journal_path = Some(p));
    J { h, _jdir: jdir, path }
}

fn sources(h: &H, n: usize, len: usize) -> Vec<PathBuf> {
    (0..n)
        .map(|i| {
            let p = h.p(&format!("src/f{i:02}.bin"));
            write(&p, &vec![i as u8; len]);
            p
        })
        .collect()
}

/// Run a throttled copy until the on-disk journal holds at least `min_records` records,
/// then pause it and return the journal as a crash would have left it.
fn interrupted_copy(j: &J, srcs: &[PathBuf], min_records: usize) -> (JobId, JournalJob) {
    let kind = JobKind::Copy { sources: srcs.to_vec(), dest_dir: j.h.p("dst"), rename_to: None };
    let id = j.h.engine.submit(throttled(kind, 200_000));
    let job = wait_for(Duration::from_secs(20), || {
        let jobs = journal::read_journal(&j.path)?;
        jobs.into_iter().find(|jj| jj.records.len() >= min_records)
    });
    j.h.engine.pause(id);
    (id, job)
}

fn leftovers(dir: &Path) -> Vec<PathBuf> {
    walkdir(dir).into_iter().filter(|p| journal::is_leftover_name(&p.file_name().unwrap().to_string_lossy())).collect()
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walkdir(&p));
            }
            out.push(p);
        }
    }
    out
}

#[test]
fn journal_tracks_running_jobs_and_disappears_after() {
    let j = harness();
    let srcs = sources(&j.h, 6, 20_000);
    let (id, job) = interrupted_copy(&j, &srcs, 2);
    assert_eq!(job.kind, JobKind::Copy { sources: srcs.clone(), dest_dir: j.h.p("dst"), rename_to: None });
    assert!(job.label.starts_with("Copy 6 items"));
    assert!(job.items_total >= 6 || job.bytes_total > 0);
    assert!(job.records.iter().all(|r| matches!(r, crate::UndoRecord::Copied { .. })));
    assert_eq!(job.stamps.len(), job.records.len(), "every copy is identity-stamped");
    j.h.engine.resume(id);
    j.h.ok(id);
    // The scheduler drops the journal right after the job finishes; allow it a moment
    // under load instead of racing it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while j.path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!j.path.exists(), "nothing in flight: no journal");
}

#[test]
fn hash_jobs_are_not_journaled() {
    let j = harness();
    let srcs = sources(&j.h, 1, 10);
    let id = j.h.submit(JobKind::Hash { paths: srcs, algo: soyuz_core::HashAlgo::Blake3 });
    j.h.ok(id);
    assert!(!j.path.exists());
}

#[test]
fn rollback_undoes_the_completed_part() {
    let j = harness();
    let srcs = sources(&j.h, 10, 20_000);
    let (id, job) = interrupted_copy(&j, &srcs, 3);
    // "Crash": the job never finishes normally (cancel keeps what was copied).
    j.h.engine.cancel(id);
    assert_eq!(j.h.done(id), JobState::Cancelled);
    let recorded: Vec<PathBuf> = job
        .records
        .iter()
        .filter_map(|r| match r {
            crate::UndoRecord::Copied { dst } => Some(dst.clone()),
            _ => None,
        })
        .collect();
    assert!(recorded.len() >= 3);
    assert!(recorded.iter().all(|p| p.exists()));
    let rb = j.h.engine.rollback_journaled(&job).expect("records to roll back");
    settle(&j.h, rb, true);
    assert!(recorded.iter().all(|p| !p.exists()), "completed copies removed");
    assert!(srcs.iter().all(|p| p.exists()), "sources untouched");
    assert!(leftovers(&j.h.p("dst")).is_empty());
    let mut nothing = job.clone();
    nothing.records.clear();
    assert!(j.h.engine.rollback_journaled(&nothing).is_none());
}

#[test]
fn resume_finishes_without_duplicates() {
    let j = harness();
    let srcs = sources(&j.h, 8, 20_000);
    let (id, job) = interrupted_copy(&j, &srcs, 2);
    j.h.engine.cancel(id);
    j.h.done(id);
    let r = j.h.engine.resume_journaled(&job).expect("something left");
    let sum = j.h.ok(r);
    assert!(sum.items_skipped >= 2, "what arrived is skipped: {sum:?}");
    for (i, s) in srcs.iter().enumerate() {
        let d = j.h.p("dst").join(s.file_name().unwrap());
        assert_eq!(read(&d), vec![i as u8; 20_000]);
    }
    assert_eq!(std::fs::read_dir(j.h.p("dst")).unwrap().count(), srcs.len(), "no duplicates");
    assert!(j.h.snap(r).label.starts_with("Resume: "));
}

#[test]
fn cancelled_jobs_leave_no_journal() {
    let jdir = tempfile::tempdir().unwrap();
    let path = jdir.path().join(journal::JOURNAL_FILE);
    let p = path.clone();
    let h = H::with(move |c| c.journal_path = Some(p));
    let srcs = sources(&h, 1, 10);
    // A conflict prompt nobody answers keeps the job open; shutdown cancels it, and a job
    // that exits leaves the journal.
    write(&h.p("dst/f00.bin"), b"existing");
    let id = h.submit(JobKind::Copy { sources: srcs, dest_dir: h.p("dst"), rename_to: None });
    h.prompt(id);
    assert!(journal::read_journal(&path).is_some_and(|v| v.len() == 1));
    h.engine.shutdown(Duration::from_secs(5));
    assert!(!path.exists(), "cancelled jobs are not interrupted ones");
}

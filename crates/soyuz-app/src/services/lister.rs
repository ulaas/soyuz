//! Background directory listing: a small worker pool runs
//! `soyuz_fs::read_dir_streaming` and streams chunks into the mailbox. Every navigation
//! bumps the pane generation and cancels the previous listing's `OpControl`; stale
//! generations are dropped on the UI side.
//!
//! Two independent, elastic pools: one for pane listings/patches and one for auxiliary
//! work (dropdowns, completion, "lost folder" resolution, sync-navigation probes). A
//! read on a dead network drive can block a worker in the kernel for minutes and
//! cannot be cancelled, so when every worker of a pool is busy a new job spawns an
//! extra worker (up to a cap) instead of queueing behind the stuck ones.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_core::{Control, EntryInfo, FsErrorKind, Location, OpControl};
use soyuz_fs::ListOptions;

use super::mailbox::{Postman, UiMsg};
use crate::state::pane::PaneId;

enum Job {
    List { pane: PaneId, generation: u64, path: PathBuf, ctl: Arc<OpControl>, soft: bool },
    Stat { pane: PaneId, generation: u64, dir: PathBuf, paths: Vec<PathBuf> },
    Aux { path: PathBuf },
    Resolve { pane: PaneId, generation: u64, path: PathBuf },
    Probe { pane: PaneId, path: PathBuf },
}

/// Elastic worker pool: `base` workers up front, grown on demand up to `max` while all
/// workers are busy (a worker stuck on a dead mount stays "busy").
struct Pool {
    name: &'static str,
    tx: Sender<Job>,
    rx: Receiver<Job>,
    postman: Postman,
    /// Workers currently waiting for a job.
    idle: Arc<AtomicUsize>,
    spawned: AtomicUsize,
    max: usize,
}

impl Pool {
    fn new(name: &'static str, postman: Postman, base: usize, max: usize) -> Self {
        let (tx, rx) = unbounded::<Job>();
        let pool = Pool {
            name,
            tx,
            rx,
            postman,
            idle: Arc::new(AtomicUsize::new(0)),
            spawned: AtomicUsize::new(0),
            max: max.max(base.max(1)),
        };
        for _ in 0..base.max(1) {
            pool.spawn_worker();
        }
        pool
    }

    fn spawn_worker(&self) {
        let i = self.spawned.fetch_add(1, Ordering::SeqCst);
        let rx = self.rx.clone();
        let postman = self.postman.clone();
        let idle = self.idle.clone();
        idle.fetch_add(1, Ordering::SeqCst);
        let spawned = thread::Builder::new().name(format!("soyuz-{}-{i}", self.name)).spawn(move || {
            while let Ok(job) = rx.recv() {
                idle.fetch_sub(1, Ordering::SeqCst);
                run(job, &postman);
                idle.fetch_add(1, Ordering::SeqCst);
            }
        });
        if let Err(e) = spawned {
            self.idle.fetch_sub(1, Ordering::SeqCst);
            log::error!("cannot spawn {} thread: {e}", self.name);
        }
    }

    fn send(&self, job: Job) {
        // Every worker busy (possibly stuck in the kernel on an unreachable share):
        // add one rather than letting this job wait behind it.
        if self.idle.load(Ordering::SeqCst) <= self.rx.len() && self.spawned.load(Ordering::SeqCst) < self.max {
            self.spawn_worker();
        }
        let _ = self.tx.send(job);
    }
}

pub struct Lister {
    main: Pool,
    aux: Pool,
}

impl Lister {
    pub fn new(postman: Postman, threads: usize) -> Self {
        let threads = threads.max(1);
        Lister {
            main: Pool::new("lister", postman.clone(), threads, threads * 4),
            aux: Pool::new("lister-aux", postman, 2, 8),
        }
    }

    /// Stream a full listing of `path`. `soft` relists (watcher rescans, refresh) deliver
    /// everything in one message so the pane swaps contents without flicker.
    pub fn list(&self, pane: PaneId, generation: u64, path: &Path, ctl: Arc<OpControl>, soft: bool) {
        self.main.send(Job::List { pane, generation, path: path.to_path_buf(), ctl, soft });
    }

    /// Re-stat changed children of `dir` for an in-place patch.
    pub fn stat(&self, pane: PaneId, generation: u64, dir: &Path, paths: Vec<PathBuf>) {
        self.main.send(Job::Stat { pane, generation, dir: dir.to_path_buf(), paths });
    }

    /// One-shot listing for dropdowns / completion / Miller columns.
    pub fn aux(&self, path: &Path) {
        self.aux.send(Job::Aux { path: path.to_path_buf() });
    }

    /// Find the nearest existing ancestor of a directory that disappeared.
    pub fn resolve(&self, pane: PaneId, generation: u64, path: &Path) {
        self.aux.send(Job::Resolve { pane, generation, path: path.to_path_buf() });
    }

    /// Check whether `path` is a directory (synchronized navigation).
    pub fn probe(&self, pane: PaneId, path: &Path) {
        self.aux.send(Job::Probe { pane, path: path.to_path_buf() });
    }
}

/// `(entry with its on-disk spelling, stale queried spelling)`; see the `Stat` job.
fn case_fixed(path: &Path, queried: &str, e: EntryInfo) -> (Option<EntryInfo>, Option<String>) {
    if !cfg!(any(windows, target_os = "macos")) {
        return (Some(e), None);
    }
    match soyuz_fs::on_disk_name(path).map(|n| n.to_string_lossy().into_owned()) {
        Some(real) if real != queried && real.to_lowercase() == queried.to_lowercase() => {
            let real_path = path.with_file_name(&real);
            match soyuz_fs::stat(&real_path, false) {
                Ok(fixed) => (Some(fixed), Some(queried.to_string())),
                Err(_) => (Some(e), None),
            }
        }
        Some(_) => (Some(e), None),
        None => (None, None),
    }
}

fn list_options() -> ListOptions {
    ListOptions { include_hidden: true, chunk: 512, ..ListOptions::default() }
}

fn run(job: Job, postman: &Postman) {
    match job {
        Job::List { pane, generation, path, ctl, soft } => {
            if ctl.is_cancelled() {
                return;
            }
            // Remote folders and `\server` share lists: one blocking call.
            if let Some(r) = super::remote::list_blocking(&path, soft) {
                if ctl.is_cancelled() {
                    return;
                }
                let (chunk, err) = match r {
                    Ok(v) => (v, None),
                    Err(e) => (Vec::new(), Some(e)),
                };
                postman.send(UiMsg::Listing { pane, generation, chunk, done: true, err });
                return;
            }
            let mut buffered: Vec<EntryInfo> = Vec::new();
            let mut alive = true;
            let result = soyuz_fs::read_dir_streaming(&path, &list_options(), &ctl, &mut |chunk| {
                if ctl.is_cancelled() {
                    return Control::Stop;
                }
                if soft {
                    buffered.extend(chunk);
                } else {
                    alive = postman.send(UiMsg::Listing { pane, generation, chunk, done: false, err: None });
                }
                if alive { Control::Continue } else { Control::Stop }
            });
            if ctl.is_cancelled() || !alive {
                return;
            }
            let err = result.err().filter(|e| e.kind != FsErrorKind::Interrupted);
            postman.send(UiMsg::Listing { pane, generation, chunk: buffered, done: true, err });
        }
        Job::Stat { pane, generation, dir, paths } => {
            let mut updated = Vec::new();
            let mut removed = Vec::new();
            for p in paths {
                if p.parent() != Some(dir.as_path()) {
                    continue;
                }
                match soyuz_fs::stat(&p, false) {
                    Ok(e) => {
                        // Case-insensitive volumes: after a case-only rename the old
                        // spelling still stats fine. Report it removed and the on-disk
                        // spelling updated instead (Windows: one FindFirstFile call).
                        let queried = soyuz_core::entry::display_name(&p);
                        match case_fixed(&p, &queried, e) {
                            (Some(e), None) => updated.push(e),
                            (Some(e), Some(stale)) => {
                                updated.push(e);
                                removed.push(stale);
                            }
                            (None, _) => removed.push(queried),
                        }
                    }
                    Err(_) => removed.push(soyuz_core::entry::display_name(&p)),
                }
            }
            // Old and new spelling of a case-only rename resolve to the same entry.
            let mut names = std::collections::HashSet::new();
            updated.retain(|e: &EntryInfo| names.insert(e.name.clone()));
            removed.retain(|n| !names.contains(n));
            postman.send(UiMsg::Patched { pane, generation, updated, removed });
        }
        Job::Aux { path } => {
            let result = match super::remote::list_blocking(&path, false) {
                Some(r) => r,
                None => soyuz_fs::read_dir(&path, &list_options()),
            };
            postman.send(UiMsg::Aux { path, result });
        }
        Job::Resolve { pane, generation, path } => {
            if path.is_dir() {
                return; // spurious "lost" (e.g. a stale watch); nothing to do
            }
            let mut cur = path.parent().map(Path::to_path_buf);
            while let Some(p) = cur {
                if p.is_dir() {
                    postman.send(UiMsg::Resolved { pane, generation, location: Location::Dir(p) });
                    return;
                }
                cur = p.parent().map(Path::to_path_buf);
            }
            postman.send(UiMsg::Resolved { pane, generation, location: Location::Computer });
        }
        Job::Probe { pane, path } => {
            let exists = path.is_dir();
            postman.send(UiMsg::Probed { pane, location: Location::Dir(path), exists });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_only_rename_reports_the_on_disk_spelling() {
        let d = tempfile::tempdir().unwrap();
        let old = d.path().join("Report.TXT");
        std::fs::write(&old, b"x").unwrap();
        soyuz_fs::rename_case_only(&old, "report.txt").unwrap();
        let e = soyuz_fs::stat(&old, false);
        if cfg!(any(windows, target_os = "macos")) {
            // The stale spelling still stats on a case-insensitive volume.
            let Ok(e) = e else { return }; // case-sensitive volume: nothing to fix
            let (fixed, stale) = case_fixed(&old, "Report.TXT", e);
            assert_eq!(fixed.map(|f| f.name), Some("report.txt".to_string()));
            assert_eq!(stale.as_deref(), Some("Report.TXT"));
        } else {
            assert!(e.is_err());
        }
        let new = d.path().join("report.txt");
        let e = soyuz_fs::stat(&new, false).unwrap();
        let (fixed, stale) = case_fixed(&new, "report.txt", e);
        assert_eq!(fixed.map(|f| f.name), Some("report.txt".to_string()));
        assert_eq!(stale, None);
    }
}

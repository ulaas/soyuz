//! Folder-tree background service: lists child folders on worker threads (never on the
//! UI thread), probes each child for subfolders so collapsed rows only show a chevron
//! when there is something inside, and watches the expanded folders so the tree
//! refreshes itself. Results come back on a channel drained by the tree UI each frame.
//!
//! Like the main lister, the pool is elastic: a read stuck on a dead network share keeps
//! its worker busy, so a new job spawns another worker (up to a cap) instead of queueing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_core::{Attributes, Control, EntryInfo, FsErrorKind, OpControl};
use soyuz_fs::{ListOptions, WatchEvent, WatchId, WatchKind, Watcher};

use crate::ui::tree::model::{ChildInfo, Probe};

/// Children probed for subfolders per listing (bigger folders probe lazily: unknown
/// rows simply keep their chevron until opened).
const PROBE_LIMIT: usize = 400;
/// At most this many folders are watched (the first open folders, roots first).
pub const WATCH_LIMIT: usize = 64;

pub use crate::host::Wake;

/// A result for the UI.
#[derive(Debug)]
pub enum TreeMsg {
    Listed {
        dir: PathBuf,
        result: Result<Vec<ChildInfo>, String>,
    },
    Probed {
        dir: PathBuf,
        probes: Vec<(PathBuf, Probe)>,
    },
    /// Something changed inside a watched folder: list it again.
    Changed(PathBuf),
    /// A watched folder vanished: list its parent again.
    Lost(PathBuf),
}

enum Job {
    List(PathBuf),
}

enum WatchCmd {
    Set(Vec<PathBuf>),
}

pub struct TreeService {
    jobs: Sender<Job>,
    job_rx: Receiver<Job>,
    out_tx: Sender<TreeMsg>,
    rx: Receiver<TreeMsg>,
    watch: Sender<WatchCmd>,
    wake: Wake,
    idle: Arc<AtomicUsize>,
    spawned: AtomicUsize,
}

impl TreeService {
    pub fn new(wake: Wake, debounce: Duration) -> Self {
        let (jobs, job_rx) = unbounded();
        let (out_tx, rx) = unbounded();
        let (watch, watch_rx) = unbounded();
        spawn_watcher(watch_rx, out_tx.clone(), wake.clone(), debounce);
        let s = TreeService {
            jobs,
            job_rx,
            out_tx,
            rx,
            watch,
            wake,
            idle: Arc::new(AtomicUsize::new(0)),
            spawned: AtomicUsize::new(0),
        };
        s.spawn_worker();
        s.spawn_worker();
        s
    }

    fn spawn_worker(&self) {
        let i = self.spawned.fetch_add(1, Ordering::SeqCst);
        let rx = self.job_rx.clone();
        let tx = self.out_tx.clone();
        let wake = self.wake.clone();
        let idle = self.idle.clone();
        idle.fetch_add(1, Ordering::SeqCst);
        let spawned = thread::Builder::new().name(format!("soyuz-tree-{i}")).spawn(move || {
            while let Ok(job) = rx.recv() {
                idle.fetch_sub(1, Ordering::SeqCst);
                match job {
                    Job::List(dir) => run_list(&dir, &tx, &wake),
                }
                idle.fetch_add(1, Ordering::SeqCst);
            }
        });
        if let Err(e) = spawned {
            self.idle.fetch_sub(1, Ordering::SeqCst);
            log::error!("cannot spawn tree worker: {e}");
        }
    }

    /// List the child folders of `dir` (then probe them).
    pub fn list(&self, dir: &Path) {
        if self.idle.load(Ordering::SeqCst) <= self.job_rx.len() && self.spawned.load(Ordering::SeqCst) < 8 {
            self.spawn_worker();
        }
        let _ = self.jobs.send(Job::List(dir.to_path_buf()));
    }

    /// Watch exactly these folders (the watcher thread diffs against the current set).
    pub fn watch(&self, dirs: Vec<PathBuf>) {
        let _ = self.watch.send(WatchCmd::Set(dirs));
    }

    /// Pending results.
    pub fn drain(&self) -> Vec<TreeMsg> {
        self.rx.try_iter().collect()
    }
}

fn child_info(e: &EntryInfo) -> ChildInfo {
    ChildInfo {
        name: e.name.clone(),
        path: e.path.clone(),
        hidden: e.is_hidden(),
        system: e.attrs.contains(Attributes::SYSTEM),
    }
}

fn options() -> ListOptions {
    let dot_hidden = super::lister::dot_hidden();
    ListOptions { include_hidden: true, stat_links: true, chunk: 256, dot_hidden, ..ListOptions::default() }
}

/// Child folders of `dir` (directories, junctions and links to directories).
pub fn list_dirs(dir: &Path) -> Result<Vec<ChildInfo>, String> {
    soyuz_fs::read_dir(dir, &options())
        .map(|v| v.iter().filter(|e| e.is_dir_like()).map(child_info).collect())
        .map_err(|e| e.to_string())
}

/// Does `dir` contain a subfolder (any / a non-hidden one)? Stops at the first
/// non-hidden subfolder, so huge folders cost one directory chunk at most in practice.
pub fn probe(dir: &Path) -> Probe {
    let mut p = Probe::default();
    let ctl = OpControl::new();
    let dot_hidden = super::lister::dot_hidden();
    let opts = ListOptions { include_hidden: true, stat_links: false, chunk: 64, dot_hidden, ..ListOptions::default() };
    let _ = soyuz_fs::read_dir_streaming(dir, &opts, &ctl, &mut |chunk| {
        for e in &chunk {
            let dirlike = e.is_dir_like() || matches!(e.kind, soyuz_core::EntryKind::Symlink { .. });
            if !dirlike {
                continue;
            }
            p.any = true;
            if !e.is_hidden() {
                p.visible = true;
                return Control::Stop;
            }
        }
        Control::Continue
    });
    p
}

fn run_list(dir: &Path, tx: &Sender<TreeMsg>, wake: &Wake) {
    let result = soyuz_fs::read_dir(dir, &options());
    let result = match result {
        Ok(v) => Ok(v.iter().filter(|e| e.is_dir_like()).map(child_info).collect::<Vec<_>>()),
        Err(e) if e.kind == FsErrorKind::Interrupted => return,
        Err(e) => Err(e.to_string()),
    };
    let children: Vec<PathBuf> = match &result {
        Ok(v) => v.iter().take(PROBE_LIMIT).map(|c| c.path.clone()).collect(),
        Err(_) => Vec::new(),
    };
    if tx.send(TreeMsg::Listed { dir: dir.to_path_buf(), result }).is_err() {
        return;
    }
    wake();
    if children.is_empty() {
        return;
    }
    // Probe in batches so chevrons settle progressively on slow disks.
    let mut batch = Vec::new();
    for (i, c) in children.iter().enumerate() {
        batch.push((c.clone(), probe(c)));
        if batch.len() >= 32 || i + 1 == children.len() {
            if tx.send(TreeMsg::Probed { dir: dir.to_path_buf(), probes: std::mem::take(&mut batch) }).is_err() {
                return;
            }
            wake();
        }
    }
}

fn spawn_watcher(rx: Receiver<WatchCmd>, out: Sender<TreeMsg>, wake: Wake, debounce: Duration) {
    let spawned = thread::Builder::new().name("soyuz-tree-watch".into()).spawn(move || {
        let (ev_tx, ev_rx) = unbounded::<WatchEvent>();
        // Forward events (mapped to folders) from a small relay thread.
        let ids: Arc<parking_lot::Mutex<HashMap<WatchId, PathBuf>>> = Arc::default();
        let relay_ids = ids.clone();
        let relay_wake = wake.clone();
        let _ = thread::Builder::new().name("soyuz-tree-watch-relay".into()).spawn(move || {
            while let Ok(ev) = ev_rx.recv() {
                let Some(dir) = relay_ids.lock().get(&ev.id).cloned() else { continue };
                let msg = match ev.kind {
                    WatchKind::Changed(_) | WatchKind::Rescan => TreeMsg::Changed(dir),
                    WatchKind::Lost => TreeMsg::Lost(dir),
                };
                if out.send(msg).is_err() {
                    return;
                }
                relay_wake();
            }
        });
        let noop: Wake = Arc::new(|| {});
        let mut watcher = match Watcher::new(ev_tx, debounce, noop) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("folder tree watching unavailable: {e}");
                while rx.recv().is_ok() {}
                return;
            }
        };
        let mut current: HashMap<PathBuf, WatchId> = HashMap::new();
        while let Ok(WatchCmd::Set(mut dirs)) = rx.recv() {
            // Only the latest set matters.
            while let Ok(WatchCmd::Set(newer)) = rx.try_recv() {
                dirs = newer;
            }
            dirs.truncate(WATCH_LIMIT);
            let gone: Vec<PathBuf> = current.keys().filter(|d| !dirs.contains(d)).cloned().collect();
            for d in gone {
                if let Some(id) = current.remove(&d) {
                    ids.lock().remove(&id);
                    watcher.unwatch(id);
                }
            }
            for d in dirs {
                if current.contains_key(&d) {
                    continue;
                }
                match watcher.watch(&d, false) {
                    Ok(id) => {
                        ids.lock().insert(id, d.clone());
                        current.insert(d, id);
                    }
                    Err(e) => log::debug!("tree: not watching {}: {e}", d.display()),
                }
            }
        }
    });
    if let Err(e) = spawned {
        log::error!("cannot spawn tree watcher: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_folders_and_probes_them() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a/inner")).unwrap();
        std::fs::create_dir(d.path().join("b")).unwrap();
        std::fs::write(d.path().join("file.txt"), b"x").unwrap();
        let mut v = list_dirs(d.path()).unwrap();
        v.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(v.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(probe(&d.path().join("a")), Probe { any: true, visible: true });
        assert_eq!(probe(&d.path().join("b")), Probe::default());
        assert!(list_dirs(&d.path().join("missing")).is_err());
    }

    #[test]
    fn service_lists_on_a_worker() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("x")).unwrap();
        let s = TreeService::new(Arc::new(|| {}), Duration::from_millis(50));
        s.list(d.path());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut listed = false;
        let mut probed = false;
        while std::time::Instant::now() < deadline && !(listed && probed) {
            for m in s.drain() {
                match m {
                    TreeMsg::Listed { result, .. } => {
                        assert_eq!(result.unwrap().len(), 1);
                        listed = true;
                    }
                    TreeMsg::Probed { probes, .. } => {
                        assert_eq!(probes.len(), 1);
                        probed = true;
                    }
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(listed && probed);
    }
}

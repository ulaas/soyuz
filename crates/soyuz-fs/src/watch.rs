//! Directory watching: a thin `notify-debouncer-full` wrapper (one notify backend per
//! [`Watcher`], N watched directories).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify::event::{EventKind, RemoveKind};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use parking_lot::Mutex;
use soyuz_core::{FsError, FsErrorKind, FsResult};

/// Maximum number of paths in one [`WatchKind::Changed`]; more → [`WatchKind::Rescan`].
pub const MAX_CHANGED_PATHS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WatchId(pub u64);

#[derive(Clone, Debug)]
pub struct WatchEvent {
    pub id: WatchId,
    pub kind: WatchKind,
}

#[derive(Clone, Debug)]
pub enum WatchKind {
    /// Changed paths (≤ 64), deduplicated.
    Changed(Vec<PathBuf>),
    /// Too many events or backend overflow: relist.
    Rescan,
    /// The watched directory was removed / unmounted.
    Lost,
}

#[derive(Debug)]
struct Watched {
    dir: PathBuf,
    recursive: bool,
}

type Registry = Arc<Mutex<HashMap<WatchId, Watched>>>;

/// Owns one notify backend; `Send`.
pub struct Watcher {
    debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    registry: Registry,
    next_id: u64,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").field("watched", &self.registry.lock().len()).finish()
    }
}

impl Watcher {
    /// Events are sent on `tx`, followed by a call to `wake` (e.g. `ctx.request_repaint`).
    pub fn new(
        tx: crossbeam_channel::Sender<WatchEvent>,
        debounce: Duration,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> FsResult<Self> {
        let registry: Registry = Arc::default();
        let reg = registry.clone();
        let handler = move |res: DebounceEventResult| {
            let events = dispatch(&reg, res);
            if events.is_empty() {
                return;
            }
            for ev in events {
                if tx.send(ev).is_err() {
                    return;
                }
            }
            wake();
        };
        let debouncer = new_debouncer(debounce, None, handler).map_err(|e| notify_err("watch", None, e))?;
        Ok(Watcher { debouncer, registry, next_id: 1 })
    }

    pub fn watch(&mut self, dir: &Path, recursive: bool) -> FsResult<WatchId> {
        let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
        self.debouncer.watch(dir, mode).map_err(|e| notify_err("watch", Some(dir), e))?;
        let id = WatchId(self.next_id);
        self.next_id += 1;
        self.registry.lock().insert(id, Watched { dir: dir.to_path_buf(), recursive });
        Ok(id)
    }

    pub fn unwatch(&mut self, id: WatchId) {
        let removed = self.registry.lock().remove(&id);
        if let Some(w) = removed {
            let still_used = self.registry.lock().values().any(|o| o.dir == w.dir);
            if !still_used {
                let _ = self.debouncer.unwatch(&w.dir);
            }
        }
    }
}

fn notify_err(op: &'static str, path: Option<&Path>, e: notify::Error) -> FsError {
    let kind = match &e.kind {
        notify::ErrorKind::PathNotFound => FsErrorKind::NotFound,
        notify::ErrorKind::Io(io) => return FsError::from_io(op, path, io),
        notify::ErrorKind::MaxFilesWatch => FsErrorKind::NoSpace,
        _ => FsErrorKind::Io,
    };
    FsError::with_message(kind, op, path, e.to_string())
}

/// Map debounced notify events onto per-watch events.
fn dispatch(reg: &Registry, res: DebounceEventResult) -> Vec<WatchEvent> {
    let reg = reg.lock();
    let events = match res {
        Ok(ev) => ev,
        Err(errors) => {
            log::warn!("watcher errors: {errors:?}");
            return reg.keys().map(|id| WatchEvent { id: *id, kind: WatchKind::Rescan }).collect();
        }
    };
    let mut changed: HashMap<WatchId, Vec<PathBuf>> = HashMap::new();
    let mut rescan: Vec<WatchId> = Vec::new();
    let mut lost: Vec<WatchId> = Vec::new();
    for ev in &events {
        if ev.need_rescan() {
            rescan.extend(reg.keys().copied());
            continue;
        }
        for p in &ev.paths {
            for (id, w) in reg.iter() {
                if p == &w.dir {
                    if matches!(ev.kind, EventKind::Remove(RemoveKind::Folder | RemoveKind::Any)) {
                        lost.push(*id);
                    }
                    continue;
                }
                let inside = if w.recursive { p.starts_with(&w.dir) } else { p.parent() == Some(w.dir.as_path()) };
                if inside {
                    let v = changed.entry(*id).or_default();
                    if !v.contains(p) {
                        v.push(p.clone());
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for id in &lost {
        out.push(WatchEvent { id: *id, kind: WatchKind::Lost });
    }
    rescan.sort_by_key(|id| id.0);
    rescan.dedup();
    for id in &rescan {
        if !lost.contains(id) {
            out.push(WatchEvent { id: *id, kind: WatchKind::Rescan });
        }
    }
    for (id, paths) in changed {
        if lost.contains(&id) || rescan.contains(&id) {
            continue;
        }
        let kind = if paths.len() > MAX_CHANGED_PATHS { WatchKind::Rescan } else { WatchKind::Changed(paths) };
        out.push(WatchEvent { id, kind });
    }
    out
}

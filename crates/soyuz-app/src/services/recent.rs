//! The `Recent` location: recently visited folders and opened files, kept in
//! `recent.toml` next to the config (portable-aware). Navigation and launches record
//! items ([`visit_folder`], [`opened_file`]); `Location::Recent` lists them, newest first,
//! through the normal listing path ([`attach`]). The "Accessed" column shows when Soyuz
//! last used the item.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, unbounded};
use serde::{Deserialize, Serialize};
use soyuz_core::{Column, ColumnSpec, SortDir};

use super::mailbox::Postman;
use crate::state::pane::{ListingStatus, Pane};

/// Folders and files are capped separately so a burst of browsing doesn't push out files.
pub const MAX_FOLDERS: usize = 150;
pub const MAX_FILES: usize = 150;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentItem {
    pub path: PathBuf,
    pub dir: bool,
    /// Unix seconds of the last use.
    pub when: u64,
    pub count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentStore {
    /// Most recent first.
    #[serde(default)]
    pub items: Vec<RecentItem>,
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl RecentStore {
    /// Record a use of `path` at `when` (moves it to the front).
    pub fn touch(&mut self, path: &Path, dir: bool, when: u64) {
        let path = soyuz_fs::strip_verbatim(path);
        let count = match self.items.iter().position(|i| i.path == path) {
            Some(i) => self.items.remove(i).count + 1,
            None => 1,
        };
        self.items.insert(0, RecentItem { path, dir, when, count });
        let (mut folders, mut files) = (0, 0);
        self.items.retain(|i| {
            let n = if i.dir { &mut folders } else { &mut files };
            *n += 1;
            *n <= if i.dir { MAX_FOLDERS } else { MAX_FILES }
        });
    }

    pub fn remove(&mut self, path: &Path) {
        let path = soyuz_fs::strip_verbatim(path);
        self.items.retain(|i| i.path != path);
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

struct Registry {
    store: RecentStore,
    loaded: bool,
    started: bool,
    dirty: bool,
    last_change: Instant,
    rx: Option<Receiver<RecentStore>>,
    path: PathBuf,
    /// Items recorded before the file finished loading.
    early: Vec<(PathBuf, bool, u64)>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            store: RecentStore::default(),
            loaded: false,
            started: false,
            dirty: false,
            last_change: Instant::now(),
            rx: None,
            path: soyuz_core::config::config_dir().join("recent.toml"),
            early: Vec::new(),
        }
    }
}

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Load on first call, merge, and write back changes after a debounce (worker threads).
/// Returns `true` in the frame the stored items arrive.
pub fn pump(wake: &std::sync::Arc<dyn Fn() + Send + Sync>) -> bool {
    REG.with_borrow_mut(|r| {
        let mut arrived = false;
        if !r.started {
            r.started = true;
            let (tx, rx) = unbounded();
            r.rx = Some(rx);
            let path = r.path.clone();
            let wake = wake.clone();
            let _ = std::thread::Builder::new().name("soyuz-recent-load".into()).spawn(move || {
                // A broken file is backed up and never overwritten (store_file).
                let store = super::store_file::load(&path, RecentStore::from_toml).value();
                let _ = tx.send(store);
                wake();
            });
        }
        if let Some(store) = r.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            r.store = store;
            for (p, dir, when) in std::mem::take(&mut r.early) {
                r.store.touch(&p, dir, when);
            }
            r.loaded = true;
            arrived = true;
        }
        if r.dirty && r.loaded && r.last_change.elapsed() > Duration::from_secs(2) {
            queue_save(r);
        }
        arrived
    })
}

/// Hand the current store to the single ordered save worker.
fn queue_save(r: &mut Registry) {
    r.dirty = false;
    super::store_file::save_async(r.path.clone(), r.store.to_toml().into_bytes());
}

/// Queue pending changes now, skipping the debounce (`App::save` then calls
/// `store_file::flush`).
pub fn flush() {
    REG.with_borrow_mut(|r| {
        if r.dirty && r.loaded {
            queue_save(r);
        }
    });
}

pub fn pending_save() -> bool {
    REG.with_borrow(|r| r.dirty)
}

fn record(path: &Path, dir: bool) {
    REG.with_borrow_mut(|r| {
        let now = unix_now();
        if r.loaded {
            r.store.touch(path, dir, now);
        } else {
            r.early.push((path.to_path_buf(), dir, now));
        }
        r.dirty = true;
        r.last_change = Instant::now();
    });
}

/// The user navigated to a folder.
pub fn visit_folder(path: &Path) {
    record(path, true);
}

/// The user opened a file.
pub fn opened_file(path: &Path) {
    record(path, false);
}

/// Forget an item ("Remove from Recent").
pub fn forget(paths: &[PathBuf]) {
    REG.with_borrow_mut(|r| {
        for p in paths {
            r.store.remove(p);
        }
        r.dirty = true;
        r.last_change = Instant::now();
    });
}

pub fn clear() {
    REG.with_borrow_mut(|r| {
        r.store.items.clear();
        r.dirty = true;
        r.last_change = Instant::now();
    });
}

/// Most recent folders (for the sidebar / palette), newest first.
pub fn folders(n: usize) -> Vec<PathBuf> {
    REG.with_borrow(|r| r.store.items.iter().filter(|i| i.dir).take(n).map(|i| i.path.clone()).collect())
}

/// Files opened at or after `since` (unix seconds) with when, newest first (timeline).
pub fn files_since(since: u64) -> Vec<(PathBuf, i64)> {
    REG.with_borrow(|r| {
        r.store.items.iter().filter(|i| !i.dir && i.when >= since).map(|i| (i.path.clone(), i.when as i64)).collect()
    })
}

/// Start listing `Location::Recent` for this pane generation.
pub fn attach(pane: &mut Pane, postman: Postman) {
    if !pane.view.columns.iter().any(|c| c.column == Column::Accessed && c.visible) {
        let mut path = ColumnSpec::new(Column::Path);
        path.width = 280.0;
        let mut acc = ColumnSpec::new(Column::Accessed);
        acc.width = 150.0;
        pane.view.columns = vec![ColumnSpec::new(Column::Name), path, acc, ColumnSpec::new(Column::Size)];
        pane.view.sort.column = Column::Accessed;
        pane.view.sort.dir = SortDir::Desc;
        pane.view.sort.folders_first = false;
    }
    pane.view.group = soyuz_core::GroupBy::None;
    pane.status = ListingStatus::Loading { since: Instant::now() };
    let items: Vec<(PathBuf, u64)> = REG.with_borrow(|r| {
        let mut v: Vec<(PathBuf, u64)> = r.store.items.iter().map(|i| (i.path.clone(), i.when)).collect();
        for (p, _, w) in &r.early {
            v.push((p.clone(), *w));
        }
        v
    });
    let when: std::collections::HashMap<PathBuf, u64> = items.iter().cloned().collect();
    let paths: Vec<PathBuf> = items.into_iter().map(|(p, _)| p).collect();
    super::tags::stat_listing(pane, paths, postman, move |p, e| {
        // "Accessed" = when Soyuz last used it (the sort key of this view).
        if let Some(w) = when.get(p) {
            e.accessed = Some(UNIX_EPOCH + Duration::from_secs(*w));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_moves_to_front_and_counts() {
        let mut s = RecentStore::default();
        s.touch(Path::new("/a"), true, 1);
        s.touch(Path::new("/b.txt"), false, 2);
        s.touch(Path::new("/a"), true, 3);
        assert_eq!(s.items[0].path, PathBuf::from("/a"));
        assert_eq!(s.items[0].count, 2);
        assert_eq!(s.items[0].when, 3);
        assert_eq!(s.items.len(), 2);
        let back = RecentStore::from_toml(&s.to_toml()).unwrap();
        assert_eq!(back, s);
        s.remove(Path::new("/a"));
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn caps_folders_and_files_separately() {
        let mut s = RecentStore::default();
        s.touch(Path::new("/keep.txt"), false, 0);
        for i in 0..(MAX_FOLDERS + 20) {
            s.touch(&PathBuf::from(format!("/d{i}")), true, i as u64);
        }
        assert_eq!(s.items.iter().filter(|i| i.dir).count(), MAX_FOLDERS);
        assert!(s.items.iter().any(|i| i.path == Path::new("/keep.txt")), "files survive folder churn");
    }
}

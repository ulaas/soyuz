//! Size, size on disk and item counts of a selection, computed by a background walk for
//! the Properties dialogs (GUI and terminal UI). The walk publishes its running totals
//! every ~120 ms and wakes the frontend; cancel it with the [`OpControl`] it was given.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use soyuz_core::{Control, EntryInfo, EntryKind, OpControl};
use soyuz_fs::{WalkItem, WalkOptions};

use crate::host::Wake;

/// Folder statistics accumulated by the background walk.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub alloc: u64,
    pub alloc_known: bool,
    pub errors: u64,
    pub done: bool,
}

/// Walk `entries` (files count themselves; a single folder does not count itself) on a
/// worker thread; the returned slot holds the totals so far.
pub fn spawn(wake: Wake, entries: &[EntryInfo], ctl: Arc<OpControl>) -> Arc<Mutex<Stats>> {
    let stats = Arc::new(Mutex::new(Stats::default()));
    let s2 = stats.clone();
    let roots: Vec<(PathBuf, bool)> = entries.iter().map(|e| (e.path.clone(), e.kind == EntryKind::Dir)).collect();
    let single_dir = roots.len() == 1 && roots[0].1;
    let _ = std::thread::Builder::new().name("soyuz-props-size".into()).spawn(move || {
        let mut last = Instant::now();
        let mut local = Stats::default();
        for (root, _) in &roots {
            let r = soyuz_fs::walk(root, &WalkOptions::default(), &ctl, &mut |item| {
                match item {
                    WalkItem::Entry { entry, depth } => {
                        if !(single_dir && depth == 0) {
                            match entry.kind {
                                EntryKind::Dir => local.dirs += 1,
                                _ => {
                                    local.files += 1;
                                    local.bytes += entry.size;
                                    match entry.alloc_size {
                                        Some(a) => {
                                            local.alloc += a;
                                            local.alloc_known = true;
                                        }
                                        None => local.alloc += entry.size.div_ceil(4096) * 4096,
                                    }
                                }
                            }
                        }
                    }
                    WalkItem::Error { .. } => local.errors += 1,
                    WalkItem::DirDone { .. } => {}
                }
                if last.elapsed() > Duration::from_millis(120) {
                    last = Instant::now();
                    *s2.lock() = local.clone();
                    wake();
                }
                if ctl.is_cancelled() { Control::Stop } else { Control::Continue }
            });
            if r.is_err() {
                local.errors += 1;
            }
            if ctl.is_cancelled() {
                return;
            }
        }
        local.done = true;
        *s2.lock() = local;
        wake();
    });
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"12345").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("b"), b"123").unwrap();
        let e = soyuz_fs::stat(dir.path(), false).unwrap();
        let s = spawn(crate::host::no_wake(), &[e], OpControl::new());
        let t0 = Instant::now();
        while !s.lock().done {
            assert!(t0.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(5));
        }
        let s = s.lock().clone();
        assert_eq!((s.files, s.dirs, s.bytes), (2, 1, 8));
    }
}

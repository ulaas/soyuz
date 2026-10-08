//! Atomic JSON index of trashed items (original path + date) for bins without native
//! restore metadata (macOS `~/.Trash`: Finder's "Put Back" data lives in `.DS_Store`, which
//! is not writable by third parties).
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use soyuz_core::time::serde_rfc3339;
use soyuz_core::{EntryKind, FsResult};

/// What we know about one trashed item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct IndexEntry {
    pub original: PathBuf,
    #[serde(with = "serde_rfc3339")]
    pub deleted_at: SystemTime,
    pub size: u64,
    pub kind: EntryKind,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    /// Key: absolute path of the item inside the bin.
    items: BTreeMap<String, IndexEntry>,
}

/// Serializes load-modify-save cycles within this process.
static LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct TrashIndex {
    path: PathBuf,
}

impl TrashIndex {
    pub(crate) fn new(path: PathBuf) -> Self {
        TrashIndex { path }
    }

    fn load(&self) -> IndexFile {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                log::warn!("trash index {} is corrupt ({e}); starting a new one", self.path.display());
                IndexFile::default()
            }),
            Err(_) => IndexFile::default(),
        }
    }

    fn save(&self, f: &IndexFile) -> FsResult<()> {
        if let Some(dir) = self.path.parent() {
            crate::create::create_dir_all(dir)?;
        }
        let bytes = serde_json::to_vec_pretty(f).expect("index serializes");
        crate::create::write_atomic(&self.path, &bytes)
    }

    fn key(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    pub(crate) fn insert(&self, item_path: &Path, e: IndexEntry) -> FsResult<()> {
        let _g = LOCK.lock();
        let mut f = self.load();
        f.version = 1;
        f.items.insert(Self::key(item_path), e);
        self.save(&f)
    }

    pub(crate) fn remove(&self, item_path: &Path) -> FsResult<()> {
        let _g = LOCK.lock();
        let mut f = self.load();
        if f.items.remove(&Self::key(item_path)).is_some() {
            self.save(&f)?;
        }
        Ok(())
    }

    #[allow(dead_code)] // used by tests and diagnostics
    pub(crate) fn get(&self, item_path: &Path) -> Option<IndexEntry> {
        let _g = LOCK.lock();
        self.load().items.get(&Self::key(item_path)).cloned()
    }

    /// All entries; entries whose item no longer exists are pruned.
    pub(crate) fn entries(&self) -> BTreeMap<String, IndexEntry> {
        let _g = LOCK.lock();
        let mut f = self.load();
        let before = f.items.len();
        f.items.retain(|k, _| crate::list::exists_nofollow(Path::new(k)));
        if f.items.len() != before {
            let _ = self.save(&f);
        }
        f.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_prune() {
        let d = tempfile::tempdir().unwrap();
        let idx = TrashIndex::new(d.path().join("sub").join("trash-index.json"));
        let item = d.path().join("item.txt");
        std::fs::write(&item, b"x").unwrap();
        let e = IndexEntry {
            original: PathBuf::from("/home/u/item.txt"),
            deleted_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
            size: 1,
            kind: EntryKind::File,
        };
        idx.insert(&item, e.clone()).unwrap();
        idx.insert(&d.path().join("gone"), e.clone()).unwrap();
        assert_eq!(idx.get(&item), Some(e.clone()));
        let all = idx.entries();
        assert_eq!(all.len(), 1, "missing item pruned");
        idx.remove(&item).unwrap();
        assert!(idx.get(&item).is_none());
        std::fs::write(d.path().join("sub").join("trash-index.json"), b"{not json").unwrap();
        assert!(idx.entries().is_empty(), "corrupt index tolerated");
    }
}

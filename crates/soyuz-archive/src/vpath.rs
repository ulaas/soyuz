//! Directory-like views of an archive, used by the app's pane for `Location::Archive`.
//!
//! Readers build a [`DirIndex`] once (O(entries)) and cache it, so navigating nested
//! directories of a 50k-entry archive costs O(children) per [`list_dir`] call.

use std::collections::{BTreeMap, HashMap};

use soyuz_core::location::normalize_inner;
use soyuz_core::{EntryInfo, EntryKind, join_vpath};

use crate::{ArchiveEntry, ArchiveError, ArchiveReader};

/// One child of a directory in a [`DirIndex`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirChild {
    pub name: String,
    pub is_dir: bool,
    /// Explicit entry (`None` for implicit directories).
    pub entry: Option<usize>,
}

/// Directory tree of an archive: `dir inner path ("" = root) -> sorted children`, plus a
/// path → entry map. Later duplicates of a path win (zip "update" semantics).
#[derive(Clone, Debug, Default)]
pub struct DirIndex {
    dirs: HashMap<String, Vec<DirChild>>,
    by_path: HashMap<String, usize>,
}

type Children = BTreeMap<String, (bool, Option<usize>)>;

impl DirIndex {
    pub fn build(entries: &[ArchiveEntry]) -> DirIndex {
        let mut dirs: HashMap<String, Children> = HashMap::new();
        let mut by_path = HashMap::with_capacity(entries.len());
        dirs.entry(String::new()).or_default();
        for (i, e) in entries.iter().enumerate() {
            let path = normalize_inner(&e.path);
            if path.is_empty() {
                continue;
            }
            by_path.insert(path.clone(), i);
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            let slot = dirs.entry(parent.to_string()).or_default().entry(name.to_string()).or_insert((false, None));
            slot.0 |= e.is_dir;
            slot.1 = Some(i);
            if e.is_dir {
                dirs.entry(path.clone()).or_default();
            }
            // Implicit ancestors (stop as soon as one is already linked to its parent).
            let mut cur = parent;
            while !cur.is_empty() {
                let (pp, n) = cur.rsplit_once('/').unwrap_or(("", cur));
                if dirs.contains_key(cur) && dirs.get(pp).is_some_and(|c| c.get(n).is_some_and(|s| s.0)) {
                    break;
                }
                dirs.entry(cur.to_string()).or_default();
                let slot = dirs.entry(pp.to_string()).or_default().entry(n.to_string()).or_insert((true, None));
                slot.0 = true;
                cur = pp;
            }
        }
        let dirs = dirs
            .into_iter()
            .map(|(k, v)| {
                let kids = v.into_iter().map(|(name, (is_dir, entry))| DirChild { name, is_dir, entry }).collect();
                (k, kids)
            })
            .collect();
        DirIndex { dirs, by_path }
    }

    /// Children of `inner` (any form accepted by `normalize_inner`); `None` if it is not a directory.
    pub fn children(&self, inner: &str) -> Option<&[DirChild]> {
        self.dirs.get(&normalize_inner(inner)).map(Vec::as_slice)
    }

    /// Entry index for `inner`.
    pub fn find(&self, inner: &str) -> Option<usize> {
        self.by_path.get(&normalize_inner(inner)).copied()
    }

    /// Whether `inner` is a directory (explicit or implicit).
    pub fn is_dir(&self, inner: &str) -> bool {
        self.dirs.contains_key(&normalize_inner(inner))
    }
}

/// Children of `inner` (`""` = root). Synthesizes `EntryInfo::minimal` with
/// `path = join_vpath(archive, "inner/name")`; implicit directories are included.
pub fn list_dir(reader: &mut dyn ArchiveReader, inner: &str) -> Result<Vec<EntryInfo>, ArchiveError> {
    let archive = reader.path().to_path_buf();
    let inner = normalize_inner(inner);
    let children: Vec<DirChild> = match reader.dir_index()? {
        Some(ix) => ix.children(&inner).map(<[DirChild]>::to_vec).unwrap_or_default(),
        None => scan_children(reader.entries()?, &inner),
    };
    let entries = reader.entries()?;
    let out = children
        .into_iter()
        .map(|DirChild { name, is_dir, entry }| {
            let child_inner = if inner.is_empty() { name.clone() } else { format!("{inner}/{name}") };
            let kind = if is_dir { EntryKind::Dir } else { EntryKind::File };
            let mut info = EntryInfo::minimal(join_vpath(&archive, &child_inner), kind);
            info.name = name;
            if let Some(e) = entry.and_then(|i| entries.get(i)) {
                if !is_dir {
                    info.size = e.size;
                    info.alloc_size = Some(e.compressed);
                }
                info.modified = e.modified;
                info.mode = e.mode;
                if e.is_symlink {
                    info.kind = EntryKind::Symlink { target: soyuz_core::LinkTarget::Unknown };
                }
            }
            info
        })
        .collect();
    Ok(out)
}

/// Fallback for readers without an index: one pass over all entries.
fn scan_children(entries: &[ArchiveEntry], inner: &str) -> Vec<DirChild> {
    let prefix = if inner.is_empty() { String::new() } else { format!("{inner}/") };
    let mut children: Children = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        let path = normalize_inner(&e.path);
        let Some(rest) = path.strip_prefix(&prefix) else { continue };
        if rest.is_empty() {
            continue;
        }
        match rest.split_once('/') {
            Some((dir, _)) => {
                let slot = children.entry(dir.to_string()).or_insert((true, None));
                slot.0 = true;
            }
            None => {
                let slot = children.entry(rest.to_string()).or_insert((e.is_dir, Some(i)));
                slot.1 = Some(i);
                slot.0 |= e.is_dir;
            }
        }
    }
    children.into_iter().map(|(name, (is_dir, entry))| DirChild { name, is_dir, entry }).collect()
}

/// Index of the entry whose normalized path equals `inner` (the last one if duplicated).
pub fn find_entry(reader: &mut dyn ArchiveReader, inner: &str) -> Option<usize> {
    if let Ok(Some(ix)) = reader.dir_index() {
        return ix.find(inner);
    }
    let inner = normalize_inner(inner);
    reader.entries().ok()?.iter().rposition(|e| normalize_inner(&e.path) == inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use soyuz_core::{OpControl, ProgressSink};
    use std::io::Read;
    use std::path::{Path, PathBuf};

    struct Fake {
        path: PathBuf,
        entries: Vec<ArchiveEntry>,
        index: Option<DirIndex>,
        use_index: bool,
    }

    fn entry(index: usize, path: &str, is_dir: bool, size: u64) -> ArchiveEntry {
        ArchiveEntry {
            index,
            path: path.into(),
            is_dir,
            size,
            compressed: size / 2,
            modified: None,
            crc32: None,
            encrypted: false,
            mode: None,
            is_symlink: false,
            link_target: None,
        }
    }

    impl ArchiveReader for Fake {
        fn format(&self) -> ArchiveFormat {
            ArchiveFormat::Zip
        }
        fn path(&self) -> &Path {
            &self.path
        }
        fn entries(&mut self) -> Result<&[ArchiveEntry], ArchiveError> {
            Ok(&self.entries)
        }
        fn open_entry(&mut self, _index: usize) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
            Ok(Box::new(std::io::empty()))
        }
        fn extract(
            &mut self,
            _selection: Option<&[usize]>,
            _dest: &Path,
            _opts: &ExtractOptions,
            _ctl: &OpControl,
            _sink: &mut dyn ExtractSink,
        ) -> Result<ExtractSummary, ArchiveError> {
            Ok(ExtractSummary::default())
        }
        fn test(&mut self, _ctl: &OpControl, _sink: &mut dyn ProgressSink) -> Result<TestReport, ArchiveError> {
            Ok(TestReport::default())
        }
        fn dir_index(&mut self) -> Result<Option<&DirIndex>, ArchiveError> {
            if !self.use_index {
                return Ok(None);
            }
            Ok(Some(self.index.get_or_insert_with(|| DirIndex::build(&self.entries))))
        }
    }

    fn fake(use_index: bool) -> Fake {
        Fake {
            path: PathBuf::from("/t/a.zip"),
            entries: vec![
                entry(0, "readme.md", false, 10),
                entry(1, "src/", true, 0),
                entry(2, "src/main.rs", false, 100),
                entry(3, "docs/guide/intro.md", false, 7), // implicit dirs docs/, docs/guide/
            ],
            index: None,
            use_index,
        }
    }

    #[test]
    fn list_root_and_nested() {
        for use_index in [false, true] {
            let mut f = fake(use_index);
            let root = list_dir(&mut f, "").unwrap();
            let names: Vec<_> = root.iter().map(|e| (e.name.as_str(), e.kind)).collect();
            assert_eq!(names, [("docs", EntryKind::Dir), ("readme.md", EntryKind::File), ("src", EntryKind::Dir)]);
            assert_eq!(root[1].size, 10);
            assert_eq!(root[1].path, PathBuf::from("/t/a.zip!/readme.md"));

            let docs = list_dir(&mut f, "/docs/").unwrap();
            assert_eq!(docs.len(), 1);
            assert_eq!(docs[0].name, "guide");
            assert_eq!(docs[0].kind, EntryKind::Dir);
            let guide = list_dir(&mut f, "docs/guide").unwrap();
            assert_eq!(guide[0].path, PathBuf::from("/t/a.zip!/docs/guide/intro.md"));
            assert!(list_dir(&mut f, "nope").unwrap().is_empty());

            assert_eq!(find_entry(&mut f, "src/main.rs"), Some(2));
            assert_eq!(find_entry(&mut f, "/src"), Some(1));
            assert_eq!(find_entry(&mut f, "nope"), None);
        }
    }

    #[test]
    fn index_handles_deep_and_unordered_paths() {
        let entries = vec![
            entry(0, "a/b/c/d.txt", false, 1),
            entry(1, "a/x.txt", false, 1),
            entry(2, "a/b/", true, 0),
            entry(3, "a/b/c/e.txt", false, 1),
            entry(4, "a/x.txt", false, 2), // duplicate: later wins
        ];
        let ix = DirIndex::build(&entries);
        let names = |d: &str| ix.children(d).unwrap().iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(""), ["a"]);
        assert_eq!(names("a"), ["b", "x.txt"]);
        assert_eq!(names("a/b"), ["c"]);
        assert_eq!(names("a/b/c"), ["d.txt", "e.txt"]);
        assert_eq!(ix.children("a").unwrap()[0].entry, Some(2));
        assert_eq!(ix.find("a/x.txt"), Some(4));
        assert!(ix.is_dir("a/b/c"));
        assert!(!ix.is_dir("a/x.txt"));
    }
}

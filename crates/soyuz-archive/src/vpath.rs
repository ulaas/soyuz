//! Directory-like views of an archive, used by the app's pane for `Location::Archive`.

use std::collections::BTreeMap;

use soyuz_core::location::normalize_inner;
use soyuz_core::{EntryInfo, EntryKind, join_vpath};

use crate::{ArchiveError, ArchiveReader};

/// Children of `inner` (`""` = root). Synthesizes `EntryInfo::minimal` with
/// `path = join_vpath(archive, "inner/name")`; implicit directories are included.
pub fn list_dir(reader: &mut dyn ArchiveReader, inner: &str) -> Result<Vec<EntryInfo>, ArchiveError> {
    let archive = reader.path().to_path_buf();
    let inner = normalize_inner(inner);
    let prefix = if inner.is_empty() { String::new() } else { format!("{inner}/") };
    // name → (is_dir, entry index if explicit)
    let mut children: BTreeMap<String, (bool, Option<usize>)> = BTreeMap::new();
    let entries = reader.entries()?;
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
    let out = children
        .into_iter()
        .map(|(name, (is_dir, idx))| {
            let child_inner = if inner.is_empty() { name.clone() } else { format!("{inner}/{name}") };
            let kind = if is_dir { EntryKind::Dir } else { EntryKind::File };
            let mut info = EntryInfo::minimal(join_vpath(&archive, &child_inner), kind);
            info.name = name;
            if let Some(e) = idx.map(|i| &entries[i]) {
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

/// Index of the entry whose normalized path equals `inner`.
pub fn find_entry(reader: &mut dyn ArchiveReader, inner: &str) -> Option<usize> {
    let inner = normalize_inner(inner);
    reader.entries().ok()?.iter().position(|e| normalize_inner(&e.path) == inner)
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
    }

    fn fake() -> Fake {
        Fake {
            path: PathBuf::from("/t/a.zip"),
            entries: vec![
                entry(0, "readme.md", false, 10),
                entry(1, "src/", true, 0),
                entry(2, "src/main.rs", false, 100),
                entry(3, "docs/guide/intro.md", false, 7), // implicit dirs docs/, docs/guide/
            ],
        }
    }

    #[test]
    fn list_root_and_nested() {
        let mut f = fake();
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

        assert_eq!(find_entry(&mut f, "src/main.rs"), Some(2));
        assert_eq!(find_entry(&mut f, "/src"), Some(1));
        assert_eq!(find_entry(&mut f, "nope"), None);
    }
}

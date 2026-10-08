//! Directory entries and their metadata.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Portable attribute bits; each platform maps its native flags onto these.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct Attributes: u32 {
        const HIDDEN = 1;
        const READONLY = 2;
        const SYSTEM = 4;
        const ARCHIVE = 8;
        const COMPRESSED = 16;
        const ENCRYPTED = 32;
        const SPARSE = 64;
        const REPARSE = 128;
        const OFFLINE = 256;
        const TEMPORARY = 512;
        const EXECUTABLE = 1024;
        const SETUID = 2048;
        const SETGID = 4096;
        const STICKY = 8192;
        const IMMUTABLE = 16384;
        const CLOUD_PLACEHOLDER = 32768;
        const NOT_CONTENT_INDEXED = 65536;
    }
}

/// What kind of filesystem object an entry is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink { target: LinkTarget },
    Junction,
    Device,
    Fifo,
    Socket,
    Unknown,
}

/// The kind of object a symbolic link points to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkTarget {
    File,
    Dir,
    Missing,
    Unknown,
}

bitflags! {
    /// A set of [`EntryKind`] classes, used by filters and search queries.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct EntryKindSet: u8 {
        const FILE = 1;
        const DIR = 2;
        const SYMLINK = 4;
        const JUNCTION = 8;
        const OTHER = 16;
    }
}

impl EntryKindSet {
    /// Whether `k` belongs to one of the classes in this set.
    pub fn contains_kind(&self, k: EntryKind) -> bool {
        let bit = match k {
            EntryKind::File => EntryKindSet::FILE,
            EntryKind::Dir => EntryKindSet::DIR,
            EntryKind::Symlink { .. } => EntryKindSet::SYMLINK,
            EntryKind::Junction => EntryKindSet::JUNCTION,
            EntryKind::Device | EntryKind::Fifo | EntryKind::Socket | EntryKind::Unknown => EntryKindSet::OTHER,
        };
        self.contains(bit)
    }
}

/// Opaque device (volume) identifier: `st_dev` on unix, volume serial on Windows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct DeviceId(pub u64);

/// Identifier of a running search (allocated by `soyuz-search`); never persisted.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct SearchId(pub u64);

/// One directory entry with its metadata. Optional fields are `None` when the
/// platform or the producer (archive listing, search hit) does not provide them.
#[derive(Clone, Debug)]
pub struct EntryInfo {
    pub path: PathBuf,
    /// Lossy display name (last path component).
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub alloc_size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub attrs: Attributes,
    pub mode: Option<u32>,
    pub nlink: Option<u32>,
    pub owner: Option<Arc<str>>,
    pub group: Option<Arc<str>>,
    pub dev: Option<DeviceId>,
    pub inode: Option<u64>,
    pub reparse_tag: Option<u32>,
}

impl EntryInfo {
    /// Directories, junctions and symlinks pointing at directories.
    pub fn is_dir_like(&self) -> bool {
        matches!(self.kind, EntryKind::Dir | EntryKind::Junction | EntryKind::Symlink { target: LinkTarget::Dir })
    }

    /// Extension without the dot (`None` for dot-files like `.bashrc` and for directories).
    pub fn extension(&self) -> Option<&str> {
        if self.is_dir_like() {
            return None;
        }
        name_extension(&self.name)
    }

    /// File name without the extension (the whole name for directories).
    pub fn stem(&self) -> &str {
        match self.extension() {
            Some(ext) => &self.name[..self.name.len() - ext.len() - 1],
            None => &self.name,
        }
    }

    /// Hidden attribute set, or a unix-style dot-file.
    pub fn is_hidden(&self) -> bool {
        self.attrs.contains(Attributes::HIDDEN) || self.name.starts_with('.')
    }

    /// Entry with only `path`, `name` and `kind` filled; every optional field is `None`.
    /// Used by archive listings and synthesized search results.
    pub fn minimal(path: PathBuf, kind: EntryKind) -> Self {
        let name = display_name(&path);
        EntryInfo {
            path,
            name,
            kind,
            size: 0,
            alloc_size: None,
            modified: None,
            created: None,
            accessed: None,
            attrs: Attributes::empty(),
            mode: None,
            nlink: None,
            owner: None,
            group: None,
            dev: None,
            inode: None,
            reparse_tag: None,
        }
    }
}

/// Lossy display name of a path: its last component, or the whole path for roots.
pub fn display_name(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

/// Extension of a file *name* (no dot), `None` for dot-files / names without a dot.
pub(crate) fn name_extension(name: &str) -> Option<&str> {
    let idx = name.rfind('.')?;
    if idx == 0 || idx + 1 == name.len() {
        return None;
    }
    Some(&name[idx + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_and_stem() {
        let e = EntryInfo::minimal(PathBuf::from("/a/b/report.final.PDF"), EntryKind::File);
        assert_eq!(e.name, "report.final.PDF");
        assert_eq!(e.extension(), Some("PDF"));
        assert_eq!(e.stem(), "report.final");

        let dot = EntryInfo::minimal(PathBuf::from("/home/.bashrc"), EntryKind::File);
        assert_eq!(dot.extension(), None);
        assert_eq!(dot.stem(), ".bashrc");
        assert!(dot.is_hidden());

        let dir = EntryInfo::minimal(PathBuf::from("/x/my.dir"), EntryKind::Dir);
        assert_eq!(dir.extension(), None);
        assert_eq!(dir.stem(), "my.dir");
        assert!(dir.is_dir_like());
    }

    #[test]
    fn kind_set() {
        let s = EntryKindSet::FILE | EntryKindSet::SYMLINK;
        assert!(s.contains_kind(EntryKind::File));
        assert!(s.contains_kind(EntryKind::Symlink { target: LinkTarget::Dir }));
        assert!(!s.contains_kind(EntryKind::Dir));
        assert!(!s.contains_kind(EntryKind::Fifo));
    }
}

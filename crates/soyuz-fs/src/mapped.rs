//! Read-only file mappings for large, rarely changing files the app keeps for the whole
//! session (system fonts): the pages are file-backed, so they count towards the working
//! set only while touched and never towards private bytes, and the OS can drop them
//! under memory pressure.
//!
//! * Windows: the file stays open with `FILE_SHARE_READ | FILE_SHARE_DELETE` while
//!   mapped, so nobody can open it for writing (the view can't change under us), while
//!   renaming or deleting it (a font update) still works.
//! * Unix: a mapped file that is truncated in place raises `SIGBUS` on the next read.
//!   [`map_or_read`] therefore maps only files no process of this user can write: owned
//!   by another user (in practice root: `/usr/share/fonts`, where package managers
//!   replace files by renaming a new one over them, which leaves the mapped inode
//!   intact) and not group / world writable. Anything else is read into memory.

use std::fs::File;
use std::io;
use std::path::Path;

use crate::sys::mmap::View;

/// A whole file mapped read-only. Empty files have no mapping (empty slice).
pub struct MappedFile {
    view: Option<View>,
    /// Kept open while mapped: on Windows its share mode keeps writers out.
    _file: File,
}

impl MappedFile {
    /// Map `path` read-only (no policy check: see [`map_or_read`]).
    pub fn open(path: &Path) -> io::Result<MappedFile> {
        let file = open_read(path)?;
        Self::from_file(file)
    }

    fn from_file(file: File) -> io::Result<MappedFile> {
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
        }
        let len = usize::try_from(meta.len()).map_err(|_| io::Error::other("file too large to map"))?;
        let view = if len == 0 { None } else { Some(View::map(&file, len)?) };
        Ok(MappedFile { view, _file: file })
    }

    pub fn len(&self) -> usize {
        self.as_ref().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl AsRef<[u8]> for MappedFile {
    fn as_ref(&self) -> &[u8] {
        self.view.as_ref().map_or(&[], View::bytes)
    }
}

impl std::ops::Deref for MappedFile {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_ref()
    }
}

impl std::fmt::Debug for MappedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedFile").field("len", &self.len()).finish_non_exhaustive()
    }
}

/// A file's bytes: mapped when that is safe (see the module docs), read otherwise.
#[derive(Debug)]
pub enum FileBytes {
    Mapped(MappedFile),
    Read(Vec<u8>),
}

impl FileBytes {
    pub fn is_mapped(&self) -> bool {
        matches!(self, FileBytes::Mapped(_))
    }
}

impl AsRef<[u8]> for FileBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            FileBytes::Mapped(m) => m.as_ref(),
            FileBytes::Read(v) => v,
        }
    }
}

/// The bytes of `path`: a read-only mapping where it can't be truncated under us
/// (Windows always; unix only files this user can't write), the file read whole
/// otherwise.
pub fn map_or_read(path: &Path) -> io::Result<FileBytes> {
    let mut file = open_read(path)?;
    if mappable(&file)? {
        return MappedFile::from_file(file).map(FileBytes::Mapped);
    }
    let mut bytes = Vec::new();
    io::Read::read_to_end(&mut file, &mut bytes)?;
    Ok(FileBytes::Read(bytes))
}

#[cfg(windows)]
fn open_read(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};
    std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE).open(path)
}

#[cfg(not(windows))]
fn open_read(path: &Path) -> io::Result<File> {
    File::open(path)
}

#[cfg(windows)]
fn mappable(file: &File) -> io::Result<bool> {
    Ok(file.metadata()?.is_file())
}

#[cfg(unix)]
fn mappable(file: &File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata()?;
    Ok(meta.is_file() && foreign_read_only(meta.uid(), meta.mode(), rustix::process::geteuid().as_raw()))
}

#[cfg(not(any(windows, unix)))]
fn mappable(_file: &File) -> io::Result<bool> {
    Ok(false)
}

/// Unix: a file owned by `uid` with `mode` can't be written by `euid` (nor by its group
/// or anyone else). Root can write everything, so it never maps.
#[cfg_attr(not(unix), allow(dead_code))]
fn foreign_read_only(uid: u32, mode: u32, euid: u32) -> bool {
    euid != 0 && uid != euid && mode & 0o022 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn maps_contents_and_unmaps_on_drop() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("blob.bin");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let m = MappedFile::open(&p).unwrap();
        assert_eq!(m.len(), data.len());
        assert_eq!(&m[..], &data[..]);
        // Usable as an egui font blob: shared across threads.
        let blob: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(m);
        let b2 = blob.clone();
        let sum = std::thread::spawn(move || {
            let bytes: &[u8] = (*b2).as_ref();
            bytes.iter().map(|b| u64::from(*b)).sum::<u64>()
        })
        .join()
        .unwrap();
        assert_eq!(sum, data.iter().map(|b| u64::from(*b)).sum::<u64>());
        drop(blob);
        // Dropped: the file is free again (Windows: no open handle denies the delete).
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn empty_and_missing_files() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("empty");
        std::fs::write(&p, b"").unwrap();
        let m = MappedFile::open(&p).unwrap();
        assert!(m.is_empty());
        assert!(MappedFile::open(&d.path().join("nope")).is_err());
        assert!(MappedFile::open(d.path()).is_err(), "a folder is not mapped");
        assert_eq!(map_or_read(&p).unwrap().as_ref(), b"");
    }

    #[test]
    fn map_or_read_returns_the_contents() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f.ttf");
        std::fs::write(&p, b"font bytes").unwrap();
        let b = map_or_read(&p).unwrap();
        assert_eq!(b.as_ref(), b"font bytes");
        // Our own temp file: mapped on Windows, read on unix (we could truncate it).
        assert_eq!(b.is_mapped(), cfg!(windows));
    }

    #[cfg(windows)]
    #[test]
    fn mapped_file_keeps_writers_out_but_allows_rename() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f.bin");
        std::fs::write(&p, b"abc").unwrap();
        let m = MappedFile::open(&p).unwrap();
        assert!(std::fs::OpenOptions::new().write(true).open(&p).is_err(), "no writer while mapped");
        let q = d.path().join("g.bin");
        std::fs::rename(&p, &q).unwrap();
        assert_eq!(&m[..], b"abc");
        drop(m);
        std::fs::write(&q, b"xyz").unwrap();
    }

    #[test]
    fn only_foreign_read_only_files_map() {
        assert!(foreign_read_only(0, 0o100644, 1000), "root's system font");
        assert!(!foreign_read_only(1000, 0o100644, 1000), "our own file");
        assert!(!foreign_read_only(0, 0o100664, 1000), "group writable");
        assert!(!foreign_read_only(0, 0o100646, 1000), "world writable");
        assert!(!foreign_read_only(0, 0o100644, 0), "root can write anything");
    }
}

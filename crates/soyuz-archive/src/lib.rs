//! `soyuz-archive`: browse, extract, create and test archives (zip, tar.*, 7z; RAR read
//! behind the `rar` feature).
//!
//! **M0 status:** the full public API (PLAN §2.5) is present; format detection by
//! extension, [`list_dir`], [`find_entry`] and [`total_input_bytes`] work. Opening and
//! creating archives return `Unsupported` until M7.
#![forbid(unsafe_code)]

pub mod format;
pub mod reader;
pub mod vpath;
pub mod writer;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use soyuz_core::{ConflictResolution, Control, ErrorAction, FsError, FsErrorKind, OpControl, Overwrite, ProgressSink};

pub use format::{ArchiveFormat, detect};
pub use vpath::{find_entry, list_dir};

/// Archive errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ArchiveError {
    /// Filesystem error (kind `Io`, `NotFound`, ...).
    #[error(transparent)]
    Io(FsError),
    #[error("{0:?} archives are not supported")]
    Unsupported(ArchiveFormat),
    #[error("corrupt archive: {0}")]
    Corrupt(String),
    #[error("password required")]
    PasswordRequired,
    #[error("wrong password")]
    WrongPassword,
    #[error("entry escapes the destination: {0}")]
    ZipSlip(String),
    #[error("cancelled")]
    Cancelled,
    /// The sink answered a conflict with `Cancel`.
    #[error("conflict at {0}")]
    Conflict(PathBuf),
}

impl From<std::io::Error> for ArchiveError {
    fn from(e: std::io::Error) -> Self {
        ArchiveError::Io(FsError::from_io("archive", None, &e))
    }
}

impl From<FsError> for ArchiveError {
    fn from(e: FsError) -> Self {
        if e.is_cancel() { ArchiveError::Cancelled } else { ArchiveError::Io(e) }
    }
}

/// One entry of an archive.
#[derive(Clone, Debug, PartialEq)]
pub struct ArchiveEntry {
    pub index: usize,
    /// `/`-separated, no leading `/`, normalized (no `..`).
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub compressed: u64,
    pub modified: Option<SystemTime>,
    pub crc32: Option<u32>,
    pub encrypted: bool,
    pub mode: Option<u32>,
    pub is_symlink: bool,
    pub link_target: Option<String>,
}

/// An opened archive.
pub trait ArchiveReader: Send {
    fn format(&self) -> ArchiveFormat;
    fn path(&self) -> &Path;
    /// Cached after the first call.
    fn entries(&mut self) -> Result<&[ArchiveEntry], ArchiveError>;
    /// 7z solid blocks: may decode preceding entries (documented cost).
    fn open_entry(&mut self, index: usize) -> Result<Box<dyn Read + Send + '_>, ArchiveError>;
    fn extract(
        &mut self,
        selection: Option<&[usize]>,
        dest: &Path,
        opts: &ExtractOptions,
        ctl: &OpControl,
        sink: &mut dyn ExtractSink,
    ) -> Result<ExtractSummary, ArchiveError>;
    fn test(&mut self, ctl: &OpControl, sink: &mut dyn ProgressSink) -> Result<TestReport, ArchiveError>;
}

#[derive(Clone, Debug, Default)]
pub struct ExtractOptions {
    pub strip_components: usize,
    pub overwrite: Overwrite,
    pub preserve_mode: bool,
    pub preserve_times: bool,
    /// "Extract to folder" smart mode: avoid a redundant top-level folder.
    pub single_top_folder: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtractSummary {
    pub entries_ok: u64,
    pub entries_skipped: u64,
    pub entries_failed: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestReport {
    pub entries: u64,
    /// `(entry path, problem)`.
    pub bad: Vec<(String, String)>,
}

/// Callbacks during extraction (jobs implements this to drive prompts and progress).
pub trait ExtractSink: Send {
    fn on_entry(&mut self, e: &ArchiveEntry, dst: &Path) -> Control;
    fn on_bytes(&mut self, n: u64);
    fn on_conflict(&mut self, dst: &Path, e: &ArchiveEntry) -> ConflictResolution;
    fn on_error(&mut self, e: &ArchiveEntry, err: &ArchiveError) -> ErrorAction;
}

/// Open an archive, detecting its format.
pub fn open(path: &Path, password: Option<&str>) -> Result<Box<dyn ArchiveReader>, ArchiveError> {
    match detect(path) {
        Some(format) => open_with_format(path, format, password),
        None => Err(ArchiveError::Io(FsError::with_message(
            FsErrorKind::Unsupported,
            "open_archive",
            Some(path),
            "unknown archive format",
        ))),
    }
}

/// Open an archive with a known format.
pub fn open_with_format(
    path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<Box<dyn ArchiveReader>, ArchiveError> {
    reader::open(path, format, password)
}

/// Options for [`create`].
#[derive(Clone, Debug)]
pub struct CreateOptions {
    pub format: ArchiveFormat,
    /// 0-9, mapped per codec.
    pub level: u8,
    pub password: Option<String>,
    /// Entry paths are relative to this.
    pub base_dir: PathBuf,
    pub store_symlinks: bool,
    pub follow_symlinks: bool,
}

/// Writes to `"<dest>.part"` then renames; removes `.part` on error / cancel.
pub fn create(
    dest: &Path,
    inputs: &[PathBuf],
    opts: &CreateOptions,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<(), ArchiveError> {
    writer::create(dest, inputs, opts, ctl, sink)
}

/// Total bytes of regular files under `inputs` (symlinks not followed), for jobs' totals.
pub fn total_input_bytes(inputs: &[PathBuf], ctl: &OpControl) -> Result<u64, ArchiveError> {
    let mut total = 0u64;
    let mut stack: Vec<PathBuf> = inputs.to_vec();
    while let Some(p) = stack.pop() {
        ctl.checkpoint()?;
        let md =
            std::fs::symlink_metadata(&p).map_err(|e| ArchiveError::Io(FsError::from_io("archive", Some(&p), &e)))?;
        if md.is_dir() {
            for de in std::fs::read_dir(&p).map_err(|e| ArchiveError::Io(FsError::from_io("archive", Some(&p), &e)))? {
                stack.push(de?.path());
            }
        } else if md.is_file() {
            total += md.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_bytes() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("x")).unwrap();
        std::fs::write(d.path().join("x/a"), [0u8; 10]).unwrap();
        std::fs::write(d.path().join("b"), [0u8; 5]).unwrap();
        let ctl = OpControl::new();
        assert_eq!(total_input_bytes(&[d.path().to_path_buf()], &ctl).unwrap(), 15);
        ctl.cancel();
        assert_eq!(total_input_bytes(&[d.path().to_path_buf()], &ctl), Err(ArchiveError::Cancelled));
    }

    #[test]
    fn open_unknown_and_unimplemented() {
        assert!(matches!(open(Path::new("/x/readme.txt"), None), Err(ArchiveError::Io(_))));
        assert!(matches!(open(Path::new("/x/a.zip"), None), Err(ArchiveError::Unsupported(ArchiveFormat::Zip))));
    }
}

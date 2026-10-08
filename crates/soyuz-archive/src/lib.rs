//! `soyuz-archive`: browse, extract, create and test archives (zip, tar.*, 7z; read-only
//! RAR, ISO / UDF, CAB, deb and rpm).
//!
//! Formats: zip (stored / deflate / bzip2 / lzma / xz read; store / deflate
//! write; ZipCrypto + AES read, AES-256 write; add / delete / rename entries through
//! [`rewrite_zip`]), tar, tar.gz, tar.bz2, tar.xz, tar.zst, 7z (read + write, AES-256 with
//! header encryption). Byte-split volumes (`x.7z.001`, `x.zip.001`, …) are read as one
//! archive. Read-only: RAR 1.3 – 7 (feature `rar`, on by default, pure-Rust `rars`
//! decoder), ISO 9660 / Joliet / Rock Ridge / UDF disc images, Microsoft CAB (stored /
//! MSZIP / LZX), `.deb` (the `data.tar.*` tree) and `.rpm` (the cpio payload). The default
//! build is pure Rust (tar.zst is read with `ruzstd`). The opt-in `zstd` feature links
//! libzstd (`zstd-sys`, needs a C compiler) and adds tar.zst writing plus zstd-compressed
//! zip entries (read + write).
//!
//! Safety: extraction rejects absolute paths, `..` escapes and symlinks pointing outside
//! the destination (`ArchiveError::ZipSlip`), never writes through a symlink, validates
//! every path segment (Windows reserved names on Windows), writes each file to a temp
//! name first and streams data in 256 KiB chunks (no entry is ever fully buffered).
//!
//! Cost notes: zip and uncompressed tar give O(1) random access; compressed tar has to
//! decompress up to the entry for [`ArchiveReader::open_entry`], and listing it decodes the
//! whole stream once. 7z solid blocks decode the preceding entries of the same block.
#![forbid(unsafe_code)]

pub mod edit;
mod extract;
pub mod format;
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzzing;
pub mod reader;
mod util;
pub mod vpath;
pub mod writer;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use soyuz_core::{ConflictResolution, Control, ErrorAction, FsError, FsErrorKind, OpControl, Overwrite, ProgressSink};

pub use edit::{ZipEdit, ZipEditSummary, can_edit, edit_temp_path, rewrite_zip};
pub use format::{ArchiveFormat, detect};
pub use vpath::{DirChild, DirIndex, find_entry, list_dir};

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
    /// A decompression-bomb limit ([`ExtractLimits`]) was exceeded. During extraction it
    /// is reported through [`ExtractSink::on_error`]: `Abort` stops, any other answer
    /// lifts that limit for the rest of the extraction.
    #[error("archive expands beyond safety limit: {0}")]
    LimitExceeded(String),
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
    /// Cached directory index (additive, see PLAN §10). The built-in readers build it on
    /// first use; the default returns `None` and [`list_dir`] falls back to a full scan.
    fn dir_index(&mut self) -> Result<Option<&DirIndex>, ArchiveError> {
        Ok(None)
    }
}

/// Zip compression method for [`create_with`] (additive, see PLAN §10). `level == 0`
/// always stores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZipMethod {
    #[default]
    Deflate,
    Zstd,
    Store,
}

/// Extra knobs for [`create_with`] that do not fit the frozen [`CreateOptions`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateExtras {
    pub zip_method: ZipMethod,
    /// 7z with a password: also encrypt the header (file names). Default `true`.
    pub encrypt_7z_headers: bool,
}

impl Default for CreateExtras {
    fn default() -> Self {
        CreateExtras { zip_method: ZipMethod::Deflate, encrypt_7z_headers: true }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExtractOptions {
    pub strip_components: usize,
    pub overwrite: Overwrite,
    pub preserve_mode: bool,
    pub preserve_times: bool,
    /// "Extract to folder" smart mode: avoid a redundant top-level folder.
    pub single_top_folder: bool,
    /// With `preserve_mode`: keep setuid / setgid / sticky bits (unix). Default `false`:
    /// modes are masked to `0o777`.
    pub preserve_special_bits: bool,
    /// Decompression-bomb limits (on by default).
    pub limits: ExtractLimits,
}

/// Decompression-bomb limits. A `0` turns that limit off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractLimits {
    /// Total extracted bytes may reach this multiple of the archive's on-disk size ...
    pub max_ratio: u64,
    /// ... but the total limit is never below this many bytes.
    pub min_total_bytes: u64,
    /// Most entries one extraction may write.
    pub max_entries: u64,
    /// Largest single entry in bytes.
    pub max_entry_bytes: u64,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        ExtractLimits { max_ratio: 100, min_total_bytes: 1 << 30, max_entries: 1_000_000, max_entry_bytes: 32 << 30 }
    }
}

impl ExtractLimits {
    /// No limits at all.
    pub fn unlimited() -> Self {
        ExtractLimits { max_ratio: 0, min_total_bytes: 0, max_entries: 0, max_entry_bytes: 0 }
    }

    /// Total-bytes limit for an archive of `archive_size` bytes (`None` = unlimited).
    pub fn total_cap(&self, archive_size: u64) -> Option<u64> {
        (self.max_ratio > 0).then(|| archive_size.saturating_mul(self.max_ratio).max(self.min_total_bytes))
    }

    /// Single-entry limit (`None` = unlimited).
    pub fn entry_cap(&self) -> Option<u64> {
        (self.max_entry_bytes > 0).then_some(self.max_entry_bytes)
    }

    /// Entry-count limit (`None` = unlimited).
    pub fn entries_cap(&self) -> Option<u64> {
        (self.max_entries > 0).then_some(self.max_entries)
    }

    /// Hard cap for reading one entry outside an extraction job (open / preview): the
    /// smaller of the total and single-entry limits.
    pub fn read_cap(&self, archive_size: u64) -> Option<u64> {
        match (self.total_cap(archive_size), self.entry_cap()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// Reader that fails with [`ArchiveError::LimitExceeded`] (as an `io::Error`) once more
/// than `cap` bytes were read.
struct CappedRead<R> {
    inner: R,
    left: u64,
    cap: u64,
}

impl<R: Read> Read for CappedRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // Read one byte past the cap so an entry of exactly `cap` bytes still succeeds.
        let want = buf.len().min(usize::try_from(self.left.saturating_add(1)).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..want])?;
        if n as u64 > self.left {
            return Err(std::io::Error::other(ArchiveError::LimitExceeded(format!(
                "entry is larger than {} bytes",
                self.cap
            ))));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

/// [`ArchiveReader::open_entry`] with a hard size cap: refuses an entry whose declared
/// size exceeds `cap`, and the returned reader fails once more than `cap` bytes come out
/// (declared sizes can lie). Use [`ExtractLimits::read_cap`] for `cap`.
pub fn open_entry_capped(
    reader: &mut dyn ArchiveReader,
    index: usize,
    cap: u64,
) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
    let declared = reader.entries()?.get(index).map(|e| e.size);
    if let Some(size) = declared
        && size > cap
    {
        return Err(ArchiveError::LimitExceeded(format!("entry is {size} bytes, the limit is {cap}")));
    }
    let inner = reader.open_entry(index)?;
    Ok(Box::new(CappedRead { inner, left: cap, cap }))
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
    writer::create(dest, inputs, opts, &CreateExtras::default(), ctl, sink)
}

/// [`create`] with [`CreateExtras`] (zip method, 7z header encryption).
pub fn create_with(
    dest: &Path,
    inputs: &[PathBuf],
    opts: &CreateOptions,
    extras: &CreateExtras,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<(), ArchiveError> {
    writer::create(dest, inputs, opts, extras, ctl, sink)
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
    fn capped_read_fails_past_the_cap() {
        let mut ok = CappedRead { inner: &[7u8; 10][..], left: 10, cap: 10 };
        let mut v = Vec::new();
        ok.read_to_end(&mut v).unwrap();
        assert_eq!(v.len(), 10);
        // A reader producing more than its (possibly lying) declared size is cut off.
        let mut over = CappedRead { inner: std::io::repeat(0), left: 1000, cap: 1000 };
        let err = std::io::copy(&mut over, &mut std::io::sink()).unwrap_err();
        assert!(err.to_string().contains("safety limit"), "{err}");
    }

    #[test]
    fn open_unknown_and_unimplemented() {
        assert!(matches!(open(Path::new("/x/readme.txt"), None), Err(ArchiveError::Io(_))));
        let missing = open(Path::new("/definitely/missing/a.zip"), None).err();
        assert!(matches!(missing, Some(ArchiveError::Io(ref e)) if e.kind == FsErrorKind::NotFound), "{missing:?}");
    }
}

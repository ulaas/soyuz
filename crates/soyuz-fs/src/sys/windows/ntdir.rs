//! Directory enumeration with `NtQueryDirectoryFile` (ntdll) and large buffers.
//!
//! `FindFirstFileExW(FIND_FIRST_EX_LARGE_FETCH)` asks the file system for 64 KiB of entries per
//! call and copies every entry into a `WIN32_FIND_DATAW`. Here we query
//! `FileIdFullDirectoryInformation` straight into a reusable buffer that grows to 256 KiB
//! after the first full batch (so a 100k-entry folder needs a few hundred kernel calls, not
//! thousands), and parse the records in place. The record also carries the file id (the
//! same value `GetFileInformationByHandle` reports as the file index), so listings get an
//! inode for free.
//!
//! Like `FindFirstFileExW`, the reparse tag of a reparse point is read from `EaSize` (the
//! file system stores the tag there for directory queries; `FindFirstFileExW` copies the
//! same field into `dwReserved0`).
//!
//! If the information class is refused (some redirectors / third-party file systems), the
//! caller falls back to the `FindFirstFileExW` iterator.

use std::ffi::OsString;
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStringExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Wdk::Storage::FileSystem::{
    FILE_ID_FULL_DIR_INFORMATION, FileIdFullDirectoryInformation, NtQueryDirectoryFile,
};
use windows_sys::Win32::Foundation::{NTSTATUS, RtlNtStatusToDosError};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

use super::Handle;
use super::fs::{FindEntry, RawStat, open};

const STATUS_NO_MORE_FILES: NTSTATUS = 0x8000_0006_u32 as i32;
const STATUS_NO_SUCH_FILE: NTSTATUS = 0xC000_000F_u32 as i32;
const STATUS_INVALID_PARAMETER: NTSTATUS = 0xC000_000D_u32 as i32;
const STATUS_BUFFER_OVERFLOW: NTSTATUS = 0x8000_0005_u32 as i32;

/// First query size (what `FIND_FIRST_EX_LARGE_FETCH` uses; safe for SMB redirectors).
const SMALL_BUF: usize = 64 * 1024;
/// Size used once a folder proved to be big (the first batch filled most of the buffer).
const LARGE_BUF: usize = 256 * 1024;

/// Why opening the fast enumerator failed.
pub(crate) enum OpenError {
    /// The directory itself could not be opened / read (a real error to report).
    Io(io::Error),
    /// The file system refused the information class: use `FindFirstFileExW` instead.
    Unsupported,
}

pub(crate) struct NtDirIter {
    handle: Handle,
    /// 8-byte aligned buffer (the records are 8-byte aligned).
    buf: Vec<u64>,
    /// Bytes filled by the last query.
    filled: usize,
    /// Byte offset of the next record in `buf`, `None` when the batch is exhausted.
    next: Option<usize>,
    done: bool,
    /// Switch to [`LARGE_BUF`] before the next query.
    grow_next: bool,
}

fn nt_err(status: NTSTATUS) -> io::Error {
    // SAFETY: pure lookup in ntdll, no pointers involved.
    let code = unsafe { RtlNtStatusToDosError(status) };
    io::Error::from_raw_os_error(code as i32)
}

impl NtDirIter {
    /// Query the next batch. Returns `Ok(false)` at the end of the directory.
    fn query(&mut self, restart: bool) -> Result<bool, NTSTATUS> {
        if std::mem::take(&mut self.grow_next) {
            self.buf = vec![0; LARGE_BUF / size_of::<u64>()];
        }
        loop {
            // SAFETY: plain-old-data struct, all-zero is valid.
            let mut iosb: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
            let len = self.buf.len() * size_of::<u64>();
            // SAFETY: `handle` is an open synchronous directory handle (CreateFileW without
            // FILE_FLAG_OVERLAPPED), so the call completes before returning; `buf` is a
            // writable buffer of `len` bytes that outlives the call; no event/APC/filter.
            let status = unsafe {
                NtQueryDirectoryFile(
                    self.handle.raw(),
                    null_mut(),
                    None,
                    null(),
                    &mut iosb,
                    self.buf.as_mut_ptr().cast(),
                    len as u32,
                    FileIdFullDirectoryInformation,
                    false,
                    null(),
                    restart,
                )
            };
            match status {
                s if s >= 0 => {
                    self.filled = iosb.Information.min(len);
                    self.next = (self.filled > 0).then_some(0);
                    // A big folder: use the large buffer from the next batch on (this
                    // batch's records are still being parsed out of the current one).
                    if len == SMALL_BUF && self.filled > SMALL_BUF / 2 {
                        self.grow_next = true;
                    }
                    return Ok(self.filled > 0);
                }
                STATUS_NO_MORE_FILES | STATUS_NO_SUCH_FILE => return Ok(false),
                STATUS_INVALID_PARAMETER if self.buf.len() * size_of::<u64>() > SMALL_BUF => {
                    // Some redirectors cap the transfer size: retry with the small buffer.
                    self.buf = vec![0; SMALL_BUF / size_of::<u64>()];
                }
                s => return Err(s),
            }
        }
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: reinterpreting an initialized `[u64]` as bytes of the same total length.
        unsafe { std::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>(), self.buf.len() * size_of::<u64>()) }
    }
}

/// Open `dir` for enumeration and fetch the first batch.
pub(crate) fn read_dir(dir: &Path) -> Result<NtDirIter, OpenError> {
    let handle = open(dir, FILE_LIST_DIRECTORY | SYNCHRONIZE, FILE_FLAG_BACKUP_SEMANTICS).map_err(OpenError::Io)?;
    let mut it = NtDirIter {
        handle,
        buf: vec![0; SMALL_BUF / size_of::<u64>()],
        filled: 0,
        next: None,
        done: false,
        grow_next: false,
    };
    match it.first_query() {
        Ok(()) => Ok(it),
        Err(s) if matches!(s as u32, 0xC000_0003 | 0xC000_00BB | 0xC000_000D | 0xC000_0010) => {
            // STATUS_INVALID_INFO_CLASS / NOT_SUPPORTED / INVALID_PARAMETER / INVALID_DEVICE_REQUEST
            Err(OpenError::Unsupported)
        }
        Err(s) => Err(OpenError::Io(nt_err(s))),
    }
}

impl NtDirIter {
    fn first_query(&mut self) -> Result<(), NTSTATUS> {
        if !self.query(true)? {
            self.done = true;
        }
        Ok(())
    }

    fn next_query(&mut self) -> Result<bool, NTSTATUS> {
        self.query(false)
    }
}

/// Parse one record at byte offset `off`. Returns the entry and the next offset.
fn parse(bytes: &[u8], off: usize, filled: usize) -> Option<(FindEntry, Option<usize>)> {
    let name_off = offset_of!(FILE_ID_FULL_DIR_INFORMATION, FileName);
    if off + name_off > filled {
        return None;
    }
    // SAFETY: `off + size_of` header bytes are inside the filled buffer (checked above for
    // the fixed part up to `FileName`); `read_unaligned` copes with any alignment.
    let rec: FILE_ID_FULL_DIR_INFORMATION = unsafe {
        let mut tmp = std::mem::MaybeUninit::<FILE_ID_FULL_DIR_INFORMATION>::zeroed();
        std::ptr::copy_nonoverlapping(bytes.as_ptr().add(off), tmp.as_mut_ptr().cast::<u8>(), name_off);
        tmp.assume_init()
    };
    let name_len = rec.FileNameLength as usize;
    let start = off + name_off;
    if start + name_len > filled {
        return None;
    }
    let name_bytes = &bytes[start..start + name_len];
    let name = if name_bytes.as_ptr().cast::<u16>().is_aligned() {
        // SAFETY: the pointer is 2-byte aligned (checked) and `name_len / 2` u16s lie inside
        // `name_bytes`, which borrows initialized memory for the slice's lifetime.
        OsString::from_wide(unsafe { std::slice::from_raw_parts(name_bytes.as_ptr().cast::<u16>(), name_len / 2) })
    } else {
        let units: Vec<u16> = name_bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        OsString::from_wide(&units)
    };
    let attrs = rec.FileAttributes;
    let stat = RawStat {
        attrs,
        created: rec.CreationTime as u64,
        accessed: rec.LastAccessTime as u64,
        written: rec.LastWriteTime as u64,
        size: rec.EndOfFile.max(0) as u64,
        alloc: Some(rec.AllocationSize.max(0) as u64),
        nlink: None,
        volume: None,
        file_index: (rec.FileId != 0).then_some(rec.FileId as u64),
        reparse_tag: if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 { rec.EaSize } else { 0 },
    };
    let next = (rec.NextEntryOffset != 0).then(|| off + rec.NextEntryOffset as usize);
    Some((FindEntry { name, stat }, next))
}

impl Iterator for NtDirIter {
    type Item = io::Result<FindEntry>;

    fn next(&mut self) -> Option<io::Result<FindEntry>> {
        loop {
            if self.done {
                return None;
            }
            let Some(off) = self.next else {
                match self.next_query() {
                    Ok(true) => continue,
                    Ok(false) => {
                        self.done = true;
                        return None;
                    }
                    Err(STATUS_BUFFER_OVERFLOW) => {
                        // A single name longer than the buffer cannot happen (names are
                        // ≤ 255 UTF-16 units); treat as the end to avoid spinning.
                        self.done = true;
                        return None;
                    }
                    Err(s) => {
                        self.done = true;
                        return Some(Err(nt_err(s)));
                    }
                }
            };
            let Some((entry, next)) = parse(self.bytes(), off, self.filled) else {
                self.next = None;
                continue;
            };
            self.next = next;
            if entry.name == "." || entry.name == ".." {
                continue;
            }
            return Some(Ok(entry));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn collect_nt(dir: &Path) -> BTreeMap<OsString, RawStat> {
        let it = match read_dir(dir) {
            Ok(it) => it,
            Err(OpenError::Io(e)) => panic!("open: {e}"),
            Err(OpenError::Unsupported) => panic!("NtQueryDirectoryFile unsupported on the temp volume"),
        };
        it.map(|e| e.unwrap()).map(|e| (e.name, e.stat)).collect()
    }

    fn collect_find(dir: &Path) -> BTreeMap<OsString, RawStat> {
        super::super::fs::read_dir(dir).unwrap().map(|e| e.unwrap()).map(|e| (e.name, e.stat)).collect()
    }

    fn assert_same(dir: &Path) {
        let nt = collect_nt(dir);
        let find = collect_find(dir);
        assert_eq!(nt.keys().collect::<Vec<_>>(), find.keys().collect::<Vec<_>>());
        for (name, a) in &nt {
            let b = &find[name];
            assert_eq!(a.attrs, b.attrs, "{name:?}");
            assert_eq!(a.size, b.size, "{name:?}");
            assert_eq!(a.written, b.written, "{name:?}");
            assert_eq!(a.created, b.created, "{name:?}");
            assert_eq!(a.reparse_tag, b.reparse_tag, "{name:?}");
        }
    }

    #[test]
    fn matches_find_first_file() {
        let d = tempfile::tempdir().unwrap();
        for n in ["a.txt", "日本語.txt", "emoji 🚀", "e\u{301}cole", "trailing.dot."] {
            let p = d.path().join(n);
            if std::fs::write(&p, n.as_bytes()).is_err() {
                // Trailing dots need the `\\?\` form; std may refuse it.
                continue;
            }
        }
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("big.bin"), vec![1u8; 70_000]).unwrap();
        // Symlinks need privilege / developer mode; a junction never does.
        let _ = std::os::windows::fs::symlink_file(d.path().join("a.txt"), d.path().join("link"));
        let _ = crate::links::junction(&d.path().join("sub"), &d.path().join("junc"));
        assert_same(d.path());
        let nt = collect_nt(d.path());
        if let Some(j) = nt.get(&OsString::from("junc")) {
            assert_eq!(j.reparse_tag, crate::sys::reparse::IO_REPARSE_TAG_MOUNT_POINT);
        }
        assert!(nt.values().all(|s| s.file_index.is_some()));
    }

    #[test]
    fn empty_and_many() {
        let d = tempfile::tempdir().unwrap();
        assert!(collect_nt(d.path()).is_empty());
        // Enough entries to need several batches and the large buffer.
        for i in 0..3000 {
            std::fs::write(d.path().join(format!("file-with-a-longish-name-{i:05}.dat")), b"").unwrap();
        }
        let nt = collect_nt(d.path());
        assert_eq!(nt.len(), 3000);
        assert_same(d.path());
    }

    #[test]
    fn missing_dir_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        match read_dir(&d.path().join("nope")) {
            Err(OpenError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::NotFound),
            _ => panic!("expected NotFound"),
        }
    }
}

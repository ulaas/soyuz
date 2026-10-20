//! NTFS change journal (USN) access over a volume handle: `FSCTL_QUERY_USN_JOURNAL`,
//! `FSCTL_READ_USN_JOURNAL`, `FSCTL_ENUM_USN_DATA` (kernel32 `DeviceIoControl`), plus
//! file-id → path resolution (`OpenFileById` + `GetFinalPathNameByHandleW`) and the
//! low thread priority used by the indexer. Buffers are returned raw; the record
//! parser lives in `soyuz-index`.

use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_HANDLE_EOF, GENERIC_READ};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_NAME_NORMALIZED,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdType, GetFinalPathNameByHandleW,
    GetVolumeNameForVolumeMountPointW, OPEN_EXISTING, OpenFileById, VOLUME_NAME_DOS,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
    MFT_ENUM_DATA_V0, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0,
};
use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST};

use super::{Handle, from_wide_nul, last_err, wide_str};

/// Raw `USN_JOURNAL_DATA_V0` fields.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RawJournal {
    pub id: u64,
    pub first_usn: i64,
    pub next_usn: i64,
    pub lowest_valid_usn: i64,
    pub max_usn: i64,
}

/// An open volume: the device (`\\.\C:`, administrator rights on most systems) or, without
/// them, the volume's root folder, which still answers `FSCTL_QUERY_USN_JOURNAL` and
/// `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` (Windows 11; records of files the user can't
/// see are left out) but not the MFT enumeration.
#[derive(Debug)]
pub(crate) struct VolumeHandle {
    h: Handle,
    privileged: bool,
}

/// `\\.\C:` for a drive root, else the `\\?\Volume{…}` name of a mount point (without
/// the trailing backslash, which would open the root directory instead of the volume).
fn device_path(root: &Path) -> io::Result<Vec<u16>> {
    let plain = crate::path::strip_verbatim(root);
    let s = plain.to_string_lossy();
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() && (b.len() == 2 || s[2..] == *"\\") {
        return Ok(wide_str(format!(r"\\.\{}:", b[0] as char)));
    }
    let mut mount = s.into_owned();
    if !mount.ends_with('\\') {
        mount.push('\\');
    }
    let w = wide_str(&mount);
    let mut out = vec![0u16; 64];
    // SAFETY: `w` is NUL-terminated; `out` has room for `out.len()` UTF-16 units.
    let ok = unsafe { GetVolumeNameForVolumeMountPointW(w.as_ptr(), out.as_mut_ptr(), out.len() as u32) };
    if ok == 0 {
        return Err(last_err());
    }
    let mut name = from_wide_nul(&out).to_string_lossy().into_owned();
    if name.ends_with('\\') {
        name.pop();
    }
    Ok(wide_str(name))
}

impl VolumeHandle {
    pub(crate) fn open(root: &Path) -> io::Result<VolumeHandle> {
        let dev = device_path(root)?;
        // SAFETY: `dev` is NUL-terminated; no security attributes or template handle.
        let h = unsafe {
            CreateFileW(
                dev.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        match Handle::new(h) {
            Ok(h) => Ok(VolumeHandle { h, privileged: true }),
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => Self::open_unprivileged(root),
            Err(e) => Err(e),
        }
    }

    /// The volume's root folder (no administrator rights needed).
    fn open_unprivileged(root: &Path) -> io::Result<VolumeHandle> {
        let mut dir = crate::path::strip_verbatim(root).to_string_lossy().into_owned();
        if !dir.ends_with('\\') {
            dir.push('\\');
        }
        let w = wide_str(&dir);
        // SAFETY: `w` is NUL-terminated; a directory handle (backup semantics) with
        // attribute access only.
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        Ok(VolumeHandle { h: Handle::new(h)?, privileged: false })
    }

    /// Opened as the device (administrator): the MFT can be enumerated.
    pub(crate) fn privileged(&self) -> bool {
        self.privileged
    }

    /// `DeviceIoControl` with typed input and a byte output buffer; returns bytes written.
    fn ioctl<I>(&self, code: u32, input: Option<&I>, out: &mut [u8]) -> io::Result<usize> {
        let mut returned = 0u32;
        let (ip, il) = match input {
            Some(i) => (i as *const I as *const c_void, size_of::<I>() as u32),
            None => (null(), 0),
        };
        // SAFETY: `ip`/`il` describe a live `I` (or nothing); `out` is a writable buffer of
        // `out.len()` bytes; synchronous call (no OVERLAPPED).
        let ok = unsafe {
            DeviceIoControl(
                self.h.raw(),
                code,
                ip,
                il,
                out.as_mut_ptr() as *mut c_void,
                out.len().min(u32::MAX as usize) as u32,
                &mut returned,
                null_mut(),
            )
        };
        if ok == 0 { Err(last_err()) } else { Ok(returned as usize) }
    }

    pub(crate) fn query(&self) -> io::Result<RawJournal> {
        let mut buf = [0u8; size_of::<USN_JOURNAL_DATA_V0>() + 16];
        let n = self.ioctl::<()>(FSCTL_QUERY_USN_JOURNAL, None, &mut buf)?;
        if n < size_of::<USN_JOURNAL_DATA_V0>() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "short USN_JOURNAL_DATA"));
        }
        let u = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap_or([0; 8]));
        Ok(RawJournal {
            id: u(0),
            first_usn: u(8) as i64,
            next_usn: u(16) as i64,
            lowest_valid_usn: u(24) as i64,
            max_usn: u(32) as i64,
        })
    }

    /// Read journal records from `start_usn`; returns the bytes in `out` (an 8-byte next
    /// USN followed by `USN_RECORD_V2`s). Doesn't wait for new records.
    pub(crate) fn read(&self, journal_id: u64, start_usn: i64, out: &mut [u8]) -> io::Result<usize> {
        let input = READ_USN_JOURNAL_DATA_V0 {
            StartUsn: start_usn,
            ReasonMask: u32::MAX,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: journal_id,
        };
        let code = if self.privileged { FSCTL_READ_USN_JOURNAL } else { FSCTL_READ_UNPRIVILEGED_USN_JOURNAL };
        self.ioctl(code, Some(&input), out)
    }

    /// Enumerate the MFT from `start_frn`; `Ok(None)` at the end. Output: 8-byte next
    /// file reference number followed by `USN_RECORD_V2`s.
    pub(crate) fn enum_mft(&self, start_frn: u64, out: &mut [u8]) -> io::Result<Option<usize>> {
        if !self.privileged {
            return Err(io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32));
        }
        let input = MFT_ENUM_DATA_V0 { StartFileReferenceNumber: start_frn, LowUsn: 0, HighUsn: i64::MAX };
        match self.ioctl(FSCTL_ENUM_USN_DATA, Some(&input), out) {
            Ok(n) => Ok(Some(n)),
            Err(e) if e.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Current path of the file with this id (DOS form, verbatim prefix removed).
    pub(crate) fn path_of(&self, frn: u64) -> io::Result<PathBuf> {
        let desc = FILE_ID_DESCRIPTOR {
            dwSize: size_of::<FILE_ID_DESCRIPTOR>() as u32,
            Type: FileIdType,
            Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: frn as i64 },
        };
        // SAFETY: `desc` is a valid descriptor; the volume handle is a valid hint.
        let h = unsafe {
            OpenFileById(
                self.h.raw(),
                &desc,
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                FILE_FLAG_BACKUP_SEMANTICS,
            )
        };
        let file = Handle::new(h)?;
        let mut buf = vec![0u16; 512];
        loop {
            // SAFETY: `buf` has room for `buf.len()` UTF-16 units.
            let n = unsafe {
                GetFinalPathNameByHandleW(
                    file.raw(),
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            } as usize;
            if n == 0 {
                return Err(last_err());
            }
            if n < buf.len() {
                let p = PathBuf::from(from_wide_nul(&buf[..n]));
                return Ok(crate::path::strip_verbatim(&p));
            }
            buf.resize(n + 1, 0);
        }
    }
}

/// Lowest CPU priority for the calling thread. Returns whether the call succeeded.
pub(crate) fn lowest_priority() -> bool {
    // SAFETY: pseudo-handle of the current thread; no other arguments.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST) != 0 }
}

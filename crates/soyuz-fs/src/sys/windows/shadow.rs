//! Volume Shadow Copy discovery (F8 "Previous versions") without VSS COM or the shell.
//!
//! * Listing: the kernel object directory `\Device` is enumerated with
//!   `NtOpenDirectoryObject` + `NtQueryDirectoryObject` (ntdll); shadow copies appear there
//!   as `HarddiskVolumeShadowCopyN` device objects. Querying `\Device` works without
//!   elevation on current Windows versions.
//! * Each device is then opened as a directory through the Win32 kernel namespace
//!   `\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopyN\` (`CreateFileW` with
//!   `FILE_FLAG_BACKUP_SEMANTICS`). Opening needs administrator rights on a default
//!   installation (the device ACL grants SYSTEM and Administrators only), so
//!   `ERROR_ACCESS_DENIED` is reported as [`Probe::Denied`] and explained in the UI.
//! * Mapping to the original volume: a shadow copy reports the same volume serial number
//!   as the volume it was taken of (`GetVolumeInformationByHandleW`).
//! * Time: `NtQueryVolumeInformationFile(FileFsVolumeInformation).VolumeCreationTime` of
//!   the snapshot device; the caller treats it as unknown when it equals the live volume's
//!   value (then the snapshot is just numbered).

use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::SystemTime;

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FileFsVolumeInformation, NtOpenDirectoryObject, NtQueryDirectoryObject, NtQueryVolumeInformationFile,
};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_BAD_PATHNAME, ERROR_FILE_NOT_FOUND, ERROR_INVALID_NAME, ERROR_PATH_NOT_FOUND, HANDLE,
    NTSTATUS, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

use super::{Handle, ticks_to_system, wide_str};

const DIRECTORY_QUERY: u32 = 0x0001;
const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
const STATUS_MORE_ENTRIES: NTSTATUS = 0x0000_0105;
const STATUS_NO_MORE_ENTRIES: NTSTATUS = 0x8000_001A_u32 as i32;

#[repr(C)]
struct ObjectDirectoryInformation {
    name: UNICODE_STRING,
    type_name: UNICODE_STRING,
}

/// Result of opening one shadow copy device.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Probe {
    /// Opened: volume serial and the reported creation time.
    Present { serial: u32, created: Option<SystemTime> },
    /// The device exists but this process may not open it (not elevated).
    Denied,
    /// No such device.
    Missing,
}

fn unicode_string(buf: &[u16]) -> UNICODE_STRING {
    // `buf` without its terminating NUL; lengths are in bytes.
    let chars = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    UNICODE_STRING { Length: (chars * 2) as u16, MaximumLength: (chars * 2) as u16, Buffer: buf.as_ptr() as *mut u16 }
}

fn nt_err(status: NTSTATUS) -> io::Error {
    // SAFETY: pure conversion function without pointers.
    io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
}

/// Names of the object-manager entries in `\Device` that start with `prefix`.
pub(crate) fn device_objects(prefix: &str) -> io::Result<Vec<String>> {
    let dir_name = wide_str(r"\Device");
    let us = unicode_string(&dir_name);
    // SAFETY: plain-old-data struct; all-zero is a valid initial value.
    let mut oa: OBJECT_ATTRIBUTES = unsafe { zeroed() };
    oa.Length = size_of::<OBJECT_ATTRIBUTES>() as u32;
    oa.ObjectName = &us;
    oa.Attributes = OBJ_CASE_INSENSITIVE;
    let mut raw: HANDLE = null_mut();
    // SAFETY: `oa` points at `us`, which points into `dir_name`; all outlive the call.
    let st = unsafe { NtOpenDirectoryObject(&mut raw, DIRECTORY_QUERY, &oa) };
    if st < 0 {
        return Err(nt_err(st));
    }
    let handle = Handle::new(raw)?;
    let mut buf = vec![0u64; 8 * 1024]; // 64 KiB, 8-byte aligned
    let mut context = 0u32;
    let mut restart = true;
    let mut out = Vec::new();
    loop {
        let mut returned = 0u32;
        // SAFETY: `buf` is writable for the length passed; `context`/`returned` are valid
        // out-pointers; the handle is open for DIRECTORY_QUERY.
        let st = unsafe {
            NtQueryDirectoryObject(
                handle.raw(),
                buf.as_mut_ptr().cast::<c_void>(),
                (buf.len() * 8) as u32,
                false,
                restart,
                &mut context,
                &mut returned,
            )
        };
        restart = false;
        if st == STATUS_NO_MORE_ENTRIES {
            break;
        }
        if st < 0 {
            return Err(nt_err(st));
        }
        // The buffer starts with an array of records terminated by an all-zero record;
        // the name strings live further down in the same buffer.
        let base = buf.as_ptr().cast::<u8>();
        let cap = buf.len() * 8;
        let rec = size_of::<ObjectDirectoryInformation>();
        let mut off = 0usize;
        while off + rec <= cap {
            // SAFETY: `off + rec <= cap`, the buffer is 8-byte aligned and records are
            // pointer-aligned; read_unaligned avoids relying on that anyway.
            let info = unsafe { std::ptr::read_unaligned(base.add(off).cast::<ObjectDirectoryInformation>()) };
            if info.name.Buffer.is_null() || info.name.Length == 0 {
                break;
            }
            let len = usize::from(info.name.Length / 2);
            let start = info.name.Buffer as usize;
            let lo = base as usize;
            // Only trust names that point inside our buffer.
            if start >= lo && start + len * 2 <= lo + cap {
                // SAFETY: bounds checked against the buffer just above.
                let chars = unsafe { std::slice::from_raw_parts(info.name.Buffer, len) };
                let name = String::from_utf16_lossy(chars);
                if name.starts_with(prefix) {
                    out.push(name);
                }
            }
            off += rec;
        }
        if st != STATUS_MORE_ENTRIES && returned == 0 {
            break;
        }
    }
    Ok(out)
}

fn open_dir_raw(path: &str) -> io::Result<Handle> {
    let w = wide_str(path);
    // SAFETY: NUL-terminated UTF-16 path; no security attributes / template.
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
    Handle::new(h)
}

/// `FileFsVolumeInformation` of an open handle: `(serial, creation time)`.
fn fs_volume_info(h: &Handle) -> io::Result<(u32, Option<SystemTime>)> {
    // FILE_FS_VOLUME_INFORMATION: i64 creation, u32 serial, u32 label length, u8 objects,
    // then the label; 8-byte aligned buffer with room for a long label.
    let mut buf = [0u64; 80];
    // SAFETY: plain-old-data out-struct.
    let mut iosb: IO_STATUS_BLOCK = unsafe { zeroed() };
    // SAFETY: valid handle; the buffer is writable for the length passed.
    let st = unsafe {
        NtQueryVolumeInformationFile(
            h.raw(),
            &mut iosb,
            buf.as_mut_ptr().cast::<c_void>(),
            (buf.len() * 8) as u32,
            FileFsVolumeInformation,
        )
    };
    // STATUS_BUFFER_OVERFLOW (label truncated) still fills the fixed part.
    if st < 0 && st != 0x8000_0005_u32 as i32 {
        return Err(nt_err(st));
    }
    let created = buf[0];
    let serial = (buf[1] & 0xFFFF_FFFF) as u32;
    Ok((serial, if created == 0 { None } else { ticks_to_system(created) }))
}

/// Open `\\?\GLOBALROOT\Device\<name>\` and read its identity.
pub(crate) fn probe_device(name: &str) -> Probe {
    let path = format!(r"\\?\GLOBALROOT\Device\{name}\");
    match open_dir_raw(&path) {
        Ok(h) => match fs_volume_info(&h) {
            Ok((serial, created)) => Probe::Present { serial, created },
            Err(_) => match super::volume::volume_info_handle(h.raw()) {
                Ok(v) => Probe::Present { serial: v.serial, created: None },
                Err(_) => Probe::Present { serial: 0, created: None },
            },
        },
        Err(e) => match e.raw_os_error().map(|c| c as u32) {
            Some(ERROR_ACCESS_DENIED) => Probe::Denied,
            Some(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_BAD_PATHNAME | ERROR_INVALID_NAME) => {
                Probe::Missing
            }
            // Anything else (device busy, not ready): treat as present but unreadable.
            _ => Probe::Denied,
        },
    }
}

/// `(serial, creation time)` of the live volume mounted at `root` (e.g. `C:\`).
pub(crate) fn volume_identity(root: &Path) -> io::Result<(u32, Option<SystemTime>)> {
    let h = super::fs::open(root, FILE_READ_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS)?;
    fs_volume_info(&h)
}

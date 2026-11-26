//! Which processes have a file or folder open: `NtQueryInformationFile` with
//! `FileProcessIdsUsingFileInformation` (ntdll). The handle used for the query asks for
//! attribute access only, so it opens whatever the sharing mode of the other handles is,
//! and it is not counted itself.

use std::io;
use std::mem::size_of;
use std::path::Path;

use windows_sys::Wdk::Storage::FileSystem::{FileProcessIdsUsingFileInformation, NtQueryInformationFile};
use windows_sys::Win32::Foundation::{NTSTATUS, RtlNtStatusToDosError};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

use super::fs::open;

const STATUS_INFO_LENGTH_MISMATCH: NTSTATUS = 0xC000_0004_u32 as i32;
/// Most process ids read for one file.
const MAX_PIDS: usize = 1 << 16;

/// The ids of the processes that have `path` (a file or a folder, never what a link points
/// to) open. Our own process is listed when it holds another handle to it.
pub(crate) fn pids_using(path: &Path) -> io::Result<Vec<u32>> {
    let h = open(path, FILE_READ_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)?;
    // `FILE_PROCESS_IDS_USING_FILE_INFORMATION`: a `u32` count padded to a pointer, then
    // that many pointer-sized ids. Pointer-sized cells keep the buffer aligned.
    let mut buf = vec![0usize; 64];
    loop {
        // SAFETY: plain-old-data struct, all-zero is valid.
        let mut iosb: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
        let len = buf.len() * size_of::<usize>();
        // SAFETY: open synchronous handle; `buf` is writable for `len` bytes and outlives
        // the call; valid status block.
        let status = unsafe {
            NtQueryInformationFile(
                h.raw(),
                &mut iosb,
                buf.as_mut_ptr().cast(),
                len as u32,
                FileProcessIdsUsingFileInformation,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH && buf.len() < MAX_PIDS {
            buf = vec![0usize; buf.len() * 4];
            continue;
        }
        if status < 0 {
            // SAFETY: pure lookup in ntdll, no pointers involved.
            return Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32));
        }
        let count = (buf[0] as u32 as usize).min(buf.len() - 1);
        return Ok(buf[1..=count].iter().map(|&p| p as u32).collect());
    }
}

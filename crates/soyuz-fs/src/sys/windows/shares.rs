//! SMB share enumeration with `NetShareEnum` (netapi32, level 1). Not a shell API.

use std::io;

use windows_sys::Win32::NetworkManagement::NetManagement::{MAX_PREFERRED_LENGTH, NERR_Success, NetApiBufferFree};
use windows_sys::Win32::Storage::FileSystem::{NetShareEnum, SHARE_INFO_1};

use crate::shares::ShareInfo;

/// `ERROR_MORE_DATA`: more entries follow (call again with the resume handle).
const ERROR_MORE_DATA: u32 = 234;

fn from_pwstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // SAFETY: `p` is a NUL-terminated UTF-16 string inside the buffer NetShareEnum
    // returned; we stop at the terminator.
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

pub(crate) fn list(server: &str) -> io::Result<Vec<ShareInfo>> {
    let wide: Vec<u16> = server.encode_utf16().chain(std::iter::once(0)).collect();
    let mut out = Vec::new();
    let mut resume: u32 = 0;
    loop {
        let mut buf: *mut u8 = std::ptr::null_mut();
        let (mut read, mut total) = (0u32, 0u32);
        // SAFETY: valid NUL-terminated server name, out-pointers to locals; the buffer
        // is freed with NetApiBufferFree below.
        let rc = unsafe {
            NetShareEnum(wide.as_ptr(), 1, &mut buf, MAX_PREFERRED_LENGTH, &mut read, &mut total, &mut resume)
        };
        if rc != NERR_Success && rc != ERROR_MORE_DATA {
            if !buf.is_null() {
                // SAFETY: buffer allocated by NetShareEnum.
                unsafe { NetApiBufferFree(buf as *const _) };
            }
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        if !buf.is_null() {
            let items = buf as *const SHARE_INFO_1;
            for i in 0..read as usize {
                // SAFETY: NetShareEnum wrote `read` SHARE_INFO_1 records at `buf`.
                let si = unsafe { *items.add(i) };
                out.push(ShareInfo {
                    name: from_pwstr(si.shi1_netname),
                    remark: from_pwstr(si.shi1_remark),
                    disk: si.shi1_type & 0xFF == 0,
                    special: si.shi1_type & 0x8000_0000 != 0,
                });
            }
            // SAFETY: buffer allocated by NetShareEnum.
            unsafe { NetApiBufferFree(buf as *const _) };
        }
        if rc != ERROR_MORE_DATA {
            break;
        }
    }
    Ok(out)
}

//! Volume queries: drives, drive types, labels/filesystems, free space, mount points.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetDriveTypeW,
    GetLogicalDrives, GetVolumeInformationByHandleW, GetVolumeInformationW, GetVolumePathNameW,
    GetVolumePathNamesForVolumeNameW,
};

use super::{from_wide_nul, last_err, wide_path, wide_str};

/// Bitmask of present drive letters (bit 0 = `A:`).
pub(crate) fn logical_drives() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetLogicalDrives() }
}

/// `GetDriveTypeW` for a root like `C:\` (plain, not verbatim).
pub(crate) fn drive_type(root: &Path) -> u32 {
    let w = wide_str(root.as_os_str());
    // SAFETY: NUL-terminated root path.
    unsafe { GetDriveTypeW(w.as_ptr()) }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct VolInfo {
    pub label: String,
    pub serial: u32,
    pub flags: u32,
    pub fs: String,
}

/// Plain (non-verbatim) root with a trailing backslash; volume roots are always short.
fn root_plain(root: &Path) -> Vec<u16> {
    let mut s = crate::path::strip_verbatim(root).into_os_string();
    if !s.to_string_lossy().ends_with('\\') {
        s.push("\\");
    }
    wide_str(&s)
}

fn root_wide(root: &Path) -> Vec<u16> {
    // GetVolumeInformationW wants a trailing backslash.
    let mut w = wide_path(root);
    w.pop();
    if w.last() != Some(&(b'\\' as u16)) {
        w.push(b'\\' as u16);
    }
    w.push(0);
    w
}

/// `GetVolumeInformationW` for a volume root (drive, mount folder or `\\srv\share\`).
pub(crate) fn volume_info(root: &Path) -> io::Result<VolInfo> {
    let w = root_plain(root);
    let mut label = [0u16; 261];
    let mut fs = [0u16; 261];
    let (mut serial, mut maxlen, mut flags) = (0u32, 0u32, 0u32);
    // SAFETY: NUL-terminated root; output buffers with the lengths we pass.
    let ok = unsafe {
        GetVolumeInformationW(
            w.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            &mut serial,
            &mut maxlen,
            &mut flags,
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    if ok == 0 {
        return Err(last_err());
    }
    Ok(VolInfo {
        label: from_wide_nul(&label).to_string_lossy().into_owned(),
        serial,
        flags,
        fs: from_wide_nul(&fs).to_string_lossy().into_owned(),
    })
}

/// Volume information of the volume holding an open handle.
pub(crate) fn volume_info_handle(h: windows_sys::Win32::Foundation::HANDLE) -> io::Result<VolInfo> {
    let mut fs = [0u16; 261];
    let (mut serial, mut maxlen, mut flags) = (0u32, 0u32, 0u32);
    // SAFETY: valid handle; output buffers with the lengths we pass (label not requested).
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            h,
            null_mut(),
            0,
            &mut serial,
            &mut maxlen,
            &mut flags,
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    if ok == 0 {
        return Err(last_err());
    }
    Ok(VolInfo { label: String::new(), serial, flags, fs: from_wide_nul(&fs).to_string_lossy().into_owned() })
}

/// `(total, free for the caller)` bytes of the volume containing `path`.
pub(crate) fn disk_free(path: &Path) -> io::Result<(u64, u64)> {
    let w = root_wide(path);
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    // SAFETY: NUL-terminated directory path and three valid out-pointers.
    if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) } == 0 {
        return Err(last_err());
    }
    Ok((total, avail))
}

/// Cluster size in bytes of the volume rooted at `root`.
pub(crate) fn cluster_size(root: &Path) -> io::Result<u64> {
    let w = root_plain(root);
    let (mut spc, mut bps, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
    // SAFETY: NUL-terminated root and four valid out-pointers.
    if unsafe { GetDiskFreeSpaceW(w.as_ptr(), &mut spc, &mut bps, &mut free, &mut total) } == 0 {
        return Err(last_err());
    }
    Ok(u64::from(spc) * u64::from(bps))
}

/// `GetVolumePathNameW`: the mount point (drive root or mounted folder) of `path`.
pub(crate) fn volume_path_name(path: &Path) -> io::Result<PathBuf> {
    let w = wide_path(path);
    let mut buf = vec![0u16; w.len() + 64];
    // SAFETY: NUL-terminated input; output buffer of the length we pass.
    if unsafe { GetVolumePathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } == 0 {
        return Err(last_err());
    }
    Ok(crate::path::strip_verbatim(Path::new(&from_wide_nul(&buf))))
}

/// All mount paths (drive roots and mounted folders) of every volume on the machine.
pub(crate) fn volume_mount_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut name = [0u16; 64];
    // SAFETY: output buffer of the length we pass.
    let h = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if h == INVALID_HANDLE_VALUE {
        return out;
    }
    loop {
        let vol = name;
        let mut len = 0u32;
        let mut buf = vec![0u16; 1024];
        loop {
            // SAFETY: `vol` is the NUL-terminated volume GUID path from FindFirst/NextVolumeW;
            // output buffer of the length we pass.
            let ok =
                unsafe { GetVolumePathNamesForVolumeNameW(vol.as_ptr(), buf.as_mut_ptr(), buf.len() as u32, &mut len) };
            if ok != 0 {
                break;
            }
            if last_err().raw_os_error() == Some(ERROR_MORE_DATA as i32) && (len as usize) > buf.len() {
                buf.resize(len as usize, 0);
                continue;
            }
            buf.clear();
            break;
        }
        // REG_MULTI_SZ-style list: NUL-separated, double-NUL terminated.
        for part in buf.split(|&c| c == 0).filter(|p| !p.is_empty()) {
            out.push(PathBuf::from(OsString::from(String::from_utf16_lossy(part))));
        }
        // SAFETY: valid volume search handle and output buffer.
        if unsafe { FindNextVolumeW(h, name.as_mut_ptr(), name.len() as u32) } == 0 {
            break;
        }
    }
    // SAFETY: valid search handle, closed once.
    unsafe {
        FindVolumeClose(h);
    }
    out
}

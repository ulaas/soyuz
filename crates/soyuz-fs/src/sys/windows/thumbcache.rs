//! Inputs of Explorer's thumbnail cache key (`System.ThumbnailCacheId`), read with plain
//! kernel32 calls: the volume GUID of the file's volume, the NTFS file index, the
//! last-write time as a FAT (DOS) date/time and the precision lost by that conversion.
//! See [`crate::thumbcache`] for the hash itself.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Storage::FileSystem::{GetVolumeNameForVolumeMountPointW, GetVolumePathNameW};
use windows_sys::Win32::System::WindowsProgramming::{DosDateTimeToFileTime, FileTimeToDosDateTime};

use super::{from_wide_nul, last_err, u64_ft, wide_path, wide_str};

/// What the cache id is computed from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KeyInputs {
    /// Volume GUID in Windows memory layout (first three fields little-endian).
    pub volume_guid: [u8; 16],
    /// `(nFileIndexHigh << 32) | nFileIndexLow` (includes the sequence number).
    pub file_index: u64,
    pub is_dir: bool,
    /// `(fat_date << 16) | fat_time` of the UTC last-write time.
    pub dos_time: u32,
    /// Low 32 bits of (DOS time converted back to a FILETIME) minus the original.
    pub precision_loss: u32,
}

/// Volume GUID per mount point (`C:\`), looked up once.
static GUIDS: Mutex<Option<HashMap<PathBuf, Option<[u8; 16]>>>> = Mutex::new(None);

fn volume_guid(path: &Path) -> Option<[u8; 16]> {
    let w = wide_path(path);
    let mut mount = [0u16; 1024];
    // SAFETY: `w` is NUL-terminated; `mount` is a writable buffer of the length we pass.
    if unsafe { GetVolumePathNameW(w.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) } == 0 {
        return None;
    }
    let mount = PathBuf::from(from_wide_nul(&mount));
    let mount = crate::path::strip_verbatim(&mount);
    let mut cache = GUIDS.lock().ok()?;
    let map = cache.get_or_insert_with(HashMap::new);
    if let Some(g) = map.get(&mount) {
        return *g;
    }
    let mut s = mount.clone().into_os_string();
    if !s.to_string_lossy().ends_with('\\') {
        s.push("\\");
    }
    let mw = wide_str(&s);
    let mut name = [0u16; 64];
    // SAFETY: NUL-terminated mount point; `name` is writable for the length we pass.
    let ok = unsafe { GetVolumeNameForVolumeMountPointW(mw.as_ptr(), name.as_mut_ptr(), name.len() as u32) } != 0;
    let guid = ok.then(|| from_wide_nul(&name).to_string_lossy().into_owned()).and_then(|n| parse_volume_guid(&n));
    map.insert(mount, guid);
    guid
}

/// `\\?\Volume{0e f7 4c 23-…}\` → the 16 GUID bytes in memory layout.
pub(crate) fn parse_volume_guid(name: &str) -> Option<[u8; 16]> {
    let start = name.find('{')?;
    let g = name.get(start + 1..start + 37)?;
    let parts: Vec<&str> = g.split('-').collect();
    if parts.len() != 5
        || [8, 4, 4, 4, 12] != [parts[0].len(), parts[1].len(), parts[2].len(), parts[3].len(), parts[4].len()]
    {
        return None;
    }
    let d1 = u32::from_str_radix(parts[0], 16).ok()?;
    let d2 = u16::from_str_radix(parts[1], 16).ok()?;
    let d3 = u16::from_str_radix(parts[2], 16).ok()?;
    let rest = format!("{}{}", parts[3], parts[4]);
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&d1.to_le_bytes());
    out[4..6].copy_from_slice(&d2.to_le_bytes());
    out[6..8].copy_from_slice(&d3.to_le_bytes());
    for (i, b) in out[8..].iter_mut().enumerate() {
        *b = u8::from_str_radix(rest.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// `FileTimeToDosDateTime` + `DosDateTimeToFileTime` round trip: `(dos, precision loss)`.
pub(crate) fn dos_time(written: u64) -> Option<(u32, u32)> {
    let ft: FILETIME = u64_ft(written);
    let (mut date, mut time) = (0u16, 0u16);
    // SAFETY: valid FILETIME in, two writable u16 out.
    if unsafe { FileTimeToDosDateTime(&ft, &mut date, &mut time) } == 0 {
        return None;
    }
    let mut back = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    // SAFETY: plain values in, writable FILETIME out.
    if unsafe { DosDateTimeToFileTime(date, time, &mut back) } == 0 {
        return None;
    }
    Some(((u32::from(date) << 16) | u32::from(time), back.dwLowDateTime.wrapping_sub(ft.dwLowDateTime)))
}

/// The key inputs of `path` (follows links like Explorer does for the target).
pub(crate) fn key_inputs(path: &Path) -> io::Result<KeyInputs> {
    let h = super::fs::open_meta(path, true)?;
    let st = super::fs::stat_handle(h.raw())?;
    let file_index = st.file_index.ok_or_else(last_err)?;
    let volume_guid = volume_guid(path).ok_or_else(|| io::Error::other("no volume GUID"))?;
    let (dos_time, precision_loss) =
        dos_time(st.written).ok_or_else(|| io::Error::other("time outside the FAT range"))?;
    Ok(KeyInputs { volume_guid, file_index, is_dir: st.is_dir(), dos_time, precision_loss })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_bytes_use_windows_memory_layout() {
        let g = parse_volume_guid("\\\\?\\Volume{b0c76d5d-8227-4f96-b2ea-a4c95ebd409c}\\").unwrap();
        assert_eq!(g, [0x5d, 0x6d, 0xc7, 0xb0, 0x27, 0x82, 0x96, 0x4f, 0xb2, 0xea, 0xa4, 0xc9, 0x5e, 0xbd, 0x40, 0x9c]);
        assert!(parse_volume_guid("\\\\?\\Volume{zz}\\").is_none());
    }

    #[test]
    fn dos_round_trip_reports_the_lost_precision() {
        // 2026-10-08 22:16:41.1234567 UTC: odd second plus a fraction.
        let ft = 134_044_498_011_234_567u64;
        let (dos, loss) = dos_time(ft).unwrap();
        assert_ne!(dos, 0);
        // The DOS time has 2 s resolution: the round trip moves the time by less than that.
        assert!((loss as i32).unsigned_abs() < 20_000_000, "{loss:#x}");
    }

    #[test]
    fn inputs_of_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.jpg");
        std::fs::write(&f, b"x").unwrap();
        let k = key_inputs(&f).unwrap();
        assert!(!k.is_dir);
        assert_ne!(k.file_index, 0);
    }
}

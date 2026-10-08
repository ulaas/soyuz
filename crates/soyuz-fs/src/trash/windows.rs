//! Windows `$Recycle.Bin`, written by hand (no shell APIs).
//!
//! Per volume and user: `<root>\$Recycle.Bin\<user SID>\`. Each item is a pair:
//! * `$R<6 chars><ext>` — the original file/folder, moved there with `MoveFileExW`
//!   (same volume, never copied);
//! * `$I<6 chars><ext>` — metadata, format v2 (Windows 10+), little-endian:
//!   `u64 version=2 | u64 size | FILETIME deleted (UTC) | u32 chars incl. NUL | UTF-16 path + NUL`.
//!   Version 1 (Vista–8.1) has a fixed 260-character path field and is parsed too.
//!
//! Only NTFS/ReFS volumes that already have a `$Recycle.Bin` are used (creating that root
//! needs admin); FAT/exFAT, network drives and others report `TrashSupport::Unsupported`.
//! Explorer picks items up on its next refresh (there is no shell notification).
#![cfg_attr(not(windows), allow(dead_code))]

/// Parsed `$I` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IFile {
    pub version: u64,
    pub size: u64,
    /// FILETIME ticks (UTC).
    pub deleted: u64,
    pub path: String,
}

/// Encode a version-2 `$I` record.
pub(crate) fn format_i(size: u64, deleted_ticks: u64, path: &str) -> Vec<u8> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut v = Vec::with_capacity(28 + wide.len() * 2);
    v.extend_from_slice(&2u64.to_le_bytes());
    v.extend_from_slice(&size.to_le_bytes());
    v.extend_from_slice(&deleted_ticks.to_le_bytes());
    v.extend_from_slice(&(wide.len() as u32).to_le_bytes());
    for c in wide {
        v.extend_from_slice(&c.to_le_bytes());
    }
    v
}

/// Decode a `$I` record (v1 or v2).
pub(crate) fn parse_i(b: &[u8]) -> Option<IFile> {
    let u64_at = |o: usize| b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().expect("8 bytes")));
    let version = u64_at(0)?;
    let size = u64_at(8)?;
    let deleted = u64_at(16)?;
    let (start, units) = match version {
        1 => (24usize, 260usize),
        2 => {
            let n = b.get(24..28).map(|s| u32::from_le_bytes(s.try_into().expect("4 bytes")))? as usize;
            if n == 0 || n > 32_768 {
                return None;
            }
            (28usize, n)
        }
        _ => return None,
    };
    let raw = b.get(start..start + units * 2)?;
    let wide: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    let path = String::from_utf16(&wide[..end]).ok()?;
    if path.is_empty() {
        return None;
    }
    Some(IFile { version, size, deleted, path })
}

/// Six random `[A-Z0-9]` characters.
pub(crate) fn random_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(SEQ.fetch_add(1, Ordering::Relaxed));
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.write_u32(std::process::id());
    let mut x = h.finish();
    const CHARS: &[u8; 36] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    (0..6)
        .map(|_| {
            let c = CHARS[(x % 36) as usize] as char;
            x /= 36;
            c
        })
        .collect()
}

/// Extension (with the dot) kept on `$I`/`$R` names, like Explorer does.
pub(crate) fn kept_ext(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 && !name[i..].contains(' ') => name[i..].to_string(),
        _ => String::new(),
    }
}

#[cfg(windows)]
pub(crate) use imp::*;

#[cfg(windows)]
mod imp {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use soyuz_core::{Control, EntryKind, FsError, FsErrorKind, FsResult, OpControl};

    use super::{format_i, kept_ext, parse_i, random_id};
    use crate::err;
    use crate::sys::windows::{fs as wfs, security, system_to_ticks, ticks_to_system, volume};
    use crate::trash::{TrashItem, TrashSupport};

    const DRIVE_REMOTE: u32 = 4;
    const HIDDEN_SYSTEM: u32 = 0x2 | 0x4;
    const RECYCLE: &str = "$Recycle.Bin";
    const DESKTOP_INI: &str = "[.ShellClassInfo]\r\nCLSID={645FF040-5081-101B-9F08-00AA002F954E}\r\n";

    fn plain(p: &Path) -> PathBuf {
        crate::path::strip_verbatim(&crate::path::normalize(p))
    }

    pub(crate) fn support(path: &Path) -> TrashSupport {
        let root = match crate::volumes::volume_root(path) {
            Ok(r) => r,
            Err(e) => return TrashSupport::Unsupported(format!("cannot determine the volume: {}", e.message)),
        };
        if volume::drive_type(&root) == DRIVE_REMOTE || root.to_string_lossy().starts_with(r"\\") {
            return TrashSupport::Unsupported("network locations have no recycle bin".into());
        }
        match volume::volume_info(&root) {
            Ok(vi) => {
                let fs = vi.fs.to_ascii_uppercase();
                if fs != "NTFS" && fs != "REFS" {
                    return TrashSupport::Unsupported(format!("{} volumes have no recycle bin", vi.fs));
                }
            }
            Err(e) => return TrashSupport::Unsupported(format!("cannot query the volume: {e}")),
        }
        if !crate::list::exists_nofollow(&root.join(RECYCLE)) {
            return TrashSupport::Unsupported("this volume has no recycle bin".into());
        }
        TrashSupport::Native
    }

    fn sid() -> FsResult<String> {
        security::current_user_sid().map_err(|e| err::io_opt("trash", None, e))
    }

    /// `<root>\$Recycle.Bin\<SID>`, created (hidden + system, user/SYSTEM/Administrators
    /// ACL, `desktop.ini`) when missing.
    fn ensure_bin(root: &Path) -> FsResult<PathBuf> {
        let base = root.join(RECYCLE);
        let dir = base.join(sid()?);
        if crate::list::exists_nofollow(&dir) {
            return Ok(dir);
        }
        let sid = sid()?;
        let sd = security::Sddl::new(&format!("D:PAI(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"))
            .map_err(err::map("trash", &dir))?;
        wfs::create_dir(&dir, Some(sd.ptr())).map_err(err::map("trash", &dir))?;
        let _ = wfs::set_attributes(&dir, HIDDEN_SYSTEM);
        let ini = dir.join("desktop.ini");
        if let Ok(mut f) = std::fs::File::create(&ini) {
            let _ = f.write_all(DESKTOP_INI.as_bytes());
            drop(f);
            let _ = wfs::set_attributes(&ini, HIDDEN_SYSTEM);
        }
        Ok(dir)
    }

    pub(crate) fn trash(path: &Path) -> FsResult<TrashItem> {
        let abs = plain(path);
        if let TrashSupport::Unsupported(why) = support(&abs) {
            return Err(FsError::with_message(FsErrorKind::Unsupported, "trash", Some(path), why));
        }
        let root = crate::volumes::volume_root(&abs)?;
        if crate::path::is_root(&abs) || abs == root {
            return Err(FsError::with_message(
                FsErrorKind::InvalidName,
                "trash",
                Some(path),
                "cannot trash a volume root",
            ));
        }
        if crate::path::path_overlaps(&abs, &root.join(RECYCLE)) {
            return Err(FsError::with_message(
                FsErrorKind::InvalidName,
                "trash",
                Some(path),
                "the item is already in the recycle bin",
            ));
        }
        let bin = ensure_bin(&root)?;
        trash_into(&abs, &bin)
    }

    /// Move `abs` into the bin directory `bin` as a `$I`/`$R` pair.
    pub(crate) fn trash_into(abs: &Path, bin: &Path) -> FsResult<TrashItem> {
        let e = crate::list::stat(abs, false).map_err(|e| FsError { op: "trash".into(), ..e })?;
        let size = crate::trash::tree_size(abs, &e);
        let name = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = if e.kind == EntryKind::Dir { String::new() } else { kept_ext(&name) };
        let now = SystemTime::now();
        let record = format_i(size, system_to_ticks(now), &abs.to_string_lossy());
        for _ in 0..64 {
            let id6 = random_id();
            let r_name = format!("$R{id6}{ext}");
            let i_path = bin.join(format!("$I{id6}{ext}"));
            let r_path = bin.join(&r_name);
            if crate::list::exists_nofollow(&r_path) {
                continue;
            }
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&i_path) {
                Ok(mut f) => {
                    if let Err(e) = f.write_all(&record).and_then(|_| f.sync_all()) {
                        drop(f);
                        let _ = std::fs::remove_file(&i_path);
                        return Err(err::io("trash", &i_path, e));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(err::io("trash", &i_path, e)),
            }
            if let Err(e) = wfs::move_file(abs, &r_path, 0) {
                let _ = std::fs::remove_file(&i_path);
                return Err(err::io("trash", abs, e));
            }
            return Ok(TrashItem {
                id: r_name,
                original: abs.to_path_buf(),
                deleted_at: now,
                size,
                kind: e.kind,
                trash_root: bin.to_path_buf(),
            });
        }
        Err(FsError::with_message(FsErrorKind::AlreadyExists, "trash", Some(bin), "no free name in the recycle bin"))
    }

    /// Items of one bin directory.
    pub(crate) fn list_bin(bin: &Path) -> Vec<TrashItem> {
        let mut out = Vec::new();
        let Ok(rd) = wfs::read_dir(bin) else { return out };
        for fe in rd.flatten() {
            let name = fe.name.to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix("$I") else { continue };
            let Ok(bytes) = std::fs::read(bin.join(&name)) else { continue };
            let Some(rec) = parse_i(&bytes) else { continue };
            let r_name = format!("$R{rest}");
            let Ok(payload) = crate::list::stat(&bin.join(&r_name), false) else { continue };
            out.push(TrashItem {
                id: r_name,
                original: PathBuf::from(rec.path),
                deleted_at: ticks_to_system(rec.deleted).unwrap_or(SystemTime::UNIX_EPOCH),
                size: rec.size,
                kind: payload.kind,
                trash_root: bin.to_path_buf(),
            });
        }
        out
    }

    pub(crate) fn list() -> FsResult<Vec<TrashItem>> {
        let sid = sid()?;
        let mask = volume::logical_drives();
        let mut out = Vec::new();
        for i in 0..26u8 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let root = PathBuf::from(format!("{}:\\", (b'A' + i) as char));
            // Fixed and removable drives only: never touch network or optical drives here.
            if !matches!(volume::drive_type(&root), 2 | 3) {
                continue;
            }
            let bin = root.join(RECYCLE).join(&sid);
            if crate::list::exists_nofollow(&bin) {
                out.extend(list_bin(&bin));
            }
        }
        Ok(out)
    }

    /// Validate that `item` names a `$R` entry with its `$I` record next to it.
    fn payload(item: &TrashItem) -> FsResult<(PathBuf, PathBuf)> {
        if !crate::trash::safe_id(&item.id) || !item.id.starts_with("$R") || item.id.len() < 3 {
            return Err(FsError::with_message(
                FsErrorKind::InvalidName,
                "trash",
                Some(&item.trash_root),
                "not a recycle bin item",
            ));
        }
        let r = item.trash_root.join(&item.id);
        let i = item.trash_root.join(format!("$I{}", &item.id[2..]));
        let paired = std::fs::read(&i).ok().and_then(|b| parse_i(&b)).is_some();
        if !paired || !crate::list::exists_nofollow(&r) {
            return Err(FsError::with_message(
                FsErrorKind::NotFound,
                "trash",
                Some(&item.original),
                "the item is no longer in the recycle bin",
            ));
        }
        Ok((r, i))
    }

    pub(crate) fn restore(item: &TrashItem, to: Option<&Path>) -> FsResult<PathBuf> {
        let (r, i) = payload(item)?;
        let dst = to.map(Path::to_path_buf).unwrap_or_else(|| item.original.clone());
        if crate::list::exists_nofollow(&dst) {
            return Err(FsError::new(FsErrorKind::AlreadyExists, "restore", Some(&dst)));
        }
        if let Some(parent) = dst.parent().filter(|p| !p.as_os_str().is_empty()) {
            crate::create::create_dir_all(parent)?;
        }
        wfs::move_file(&r, &dst, 0).map_err(|e| {
            let code = e.raw_os_error();
            let mut fe = err::io("restore", &dst, e);
            if code == Some(17) {
                fe.message = "the destination is on another volume; copy the item out instead".into();
            }
            fe
        })?;
        let _ = std::fs::remove_file(&i);
        Ok(dst)
    }

    pub(crate) fn purge(item: &TrashItem) -> FsResult<()> {
        let (r, i) = payload(item)?;
        crate::trash::remove_any(&r)?;
        match std::fs::remove_file(&i) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(err::io("purge", &i, e)),
        }
    }

    pub(crate) fn empty(ctl: &OpControl, on_item: &mut dyn FnMut(&TrashItem, FsResult<()>) -> Control) -> FsResult<()> {
        for item in list()? {
            ctl.checkpoint()?;
            let res = purge(&item);
            if on_item(&item, res) == Control::Stop {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte layout of a v2 `$I` record as documented by libyal (dtformats, "Windows
    /// Recycle.Bin file formats") for a 0x1234-byte file `C:\a.txt` deleted at FILETIME
    /// 0x01D9_0000_0000_0000.
    #[test]
    fn i_record_v2_layout() {
        let b = format_i(0x1234, 0x01D9_0000_0000_0000, r"C:\a.txt");
        let mut expected = vec![2, 0, 0, 0, 0, 0, 0, 0, 0x34, 0x12, 0, 0, 0, 0, 0, 0];
        expected.extend_from_slice(&0x01D9_0000_0000_0000u64.to_le_bytes());
        expected.extend_from_slice(&9u32.to_le_bytes()); // 8 chars + NUL
        for c in r"C:\a.txt".encode_utf16().chain([0]) {
            expected.extend_from_slice(&c.to_le_bytes());
        }
        assert_eq!(b, expected);
        let p = parse_i(&b).unwrap();
        assert_eq!(p, IFile { version: 2, size: 0x1234, deleted: 0x01D9_0000_0000_0000, path: r"C:\a.txt".into() });
    }

    #[test]
    fn i_record_v1_and_garbage() {
        let mut v1 = Vec::new();
        v1.extend_from_slice(&1u64.to_le_bytes());
        v1.extend_from_slice(&42u64.to_le_bytes());
        v1.extend_from_slice(&7u64.to_le_bytes());
        let mut path: Vec<u16> = r"D:\old\file.doc".encode_utf16().collect();
        path.resize(260, 0);
        for c in path {
            v1.extend_from_slice(&c.to_le_bytes());
        }
        assert_eq!(v1.len(), 544);
        let p = parse_i(&v1).unwrap();
        assert_eq!((p.version, p.size, p.path.as_str()), (1, 42, r"D:\old\file.doc"));
        assert!(parse_i(&[0; 10]).is_none());
        assert!(parse_i(&format_i(1, 1, "")).is_none());
        let mut bad = format_i(1, 1, "x");
        bad[24] = 0xFF;
        bad[25] = 0xFF;
        assert!(parse_i(&bad).is_none());
    }

    #[test]
    fn names() {
        let id = random_id();
        assert_eq!(id.len(), 6);
        assert!(id.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        assert_ne!(random_id(), random_id());
        assert_eq!(kept_ext("report.final.docx"), ".docx");
        assert_eq!(kept_ext(".gitignore"), "");
        assert_eq!(kept_ext("noext"), "");
    }

    #[cfg(windows)]
    #[test]
    fn bin_round_trip_in_temp_dir() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let f = d.path().join("doc.txt");
        std::fs::write(&f, b"payload").unwrap();
        let item = trash_into(&f, &bin).unwrap();
        assert!(!f.exists());
        assert!(item.id.starts_with("$R") && item.id.ends_with(".txt"));
        let i_name = format!("$I{}", &item.id[2..]);
        let rec = parse_i(&std::fs::read(bin.join(&i_name)).unwrap()).unwrap();
        assert_eq!(rec.size, 7);
        assert_eq!(std::path::PathBuf::from(&rec.path), f);

        let dir = d.path().join("folder");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("x"), b"123").unwrap();
        let ditem = trash_into(&dir, &bin).unwrap();
        assert_eq!(ditem.size, 3);
        assert_eq!(ditem.kind, soyuz_core::EntryKind::Dir);

        let listed = list_bin(&bin);
        assert_eq!(listed.len(), 2);
        let back = restore(&item, None).unwrap();
        assert_eq!(std::fs::read(back).unwrap(), b"payload");
        assert!(!bin.join(&i_name).exists());
        purge(&ditem).unwrap();
        assert!(list_bin(&bin).is_empty());
    }
}

//! freedesktop.org trash specification 1.0 (Linux / BSD).
//!
//! * Home trash: `$XDG_DATA_HOME/Trash` (default `~/.local/share/Trash`), `Path=` absolute.
//! * Other mounts: `$topdir/.Trash/$uid` when `$topdir/.Trash` is a sticky, non-symlink
//!   directory, else `$topdir/.Trash-$uid` (created 0700); `Path=` relative to `$topdir`.
//!   `$topdir` is the mount point from `/proc/self/mountinfo` ([`crate::volumes::volume_root`]);
//!   a bin directory that is a symlink or owned by someone else is never used.
//! * The name is reserved by creating `info/<name>.trashinfo` with `O_EXCL` first; then the
//!   item is renamed into `files/<name>`. Directory sizes are cached in `directorysizes`.
//!
//! The [`Bin`] logic is plain filesystem work and compiles on every OS (unit-tested
//! everywhere); bin discovery is unix-only.
#![cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{Local, NaiveDateTime, TimeZone};
use soyuz_core::{EntryKind, FsError, FsErrorKind, FsResult};

use super::TrashItem;
use crate::err;

/// One trash directory (`files/`, `info/`, `directorysizes`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bin {
    pub root: PathBuf,
    /// `None` = home trash (absolute `Path=`); `Some(topdir)` = per-mount trash.
    pub topdir: Option<PathBuf>,
}

const INFO_EXT: &str = ".trashinfo";

impl Bin {
    pub(crate) fn files(&self) -> PathBuf {
        self.root.join("files")
    }

    pub(crate) fn info(&self) -> PathBuf {
        self.root.join("info")
    }

    fn info_path(&self, id: &str) -> PathBuf {
        self.info().join(format!("{id}{INFO_EXT}"))
    }

    pub(crate) fn ensure(&self) -> FsResult<()> {
        for d in [&self.root, &self.files(), &self.info()] {
            if !crate::list::exists_nofollow(d) {
                crate::create::create_dir_all(d)?;
                #[cfg(unix)]
                let _ = crate::perms::set_mode(d, 0o700);
            }
        }
        Ok(())
    }

    /// Move `path` (absolute) into this bin.
    pub(crate) fn trash(&self, path: &Path) -> FsResult<TrashItem> {
        let e = crate::list::stat(path, false).map_err(|e| FsError { op: "trash".into(), ..e })?;
        self.ensure()?;
        // Ids are strings: a non-UTF-8 name is stored under its lossy form (U+FFFD); the
        // exact original bytes stay in `Path=`, so restoring brings back the real name.
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| {
            FsError::with_message(FsErrorKind::InvalidName, "trash", Some(path), "cannot trash a root")
        })?;
        let rel: PathBuf = match &self.topdir {
            Some(top) => path.strip_prefix(top).map(Path::to_path_buf).unwrap_or_else(|_| path.to_path_buf()),
            None => path.to_path_buf(),
        };
        let now = SystemTime::now();
        let date = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let content = format!("[Trash Info]\nPath={}\nDeletionDate={date}\n", percent_encode(&path_bytes(&rel)));
        let (id, info_path) = self.reserve(&name, content.as_bytes())?;
        let size = super::tree_size(path, &e);
        let dest = self.files().join(&id);
        if let Err(mut fe) = crate::mv::rename(path, &dest, crate::mv::RenameMode::NoReplace) {
            let _ = std::fs::remove_file(&info_path);
            fe.op = "trash".into();
            if fe.kind == FsErrorKind::CrossDevice {
                fe.message = "the trash is on a different filesystem than the item".into();
            }
            return Err(fe);
        }
        if e.kind == EntryKind::Dir {
            let mtime = info_mtime_secs(&info_path);
            let _ = self.update_dirsizes(&id, Some((size, mtime)));
        }
        Ok(TrashItem {
            id,
            original: path.to_path_buf(),
            deleted_at: now,
            size,
            kind: e.kind,
            trash_root: self.root.clone(),
        })
    }

    /// Reserve a unique name by creating its `.trashinfo` exclusively.
    fn reserve(&self, name: &str, content: &[u8]) -> FsResult<(String, PathBuf)> {
        let (stem, ext) = split_ext(name);
        for n in 0..10_000u32 {
            let id = if n == 0 { name.to_string() } else { format!("{stem}.{n}{ext}") };
            let info = self.info_path(&id);
            if crate::list::exists_nofollow(&self.files().join(&id)) {
                continue;
            }
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&info) {
                Ok(mut f) => {
                    f.write_all(content).and_then(|_| f.sync_all()).map_err(|e| {
                        let _ = std::fs::remove_file(&info);
                        err::io("trash", &info, e)
                    })?;
                    return Ok((id, info));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(err::io("trash", &info, e)),
            }
        }
        Err(FsError::with_message(FsErrorKind::AlreadyExists, "trash", Some(&self.root), "no free name in the trash"))
    }

    /// Items of this bin (orphaned `.trashinfo` files without payload are skipped).
    pub(crate) fn list(&self) -> Vec<TrashItem> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(self.info()) else { return out };
        let sizes = self.read_dirsizes();
        for de in rd.flatten() {
            let fname = de.file_name().to_string_lossy().into_owned();
            let Some(id) = fname.strip_suffix(INFO_EXT) else { continue };
            let payload = self.files().join(id);
            let Ok(e) = crate::list::stat(&payload, false) else { continue };
            let Ok(text) = std::fs::read_to_string(de.path()) else { continue };
            let Some((raw_path, date)) = parse_trashinfo(&text) else { continue };
            let p = path_from_bytes(percent_decode(&raw_path));
            let original = match &self.topdir {
                Some(top) if !p.is_absolute() => top.join(p),
                _ => p,
            };
            let deleted_at = date
                .and_then(|d| Local.from_local_datetime(&d).earliest())
                .map(SystemTime::from)
                .or(e.modified)
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let size = if e.kind == EntryKind::Dir {
                sizes
                    .iter()
                    .find(|(n, _, _)| n == id)
                    .map(|(_, s, _)| *s)
                    .unwrap_or_else(|| super::tree_size(&payload, &e))
            } else {
                e.size
            };
            out.push(TrashItem {
                id: id.to_string(),
                original,
                deleted_at,
                size,
                kind: e.kind,
                trash_root: self.root.clone(),
            });
        }
        out
    }

    fn check(&self, item: &TrashItem) -> FsResult<PathBuf> {
        if !super::safe_id(&item.id) || !crate::list::exists_nofollow(&self.info_path(&item.id)) {
            return Err(FsError::with_message(
                FsErrorKind::NotFound,
                "trash",
                Some(&item.original),
                "the item is no longer in the trash",
            ));
        }
        Ok(self.files().join(&item.id))
    }

    pub(crate) fn restore(&self, item: &TrashItem, to: Option<&Path>) -> FsResult<PathBuf> {
        let src = self.check(item)?;
        let dst = to.map(Path::to_path_buf).unwrap_or_else(|| item.original.clone());
        if crate::list::exists_nofollow(&dst) {
            return Err(FsError::new(FsErrorKind::AlreadyExists, "restore", Some(&dst)));
        }
        if let Some(parent) = dst.parent().filter(|p| !p.as_os_str().is_empty()) {
            crate::create::create_dir_all(parent)?;
        }
        crate::mv::rename(&src, &dst, crate::mv::RenameMode::NoReplace)
            .map_err(|e| FsError { op: "restore".into(), ..e })?;
        let _ = std::fs::remove_file(self.info_path(&item.id));
        let _ = self.update_dirsizes(&item.id, None);
        Ok(dst)
    }

    pub(crate) fn purge(&self, item: &TrashItem) -> FsResult<()> {
        let payload = self.check(item)?;
        super::remove_any(&payload)?;
        let _ = std::fs::remove_file(self.info_path(&item.id));
        let _ = self.update_dirsizes(&item.id, None);
        Ok(())
    }

    fn dirsizes_path(&self) -> PathBuf {
        self.root.join("directorysizes")
    }

    /// `(name, size, mtime)` entries of `directorysizes`.
    fn read_dirsizes(&self) -> Vec<(String, u64, u64)> {
        let Ok(text) = std::fs::read_to_string(self.dirsizes_path()) else { return Vec::new() };
        text.lines()
            .filter_map(|l| {
                let mut it = l.splitn(3, ' ');
                let size = it.next()?.parse().ok()?;
                let mtime = it.next()?.parse().ok()?;
                let name = String::from_utf8_lossy(&percent_decode(it.next()?)).into_owned();
                Some((name, size, mtime))
            })
            .collect()
    }

    /// Replace (or drop, with `None`) the `directorysizes` line of `id`, atomically.
    fn update_dirsizes(&self, id: &str, entry: Option<(u64, u64)>) -> FsResult<()> {
        let mut lines: Vec<(String, u64, u64)> = self.read_dirsizes().into_iter().filter(|(n, _, _)| n != id).collect();
        if let Some((size, mtime)) = entry {
            lines.push((id.to_string(), size, mtime));
        } else if !crate::list::exists_nofollow(&self.dirsizes_path()) {
            return Ok(());
        }
        let mut text = String::new();
        for (n, s, m) in lines {
            text.push_str(&format!("{s} {m} {}\n", percent_encode(n.as_bytes())));
        }
        crate::create::write_atomic(&self.dirsizes_path(), text.as_bytes())
    }
}

fn info_mtime_secs(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// `(Path= value, DeletionDate)` of a `.trashinfo` file.
pub(crate) fn parse_trashinfo(text: &str) -> Option<(String, Option<NaiveDateTime>)> {
    let mut in_group = false;
    let (mut path, mut date) = (None, None);
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with('[') {
            in_group = line == "[Trash Info]";
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some(v) = line.strip_prefix("Path=") {
            path = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("DeletionDate=") {
            date = NaiveDateTime::parse_from_str(v.trim(), "%Y-%m-%dT%H:%M:%S").ok();
        }
    }
    Some((path?, date))
}

/// RFC 2396 escaping of everything but unreserved characters and `/`.
pub(crate) fn percent_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()/".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

pub(crate) fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

#[cfg(unix)]
fn path_bytes(p: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(p: &Path) -> Vec<u8> {
    p.to_string_lossy().into_owned().into_bytes()
}

#[cfg(unix)]
fn path_from_bytes(b: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(b))
}

#[cfg(not(unix))]
fn path_from_bytes(b: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&b).into_owned())
}

// ---------------------------------------------------------------------------
// Bin discovery (unix)
// ---------------------------------------------------------------------------

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) mod discover {
    use std::path::{Path, PathBuf};

    use soyuz_core::{FsError, FsErrorKind, FsResult};

    use super::Bin;
    use crate::sys::unix as us;
    use crate::trash::TrashSupport;

    pub(crate) fn home_bin() -> Bin {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| crate::path::home().join(".local").join("share"));
        Bin { root: data.join("Trash"), topdir: None }
    }

    fn dev(p: &Path) -> Option<u64> {
        let mut cur = Some(p);
        while let Some(c) = cur {
            if let Ok(e) = crate::list::stat(c, true) {
                return e.dev.map(|d| d.0);
            }
            cur = c.parent();
        }
        None
    }

    /// `$topdir/.Trash/$uid` if `.Trash` is a sticky, non-symlink directory.
    fn shared_trash(topdir: &Path, uid: u32) -> Option<PathBuf> {
        let shared = topdir.join(".Trash");
        let st = us::ustat_at(rustix::fs::CWD, shared.as_os_str(), false).ok()?;
        (st.mode & 0o170000 == 0o040000 && st.mode & 0o1000 != 0).then(|| shared.join(uid.to_string()))
    }

    /// A real directory (not a symlink) owned by `uid`.
    fn valid_private(dir: &Path, uid: u32) -> bool {
        us::ustat_at(rustix::fs::CWD, dir.as_os_str(), false)
            .is_ok_and(|st| st.mode & 0o170000 == 0o040000 && st.uid == uid)
    }

    /// `mkdir(dir, 0700)`: never briefly readable by others (no chmod after the fact).
    fn mkdir_private(dir: &Path) -> rustix::io::Result<()> {
        rustix::fs::mkdirat(rustix::fs::CWD, dir, rustix::fs::Mode::from_raw_mode(0o700))
    }

    /// The bin for `path` (created on demand for per-mount bins).
    pub(crate) fn bin_for(path: &Path) -> FsResult<Bin> {
        let home = home_bin();
        let item_dev = crate::list::stat(path, false).ok().and_then(|e| e.dev.map(|d| d.0));
        if item_dev.is_some() && item_dev == dev(&home.root) {
            return Ok(home);
        }
        let topdir = crate::volumes::volume_root(path)?;
        let uid = us::euid();
        if let Some(root) = shared_trash(&topdir, uid) {
            if !crate::list::exists_nofollow(&root) {
                let _ = mkdir_private(&root);
            }
            if valid_private(&root, uid) {
                return Ok(Bin { root, topdir: Some(topdir) });
            }
        }
        let private = topdir.join(format!(".Trash-{uid}"));
        if !crate::list::exists_nofollow(&private) {
            let _ = mkdir_private(&private);
        }
        if valid_private(&private, uid) {
            return Ok(Bin { root: private, topdir: Some(topdir) });
        }
        Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "trash",
            Some(path),
            format!("no usable trash directory on {}", topdir.display()),
        ))
    }

    pub(crate) fn support(path: &Path) -> TrashSupport {
        let home = home_bin();
        let item_dev = crate::list::stat(path, false).ok().and_then(|e| e.dev.map(|d| d.0));
        if item_dev.is_none() {
            return TrashSupport::Unsupported("the item does not exist".into());
        }
        if item_dev == dev(&home.root) {
            return TrashSupport::Native;
        }
        let Ok(topdir) = crate::volumes::volume_root(path) else {
            return TrashSupport::Unsupported("cannot determine the mount point".into());
        };
        let uid = us::euid();
        if shared_trash(&topdir, uid).is_some() || valid_private(&topdir.join(format!(".Trash-{uid}")), uid) {
            return TrashSupport::Native;
        }
        let writable = rustix::fs::accessat(
            rustix::fs::CWD,
            &topdir,
            rustix::fs::Access::WRITE_OK | rustix::fs::Access::EXEC_OK,
            rustix::fs::AtFlags::EACCESS,
        )
        .is_ok();
        if writable {
            TrashSupport::Native
        } else {
            TrashSupport::Unsupported(format!("{} has no trash and is not writable", topdir.display()))
        }
    }

    /// Home bin plus every existing per-mount bin.
    pub(crate) fn all_bins() -> Vec<Bin> {
        let mut out = vec![home_bin()];
        let uid = us::euid();
        #[cfg(target_os = "linux")]
        let tops = crate::volumes::user_mounts();
        #[cfg(not(target_os = "linux"))]
        let tops: Vec<PathBuf> = crate::volumes::volumes().into_iter().map(|v| v.mount).collect();
        for top in tops {
            let mut cands = Vec::new();
            if let Some(shared) = shared_trash(&top, uid) {
                cands.push(shared);
            }
            cands.push(top.join(format!(".Trash-{uid}")));
            for root in cands {
                if valid_private(&root, uid) && !out.iter().any(|b| b.root == root) {
                    out.push(Bin { root, topdir: Some(top.clone()) });
                }
            }
        }
        out
    }

    /// The bin a [`TrashItem`] belongs to (validated against the known bins).
    pub(crate) fn bin_of(trash_root: &Path) -> Option<Bin> {
        all_bins().into_iter().find(|b| b.root == trash_root)
    }

    #[cfg(test)]
    mod tests {
        use std::os::unix::fs::{PermissionsExt, symlink};

        use super::*;

        #[test]
        fn shared_trash_needs_a_sticky_real_directory() {
            let d = tempfile::tempdir().unwrap();
            let top = d.path();
            let uid = us::euid();
            assert_eq!(shared_trash(top, uid), None, "no .Trash");
            let shared = top.join(".Trash");
            std::fs::create_dir(&shared).unwrap();
            std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
            assert_eq!(shared_trash(top, uid), None, "not sticky");
            std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();
            assert_eq!(shared_trash(top, uid), Some(shared.join(uid.to_string())));
            // A symlink to a sticky directory does not count.
            let other = tempfile::tempdir().unwrap();
            std::fs::remove_dir(&shared).unwrap();
            let target = other.path().join("sticky");
            std::fs::create_dir(&target).unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o1777)).unwrap();
            symlink(&target, &shared).unwrap();
            assert_eq!(shared_trash(top, uid), None, "symlinked .Trash");
        }

        #[test]
        fn private_bins_are_created_0700_and_validated() {
            let d = tempfile::tempdir().unwrap();
            let uid = us::euid();
            let private = d.path().join(format!(".Trash-{uid}"));
            mkdir_private(&private).unwrap();
            let mode = std::fs::metadata(&private).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "never readable by others: {mode:o}");
            assert!(valid_private(&private, uid));
            assert!(!valid_private(&private, uid.wrapping_add(1)), "someone else's");
            let link = d.path().join(".Trash-link");
            symlink(&private, &link).unwrap();
            assert!(!valid_private(&link, uid), "symlinked bin");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_round_trip() {
        let raw = "/home/ü ser/a b%c#d.txt".as_bytes();
        let enc = percent_encode(raw);
        assert_eq!(enc, "/home/%C3%BC%20ser/a%20b%25c%23d.txt");
        assert_eq!(percent_decode(&enc), raw);
        assert_eq!(percent_decode("%zz%4"), b"%zz%4");
    }

    #[test]
    fn trashinfo_parsing() {
        let t = "[Trash Info]\nPath=/home/u/My%20File.txt\nDeletionDate=2024-03-05T14:07:09\n";
        let (p, d) = parse_trashinfo(t).unwrap();
        assert_eq!(p, "/home/u/My%20File.txt");
        assert_eq!(d.unwrap().format("%Y-%m-%d %H:%M:%S").to_string(), "2024-03-05 14:07:09");
        assert!(parse_trashinfo("[Other]\nPath=/x\n").is_none());
        assert!(parse_trashinfo("[Trash Info]\r\nPath=x\r\n").is_some());
    }

    #[test]
    fn bin_trash_list_restore_purge() {
        let d = tempfile::tempdir().unwrap();
        let bin = Bin { root: d.path().join("Trash"), topdir: None };
        let work = d.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let f = work.join("note.txt");
        std::fs::write(&f, b"hello").unwrap();
        let item = bin.trash(&f).unwrap();
        assert_eq!(item.id, "note.txt");
        assert!(!f.exists());
        assert!(bin.files().join("note.txt").exists());
        let info = std::fs::read_to_string(bin.info().join("note.txt.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath="), "{info}");
        assert!(info.contains("DeletionDate="));

        // Same name again → reserved as note.1.txt.
        std::fs::write(&f, b"second").unwrap();
        let item2 = bin.trash(&f).unwrap();
        assert_eq!(item2.id, "note.1.txt");

        // A directory gets a directorysizes line.
        let sub = work.join("folder");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("a"), b"1234").unwrap();
        let item3 = bin.trash(&sub).unwrap();
        assert_eq!(item3.size, 4);
        assert!(std::fs::read_to_string(bin.root.join("directorysizes")).unwrap().contains(" folder\n"));

        let mut listed = bin.list();
        listed.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(listed.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["folder", "note.1.txt", "note.txt"]);
        let l = listed.iter().find(|i| i.id == "note.txt").unwrap();
        assert_eq!(l.original, f);
        assert_eq!(l.size, 5);
        assert_eq!(listed.iter().find(|i| i.id == "folder").unwrap().size, 4);

        // Restore conflicts with an existing file; restore elsewhere works.
        std::fs::write(&f, b"new").unwrap();
        assert_eq!(bin.restore(l, None).unwrap_err().kind, FsErrorKind::AlreadyExists);
        let alt = work.join("restored").join("note.txt");
        assert_eq!(bin.restore(l, Some(&alt)).unwrap(), alt);
        assert_eq!(std::fs::read(&alt).unwrap(), b"hello");
        assert!(!bin.info().join("note.txt.trashinfo").exists());

        bin.purge(&item3).unwrap();
        assert!(!bin.files().join("folder").exists());
        assert!(!std::fs::read_to_string(bin.root.join("directorysizes")).unwrap().contains("folder"));
        assert_eq!(bin.purge(&item3).unwrap_err().kind, FsErrorKind::NotFound);

        // Crafted ids never escape the bin.
        let evil = TrashItem { id: "../work".into(), ..item2.clone() };
        assert!(bin.purge(&evil).is_err());
        assert!(work.exists());
    }

    /// Non-UTF-8 names: the id is the lossy form, `Path=` keeps the exact bytes
    /// (percent-encoded), and a restore brings the original name back.
    #[cfg(unix)]
    #[test]
    fn non_utf8_names_round_trip() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let d = tempfile::tempdir().unwrap();
        let bin = Bin { root: d.path().join("Trash"), topdir: Some(d.path().to_path_buf()) };
        let raw = std::ffi::OsString::from_vec(b"caf\xe9 \xff.txt".to_vec());
        let f = d.path().join(&raw);
        std::fs::write(&f, b"x").unwrap();
        let item = bin.trash(&f).unwrap();
        assert_eq!(item.id, "caf\u{FFFD} \u{FFFD}.txt");
        let info = std::fs::read_to_string(bin.info().join(format!("{}.trashinfo", item.id))).unwrap();
        assert!(info.contains("Path=caf%E9%20%FF.txt\n"), "{info}");
        let listed = bin.list();
        assert_eq!(listed[0].original.file_name().unwrap().as_bytes(), raw.as_bytes());
        let back = bin.restore(&listed[0], None).unwrap();
        assert_eq!(back, f);
        assert!(f.exists());
    }

    /// DeletionDate is local time without a zone, `directorysizes` lines are
    /// `<bytes> <info mtime> <percent-encoded name>`.
    #[test]
    fn info_date_and_directorysizes_format() {
        let d = tempfile::tempdir().unwrap();
        let bin = Bin { root: d.path().join("Trash"), topdir: None };
        let dir = d.path().join("my dir");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("a"), b"123").unwrap();
        let before = Local::now().naive_local() - chrono::Duration::seconds(2);
        let item = bin.trash(&dir).unwrap();
        let info = std::fs::read_to_string(bin.info().join("my dir.trashinfo")).unwrap();
        let (path, date) = parse_trashinfo(&info).unwrap();
        assert_eq!(percent_decode(&path), path_bytes(&dir));
        let date = date.unwrap();
        assert!(date >= before && date <= Local::now().naive_local() + chrono::Duration::seconds(2), "{date}");
        let sizes = std::fs::read_to_string(bin.root.join("directorysizes")).unwrap();
        let line = sizes.lines().next().unwrap();
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        assert_eq!(parts[0], "3");
        assert_eq!(parts[1].parse::<u64>().unwrap(), info_mtime_secs(&bin.info().join("my dir.trashinfo")));
        assert_eq!(parts[2], "my%20dir");
        bin.purge(&item).unwrap();
        assert_eq!(std::fs::read_to_string(bin.root.join("directorysizes")).unwrap(), "");
    }

    #[test]
    fn topdir_bins_store_relative_paths() {
        let d = tempfile::tempdir().unwrap();
        let top = d.path().to_path_buf();
        let bin = Bin { root: top.join(".Trash-1000"), topdir: Some(top.clone()) };
        let f = top.join("data").join("x.bin");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"x").unwrap();
        bin.trash(&f).unwrap();
        let info = std::fs::read_to_string(bin.info().join("x.bin.trashinfo")).unwrap();
        let sep = if cfg!(windows) { "%5C" } else { "/" };
        assert!(info.contains(&format!("Path=data{sep}x.bin\n")), "{info}");
        assert_eq!(bin.list()[0].original, f);
    }
}

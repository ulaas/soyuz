//! macOS trash: `~/.Trash` for the home volume, `<volume>/.Trashes/<uid>` elsewhere, plus
//! our own restore index (`~/Library/Application Support/soyuz/trash-index.json`) because
//! Finder's "Put Back" metadata (in `.DS_Store`) is not writable by other applications.
//! Items trashed by other apps are listed without an original location.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use soyuz_core::{Control, FsError, FsErrorKind, FsResult, OpControl};

use super::TrashItem;
use super::index::{IndexEntry, TrashIndex};

/// Names Finder keeps in every trash directory.
const IGNORED: &[&str] = &[".DS_Store", ".localized"];

/// A trash directory plus the index that remembers where items came from.
pub(crate) struct MacBin {
    pub root: PathBuf,
    pub index: PathBuf,
}

impl MacBin {
    fn idx(&self) -> TrashIndex {
        TrashIndex::new(self.index.clone())
    }

    /// Move `path` (absolute) into this bin; Finder-style `name 2.ext` on collisions.
    pub(crate) fn trash(&self, path: &Path) -> FsResult<TrashItem> {
        let e = crate::list::stat(path, false).map_err(|e| FsError { op: "trash".into(), ..e })?;
        if !crate::list::exists_nofollow(&self.root) {
            crate::create::create_dir_all(&self.root)?;
            #[cfg(unix)]
            let _ = crate::perms::set_mode(&self.root, 0o700);
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| {
            FsError::with_message(FsErrorKind::InvalidName, "trash", Some(path), "cannot trash a root")
        })?;
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name.as_str(), ""),
        };
        let size = super::tree_size(path, &e);
        for n in 1..10_000u32 {
            let id = if n == 1 { name.clone() } else { format!("{stem} {n}{ext}") };
            let dest = self.root.join(&id);
            match crate::mv::rename(path, &dest, crate::mv::RenameMode::NoReplace) {
                Ok(()) => {
                    let now = SystemTime::now();
                    let entry = IndexEntry { original: path.to_path_buf(), deleted_at: now, size, kind: e.kind };
                    if let Err(err) = self.idx().insert(&dest, entry) {
                        log::warn!("trash index not updated for {}: {err}", dest.display());
                    }
                    return Ok(TrashItem {
                        id,
                        original: path.to_path_buf(),
                        deleted_at: now,
                        size,
                        kind: e.kind,
                        trash_root: self.root.clone(),
                    });
                }
                Err(fe) if fe.kind == FsErrorKind::AlreadyExists => continue,
                Err(fe) => return Err(FsError { op: "trash".into(), ..fe }),
            }
        }
        Err(FsError::with_message(FsErrorKind::AlreadyExists, "trash", Some(&self.root), "no free name in the trash"))
    }

    pub(crate) fn list(&self) -> Vec<TrashItem> {
        let known = self.idx().entries();
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.root) else { return out };
        for de in rd.flatten() {
            let id = de.file_name().to_string_lossy().into_owned();
            if IGNORED.contains(&id.as_str()) {
                continue;
            }
            let p = de.path();
            let Ok(e) = crate::list::stat(&p, false) else { continue };
            let item = match known.get(&p.to_string_lossy().into_owned()) {
                Some(k) => TrashItem {
                    id,
                    original: k.original.clone(),
                    deleted_at: k.deleted_at,
                    size: k.size,
                    kind: e.kind,
                    trash_root: self.root.clone(),
                },
                None => TrashItem {
                    id,
                    original: PathBuf::new(),
                    deleted_at: e.modified.unwrap_or(SystemTime::UNIX_EPOCH),
                    size: super::tree_size(&p, &e),
                    kind: e.kind,
                    trash_root: self.root.clone(),
                },
            };
            out.push(item);
        }
        out
    }

    fn payload(&self, item: &TrashItem) -> FsResult<PathBuf> {
        let p = self.root.join(&item.id);
        if !super::safe_id(&item.id) || IGNORED.contains(&item.id.as_str()) || !crate::list::exists_nofollow(&p) {
            return Err(FsError::with_message(
                FsErrorKind::NotFound,
                "trash",
                Some(&item.original),
                "the item is no longer in the trash",
            ));
        }
        Ok(p)
    }

    pub(crate) fn restore(&self, item: &TrashItem, to: Option<&Path>) -> FsResult<PathBuf> {
        let src = self.payload(item)?;
        let dst = match to {
            Some(t) => t.to_path_buf(),
            None if item.original.as_os_str().is_empty() => {
                return Err(FsError::with_message(
                    FsErrorKind::Unsupported,
                    "restore",
                    Some(&src),
                    "the original location is unknown (trashed by another application); choose a destination",
                ));
            }
            None => item.original.clone(),
        };
        if crate::list::exists_nofollow(&dst) {
            return Err(FsError::new(FsErrorKind::AlreadyExists, "restore", Some(&dst)));
        }
        if let Some(parent) = dst.parent().filter(|p| !p.as_os_str().is_empty()) {
            crate::create::create_dir_all(parent)?;
        }
        crate::mv::rename(&src, &dst, crate::mv::RenameMode::NoReplace)
            .map_err(|e| FsError { op: "restore".into(), ..e })?;
        let _ = self.idx().remove(&src);
        Ok(dst)
    }

    pub(crate) fn purge(&self, item: &TrashItem) -> FsResult<()> {
        let p = self.payload(item)?;
        super::remove_any(&p)?;
        let _ = self.idx().remove(&p);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub(crate) mod discover {
    use std::path::{Path, PathBuf};

    use soyuz_core::FsResult;

    use super::MacBin;
    use crate::trash::TrashSupport;

    fn index_path() -> PathBuf {
        crate::path::home().join("Library").join("Application Support").join("soyuz").join("trash-index.json")
    }

    fn home_bin() -> MacBin {
        MacBin { root: crate::path::home().join(".Trash"), index: index_path() }
    }

    fn dev(p: &Path) -> Option<u64> {
        crate::list::stat(p, true).ok().and_then(|e| e.dev.map(|d| d.0))
    }

    pub(crate) fn bin_for(path: &Path) -> FsResult<MacBin> {
        let item_dev = crate::list::stat(path, false).ok().and_then(|e| e.dev.map(|d| d.0));
        if item_dev.is_some() && item_dev == dev(&crate::path::home()) {
            return Ok(home_bin());
        }
        let top = crate::volumes::volume_root(path)?;
        let uid = crate::sys::unix::euid();
        Ok(MacBin { root: top.join(".Trashes").join(uid.to_string()), index: index_path() })
    }

    pub(crate) fn support(path: &Path) -> TrashSupport {
        let item_dev = crate::list::stat(path, false).ok().and_then(|e| e.dev.map(|d| d.0));
        if item_dev.is_none() {
            return TrashSupport::Unsupported("the item does not exist".into());
        }
        if item_dev == dev(&crate::path::home()) {
            return TrashSupport::Native;
        }
        match crate::volumes::volume_root(path) {
            Ok(top) if crate::list::exists_nofollow(&top.join(".Trashes")) => TrashSupport::Native,
            Ok(top)
                if rustix::fs::accessat(
                    rustix::fs::CWD,
                    &top,
                    rustix::fs::Access::WRITE_OK,
                    rustix::fs::AtFlags::EACCESS,
                )
                .is_ok() =>
            {
                TrashSupport::Native
            }
            Ok(top) => TrashSupport::Unsupported(format!("{} has no trash and is not writable", top.display())),
            Err(e) => TrashSupport::Unsupported(e.message),
        }
    }

    pub(crate) fn all_bins() -> Vec<MacBin> {
        let mut out = vec![home_bin()];
        let uid = crate::sys::unix::euid();
        for v in crate::volumes::volumes() {
            let root = v.mount.join(".Trashes").join(uid.to_string());
            if v.mount != Path::new("/") && crate::list::exists_nofollow(&root) {
                out.push(MacBin { root, index: index_path() });
            }
        }
        out
    }

    pub(crate) fn bin_of(trash_root: &Path) -> Option<MacBin> {
        all_bins().into_iter().find(|b| b.root == trash_root)
    }
}

/// Empty every bin of `bins`.
pub(crate) fn empty_all(
    bins: &[MacBin],
    ctl: &OpControl,
    on_item: &mut dyn FnMut(&TrashItem, FsResult<()>) -> Control,
) -> FsResult<()> {
    for b in bins {
        for item in b.list() {
            ctl.checkpoint()?;
            let res = b.purge(&item);
            if on_item(&item, res) == Control::Stop {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_bin_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let bin = MacBin { root: d.path().join(".Trash"), index: d.path().join("idx").join("trash-index.json") };
        let f = d.path().join("a.txt");
        std::fs::write(&f, b"1").unwrap();
        let i1 = bin.trash(&f).unwrap();
        std::fs::write(&f, b"2").unwrap();
        let i2 = bin.trash(&f).unwrap();
        assert_eq!(i1.id, "a.txt");
        assert_eq!(i2.id, "a 2.txt");
        // An item put there by someone else has no known origin.
        std::fs::write(bin.root.join("foreign.bin"), b"zz").unwrap();
        std::fs::write(bin.root.join(".DS_Store"), b"").unwrap();
        let mut items = bin.list();
        items.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(items.len(), 3);
        let foreign = items.iter().find(|i| i.id == "foreign.bin").unwrap();
        assert!(foreign.original.as_os_str().is_empty());
        assert_eq!(bin.restore(foreign, None).unwrap_err().kind, FsErrorKind::Unsupported);
        assert_eq!(bin.restore(&i2, None).unwrap(), f);
        assert_eq!(std::fs::read(&f).unwrap(), b"2");
        bin.purge(&i1).unwrap();
        assert_eq!(bin.list().len(), 1);
        let ctl = OpControl::new();
        let mut n = 0;
        empty_all(std::slice::from_ref(&bin), &ctl, &mut |_, r| {
            r.unwrap();
            n += 1;
            Control::Continue
        })
        .unwrap();
        assert_eq!(n, 1);
        assert!(bin.list().is_empty());
    }
}

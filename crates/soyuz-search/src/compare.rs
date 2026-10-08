//! Folder compare (M12) — blocking, run on a worker thread.
//!
//! Rules (all paths relative to the roots, sorted, symlinks never followed):
//! - an entry present on one side only goes to `only_left` / `only_right`; for a missing
//!   folder only the folder itself is listed, not its contents (a sync copies it whole);
//! - two files are *equal* when their sizes match and either their modification times are
//!   within [`MTIME_TOLERANCE`] (`by_content == false`) or their bytes are identical
//!   (`by_content == true`); equal files only bump `same`;
//! - unequal files go to `newer_left` / `newer_right` when one side is newer by more than
//!   the tolerance, else to `differ`;
//! - a file on one side and a folder on the other goes to `differ` (contents of that
//!   folder are not listed); folders present on both sides are recursed, never counted;
//! - two symlinks are equal when their targets are equal.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use soyuz_core::{Control, EntryKind, FsError, FsErrorKind, FsResult, OpControl};
use soyuz_fs::{WalkItem, WalkOptions};

/// Modification times closer than this are considered equal (FAT has 2 s resolution).
pub const MTIME_TOLERANCE: Duration = Duration::from_secs(2);

/// All paths are relative to the compared roots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompareResult {
    pub only_left: Vec<PathBuf>,
    pub only_right: Vec<PathBuf>,
    pub newer_left: Vec<PathBuf>,
    pub newer_right: Vec<PathBuf>,
    pub differ: Vec<PathBuf>,
    pub same: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Dir,
    File,
    Link,
    Other,
}

#[derive(Clone, Debug)]
struct Meta {
    kind: Kind,
    size: u64,
    modified: Option<SystemTime>,
}

pub fn compare_dirs(a: &Path, b: &Path, by_content: bool, ctl: &OpControl) -> FsResult<CompareResult> {
    let left = scan(a, ctl)?;
    let right = scan(b, ctl)?;
    let mut res = CompareResult::default();
    // Folders whose contents must not be listed individually (missing or kind clash).
    let mut skip: Vec<PathBuf> = Vec::new();
    let under = |skip: &[PathBuf], rel: &Path| skip.iter().any(|s| rel.starts_with(s) && rel != s);

    for (rel, l) in &left {
        ctl.checkpoint()?;
        if under(&skip, rel) {
            continue;
        }
        let Some(r) = right.get(rel) else {
            res.only_left.push(rel.clone());
            if l.kind == Kind::Dir {
                skip.push(rel.clone());
            }
            continue;
        };
        match (l.kind, r.kind) {
            (Kind::Dir, Kind::Dir) => {}
            (lk, rk) if lk != rk => {
                res.differ.push(rel.clone());
                if lk == Kind::Dir || rk == Kind::Dir {
                    skip.push(rel.clone());
                }
            }
            (Kind::Link, _) => {
                let same = soyuz_fs::read_link(&a.join(rel)).ok() == soyuz_fs::read_link(&b.join(rel)).ok();
                if same {
                    res.same += 1;
                } else {
                    res.differ.push(rel.clone());
                }
            }
            _ => {
                let equal = l.size == r.size
                    && if by_content {
                        contents_equal(&a.join(rel), &b.join(rel), ctl)?
                    } else {
                        !newer(l.modified, r.modified) && !newer(r.modified, l.modified)
                    };
                if equal {
                    res.same += 1;
                } else if newer(l.modified, r.modified) {
                    res.newer_left.push(rel.clone());
                } else if newer(r.modified, l.modified) {
                    res.newer_right.push(rel.clone());
                } else {
                    res.differ.push(rel.clone());
                }
            }
        }
    }
    let mut orphan: Vec<PathBuf> = Vec::new();
    for (rel, r) in &right {
        ctl.checkpoint()?;
        if left.contains_key(rel) || under(&skip, rel) || under(&orphan, rel) {
            continue;
        }
        res.only_right.push(rel.clone());
        if r.kind == Kind::Dir {
            orphan.push(rel.clone());
        }
    }
    Ok(res)
}

/// `a` is newer than `b` by more than the tolerance.
fn newer(a: Option<SystemTime>, b: Option<SystemTime>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.duration_since(b).is_ok_and(|d| d > MTIME_TOLERANCE),
        _ => false,
    }
}

fn scan(root: &Path, ctl: &OpControl) -> FsResult<BTreeMap<PathBuf, Meta>> {
    let md = std::fs::metadata(root).map_err(|e| FsError::from_io("compare", Some(root), &e))?;
    if !md.is_dir() {
        return Err(FsError::new(FsErrorKind::NotDirectory, "compare", Some(root)));
    }
    let mut map = BTreeMap::new();
    soyuz_fs::walk(root, &WalkOptions::default(), ctl, &mut |item| {
        if let WalkItem::Entry { entry, depth } = item
            && depth > 0
            && let Ok(rel) = entry.path.strip_prefix(root)
        {
            let kind = match entry.kind {
                EntryKind::Dir => Kind::Dir,
                EntryKind::File => Kind::File,
                EntryKind::Symlink { .. } | EntryKind::Junction => Kind::Link,
                _ => Kind::Other,
            };
            map.insert(rel.to_path_buf(), Meta { kind, size: entry.size, modified: entry.modified });
        }
        if ctl.is_cancelled() { Control::Stop } else { Control::Continue }
    })?;
    ctl.checkpoint()?;
    Ok(map)
}

/// Byte-wise comparison (early exit on the first difference). I/O errors count as
/// "different"; a cancel is an error.
fn contents_equal(a: &Path, b: &Path, ctl: &OpControl) -> FsResult<bool> {
    const CHUNK: usize = 256 * 1024;
    let (Ok(mut fa), Ok(mut fb)) = (File::open(a), File::open(b)) else { return Ok(false) };
    let mut ba = vec![0u8; CHUNK];
    let mut bb = vec![0u8; CHUNK];
    loop {
        ctl.checkpoint()?;
        let Ok(na) = read_full(&mut fa, &mut ba) else { return Ok(false) };
        let Ok(nb) = read_full(&mut fb, &mut bb) else { return Ok(false) };
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
    }
}

fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

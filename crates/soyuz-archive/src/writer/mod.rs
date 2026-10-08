//! Archive writers (zip, tar.*, 7z).

mod sevenz;
mod tar;
pub(crate) mod zip;

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use soyuz_core::{OpControl, ProgressSink};

use crate::util::fs_err;
use crate::{ArchiveError, ArchiveFormat, CreateExtras, CreateOptions};

/// One filesystem object to store.
pub(crate) struct Item {
    pub src: PathBuf,
    /// `/`-separated archive path (no trailing `/`).
    pub name: String,
    pub kind: ItemKind,
    pub md: Metadata,
}

pub(crate) enum ItemKind {
    File { len: u64 },
    Dir,
    Symlink { target: String },
}

impl Item {
    pub fn modified(&self) -> Option<SystemTime> {
        self.md.modified().ok()
    }

    /// Unix mode bits when the platform has them.
    #[cfg(unix)]
    pub fn mode(&self) -> Option<u32> {
        use std::os::unix::fs::PermissionsExt;
        Some(self.md.permissions().mode())
    }

    #[cfg(not(unix))]
    pub fn mode(&self) -> Option<u32> {
        None
    }
}

pub(crate) fn create(
    dest: &Path,
    inputs: &[PathBuf],
    opts: &CreateOptions,
    extras: &CreateExtras,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<(), ArchiveError> {
    if !opts.format.can_write() {
        return Err(ArchiveError::Unsupported(opts.format));
    }
    let password = opts.password.as_deref().filter(|p| !p.is_empty());
    if password.is_some() && !opts.format.supports_password() {
        return Err(ArchiveError::Unsupported(opts.format));
    }
    let items = collect(inputs, opts, ctl)?;
    let mut part: OsString = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let level = opts.level.min(9);
    let res = match opts.format {
        ArchiveFormat::Zip => zip::write(&part, &items, level, password, extras.zip_method, ctl, sink),
        ArchiveFormat::SevenZ => sevenz::write(&part, &items, level, password, extras.encrypt_7z_headers, ctl, sink),
        f => tar::write(&part, &items, f, level, ctl, sink),
    };
    let res = res.map_err(|e| if ctl.is_cancelled() { ArchiveError::Cancelled } else { e });
    match res.and_then(|()| fs::rename(&part, dest).map_err(|e| fs_err("rename", dest, &e))) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Archive path of `p`: relative to `base` when possible, else its file name.
fn entry_name(p: &Path, base: &Path) -> String {
    let rel = match p.strip_prefix(base) {
        Ok(r) if !r.as_os_str().is_empty() => r.to_path_buf(),
        _ => PathBuf::from(p.file_name().unwrap_or(p.as_os_str())),
    };
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts.join("/")
}

/// Walk `inputs` (depth-first, children sorted by name) into a flat item list.
pub(crate) fn collect(inputs: &[PathBuf], opts: &CreateOptions, ctl: &OpControl) -> Result<Vec<Item>, ArchiveError> {
    let mut out = Vec::new();
    let mut seen_names = HashSet::new();
    for input in inputs {
        let name = entry_name(input, &opts.base_dir);
        let mut ancestors = HashSet::new();
        walk(input, name, opts, ctl, &mut out, &mut seen_names, &mut ancestors)?;
    }
    Ok(out)
}

fn walk(
    path: &Path,
    name: String,
    opts: &CreateOptions,
    ctl: &OpControl,
    out: &mut Vec<Item>,
    seen: &mut HashSet<String>,
    ancestors: &mut HashSet<PathBuf>,
) -> Result<(), ArchiveError> {
    ctl.checkpoint()?;
    if name.is_empty() {
        return Ok(());
    }
    let lmd = fs::symlink_metadata(path).map_err(|e| fs_err("archive", path, &e))?;
    let (md, kind) = if lmd.file_type().is_symlink() {
        if opts.follow_symlinks {
            match fs::metadata(path) {
                Ok(md) if md.is_dir() => (md, ItemKind::Dir),
                Ok(md) => {
                    let len = md.len();
                    (md, ItemKind::File { len })
                }
                Err(e) => {
                    log::warn!("skipping broken symlink {}: {e}", path.display());
                    return Ok(());
                }
            }
        } else if opts.store_symlinks {
            let target = fs::read_link(path).map_err(|e| fs_err("read_link", path, &e))?;
            let target = target.to_string_lossy().replace('\\', "/");
            (lmd, ItemKind::Symlink { target })
        } else {
            log::debug!("skipping symlink {}", path.display());
            return Ok(());
        }
    } else if lmd.is_dir() {
        (lmd, ItemKind::Dir)
    } else if lmd.is_file() {
        let len = lmd.len();
        (lmd, ItemKind::File { len })
    } else {
        log::debug!("skipping special file {}", path.display());
        return Ok(());
    };
    if !seen.insert(name.clone()) {
        log::warn!("duplicate archive path {name}, skipping {}", path.display());
        return Ok(());
    }
    let is_dir = matches!(kind, ItemKind::Dir);
    out.push(Item { src: path.to_path_buf(), name: name.clone(), kind, md });
    if is_dir {
        // Loop protection when following symlinks.
        let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !ancestors.insert(canon.clone()) {
            log::warn!("symlink loop at {}, not descending", path.display());
            return Ok(());
        }
        let mut children: Vec<(OsString, PathBuf)> = fs::read_dir(path)
            .map_err(|e| fs_err("read_dir", path, &e))?
            .map(|de| de.map(|d| (d.file_name(), d.path())))
            .collect::<Result<_, _>>()
            .map_err(|e| fs_err("read_dir", path, &e))?;
        children.sort();
        for (fname, child) in children {
            let child_name = format!("{name}/{}", fname.to_string_lossy());
            walk(&child, child_name, opts, ctl, out, seen, ancestors)?;
        }
        ancestors.remove(&canon);
    }
    Ok(())
}

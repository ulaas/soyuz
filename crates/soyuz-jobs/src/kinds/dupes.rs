//! Duplicate-aware copy (`CopyOptions::duplicates`): before a copy job moves any bytes,
//! the files it copies that share a size are hashed (BLAKE3), and every file whose
//! content already reached the destination earlier in the job becomes a copy-on-write
//! clone of that copy ([`Duplicates::Clone`]: ReFS / Dev Drive block cloning, btrfs / XFS
//! reflinks, APFS clones) or a hard link to it ([`Duplicates::Link`]) instead of being
//! copied again. Only when the destination volume can do it: elsewhere (NTFS, ext4 for
//! clones) nothing is hashed and the job copies as usual. A clone or link that fails
//! falls back to a normal copy.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use soyuz_core::{Control, EntryInfo, EntryKind, FsResult, HashAlgo, Overwrite};
use soyuz_fs::{CopyOptions, Duplicates, FsCaps, Reflink, TimesSet, WalkItem, WalkOptions};

use crate::ctx::JobCtx;
use crate::job::LogLevel;

/// Files smaller than this are always copied (a clone or link saves little).
pub const MIN_SIZE: u64 = 64 * 1024;

/// The copied set's duplicate groups and, per group, the first copy made.
#[derive(Debug, Default)]
pub(crate) struct DupeIndex {
    mode: Duplicates,
    /// Source file → its group.
    group: HashMap<PathBuf, usize>,
    /// Group → the destination of its first finished copy.
    first: RefCell<HashMap<usize, PathBuf>>,
    /// Files cloned / linked and the bytes not copied.
    saved: RefCell<(u64, u64)>,
}

/// Whether the volume of `dest` can take `mode`.
fn supported(dest: &Path, mode: Duplicates) -> bool {
    let need = match mode {
        Duplicates::Copy => return false,
        Duplicates::Clone => FsCaps::REFLINK,
        Duplicates::Link => FsCaps::HARDLINK,
    };
    let Ok(root) = soyuz_fs::volume_root(dest) else { return false };
    let root = soyuz_fs::strip_verbatim(&root).to_path_buf();
    soyuz_fs::volumes()
        .into_iter()
        .find(|v| soyuz_fs::strip_verbatim(&v.mount) == root.as_path())
        .is_some_and(|v| v.supports.contains(need))
}

impl DupeIndex {
    /// Group the files under `sources` by content (`None`: nothing to do: duplicates are
    /// copied, the destination can't clone / link, or no two files are equal).
    pub(crate) fn build(ctx: &JobCtx, sources: &[PathBuf], dest: &Path) -> FsResult<Option<DupeIndex>> {
        let mode = ctx.opts.copy.duplicates;
        if mode == Duplicates::Copy || !supported(dest, mode) {
            return Ok(None);
        }
        // Same size first; only those are read.
        let mut by_size: BTreeMap<u64, Vec<PathBuf>> = BTreeMap::new();
        let opts = WalkOptions { follow_symlinks: ctx.opts.copy.follow_symlinks, ..WalkOptions::default() };
        for src in sources {
            let _ = soyuz_fs::walk(src, &opts, ctx.ctl(), &mut |item| {
                if let WalkItem::Entry { entry, .. } = item
                    && entry.kind == EntryKind::File
                    && entry.size >= MIN_SIZE
                {
                    by_size.entry(entry.size).or_default().push(entry.path.clone());
                }
                if ctx.ctl().is_cancelled() { Control::Stop } else { Control::Continue }
            });
            ctx.checkpoint()?;
        }
        let mut by_hash: HashMap<(u64, String), Vec<PathBuf>> = HashMap::new();
        for (size, paths) in by_size.into_iter().filter(|(_, p)| p.len() > 1) {
            for p in paths {
                ctx.checkpoint()?;
                match soyuz_fs::hash_file(&p, HashAlgo::Blake3, ctx.ctl(), &mut ()) {
                    Ok(h) => by_hash.entry((size, h)).or_default().push(p),
                    Err(e) if e.is_cancel() => return Err(e),
                    // Unreadable: the copy reports it.
                    Err(_) => {}
                }
            }
        }
        let mut group = HashMap::new();
        for (n, paths) in by_hash.into_values().filter(|p| p.len() > 1).enumerate() {
            for p in paths {
                group.insert(p, n);
            }
        }
        if group.is_empty() {
            return Ok(None);
        }
        ctx.log(LogLevel::Info, None, format!("{} files have a duplicate among the copied files", group.len()));
        Ok(Some(DupeIndex { mode, group, ..DupeIndex::default() }))
    }

    /// An earlier copy of `src`'s content in the destination.
    fn earlier(&self, src: &Path) -> Option<PathBuf> {
        let g = self.group.get(src)?;
        self.first.borrow().get(g).cloned()
    }

    /// `src` was copied to `dst`: later duplicates clone / link from it.
    pub(crate) fn copied(&self, src: &Path, dst: &Path) {
        if let Some(g) = self.group.get(src) {
            self.first.borrow_mut().entry(*g).or_insert_with(|| dst.to_path_buf());
        }
    }

    /// Make `dst` (which does not exist) a clone of / link to an earlier copy of `entry`'s
    /// content. `false`: there is none, or it failed (logged; copy normally).
    pub(crate) fn place(&self, ctx: &JobCtx, entry: &EntryInfo, dst: &Path) -> bool {
        let Some(from) = self.earlier(&entry.path) else { return false };
        // The earlier copy may have been changed or replaced since.
        if soyuz_fs::stat(&from, false).map_or(true, |e| e.size != entry.size) {
            return false;
        }
        let res = match self.mode {
            Duplicates::Copy => return false,
            Duplicates::Clone => {
                let opts = CopyOptions {
                    overwrite: Overwrite::Never,
                    reflink: Reflink::Always,
                    verify: None,
                    ..ctx.opts.copy.clone()
                };
                soyuz_fs::copy_file(&from, dst, &opts, ctx.ctl(), &mut ()).map(|_| ())
            }
            Duplicates::Link => soyuz_fs::hardlink(&from, dst),
        };
        match res {
            Ok(()) => {
                if self.mode == Duplicates::Clone {
                    // A clone carries the earlier copy's times; this file keeps its own.
                    let t = TimesSet { modified: entry.modified, created: entry.created, accessed: entry.accessed };
                    if let Err(e) = soyuz_fs::set_times(dst, &t, false) {
                        ctx.log(LogLevel::Warn, Some(dst.to_path_buf()), format!("times not copied: {e}"));
                    }
                }
                let mut s = self.saved.borrow_mut();
                s.0 += 1;
                s.1 += entry.size;
                ctx.log(LogLevel::Info, Some(dst.to_path_buf()), format!("same content as {}", from.display()));
                true
            }
            Err(e) => {
                ctx.log(LogLevel::Warn, Some(dst.to_path_buf()), format!("not cloned / linked ({e}); copied instead"));
                false
            }
        }
    }

    /// Files cloned / linked and the bytes they did not copy.
    pub(crate) fn saved(&self) -> (u64, u64) {
        *self.saved.borrow()
    }
}

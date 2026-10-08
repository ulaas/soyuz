//! Planner: walks sources (totals, device ids, free space, MoveIntoSelf) before execution.

use std::path::{Component, Path, PathBuf};

use soyuz_core::{Control, EntryInfo, EntryKind, FsError, FsErrorKind, FsResult};
use soyuz_fs::{WalkItem, WalkOptions};

use crate::ctx::{JobCtx, nearest_existing};

/// One top-level source of a copy / move.
#[derive(Clone, Debug)]
pub(crate) struct SourcePlan {
    pub path: PathBuf,
    pub entry: EntryInfo,
    /// File bytes in the tree (0 for a same-device move).
    pub bytes: u64,
    /// Entries in the tree (1 for a same-device move).
    pub items: u64,
    /// Bytes must be transferred (always for copy; cross-device or forced for move).
    pub transfer: bool,
}

/// `true` for entries whose bytes are copied (not dirs / links).
pub(crate) fn has_bytes(kind: EntryKind) -> bool {
    !matches!(kind, EntryKind::Dir | EntryKind::Symlink { .. } | EntryKind::Junction)
}

/// Totals of a tree: `(file bytes, entries)`. Walk errors are ignored here; they are
/// reported (with the error policy) during execution.
pub(crate) fn tree_totals(ctx: &JobCtx, root: &Path, follow: bool) -> FsResult<(u64, u64)> {
    let mut bytes = 0u64;
    let mut items = 0u64;
    let opts = WalkOptions { follow_symlinks: follow, ..WalkOptions::default() };
    soyuz_fs::walk(root, &opts, ctx.ctl(), &mut |item| {
        if let WalkItem::Entry { entry, .. } = item {
            items += 1;
            if has_bytes(entry.kind) {
                bytes += entry.size;
            }
            if items.is_multiple_of(512) {
                ctx.progress(|p| p.items_total = p.items_total.max(items));
            }
        }
        Control::Continue
    })?;
    Ok((bytes, items))
}

/// Stat every path with the error policy; skipped paths are accounted as failed items.
pub(crate) fn stat_all(ctx: &JobCtx, paths: &[PathBuf]) -> FsResult<Vec<(PathBuf, EntryInfo)>> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        ctx.checkpoint()?;
        match ctx.attempt(p, || soyuz_fs::stat(p, false))? {
            Ok(e) => out.push((p.clone(), e)),
            Err(e) => ctx.item_failed(p, e, 0),
        }
    }
    Ok(out)
}

/// Plan a copy (`is_move == false`) or move: stat sources, reject moving/copying a folder
/// into itself, count totals, check free space. Sets the job's progress totals.
pub(crate) fn plan_transfer(
    ctx: &JobCtx,
    sources: &[PathBuf],
    dest_dir: &Path,
    rename_to: Option<&str>,
    is_move: bool,
) -> FsResult<Vec<SourcePlan>> {
    if rename_to.is_some() && sources.len() != 1 {
        return Err(FsError::with_message(
            FsErrorKind::InvalidName,
            "plan",
            None,
            "rename_to requires exactly one source",
        ));
    }
    let verb = if is_move { "move" } else { "copy" };
    let dest_probe = nearest_existing(dest_dir);
    let dest_dev = ctx.probe().device_of(&dest_probe).ok();
    let follow = ctx.opts.copy.follow_symlinks;
    let mut plans = Vec::new();
    let mut needed = 0u64;
    let mut items_total = 0u64;
    for (path, entry) in stat_all(ctx, sources)? {
        if entry.is_dir_like() && is_within(dest_dir, &path) {
            return Err(FsError::with_message(
                FsErrorKind::InvalidName,
                "plan",
                Some(&path),
                format!("cannot {verb} \u{201c}{}\u{201d} into itself", entry.name),
            ));
        }
        let transfer = !is_move || ctx.opts.mv.force_cross_device || {
            let src_dev = ctx.probe().device_of(&path).ok();
            src_dev.is_none() || dest_dev.is_none() || src_dev != dest_dev
        };
        let (bytes, items) = if transfer { tree_totals(ctx, &path, follow)? } else { (0, 1) };
        if transfer {
            needed += bytes;
        }
        items_total += items;
        plans.push(SourcePlan { path, entry, bytes, items, transfer });
    }
    let failed = ctx.summary_counts().0;
    ctx.set_totals(needed, items_total + failed);
    ctx.check_space(needed, dest_dir)?;
    Ok(plans)
}

/// Comparable components (case-folded where the usual filesystems are case-insensitive).
fn components(p: &Path) -> Vec<String> {
    soyuz_fs::normalize(p)
        .components()
        .filter(|c| !matches!(c, Component::RootDir))
        .map(|c| {
            let s = c.as_os_str().to_string_lossy();
            if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s.into_owned() }
        })
        .collect()
}

/// `child == ancestor` or `child` is inside `ancestor` (lexically, after normalization).
pub(crate) fn is_within(child: &Path, ancestor: &Path) -> bool {
    let c = components(child);
    let a = components(ancestor);
    c.len() >= a.len() && c[..a.len()] == a[..]
}

/// Same path after normalization (case-folded on Windows/macOS).
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    components(a) == components(b)
}

/// Key for comparing paths in batch renames.
pub(crate) fn path_key(p: &Path) -> Vec<String> {
    components(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within() {
        let base = std::env::temp_dir();
        assert!(is_within(&base.join("a").join("b"), &base.join("a")));
        assert!(is_within(&base.join("a"), &base.join("a")));
        assert!(!is_within(&base.join("a"), &base.join("a").join("b")));
        assert!(!is_within(&base.join("ab"), &base.join("a")));
        assert!(same_path(&base.join("x").join("..").join("y"), &base.join("y")));
    }
}

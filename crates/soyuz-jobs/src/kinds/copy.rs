//! `copy` job executor and the tree-transfer engine shared with cross-device moves.

use std::path::{Path, PathBuf};

use soyuz_core::{
    ByteProgress, ConflictResolution, Control, EntryInfo, EntryKind, FsError, FsErrorKind, FsResult, LinkTarget,
    Overwrite,
};
use soyuz_fs::{CopyOutcome, WalkItem, WalkOptions};

use super::dupes::DupeIndex;
use crate::ctx::JobCtx;
use crate::planner::{self, has_bytes};
use crate::policy::resolution_to_overwrite;
use crate::prompt::{ConflictInfo, ConflictKind};
use crate::undo::UndoRecord;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TransferMode {
    Copy,
    /// Cross-device move: records `Moved`, sources are deleted by the caller.
    Move,
}

/// Result of [`transfer_tree`].
#[derive(Debug, Default)]
pub(crate) struct TreeResult {
    /// Source files / links that were fully copied (safe to delete for a move).
    pub copied: Vec<PathBuf>,
    /// Source directories in post-order (children before parents).
    pub dirs: Vec<PathBuf>,
}

pub(crate) fn run_copy(ctx: &JobCtx, sources: &[PathBuf], dest_dir: &Path, rename_to: Option<&str>) -> FsResult<()> {
    let plans = planner::plan_transfer(ctx, sources, dest_dir, rename_to, false)?;
    ctx.set_running();
    if !ensure_dir(ctx, dest_dir)? {
        return Ok(());
    }
    let roots: Vec<PathBuf> = plans.iter().map(|p| p.path.clone()).collect();
    let dupes = DupeIndex::build(ctx, &roots, dest_dir)?;
    for p in plans {
        ctx.checkpoint()?;
        let name = rename_to.map(str::to_string).unwrap_or_else(|| p.entry.name.clone());
        transfer_tree(ctx, &p.path, dest_dir.join(name), TransferMode::Copy, None, dupes.as_ref())?;
    }
    if let Some((files, bytes)) = dupes.as_ref().map(DupeIndex::saved).filter(|(n, _)| *n > 0) {
        let how = if ctx.opts.copy.duplicates == soyuz_fs::Duplicates::Link { "hard-linked" } else { "cloned" };
        ctx.note(format!("{files} duplicate files {how} instead of copied ({bytes} bytes)"));
    }
    Ok(())
}

/// Create `dir` (and parents) if missing. `Ok(false)` = the user skipped it.
pub(crate) fn ensure_dir(ctx: &JobCtx, dir: &Path) -> FsResult<bool> {
    if dir.is_dir() {
        return Ok(true);
    }
    match ctx.attempt(dir, || soyuz_fs::create_dir_all(dir))? {
        Ok(()) => Ok(true),
        Err(e) => {
            ctx.item_failed(dir, e, 0);
            Ok(false)
        }
    }
}

/// Stat for conflict dialogs (falls back to a minimal entry).
pub(crate) fn entry_of(p: &Path) -> EntryInfo {
    soyuz_fs::stat(p, false).unwrap_or_else(|_| EntryInfo::minimal(p.to_path_buf(), EntryKind::Unknown))
}

pub(crate) fn suggested_name(dst: &Path) -> String {
    let parent = dst.parent().unwrap_or(Path::new(""));
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let p = soyuz_core::unique_name(parent, &name, "", &|p: &Path| soyuz_fs::exists_nofollow(p));
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name)
}

pub(crate) fn unique_path(dst: &Path) -> PathBuf {
    let parent = dst.parent().unwrap_or(Path::new(""));
    parent.join(suggested_name(dst))
}

/// Conflict kind between an existing `dst` and `src`.
pub(crate) fn conflict_kind(src: &EntryInfo, dst: &EntryInfo) -> ConflictKind {
    if soyuz_fs::same_file(&src.path, &dst.path).unwrap_or(false) {
        if src.name != dst.name && src.name.to_lowercase() == dst.name.to_lowercase() {
            return ConflictKind::CaseOnly;
        }
        return ConflictKind::SameFile;
    }
    match (is_dir(src), is_dir(dst)) {
        (true, true) => ConflictKind::DirExists,
        (false, false) => ConflictKind::FileExists,
        _ => ConflictKind::FileVsDir,
    }
}

fn is_dir(e: &EntryInfo) -> bool {
    matches!(e.kind, EntryKind::Dir | EntryKind::Junction | EntryKind::Symlink { target: LinkTarget::Dir })
}

/// Build the conflict prompt payload.
pub(crate) fn conflict_info(src: &EntryInfo, dst: &Path) -> ConflictInfo {
    let dst_entry = entry_of(dst);
    let kind = conflict_kind(src, &dst_entry);
    ConflictInfo { src: src.clone(), dst: dst_entry, kind, suggested_name: suggested_name(dst) }
}

/// Copy `src_root` (file, link or directory tree) to `dst_root` with conflict handling,
/// progress, error policy and journaling. `root_resolution` pre-answers a conflict at the
/// root (used when the caller already asked, e.g. a move that fell back to copying).
/// With `dupes`, files whose content was copied before become clones of / links to that
/// copy (`kinds::dupes`).
pub(crate) fn transfer_tree(
    ctx: &JobCtx,
    src_root: &Path,
    dst_root: PathBuf,
    mode: TransferMode,
    root_resolution: Option<ConflictResolution>,
    dupes: Option<&DupeIndex>,
) -> FsResult<TreeResult> {
    let mut result = TreeResult::default();
    let mut fatal: Option<FsError> = None;
    // dir_map[d] = destination of the directory currently open at depth d:
    // Some((dst, created_new)) or None when that subtree is skipped.
    let mut dir_map: Vec<Option<(PathBuf, bool)>> = Vec::new();
    // A move never follows links: the copied sources are deleted afterwards, and deleting
    // through a followed link would destroy the link target's contents.
    let follow = mode == TransferMode::Copy && ctx.opts.copy.follow_symlinks;
    let opts = WalkOptions { follow_symlinks: follow, post_order: true, ..WalkOptions::default() };
    let mut root_res = root_resolution;
    let walk_res = soyuz_fs::walk(src_root, &opts, ctx.ctl(), &mut |item| {
        let step = match item {
            WalkItem::Entry { entry, depth } => {
                dir_map.truncate(depth);
                let (dst, parent_new) = if depth == 0 {
                    (dst_root.clone(), false)
                } else {
                    match dir_map.get(depth - 1).cloned().flatten() {
                        Some((parent, new)) => (parent.join(&entry.name), new),
                        None => {
                            // Inside a skipped directory: account silently.
                            let credit = if has_bytes(entry.kind) { entry.size } else { 0 };
                            ctx.progress(|p| {
                                p.items_done += 1;
                                p.bytes_done += credit;
                            });
                            if is_walk_dir(&entry, follow) {
                                dir_map.push(None);
                            }
                            return Control::Continue;
                        }
                    }
                };
                let pre = if depth == 0 { root_res.take() } else { None };
                if is_walk_dir(&entry, follow) {
                    match transfer_dir(ctx, &entry, dst, parent_new, mode, pre) {
                        Ok(slot) => {
                            dir_map.push(slot);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    transfer_file(ctx, &entry, dst, parent_new, mode, pre, &mut result, dupes)
                }
            }
            WalkItem::DirDone { path, depth } => {
                if let Some(Some((dst, _))) = dir_map.get(depth)
                    && let Err(e) = soyuz_fs::copy_dir_metadata(&path, dst, ctx.opts.copy.preserve)
                {
                    ctx.log(crate::job::LogLevel::Warn, Some(dst.clone()), format!("folder metadata not copied: {e}"));
                }
                if dir_map.get(depth).map(|s| s.is_some()).unwrap_or(false) {
                    result.dirs.push(path);
                }
                Ok(())
            }
            WalkItem::Error { path, error } => match ctx.on_error(&path, &error) {
                Ok(_) => {
                    ctx.item_failed(&path, error, 0);
                    Ok(())
                }
                Err(e) => Err(e),
            },
        };
        match step {
            Ok(()) => Control::Continue,
            Err(e) => {
                fatal = Some(e);
                Control::Stop
            }
        }
    });
    if let Some(e) = fatal {
        return Err(e);
    }
    match walk_res {
        Ok(_) => Ok(result),
        Err(e) if e.is_cancel() => Err(e),
        Err(e) => {
            // The root itself vanished / is unreadable.
            ctx.on_error(src_root, &e)?;
            ctx.item_failed(src_root, e, 0);
            Ok(result)
        }
    }
}

fn is_walk_dir(e: &EntryInfo, follow: bool) -> bool {
    match e.kind {
        EntryKind::Dir => true,
        EntryKind::Symlink { target: LinkTarget::Dir } => follow,
        _ => false,
    }
}

fn new_root_record(mode: TransferMode, src: &Path, dst: &Path) -> UndoRecord {
    match mode {
        TransferMode::Copy => UndoRecord::Copied { dst: dst.to_path_buf() },
        TransferMode::Move => UndoRecord::Moved { from: src.to_path_buf(), to: dst.to_path_buf() },
    }
}

/// Create or merge a destination directory. Returns the dir_map slot.
fn transfer_dir(
    ctx: &JobCtx,
    entry: &EntryInfo,
    mut dst: PathBuf,
    parent_new: bool,
    mode: TransferMode,
    mut pre: Option<ConflictResolution>,
) -> FsResult<Option<(PathBuf, bool)>> {
    ctx.begin_item(&entry.path, 0);
    loop {
        ctx.checkpoint()?;
        if !soyuz_fs::exists_nofollow(&dst) {
            return match ctx.attempt(&dst, || soyuz_fs::create_dir(&dst))? {
                Ok(()) => {
                    if !parent_new {
                        ctx.record(new_root_record(mode, &entry.path, &dst));
                    }
                    ctx.item_ok(0);
                    Ok(Some((dst, true)))
                }
                Err(e) => {
                    ctx.item_failed(&entry.path, e, 0);
                    Ok(None)
                }
            };
        }
        let info = conflict_info(entry, &dst);
        let kind = info.kind;
        if kind == ConflictKind::DirExists && ctx.opts.copy.merge_dirs && pre.is_none() {
            ctx.item_ok(0);
            return Ok(Some((dst, false)));
        }
        let res = match pre.take() {
            Some(r) => r,
            None => ctx.resolve_conflict(info)?,
        };
        match res {
            ConflictResolution::Skip => {
                ctx.item_skipped(&entry.path, 0, "destination exists");
                return Ok(None);
            }
            ConflictResolution::KeepBoth => dst = unique_path(&dst),
            ConflictResolution::Rename(name) => dst = dst.with_file_name(name),
            ConflictResolution::Cancel => return Err(ctx.cancel_now()),
            // Overwrite* / MergeDir on a folder = merge.
            _ if kind == ConflictKind::DirExists => {
                ctx.item_ok(0);
                return Ok(Some((dst, false)));
            }
            _ => {
                let why = match kind {
                    ConflictKind::SameFile | ConflictKind::CaseOnly => "source and destination are the same folder",
                    _ => "cannot replace a file with a folder",
                };
                let err = FsError::with_message(FsErrorKind::AlreadyExists, "copy", Some(&dst), why);
                ctx.item_failed(&entry.path, err, 0);
                return Ok(None);
            }
        }
    }
}

/// Copy one non-directory entry (file, symlink, junction, special).
#[allow(clippy::too_many_arguments)]
fn transfer_file(
    ctx: &JobCtx,
    entry: &EntryInfo,
    mut dst: PathBuf,
    parent_new: bool,
    mode: TransferMode,
    mut pre: Option<ConflictResolution>,
    result: &mut TreeResult,
    dupes: Option<&DupeIndex>,
) -> FsResult<()> {
    let src = &entry.path;
    let size = if has_bytes(entry.kind) { entry.size } else { 0 };
    ctx.begin_item(src, size);
    let mut overwrite = Overwrite::Never;
    loop {
        ctx.checkpoint()?;
        if !soyuz_fs::exists_nofollow(&dst) {
            break;
        }
        let info = conflict_info(entry, &dst);
        let kind = info.kind;
        let res = match pre.take() {
            Some(r) => r,
            None => ctx.resolve_conflict(info)?,
        };
        match res {
            ConflictResolution::Skip | ConflictResolution::MergeDir => {
                ctx.item_skipped(src, size, "destination exists");
                return Ok(());
            }
            ConflictResolution::KeepBoth => dst = unique_path(&dst),
            ConflictResolution::Rename(name) => dst = dst.with_file_name(name),
            ConflictResolution::Cancel => return Err(ctx.cancel_now()),
            r => {
                match kind {
                    ConflictKind::SameFile | ConflictKind::CaseOnly => {
                        ctx.item_skipped(src, size, "source and destination are the same file");
                        return Ok(());
                    }
                    ConflictKind::FileVsDir | ConflictKind::DirExists | ConflictKind::MoveIntoSelf => {
                        let err = FsError::with_message(
                            FsErrorKind::IsDirectory,
                            "copy",
                            Some(&dst),
                            "cannot replace a folder with a file",
                        );
                        ctx.item_failed(src, err, size);
                        return Ok(());
                    }
                    ConflictKind::FileExists => {}
                }
                overwrite = resolution_to_overwrite(&r).unwrap_or(Overwrite::Always);
                break;
            }
        }
    }
    // Same content as a file copied earlier in this job: a clone of / link to that copy.
    if overwrite == Overwrite::Never
        && entry.kind == EntryKind::File
        && let Some(d) = dupes
        && d.place(ctx, entry, &dst)
    {
        ctx.credit_bytes(size as i64);
        if !parent_new {
            ctx.record(new_root_record(mode, src, &dst));
        }
        ctx.item_ok(size);
        result.copied.push(src.clone());
        return Ok(());
    }
    let mut copy_opts = ctx.opts.copy.clone();
    copy_opts.overwrite = overwrite;
    if mode == TransferMode::Move {
        copy_opts.follow_symlinks = false;
    }
    let attempt = ctx.attempt(src, || {
        let mut reported = 0u64;
        let res = {
            let mut sink = |bp: ByteProgress| {
                reported += bp.delta;
                ctx.on_bytes(bp);
            };
            soyuz_fs::copy_file(src, &dst, &copy_opts, ctx.ctl(), &mut sink)
        };
        if res.is_err() {
            ctx.credit_bytes(-(reported as i64));
        }
        res
    })?;
    match attempt {
        Ok(CopyOutcome::Copied { bytes, .. }) => {
            // Bytes not reported through the sink (e.g. symlinks) still complete the item.
            let reported = ctx.progress(|p| p.current_bytes);
            if size > reported {
                ctx.credit_bytes((size - reported) as i64);
            }
            if overwrite != Overwrite::Never {
                ctx.record(UndoRecord::Irreversible(dst.clone()));
            } else if !parent_new {
                ctx.record(new_root_record(mode, src, &dst));
            }
            ctx.item_ok(bytes);
            result.copied.push(src.clone());
            if let Some(d) = dupes {
                d.copied(src, &dst);
            }
        }
        Ok(CopyOutcome::Skipped(reason)) => {
            ctx.item_skipped(src, size, &format!("{reason:?}").to_lowercase());
        }
        Err(e) => ctx.item_failed(src, e, size),
    }
    Ok(())
}

//! `move` job executor: same-device rename, merge into existing folders, cross-device
//! copy + delete (a source is deleted only after it was fully copied).

use std::path::{Path, PathBuf};

use soyuz_core::{ConflictResolution, EntryInfo, FsError, FsErrorKind, FsResult};
use soyuz_fs::{MoveFast, MoveOptions, RenameMode};

use super::copy::{self, TransferMode};
use crate::ctx::JobCtx;
use crate::job::LogLevel;
use crate::planner::{self, SourcePlan, has_bytes};
use crate::prompt::ConflictKind;
use crate::undo::UndoRecord;

pub(crate) fn run_move(ctx: &JobCtx, sources: &[PathBuf], dest_dir: &Path, rename_to: Option<&str>) -> FsResult<()> {
    let plans = planner::plan_transfer(ctx, sources, dest_dir, rename_to, true)?;
    ctx.set_running();
    if !copy::ensure_dir(ctx, dest_dir)? {
        return Ok(());
    }
    for p in &plans {
        ctx.checkpoint()?;
        let name = rename_to.map(str::to_string).unwrap_or_else(|| p.entry.name.clone());
        move_item(ctx, &p.entry, dest_dir.join(name), Some(p))?;
    }
    Ok(())
}

/// Should an `OverwriteIf*` resolution replace `dst`?
fn condition_holds(r: &ConflictResolution, src: &EntryInfo, dst: &EntryInfo) -> bool {
    match r {
        ConflictResolution::OverwriteIfNewer => src.modified > dst.modified,
        ConflictResolution::OverwriteIfLarger => src.size > dst.size,
        ConflictResolution::OverwriteIfDifferent => src.size != dst.size,
        _ => true,
    }
}

/// Move one entry to `dst`. `plan` is set for top-level sources (their planned totals
/// are credited when a rename makes the transfer unnecessary).
pub(crate) fn move_item(ctx: &JobCtx, src: &EntryInfo, mut dst: PathBuf, plan: Option<&SourcePlan>) -> FsResult<()> {
    let src_path = &src.path;
    let (plan_items, plan_bytes) = plan.map(|p| (p.items, p.bytes)).unwrap_or((1, 0));
    let skip_credit = if plan.map(|p| p.transfer).unwrap_or(false) { plan_bytes } else { 0 };
    ctx.begin_item(src_path, 0);
    let mut replace = false;
    let mut root_resolution = None;
    loop {
        ctx.checkpoint()?;
        if planner::same_path(src_path, &dst) {
            ctx.item_skipped(src_path, skip_credit, "already at the destination");
            credit_rest(ctx, plan_items, 0);
            return Ok(());
        }
        if !soyuz_fs::exists_nofollow(&dst) {
            break;
        }
        let info = copy::conflict_info(src, &dst);
        let kind = info.kind;
        if kind == ConflictKind::CaseOnly {
            let new_name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match ctx.attempt(src_path, || soyuz_fs::rename_case_only(src_path, &new_name))? {
                Ok(()) => {
                    ctx.record(UndoRecord::Moved { from: src_path.clone(), to: dst.clone() });
                    ctx.items_ok(plan_items, 0);
                    ctx.credit_bytes(skip_credit as i64);
                }
                Err(e) => ctx.item_failed(src_path, e, skip_credit),
            }
            return Ok(());
        }
        if kind == ConflictKind::DirExists && ctx.opts.copy.merge_dirs && ctx.opts.conflict.is_none() {
            return merge_move(ctx, src, &dst, plan);
        }
        let dst_entry = info.dst.clone();
        let res = ctx.resolve_conflict(info)?;
        match res {
            ConflictResolution::Skip => {
                ctx.item_skipped(src_path, skip_credit, "destination exists");
                credit_rest(ctx, plan_items, 0);
                return Ok(());
            }
            ConflictResolution::KeepBoth => dst = copy::unique_path(&dst),
            ConflictResolution::Rename(name) => dst = dst.with_file_name(name),
            ConflictResolution::Cancel => return Err(ctx.cancel_now()),
            _ if kind == ConflictKind::DirExists => return merge_move(ctx, src, &dst, plan),
            ConflictResolution::MergeDir => {
                ctx.item_skipped(src_path, skip_credit, "destination exists");
                credit_rest(ctx, plan_items, 0);
                return Ok(());
            }
            r => {
                if kind != ConflictKind::FileExists {
                    let why = match kind {
                        ConflictKind::SameFile => "source and destination are the same file",
                        _ => "cannot replace a folder with a file or a file with a folder",
                    };
                    let err = FsError::with_message(FsErrorKind::AlreadyExists, "move", Some(&dst), why);
                    ctx.item_failed(src_path, err, skip_credit);
                    credit_rest(ctx, plan_items, 0);
                    return Ok(());
                }
                if !condition_holds(&r, src, &dst_entry) {
                    ctx.item_skipped(src_path, skip_credit, "destination is newer / larger / same size");
                    credit_rest(ctx, plan_items, 0);
                    return Ok(());
                }
                replace = true;
                root_resolution = Some(ConflictResolution::Overwrite);
                break;
            }
        }
    }
    let mv = MoveOptions {
        mode: if replace { RenameMode::Replace } else { RenameMode::NoReplace },
        force_cross_device: ctx.opts.mv.force_cross_device,
    };
    match ctx.attempt(src_path, || soyuz_fs::try_rename_or_crossdev(src_path, &dst, &mv))? {
        Ok(MoveFast::Renamed) => {
            if replace {
                ctx.record(UndoRecord::Irreversible(dst.clone()));
            } else {
                ctx.record(UndoRecord::Moved { from: src_path.clone(), to: dst.clone() });
            }
            ctx.items_ok(plan_items, 0);
            ctx.credit_bytes(skip_credit as i64);
        }
        Ok(MoveFast::NeedsCopy) => cross_device_move(ctx, src, dst, plan, root_resolution)?,
        Err(e) => {
            ctx.item_failed(src_path, e, skip_credit);
            credit_rest(ctx, plan_items, 0);
        }
    }
    Ok(())
}

/// Account the remaining planned items of a source that was handled as one unit.
fn credit_rest(ctx: &JobCtx, plan_items: u64, bytes: u64) {
    if plan_items > 1 || bytes > 0 {
        ctx.progress(|p| {
            p.items_done += plan_items.saturating_sub(1);
            p.bytes_done += bytes;
        });
    }
}

/// Copy the tree, then delete exactly the sources that were copied; never deletes
/// anything when cancelled midway.
fn cross_device_move(
    ctx: &JobCtx,
    src: &EntryInfo,
    dst: PathBuf,
    plan: Option<&SourcePlan>,
    root_resolution: Option<ConflictResolution>,
) -> FsResult<()> {
    let planned_transfer = plan.map(|p| p.transfer).unwrap_or(false);
    if !planned_transfer {
        // Planned as a rename: the tree was counted as one item.
        let (bytes, items) = planner::tree_totals(ctx, &src.path, false)?;
        ctx.grow_totals(bytes, items.saturating_sub(1));
        ctx.check_space(bytes, dst.parent().unwrap_or(&dst))?;
    }
    let tree = copy::transfer_tree(ctx, &src.path, dst, TransferMode::Move, root_resolution)?;
    ctx.checkpoint()?;
    ctx.set_finishing();
    for f in &tree.copied {
        let r = match soyuz_fs::stat(f, false) {
            Ok(e) if !has_bytes(e.kind) => soyuz_fs::delete_link(f),
            _ => soyuz_fs::delete_file(f, &soyuz_fs::DeleteOptions { clear_readonly: true, posix_semantics: true }),
        };
        if let Err(e) = r {
            ctx.log(LogLevel::Warn, Some(f.clone()), format!("source not removed after copy: {e}"));
        }
    }
    for d in &tree.dirs {
        // Folders still holding skipped / failed items stay.
        let _ = soyuz_fs::delete_empty_dir(d);
    }
    ctx.set_running();
    Ok(())
}

/// Move the children of `src` into the existing folder `dst`, then remove `src` if empty.
fn merge_move(ctx: &JobCtx, src: &EntryInfo, dst: &Path, plan: Option<&SourcePlan>) -> FsResult<()> {
    let children = match ctx.attempt(&src.path, || soyuz_fs::read_dir(&src.path, &soyuz_fs::ListOptions::default()))? {
        Ok(c) => c,
        Err(e) => {
            ctx.item_failed(&src.path, e, 0);
            return Ok(());
        }
    };
    if plan.map(|p| !p.transfer).unwrap_or(true) {
        ctx.grow_totals(0, children.len() as u64);
    }
    for child in &children {
        ctx.checkpoint()?;
        move_item(ctx, child, dst.join(&child.name), None)?;
    }
    if let Err(e) = soyuz_fs::delete_empty_dir(&src.path) {
        ctx.log(LogLevel::Info, Some(src.path.clone()), format!("source folder kept: {e}"));
    }
    ctx.item_ok(0);
    Ok(())
}

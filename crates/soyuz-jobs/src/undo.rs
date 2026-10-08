//! Undo journal records and the undo executor.
//!
//! Undo semantics (PLAN §2.4): `Copied`/`Extracted` delete the copies (to the trash when
//! supported, else permanently); `Created` likewise, but a non-empty folder is only ever
//! trashed, never deleted permanently; `Trashed` restores; `Moved`/`Renamed` reverse
//! (batched through the cycle-aware rename scheduler, cross-device moves fall back to
//! copy + delete); `MetaChanged` restores the snapshot; `Irreversible` makes the entry
//! not undoable (the engine refuses to undo it).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soyuz_core::{ConflictResolution, FsError, FsErrorKind, FsResult};
use soyuz_fs::{MetaSnapshot, MoveFast, MoveOptions, RenameMode, TrashItem, TrashSupport};

use crate::ctx::JobCtx;
use crate::kinds::{attrs, copy, delete, rename};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoRecord {
    Moved { from: PathBuf, to: PathBuf },
    Copied { dst: PathBuf },
    Renamed { from: PathBuf, to: PathBuf },
    Trashed(TrashItem),
    Created(PathBuf),
    Extracted { dst: PathBuf },
    MetaChanged { path: PathBuf, before: MetaSnapshot },
    Irreversible(PathBuf),
}

/// Records in the order they must be undone (last done, first undone).
pub(crate) fn undo_order(records: &[UndoRecord]) -> Vec<UndoRecord> {
    records.iter().rev().cloned().collect()
}

/// Paths an undo job touches (for overlap serialization).
pub(crate) fn touched_roots(records: &[UndoRecord]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for r in records {
        match r {
            UndoRecord::Moved { from, to } | UndoRecord::Renamed { from, to } => {
                out.push(from.clone());
                out.push(to.clone());
            }
            UndoRecord::Copied { dst } | UndoRecord::Extracted { dst } => out.push(dst.clone()),
            UndoRecord::Created(p) | UndoRecord::Irreversible(p) => out.push(p.clone()),
            UndoRecord::MetaChanged { path, .. } => out.push(path.clone()),
            UndoRecord::Trashed(item) => out.push(item.original.clone()),
        }
    }
    out
}

/// Execute an undo (records already in undo order).
pub(crate) fn run_undo(ctx: &JobCtx, records: &[UndoRecord]) -> FsResult<()> {
    ctx.set_totals(0, records.len() as u64);
    ctx.set_running();
    // Reverse moves/renames as one batch (handles a→b, b→a cycles).
    let pairs: Vec<(PathBuf, PathBuf)> = records
        .iter()
        .filter_map(|r| match r {
            UndoRecord::Moved { from, to } | UndoRecord::Renamed { from, to } => Some((to.clone(), from.clone())),
            _ => None,
        })
        .collect();
    if !pairs.is_empty() {
        for (_, back) in &pairs {
            if let Some(parent) = back.parent()
                && !parent.as_os_str().is_empty()
                && !parent.exists()
            {
                let _ = soyuz_fs::create_dir_all(parent);
            }
        }
        let no_record = |_: &Path, _: &Path| None;
        rename::run_pairs(ctx, &pairs, &move_prim, &no_record)?;
    }
    for r in records {
        ctx.checkpoint()?;
        match r {
            UndoRecord::Moved { .. } | UndoRecord::Renamed { .. } => {}
            UndoRecord::Copied { dst } | UndoRecord::Extracted { dst } => remove_created(ctx, dst, true)?,
            UndoRecord::Created(p) => remove_created(ctx, p, false)?,
            UndoRecord::Trashed(item) => {
                ctx.begin_item(&item.original, 0);
                match ctx.attempt(&item.original, || soyuz_fs::trash::restore(item, None))? {
                    Ok(_) => ctx.item_ok(0),
                    Err(e) => ctx.item_failed(&item.original, e, 0),
                }
            }
            UndoRecord::MetaChanged { path, before } => {
                ctx.begin_item(path, 0);
                match ctx.attempt(path, || attrs::restore_meta(path, before))? {
                    Ok(()) => ctx.item_ok(0),
                    Err(e) => ctx.item_failed(path, e, 0),
                }
            }
            UndoRecord::Irreversible(p) => ctx.item_skipped(p, 0, "cannot be undone"),
        }
    }
    Ok(())
}

/// Move `a` back to `b`: rename, or copy + delete across devices.
fn move_prim(ctx: &JobCtx, a: &Path, b: &Path, replace: bool) -> FsResult<()> {
    let o = MoveOptions {
        mode: if replace { RenameMode::Replace } else { RenameMode::NoReplace },
        force_cross_device: ctx.opts.mv.force_cross_device,
    };
    match soyuz_fs::try_rename_or_crossdev(a, b, &o)? {
        MoveFast::Renamed => Ok(()),
        MoveFast::NeedsCopy => {
            let pre = if replace { Some(ConflictResolution::Overwrite) } else { None };
            let (failed_before, skipped_before) = ctx.summary_counts();
            copy::transfer_tree(ctx, a, b.to_path_buf(), copy::TransferMode::Copy, pre)?;
            ctx.checkpoint()?;
            if ctx.summary_counts() != (failed_before, skipped_before) {
                return Err(FsError::with_message(
                    FsErrorKind::Io,
                    "undo",
                    Some(a),
                    "not everything could be copied back; the source was kept",
                ));
            }
            delete::remove_path_quiet(ctx, a)
        }
    }
}

/// Undo of something this job created: trash when supported, else delete permanently
/// (`allow_tree == false`: a non-empty folder is never deleted permanently).
fn remove_created(ctx: &JobCtx, p: &Path, allow_tree: bool) -> FsResult<()> {
    ctx.begin_item(p, 0);
    if !soyuz_fs::exists_nofollow(p) {
        ctx.item_skipped(p, 0, "already gone");
        return Ok(());
    }
    if ctx.probe().trash_support(p) == TrashSupport::Native {
        match soyuz_fs::trash::trash(p) {
            Ok(_) => {
                ctx.item_ok(0);
                return Ok(());
            }
            Err(e) if e.kind == FsErrorKind::Unsupported => {}
            Err(e) => {
                ctx.on_error(p, &e)?;
                ctx.item_failed(p, e, 0);
                return Ok(());
            }
        }
    }
    let r = ctx.attempt(p, || {
        let e = soyuz_fs::stat(p, false)?;
        if !allow_tree && e.kind == soyuz_core::EntryKind::Dir {
            soyuz_fs::delete_empty_dir(p)
        } else {
            delete::remove_path_quiet(ctx, p)
        }
    })?;
    match r {
        Ok(()) => ctx.item_ok(0),
        Err(e) => ctx.item_failed(p, e, 0),
    }
    Ok(())
}

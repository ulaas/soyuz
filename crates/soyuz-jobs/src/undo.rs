//! Undo journal records and the undo executor.
//!
//! Undo semantics (PLAN §2.4): `Copied`/`Extracted` delete the copies (to the trash when
//! supported, else permanently); `Created` likewise, but a non-empty folder is only ever
//! trashed, never deleted permanently; `Trashed` restores; `Moved`/`Renamed` reverse
//! (batched through the cycle-aware rename scheduler, cross-device moves fall back to
//! copy + delete); `MetaChanged` restores the snapshot; `Replaced` (an edited archive)
//! moves the new file to the trash and restores the backup from the trash;
//! `Irreversible` makes the entry not undoable (the engine refuses to undo it).
//!
//! Safety (review 1): a path is only removed when its identity still matches the
//! [`UndoStamp`] taken when the job finished (same object; same size and modification time
//! for files), otherwise the record fails with "changed since". A permanent delete (no
//! trash on that volume) of anything but an empty folder / empty file asks first through
//! the `TrashUnsupported` prompt. Every record that was actually reversed is reported with
//! [`JobCtx::mark_undone`] so a partially failed undo can drop exactly those records.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soyuz_core::{ConflictResolution, FsError, FsErrorKind, FsResult};
use soyuz_fs::{MetaSnapshot, MoveFast, MoveOptions, RenameMode, TrashItem, TrashSupport};

use crate::ctx::JobCtx;
use crate::history::UndoStamp;
use crate::kinds::{attrs, copy, delete, rename};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoRecord {
    Moved {
        from: PathBuf,
        to: PathBuf,
    },
    Copied {
        dst: PathBuf,
    },
    Renamed {
        from: PathBuf,
        to: PathBuf,
    },
    Trashed(TrashItem),
    Created(PathBuf),
    Extracted {
        dst: PathBuf,
    },
    MetaChanged {
        path: PathBuf,
        before: MetaSnapshot,
    },
    Irreversible(PathBuf),
    /// `path` was rewritten in place (F9 archive editing); `backup` is the original, in
    /// the trash.
    Replaced {
        path: PathBuf,
        backup: TrashItem,
    },
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
            UndoRecord::Replaced { path, .. } => out.push(path.clone()),
        }
    }
    out
}

/// Execute an undo (records already in undo order). `stamps` are the identities taken
/// when the original job finished (empty for entries written before stamps existed).
pub(crate) fn run_undo(ctx: &JobCtx, records: &[UndoRecord], stamps: &[UndoStamp]) -> FsResult<()> {
    ctx.set_totals(0, records.len() as u64);
    ctx.set_running();
    // Reverse moves/renames as one batch (handles a→b, b→a cycles).
    let (pair_recs, pairs): (Vec<&UndoRecord>, Vec<(PathBuf, PathBuf)>) = records
        .iter()
        .filter_map(|r| match r {
            UndoRecord::Moved { from, to } | UndoRecord::Renamed { from, to } => Some((r, (to.clone(), from.clone()))),
            _ => None,
        })
        .unzip();
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
        let mut done = Vec::new();
        let r = rename::run_pairs_tracked(ctx, &pairs, &move_prim, &no_record, &mut done);
        for (rec, ok) in pair_recs.iter().zip(&done) {
            if *ok {
                ctx.mark_undone((*rec).clone());
            }
        }
        r?;
    }
    let stamp_of = |p: &Path| stamps.iter().rev().find(|s| s.path == p);
    for r in records {
        ctx.checkpoint()?;
        let undone = match r {
            UndoRecord::Moved { .. } | UndoRecord::Renamed { .. } => false,
            UndoRecord::Copied { dst } | UndoRecord::Extracted { dst } => {
                remove_created(ctx, dst, true, stamp_of(dst))?
            }
            UndoRecord::Created(p) => remove_created(ctx, p, false, stamp_of(p))?,
            UndoRecord::Trashed(item) => {
                ctx.begin_item(&item.original, 0);
                match ctx.attempt(&item.original, || soyuz_fs::trash::restore(item, None))? {
                    Ok(_) => {
                        ctx.item_ok(0);
                        true
                    }
                    Err(e) => {
                        ctx.item_failed(&item.original, e, 0);
                        false
                    }
                }
            }
            UndoRecord::MetaChanged { path, before } => {
                ctx.begin_item(path, 0);
                match ctx.attempt(path, || attrs::restore_meta(path, before))? {
                    Ok(()) => {
                        ctx.item_ok(0);
                        true
                    }
                    Err(e) => {
                        ctx.item_failed(path, e, 0);
                        false
                    }
                }
            }
            UndoRecord::Irreversible(p) => {
                ctx.item_skipped(p, 0, "cannot be undone");
                false
            }
            UndoRecord::Replaced { path, backup } => {
                // The edited file goes to the trash (identity-checked), then the backup returns.
                if remove_created(ctx, path, false, stamp_of(path))? {
                    ctx.begin_item(path, 0);
                    match ctx.attempt(path, || soyuz_fs::trash::restore(backup, None))? {
                        Ok(_) => {
                            ctx.item_ok(0);
                            true
                        }
                        Err(e) => {
                            ctx.item_failed(path, e, 0);
                            false
                        }
                    }
                } else {
                    false
                }
            }
        };
        if undone {
            ctx.mark_undone(r.clone());
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
/// after asking (`allow_tree == false`: a non-empty folder is never deleted permanently).
/// Returns `true` when the path is gone afterwards (removed, or already gone).
fn remove_created(ctx: &JobCtx, p: &Path, allow_tree: bool, stamp: Option<&UndoStamp>) -> FsResult<bool> {
    ctx.begin_item(p, 0);
    if !soyuz_fs::exists_nofollow(p) {
        ctx.item_skipped(p, 0, "already gone");
        return Ok(true);
    }
    if let Some(s) = stamp
        && !UndoStamp::take(p).is_some_and(|now| s.matches(&now))
    {
        let e = FsError::with_message(
            FsErrorKind::Io,
            "undo",
            Some(p),
            "it changed after the operation (different item, size or modification time), so undo left it alone",
        );
        ctx.item_failed(p, e, 0);
        return Ok(false);
    }
    let support = ctx.probe().trash_support(p);
    let mut reason = match &support {
        TrashSupport::Native => None,
        TrashSupport::Unsupported(r) => Some(r.clone()),
    };
    if reason.is_none() {
        match soyuz_fs::trash::trash(p) {
            Ok(_) => {
                ctx.item_ok(0);
                return Ok(true);
            }
            Err(e) if e.kind == FsErrorKind::Unsupported => reason = Some(e.message),
            Err(e) => {
                ctx.on_error(p, &e)?;
                ctx.item_failed(p, e, 0);
                return Ok(false);
            }
        }
    }
    let entry = match soyuz_fs::stat(p, false) {
        Ok(e) => e,
        Err(e) => {
            ctx.item_failed(p, e, 0);
            return Ok(false);
        }
    };
    let is_dir = entry.kind == soyuz_core::EntryKind::Dir;
    // Nothing is lost by permanently removing an empty folder or an empty file (and a
    // `Created` folder is only ever removed when empty).
    let empty = if !allow_tree && is_dir {
        true
    } else if is_dir {
        soyuz_fs::read_dir(p, &soyuz_fs::ListOptions::default()).is_ok_and(|c| c.is_empty())
    } else {
        entry.size == 0 && crate::planner::has_bytes(entry.kind)
    };
    if !empty {
        let why = format!("{} — undo would delete it permanently", reason.unwrap_or_default());
        if !ctx.trash_unsupported(p, &why)? {
            ctx.item_skipped(p, 0, "kept: the trash is not available");
            return Ok(false);
        }
    }
    let r = ctx.attempt(p, || {
        if !allow_tree && is_dir { soyuz_fs::delete_empty_dir(p) } else { delete::remove_path_quiet(ctx, p) }
    })?;
    match r {
        Ok(()) => {
            ctx.item_ok(0);
            Ok(true)
        }
        Err(e) => {
            ctx.item_failed(p, e, 0);
            Ok(false)
        }
    }
}

/// Identities of what `records` produced (taken on the worker when the job finishes).
pub(crate) fn stamp_records(records: &[UndoRecord]) -> Vec<UndoStamp> {
    records
        .iter()
        .filter_map(|r| match r {
            UndoRecord::Copied { dst } | UndoRecord::Extracted { dst } => Some(dst),
            UndoRecord::Created(p) | UndoRecord::Replaced { path: p, .. } => Some(p),
            _ => None,
        })
        .filter_map(|p| UndoStamp::take(p))
        .collect()
}

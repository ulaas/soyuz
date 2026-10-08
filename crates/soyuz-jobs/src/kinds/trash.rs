//! Trash executors: move to trash (with the `TrashUnsupported` flow), restore, purge, empty.

use std::path::{Path, PathBuf};

use soyuz_core::{Control, FsErrorKind, FsResult};
use soyuz_fs::{TrashItem, TrashSupport};

use super::delete;
use crate::ctx::JobCtx;
use crate::undo::UndoRecord;

pub(crate) fn run_trash(ctx: &JobCtx, paths: &[PathBuf]) -> FsResult<()> {
    ctx.set_totals(0, paths.len() as u64);
    ctx.set_running();
    for p in paths {
        ctx.checkpoint()?;
        trash_one(ctx, p)?;
    }
    Ok(())
}

/// Trash `p`; when the volume has no trash, ask (permanent / skip / cancel).
pub(crate) fn trash_one(ctx: &JobCtx, p: &Path) -> FsResult<()> {
    ctx.begin_item(p, 0);
    if !soyuz_fs::exists_nofollow(p) {
        match ctx.attempt(p, || soyuz_fs::stat(p, false))? {
            Ok(_) => {}
            Err(e) => {
                ctx.item_failed(p, e, 0);
                return Ok(());
            }
        }
    }
    let mut reason = match ctx.probe().trash_support(p) {
        TrashSupport::Native => None,
        TrashSupport::Unsupported(r) => Some(r),
    };
    if reason.is_none() {
        let r = ctx.attempt(p, || match soyuz_fs::trash::trash(p) {
            Err(e) if e.kind == FsErrorKind::Unsupported => Ok(Err(e)),
            other => other.map(Ok),
        })?;
        match r {
            Ok(Ok(item)) => {
                ctx.record(UndoRecord::Trashed(item));
                ctx.item_ok(0);
                return Ok(());
            }
            Ok(Err(e)) => reason = Some(e.message),
            Err(e) => {
                ctx.item_failed(p, e, 0);
                return Ok(());
            }
        }
    }
    let reason = reason.unwrap_or_default();
    if !ctx.trash_unsupported(p, &reason)? {
        ctx.item_skipped(p, 0, "trash not available");
        return Ok(());
    }
    match ctx.attempt(p, || delete::remove_path_quiet(ctx, p))? {
        Ok(()) => {
            ctx.record(UndoRecord::Irreversible(p.to_path_buf()));
            ctx.item_ok(0);
        }
        Err(e) => ctx.item_failed(p, e, 0),
    }
    Ok(())
}

pub(crate) fn run_restore(ctx: &JobCtx, items: &[TrashItem], to: Option<&Path>) -> FsResult<()> {
    ctx.set_totals(items.iter().map(|i| i.size).sum(), items.len() as u64);
    ctx.set_running();
    for item in items {
        ctx.checkpoint()?;
        ctx.begin_item(&item.original, item.size);
        let dest = to.map(|t| t.join(soyuz_core::entry::display_name(&item.original)));
        match ctx.attempt(&item.original, || soyuz_fs::trash::restore(item, dest.as_deref()))? {
            Ok(restored) => {
                // Undo of a restore = trash it again (`Created` undo trashes when supported).
                ctx.record(UndoRecord::Created(restored));
                ctx.item_ok(item.size);
                ctx.credit_bytes(item.size as i64);
            }
            Err(e) => ctx.item_failed(&item.original, e, item.size),
        }
    }
    Ok(())
}

pub(crate) fn run_empty(ctx: &JobCtx) -> FsResult<()> {
    ctx.set_running();
    soyuz_fs::trash::empty(ctx.ctl(), &mut |item, r| {
        ctx.begin_item(&item.original, 0);
        match r {
            Ok(()) => ctx.item_ok(item.size),
            Err(e) => ctx.item_failed(&item.original, e, 0),
        }
        Control::Continue
    })
}

/// Permanently delete individual trash items (irreversible; not journaled for undo).
pub(crate) fn run_purge(ctx: &JobCtx, items: &[TrashItem]) -> FsResult<()> {
    ctx.set_totals(0, items.len() as u64);
    ctx.set_running();
    for item in items {
        ctx.checkpoint()?;
        ctx.begin_item(&item.original, 0);
        match ctx.attempt(&item.original, || soyuz_fs::trash::purge(item))? {
            Ok(()) => {
                ctx.record(UndoRecord::Irreversible(item.original.clone()));
                ctx.item_ok(item.size);
            }
            Err(e) => ctx.item_failed(&item.original, e, 0),
        }
    }
    Ok(())
}

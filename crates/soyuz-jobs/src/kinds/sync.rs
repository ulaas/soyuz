//! `sync` job executor (M12): apply a precomputed [`SyncPlan`] — copies overwrite the
//! other side, deletions go to the trash when available (else permanent, journaled as
//! irreversible).

use std::path::Path;

use soyuz_core::{ConflictResolution, FsResult};

use super::copy::{self, TransferMode};
use super::trash;
use crate::ctx::JobCtx;
use crate::job::{SyncAction, SyncPlan};
use crate::planner;

pub(crate) fn run_sync(ctx: &JobCtx, plan: &SyncPlan) -> FsResult<()> {
    let mut bytes = 0u64;
    let mut items = 0u64;
    for a in &plan.actions {
        match plan.copy_paths(a) {
            Some((src, _)) => {
                let (b, i) = planner::tree_totals(ctx, &src, false).unwrap_or((0, 1));
                bytes += b;
                items += i;
            }
            None => items += 1,
        }
    }
    ctx.set_totals(bytes, items);
    ctx.set_running();
    for a in &plan.actions {
        ctx.checkpoint()?;
        match a {
            SyncAction::DeleteL(rel) => delete_one(ctx, &plan.left.join(rel))?,
            SyncAction::DeleteR(rel) => delete_one(ctx, &plan.right.join(rel))?,
            copy => {
                if let Some((src, dst)) = plan.copy_paths(copy) {
                    copy_one(ctx, &src, &dst)?;
                }
            }
        }
    }
    Ok(())
}

fn copy_one(ctx: &JobCtx, src: &Path, dst: &Path) -> FsResult<()> {
    if let Some(parent) = dst.parent()
        && !copy::ensure_dir(ctx, parent)?
    {
        return Ok(());
    }
    copy::transfer_tree(ctx, src, dst.to_path_buf(), TransferMode::Copy, Some(ConflictResolution::Overwrite))?;
    Ok(())
}

fn delete_one(ctx: &JobCtx, p: &Path) -> FsResult<()> {
    if !soyuz_fs::exists_nofollow(p) {
        ctx.item_skipped(p, 0, "already gone");
        return Ok(());
    }
    trash::trash_one(ctx, p)
}

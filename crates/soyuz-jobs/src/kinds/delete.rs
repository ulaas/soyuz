//! `delete` job executor: permanent delete and secure wipe (trash lives in `trash.rs`).

use std::path::{Path, PathBuf};

use soyuz_core::{Attributes, ByteProgress, Control, EntryKind, ErrorAction, FsError, FsErrorKind, FsResult};
use soyuz_fs::{DeleteOptions, WalkItem, WalkOptions};

use crate::ctx::JobCtx;
use crate::job::DeleteMode;
use crate::planner;
use crate::undo::UndoRecord;

pub(crate) fn run_delete(ctx: &JobCtx, paths: &[PathBuf], mode: DeleteMode) -> FsResult<()> {
    match mode {
        DeleteMode::Trash => super::trash::run_trash(ctx, paths),
        DeleteMode::Permanent => run_permanent(ctx, paths, None),
        DeleteMode::Wipe { passes } => run_permanent(ctx, paths, Some(passes.max(1))),
    }
}

fn run_permanent(ctx: &JobCtx, paths: &[PathBuf], wipe: Option<u8>) -> FsResult<()> {
    let roots = planner::stat_all(ctx, paths)?;
    let mut bytes = 0u64;
    let mut items = ctx.summary_counts().0;
    for (p, _) in &roots {
        let (b, i) = planner::tree_totals(ctx, p, false)?;
        bytes += b;
        items += i;
    }
    let passes = wipe.map(u64::from).unwrap_or(0);
    ctx.set_totals(bytes * passes, items);
    ctx.set_running();
    for (p, _) in &roots {
        ctx.checkpoint()?;
        delete_tree(ctx, p, wipe)?;
        ctx.record(UndoRecord::Irreversible(p.clone()));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum What {
    File,
    Dir,
    Link,
}

/// Delete a tree (post-order, links never followed), one item at a time with the error
/// policy and the read-only prompt. `wipe` = overwrite files first.
pub(crate) fn delete_tree(ctx: &JobCtx, root: &Path, wipe: Option<u8>) -> FsResult<()> {
    let mut fatal: Option<FsError> = None;
    let opts = WalkOptions { follow_symlinks: false, post_order: true, ..WalkOptions::default() };
    let res = soyuz_fs::walk(root, &opts, ctx.ctl(), &mut |item| {
        let step = match item {
            WalkItem::Entry { entry, .. } => match entry.kind {
                EntryKind::Dir => Ok(()),
                EntryKind::Symlink { .. } | EntryKind::Junction => delete_one(ctx, &entry.path, What::Link, 0),
                _ => match wipe {
                    Some(passes) => wipe_one(ctx, &entry.path, passes, entry.size),
                    None => delete_one(ctx, &entry.path, What::File, 0),
                },
            },
            WalkItem::DirDone { path, .. } => delete_one(ctx, &path, What::Dir, 0),
            WalkItem::Error { path, error } => ctx.on_error(&path, &error).map(|_| ctx.item_failed(&path, error, 0)),
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
    match res {
        Ok(_) => Ok(()),
        Err(e) if e.is_cancel() => Err(e),
        Err(e) => {
            ctx.on_error(root, &e)?;
            ctx.item_failed(root, e, 0);
            Ok(())
        }
    }
}

fn is_readonly(p: &Path) -> bool {
    soyuz_fs::stat(p, false).map(|e| e.attrs.contains(Attributes::READONLY)).unwrap_or(false)
}

fn delete_one(ctx: &JobCtx, path: &Path, what: What, credit: u64) -> FsResult<()> {
    let mut force = ctx.opts.delete.clear_readonly;
    ctx.begin_item(path, 0);
    loop {
        ctx.checkpoint()?;
        let opts = DeleteOptions { clear_readonly: force, posix_semantics: ctx.opts.delete.posix_semantics };
        let r = match what {
            What::File => soyuz_fs::delete_file(path, &opts),
            What::Dir => soyuz_fs::delete_empty_dir(path),
            What::Link => soyuz_fs::delete_link(path),
        };
        match r {
            Ok(()) => {
                ctx.item_ok(0);
                ctx.credit_bytes(credit as i64);
                return Ok(());
            }
            Err(e) if ctx.ctl().is_cancelled() => return Err(e),
            Err(e) if !force && what == What::File && e.kind == FsErrorKind::PermissionDenied && is_readonly(path) => {
                if ctx.read_only_force(path)? {
                    force = true;
                    continue;
                }
                ctx.item_skipped(path, credit, "read-only");
                return Ok(());
            }
            Err(e) if what == What::Dir && e.kind == FsErrorKind::NotEmpty && ctx.summary_counts() != (0, 0) => {
                // Holds items that were skipped or failed: not a new error.
                ctx.item_skipped(path, credit, "folder still contains skipped items");
                return Ok(());
            }
            Err(e) => match ctx.on_error(path, &e)? {
                ErrorAction::Retry => continue,
                _ => {
                    ctx.item_failed(path, e, credit);
                    return Ok(());
                }
            },
        }
    }
}

fn wipe_one(ctx: &JobCtx, path: &Path, passes: u8, size: u64) -> FsResult<()> {
    let total = size * passes as u64;
    ctx.begin_item(path, total);
    let r = ctx.attempt(path, || {
        let mut reported = 0u64;
        let res = {
            let mut sink = |bp: ByteProgress| {
                reported += bp.delta;
                ctx.on_bytes(bp);
            };
            soyuz_fs::secure_wipe(path, passes, ctx.ctl(), &mut sink)
        };
        if res.is_err() {
            ctx.credit_bytes(-(reported as i64));
        }
        res
    })?;
    match r {
        Ok(()) => ctx.item_ok(total),
        Err(e) => ctx.item_failed(path, e, total),
    }
    Ok(())
}

/// Delete `p` (file, link or whole tree) without per-entry accounting; the first error is
/// returned (for use inside `JobCtx::attempt`).
pub(crate) fn remove_path_quiet(ctx: &JobCtx, p: &Path) -> FsResult<()> {
    let e = soyuz_fs::stat(p, false)?;
    let opts = DeleteOptions { clear_readonly: true, posix_semantics: ctx.opts.delete.posix_semantics };
    match e.kind {
        EntryKind::Dir => {
            let mut first: Option<FsError> = None;
            soyuz_fs::remove_tree(p, &opts, ctx.ctl(), &mut |_, r| match r {
                Ok(()) => Control::Continue,
                Err(err) => {
                    first.get_or_insert(err);
                    Control::Stop
                }
            })?;
            match first {
                Some(err) => Err(err),
                None => Ok(()),
            }
        }
        EntryKind::Symlink { .. } | EntryKind::Junction => soyuz_fs::delete_link(p),
        _ => soyuz_fs::delete_file(p, &opts),
    }
}

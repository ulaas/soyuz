//! `edit_archive` job executor: add / delete / rename entries inside a zip, tar.* or 7z.
//!
//! Steps: rewrite into a hidden sibling (`soyuz_archive::rewrite_archive`: zip entries are
//! copied raw, tar entries stream through, 7z entries are encoded again), move the
//! original to the trash (the undo backup), then rename the new file into place. When the volume has no trash the `TrashUnsupported` prompt decides:
//! "permanent" replaces the archive in one atomic rename without a backup (not undoable),
//! "skip" leaves it untouched.

use std::path::Path;

use soyuz_archive::{ArchiveError, ZipEdit};
use soyuz_core::{ByteProgress, FsError, FsErrorKind, FsResult};
use soyuz_fs::TrashSupport;

use super::extract::fs_error;
use crate::ctx::JobCtx;
use crate::undo::UndoRecord;

/// Default compression level for added files (and recompressed tar.* / 7z archives).
pub const DEFAULT_EDIT_LEVEL: u8 = 6;

pub(crate) fn run_edit_archive(ctx: &JobCtx, archive: &Path, edits: &[ZipEdit], level: Option<u8>) -> FsResult<()> {
    if !soyuz_archive::can_edit(archive) {
        return Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "edit_archive",
            Some(archive),
            "only zip, tar and 7z archives can be edited",
        ));
    }
    let size = soyuz_fs::stat(archive, false)?.size;
    let added: Vec<_> = edits
        .iter()
        .filter_map(|e| match e {
            ZipEdit::Add { sources, .. } => Some(sources.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let add_bytes = match soyuz_archive::total_input_bytes(&added, ctx.ctl()) {
        Ok(t) => t,
        Err(ArchiveError::Cancelled) => return Err(FsError::interrupted("edit_archive")),
        Err(e) => return Err(fs_error(&e, archive)),
    };
    let total = size + add_bytes;
    ctx.set_totals(total, 1);
    ctx.check_space(total, archive.parent().unwrap_or(archive))?;
    ctx.set_running();
    ctx.begin_item(archive, total);
    let tmp = soyuz_archive::edit_temp_path(archive);
    let r = ctx.attempt(archive, || {
        let mut reported = 0u64;
        let res = {
            let mut sink = |bp: ByteProgress| {
                reported += bp.delta;
                ctx.on_bytes(bp);
            };
            soyuz_archive::rewrite_archive(
                archive,
                &tmp,
                edits,
                level.unwrap_or(DEFAULT_EDIT_LEVEL),
                ctx.ctl(),
                &mut sink,
            )
        };
        if res.is_err() {
            ctx.credit_bytes(-(reported as i64));
        }
        res.map_err(|e| fs_error(&e, archive))
    })?;
    if let Err(e) = r {
        ctx.item_failed(archive, e, total);
        return Ok(());
    }
    if let Err(e) = ctx.checkpoint() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    replace_with_backup(ctx, archive, &tmp, total)
}

/// Swap `tmp` into `archive`, keeping the original in the trash.
fn replace_with_backup(ctx: &JobCtx, archive: &Path, tmp: &Path, total: u64) -> FsResult<()> {
    let mut reason = match ctx.probe().trash_support(archive) {
        TrashSupport::Native => None,
        TrashSupport::Unsupported(r) => Some(r),
    };
    if reason.is_none() {
        match soyuz_fs::trash::trash(archive) {
            Ok(backup) => {
                if let Err(e) = std::fs::rename(tmp, archive) {
                    // Put the original back; the edit is lost but nothing else is.
                    let restored = soyuz_fs::trash::restore(&backup, None);
                    let _ = std::fs::remove_file(tmp);
                    let fe = FsError::from_io("edit_archive", Some(archive), &e);
                    if let Err(re) = restored {
                        log::error!("could not restore {} from the trash: {re}", archive.display());
                    }
                    ctx.item_failed(archive, fe, 0);
                    return Ok(());
                }
                ctx.record(UndoRecord::Replaced { path: archive.to_path_buf(), backup });
                ctx.item_ok(total);
                return Ok(());
            }
            Err(e) if e.kind == FsErrorKind::Unsupported => reason = Some(e.message),
            Err(e) => {
                let _ = std::fs::remove_file(tmp);
                ctx.item_failed(archive, e, 0);
                return Ok(());
            }
        }
    }
    let why = format!("{} — the original archive would be replaced without a backup", reason.unwrap_or_default());
    match ctx.trash_unsupported(archive, &why) {
        Ok(true) => {}
        Ok(false) => {
            let _ = std::fs::remove_file(tmp);
            ctx.item_skipped(archive, 0, "kept: the trash is not available");
            return Ok(());
        }
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            return Err(e);
        }
    }
    // Atomic replace (MoveFileEx REPLACE_EXISTING / rename(2)).
    match std::fs::rename(tmp, archive) {
        Ok(()) => {
            ctx.record(UndoRecord::Irreversible(archive.to_path_buf()));
            ctx.item_ok(total);
        }
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            ctx.item_failed(archive, FsError::from_io("edit_archive", Some(archive), &e), 0);
        }
    }
    Ok(())
}

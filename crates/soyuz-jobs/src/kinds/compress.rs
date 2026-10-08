//! `compress` job executor (drives `soyuz_archive::create`).

use std::path::{Path, PathBuf};

use soyuz_archive::{ArchiveError, ArchiveFormat, CreateOptions};
use soyuz_core::{ByteProgress, ConflictResolution, EntryInfo, EntryKind, FsError, FsErrorKind, FsResult};

use super::copy;
use super::extract::fs_error;
use crate::ctx::JobCtx;
use crate::undo::UndoRecord;

pub(crate) fn run_compress(
    ctx: &JobCtx,
    sources: &[PathBuf],
    dest: &Path,
    format: ArchiveFormat,
    level: u8,
    password: Option<&str>,
) -> FsResult<()> {
    if !format.can_write() {
        return Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "compress",
            Some(dest),
            format!("{format:?} archives cannot be created"),
        ));
    }
    let total = match soyuz_archive::total_input_bytes(sources, ctx.ctl()) {
        Ok(t) => t,
        Err(ArchiveError::Cancelled) => return Err(FsError::interrupted("compress")),
        Err(e) => return Err(fs_error(&e, dest)),
    };
    ctx.set_totals(total, 1);
    let mut dest = dest.to_path_buf();
    let mut replace = false;
    loop {
        ctx.checkpoint()?;
        if !soyuz_fs::exists_nofollow(&dest) {
            break;
        }
        let src = EntryInfo::minimal(dest.clone(), EntryKind::File);
        let mut info = copy::conflict_info(&src, &dest);
        info.kind = crate::prompt::ConflictKind::FileExists;
        match ctx.resolve_conflict(info)? {
            ConflictResolution::KeepBoth => dest = copy::unique_path(&dest),
            ConflictResolution::Rename(n) => dest = dest.with_file_name(n),
            ConflictResolution::Cancel => return Err(ctx.cancel_now()),
            ConflictResolution::Skip | ConflictResolution::MergeDir => {
                ctx.item_skipped(&dest, total, "archive exists");
                return Ok(());
            }
            _ => {
                replace = true;
                break;
            }
        }
    }
    ctx.check_space(total, dest.parent().unwrap_or(&dest))?;
    ctx.set_running();
    let base_dir = sources.first().and_then(|s| s.parent()).map(Path::to_path_buf).unwrap_or_default();
    let opts = CreateOptions {
        format,
        level,
        password: password.map(str::to_string),
        base_dir,
        store_symlinks: true,
        follow_symlinks: ctx.opts.copy.follow_symlinks,
    };
    ctx.begin_item(&dest, total);
    let r = ctx.attempt(&dest, || {
        let mut reported = 0u64;
        let res = {
            let mut sink = |bp: ByteProgress| {
                reported += bp.delta;
                ctx.on_bytes(bp);
            };
            soyuz_archive::create(&dest, sources, &opts, ctx.ctl(), &mut sink)
        };
        if res.is_err() {
            ctx.credit_bytes(-(reported as i64));
        }
        res.map_err(|e| fs_error(&e, &dest))
    })?;
    match r {
        Ok(()) => {
            ctx.record(if replace {
                UndoRecord::Irreversible(dest.clone())
            } else {
                UndoRecord::Created(dest.clone())
            });
            ctx.item_ok(total);
        }
        Err(e) => ctx.item_failed(&dest, e, total),
    }
    Ok(())
}

//! `media` job executor (Phase V): drives `soyuz_media::transcode::run` for one input
//! file. Convert, extract audio, trim, remux and save a frame all look the same from here:
//! `src` → `dest` with a [`soyuz_media::MediaOp`].
//!
//! Steps: load (or reuse) the FFmpeg runtime, probe the input size and the dry-run plan
//! (its warnings go into the log and the summary notes, its size estimate into the free
//! space check), resolve a conflict on `dest` through the usual prompt (Keep both renames,
//! Overwrite swaps the new file in with the old one in the trash, Skip skips), then run
//! the transcoder with progress mapped to bytes of input consumed and `check` wired to
//! the job's checkpoint (pause blocks between packets, cancel removes the temporary).
//! The output is journaled as `UndoRecord::Copied { dst }` and identity-stamped when the
//! job finishes, so undo trashes it only while it is still the file this job wrote.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use soyuz_core::{ByteProgress, ConflictResolution, EntryInfo, EntryKind, FsError, FsErrorKind, FsResult};
use soyuz_fs::TrashSupport;
use soyuz_media::transcode::{Cancelled, MediaOp, Progress};
use soyuz_media::{MediaError, Runtime};

use super::copy;
use crate::ctx::JobCtx;
use crate::job::LogLevel;
use crate::undo::UndoRecord;

/// The `op` name in `FsError`s of this executor.
const OP: &str = "media";

/// Marker in the name of the temporary a replacing run writes next to the destination
/// (`clip.soyuz-new.mp4`): the extension stays last so the container choice is the same,
/// and the crash journal's leftover scan recognises it.
pub const NEW_MARKER: &str = ".soyuz-new.";

/// `MediaError` → `FsError` (the kind decides retryability and the toast wording).
pub(crate) fn fs_error(e: &MediaError, path: &Path) -> FsError {
    match e {
        MediaError::NotLoaded => FsError::with_message(
            FsErrorKind::Unsupported,
            OP,
            Some(path),
            "the FFmpeg libraries are not loaded (Settings → Preview → Media)",
        ),
        MediaError::Io(io) => FsError::from_io(OP, Some(path), io),
        MediaError::Cancelled | MediaError::Stale => FsError::interrupted(OP),
        MediaError::Unsupported(_) | MediaError::NoStream(_) => {
            FsError::with_message(FsErrorKind::Unsupported, OP, Some(path), e.to_string())
        }
        MediaError::Av { .. } => FsError::with_message(FsErrorKind::Io, OP, Some(path), e.to_string()),
    }
}

/// The loaded runtime, or one loaded from `EngineConfig::media_dirs` (empty: never load).
fn runtime(ctx: &JobCtx, src: &Path) -> FsResult<Arc<Runtime>> {
    if let Some(rt) = Runtime::loaded() {
        return Ok(rt);
    }
    let dirs = &ctx.inner.cfg.media_dirs;
    if dirs.is_empty() {
        return Err(fs_error(&MediaError::NotLoaded, src));
    }
    Runtime::get(dirs).map_err(|e| FsError::with_message(FsErrorKind::Unsupported, OP, Some(src), e.to_string()))
}

/// `clip.mp4` → `clip.soyuz-new.mp4` (a replacing run's temporary, same extension).
fn new_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name.as_str(), ""),
    };
    let tmp = if ext.is_empty() { format!("{stem}{NEW_MARKER}out") } else { format!("{stem}{NEW_MARKER}{ext}") };
    dest.with_file_name(tmp)
}

pub(crate) fn run_media(ctx: &JobCtx, src: &Path, dest: &Path, op: &MediaOp) -> FsResult<()> {
    let rt = runtime(ctx, src)?;
    let input = soyuz_fs::stat(src, true)?;
    if input.kind != EntryKind::File {
        return Err(FsError::with_message(FsErrorKind::Unsupported, OP, Some(src), "not a file"));
    }
    let total = input.size.max(1);
    ctx.set_totals(total, 1);
    // The dry run: an input this build cannot handle fails before any prompt; its
    // warnings are worth keeping (a dropped stream, chapters not copied).
    let planned = soyuz_media::plan(&rt, src, op).map_err(|e| fs_error(&e, src))?;
    for w in &planned.warnings {
        ctx.log(LogLevel::Warn, Some(src.to_path_buf()), w.clone());
        ctx.note(w.clone());
    }
    let mut dest = dest.to_path_buf();
    let mut replace = false;
    loop {
        ctx.checkpoint()?;
        if !soyuz_fs::exists_nofollow(&dest) {
            break;
        }
        let src_entry = EntryInfo::minimal(dest.clone(), EntryKind::File);
        let mut info = copy::conflict_info(&src_entry, &dest);
        info.kind = crate::prompt::ConflictKind::FileExists;
        match ctx.resolve_conflict(info)? {
            ConflictResolution::KeepBoth => dest = copy::unique_path(&dest),
            ConflictResolution::Rename(n) => dest = dest.with_file_name(n),
            ConflictResolution::Cancel => return Err(ctx.cancel_now()),
            ConflictResolution::Skip | ConflictResolution::MergeDir => {
                ctx.item_skipped(&dest, total, "output exists");
                return Ok(());
            }
            _ => {
                replace = true;
                break;
            }
        }
    }
    ctx.check_space(planned.estimated_bytes.unwrap_or(total), dest.parent().unwrap_or(&dest))?;
    ctx.set_running();
    ctx.begin_item(&dest, total);
    // A replacing run writes next to the old file first; the swap comes after.
    let out = if replace { new_path(&dest) } else { dest.clone() };
    let r = ctx.attempt(&dest, || {
        let mut last = 0u64;
        let mut progress = |p: Progress| {
            let done = p.input_bytes.min(p.input_total.max(total));
            ctx.on_bytes(ByteProgress { done, total: p.input_total.max(total), delta: done.saturating_sub(last) });
            last = last.max(done);
        };
        let mut check = || ctx.checkpoint().map_err(|_| Cancelled);
        let res = soyuz_media::run(&rt, src, &out, op, &mut progress, &mut check);
        if res.is_err() {
            ctx.credit_bytes(-(last as i64));
        }
        res.map_err(|e| fs_error(&e, &out))
    })?;
    let outcome = match r {
        Ok(o) => o,
        Err(e) => {
            ctx.item_failed(&dest, e, total);
            return Ok(());
        }
    };
    for w in &outcome.warnings {
        ctx.log(LogLevel::Warn, Some(dest.clone()), w.clone());
        ctx.note(w.clone());
    }
    if replace {
        if let Err(e) = ctx.checkpoint() {
            let _ = std::fs::remove_file(&out);
            return Err(e);
        }
        return replace_with_backup(ctx, &dest, &out, total);
    }
    ctx.record(UndoRecord::Copied { dst: dest.clone() });
    ctx.item_ok(total);
    Ok(())
}

/// Swap `tmp` into `dest`, keeping the old file in the trash (undo restores it after
/// trashing the new one). Without a trash the `TrashUnsupported` prompt decides between
/// a permanent replace (not undoable) and keeping the old file.
fn replace_with_backup(ctx: &JobCtx, dest: &Path, tmp: &Path, total: u64) -> FsResult<()> {
    let mut reason = match ctx.probe().trash_support(dest) {
        TrashSupport::Native => None,
        TrashSupport::Unsupported(r) => Some(r),
    };
    if reason.is_none() {
        match soyuz_fs::trash::trash(dest) {
            Ok(backup) => {
                if let Err(e) = std::fs::rename(tmp, dest) {
                    // Put the old file back; the new one is dropped.
                    let restored = soyuz_fs::trash::restore(&backup, None);
                    let _ = std::fs::remove_file(tmp);
                    if let Err(re) = restored {
                        log::error!("could not restore {} from the trash: {re}", dest.display());
                    }
                    ctx.item_failed(dest, FsError::from_io(OP, Some(dest), &e), 0);
                    return Ok(());
                }
                ctx.record(UndoRecord::Trashed(backup));
                ctx.record(UndoRecord::Copied { dst: dest.to_path_buf() });
                ctx.item_ok(total);
                return Ok(());
            }
            Err(e) if e.kind == FsErrorKind::Unsupported => reason = Some(e.message),
            Err(e) => {
                let _ = std::fs::remove_file(tmp);
                ctx.item_failed(dest, e, 0);
                return Ok(());
            }
        }
    }
    let why = format!("{} — the existing file would be replaced without a backup", reason.unwrap_or_default());
    match ctx.trash_unsupported(dest, &why) {
        Ok(true) => {}
        Ok(false) => {
            let _ = std::fs::remove_file(tmp);
            ctx.item_skipped(dest, 0, "kept: the trash is not available");
            return Ok(());
        }
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            return Err(e);
        }
    }
    // Atomic replace (MoveFileEx REPLACE_EXISTING / rename(2)).
    match std::fs::rename(tmp, dest) {
        Ok(()) => {
            ctx.record(UndoRecord::Irreversible(dest.to_path_buf()));
            ctx.item_ok(total);
        }
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            ctx.item_failed(dest, FsError::from_io(OP, Some(dest), &e), 0);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_keeps_the_extension() {
        assert_eq!(new_path(Path::new("/v/clip.mp4")), PathBuf::from("/v/clip.soyuz-new.mp4"));
        assert_eq!(new_path(Path::new("/v/.hidden")), PathBuf::from("/v/.hidden.soyuz-new.out"));
        assert_eq!(new_path(Path::new("/v/noext")), PathBuf::from("/v/noext.soyuz-new.out"));
        assert!(crate::journal::is_leftover_name("clip.soyuz-new.mp4"));
        assert!(crate::journal::is_leftover_name("clip.soyuz-new.mp4.part"));
    }

    #[test]
    fn errors_map_to_readable_kinds() {
        let p = Path::new("/v/a.mp4");
        let e = fs_error(&MediaError::NotLoaded, p);
        assert_eq!(e.kind, FsErrorKind::Unsupported);
        assert!(e.to_string().contains("FFmpeg libraries are not loaded"), "{e}");
        assert_eq!(fs_error(&MediaError::Cancelled, p).kind, FsErrorKind::Interrupted);
        assert_eq!(fs_error(&MediaError::NoStream(soyuz_media::StreamKind::Video), p).kind, FsErrorKind::Unsupported);
    }
}

//! `create` job executor: folder, file (optionally from a template), symlink, hardlink, junction.

use std::path::{Path, PathBuf};

use soyuz_core::{ConflictResolution, EntryInfo, EntryKind, FsResult};

use super::copy;
use crate::ctx::JobCtx;
use crate::job::CreateKind;
use crate::prompt::{ConflictInfo, ConflictKind};
use crate::undo::UndoRecord;

fn kind_of(k: &CreateKind) -> EntryKind {
    match k {
        CreateKind::Dir | CreateKind::Junction { .. } => EntryKind::Dir,
        _ => EntryKind::File,
    }
}

/// File vs directory symlink: an existing target's type wins over the requested kind (a
/// file link to a folder is broken on Windows); `kind` decides for missing targets. Done
/// here on the worker so callers need not stat the target on the UI thread.
fn link_kind(link: &Path, target: &Path, kind: soyuz_fs::LinkKind) -> soyuz_fs::LinkKind {
    let resolved = if target.is_absolute() {
        target.to_path_buf()
    } else {
        link.parent().map(|d| d.join(target)).unwrap_or_else(|| target.to_path_buf())
    };
    match soyuz_fs::stat(&resolved, true) {
        Ok(e) if e.kind == EntryKind::Dir => soyuz_fs::LinkKind::Dir,
        Ok(_) => soyuz_fs::LinkKind::File,
        Err(_) => kind,
    }
}

pub(crate) fn run_create(ctx: &JobCtx, path: &Path, kind: &CreateKind) -> FsResult<()> {
    ctx.set_totals(0, 1);
    ctx.set_running();
    let mut path: PathBuf = path.to_path_buf();
    let mut replace = false;
    loop {
        ctx.checkpoint()?;
        if !soyuz_fs::exists_nofollow(&path) {
            break;
        }
        let src = EntryInfo::minimal(path.clone(), kind_of(kind));
        let dst = copy::entry_of(&path);
        let conflict = match (src.kind == EntryKind::Dir, dst.is_dir_like()) {
            (true, true) => ConflictKind::DirExists,
            (false, false) => ConflictKind::FileExists,
            _ => ConflictKind::FileVsDir,
        };
        let info = ConflictInfo { src, dst, kind: conflict, suggested_name: copy::suggested_name(&path) };
        match ctx.resolve_conflict(info)? {
            ConflictResolution::KeepBoth => path = copy::unique_path(&path),
            ConflictResolution::Rename(n) => path = path.with_file_name(n),
            ConflictResolution::Cancel => return Err(ctx.cancel_now()),
            ConflictResolution::Overwrite
                if matches!(kind, CreateKind::File { .. }) && conflict == ConflictKind::FileExists =>
            {
                replace = true;
                break;
            }
            _ => {
                ctx.item_skipped(&path, 0, "already exists");
                return Ok(());
            }
        }
    }
    ctx.begin_item(&path, 0);
    let p = path.clone();
    let r = ctx.attempt(&path, || match kind {
        CreateKind::Dir => soyuz_fs::create_dir(&p),
        CreateKind::File { template: Some(t) } => {
            let mut o = ctx.opts.copy.clone();
            o.overwrite = if replace { soyuz_core::Overwrite::Always } else { soyuz_core::Overwrite::Never };
            soyuz_fs::copy_file(t, &p, &o, ctx.ctl(), &mut ()).map(|_| ())
        }
        CreateKind::File { template: None } => {
            if replace {
                soyuz_fs::delete_file(&p, &ctx.opts.delete)?;
            }
            soyuz_fs::create_file(&p, true)
        }
        CreateKind::Symlink { target, kind } => soyuz_fs::symlink(target, &p, link_kind(&p, target, *kind)),
        CreateKind::Hardlink { src } => soyuz_fs::hardlink(src, &p),
        CreateKind::Junction { target } => soyuz_fs::junction(target, &p),
    })?;
    match r {
        Ok(()) => {
            ctx.record(if replace {
                UndoRecord::Irreversible(path.clone())
            } else {
                UndoRecord::Created(path.clone())
            });
            ctx.item_ok(0);
        }
        Err(e) => ctx.item_failed(&path, e, 0),
    }
    Ok(())
}

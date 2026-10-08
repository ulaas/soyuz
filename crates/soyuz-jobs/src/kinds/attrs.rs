//! `SetMeta` job executor (times / attributes / unix mode), journaling a `MetaSnapshot`.

use std::path::PathBuf;

use soyuz_core::{Attributes, FsResult};
use soyuz_fs::{AttrSet, MetaSnapshot, TimesSet};

use crate::ctx::JobCtx;
use crate::undo::UndoRecord;

pub(crate) fn run_set_meta(
    ctx: &JobCtx,
    paths: &[PathBuf],
    times: Option<&TimesSet>,
    attrs: Option<&AttrSet>,
    mode: Option<u32>,
) -> FsResult<()> {
    ctx.set_totals(0, paths.len() as u64);
    ctx.set_running();
    for p in paths {
        ctx.checkpoint()?;
        ctx.begin_item(p, 0);
        let before = match ctx.attempt(p, || soyuz_fs::meta_snapshot(p))? {
            Ok(b) => b,
            Err(e) => {
                ctx.item_failed(p, e, 0);
                continue;
            }
        };
        // Clear flags first (a read-only file may refuse other changes), set them last.
        let clear = attrs.map(|a| a.clear).unwrap_or_else(Attributes::empty);
        let set = attrs.map(|a| a.set).unwrap_or_else(Attributes::empty);
        let r = ctx.attempt(p, || {
            if !clear.is_empty() {
                soyuz_fs::set_attributes(p, &AttrSet { set: Attributes::empty(), clear })?;
            }
            if let Some(t) = times {
                soyuz_fs::set_times(p, t, false)?;
            }
            if let Some(m) = mode {
                soyuz_fs::set_mode(p, m)?;
            }
            if !set.is_empty() {
                soyuz_fs::set_attributes(p, &AttrSet { set, clear: Attributes::empty() })?;
            }
            Ok(())
        })?;
        match r {
            Ok(()) => {
                ctx.record(UndoRecord::MetaChanged { path: p.clone(), before });
                ctx.item_ok(0);
            }
            Err(e) => {
                // Partial changes may have been applied: keep the snapshot so undo restores it.
                ctx.record(UndoRecord::MetaChanged { path: p.clone(), before });
                ctx.item_failed(p, e, 0);
            }
        }
    }
    Ok(())
}

/// Restore `before` (used by undo): attributes that differ are flipped one by one
/// (unsupported bits are ignored), then times and mode.
pub(crate) fn restore_meta(path: &std::path::Path, before: &MetaSnapshot) -> FsResult<()> {
    let cur = soyuz_fs::meta_snapshot(path)?;
    let settable = Attributes::READONLY
        | Attributes::HIDDEN
        | Attributes::SYSTEM
        | Attributes::ARCHIVE
        | Attributes::NOT_CONTENT_INDEXED
        | Attributes::IMMUTABLE;
    let diff = (cur.attrs ^ before.attrs) & settable;
    let to_clear = diff & cur.attrs;
    let to_set = diff & before.attrs;
    let flip = |set: Attributes, clear: Attributes| -> FsResult<()> {
        for bit in set.iter().map(|b| (b, true)).chain(clear.iter().map(|b| (b, false))) {
            let a = if bit.1 {
                AttrSet { set: bit.0, clear: Attributes::empty() }
            } else {
                AttrSet { set: Attributes::empty(), clear: bit.0 }
            };
            match soyuz_fs::set_attributes(path, &a) {
                Err(e) if e.kind == soyuz_core::FsErrorKind::Unsupported => {}
                other => other?,
            }
        }
        Ok(())
    };
    flip(Attributes::empty(), to_clear)?;
    soyuz_fs::set_times(path, &before.times, false)?;
    if let Some(m) = before.mode
        && cur.mode != Some(m)
    {
        match soyuz_fs::set_mode(path, m) {
            Err(e) if e.kind == soyuz_core::FsErrorKind::Unsupported => {}
            other => other?,
        }
    }
    flip(to_set, Attributes::empty())
}

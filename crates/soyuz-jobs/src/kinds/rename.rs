//! Batch rename with dependency ordering and two-phase (temp name) cycle breaking.
//!
//! Pair `i` depends on pair `j` when `to_i == from_j`: `j` must vacate its name first.
//! Pairs whose dependency is resolved run directly; when only cycles remain
//! (`a→b, b→a`), one member is parked under a temporary name, which unblocks the rest,
//! and is renamed to its target afterwards. The same routine reverses `Moved`/`Renamed`
//! records for undo (with a primitive that can fall back to copy+delete across devices).

use std::path::{Path, PathBuf};

use soyuz_core::{ConflictResolution, EntryKind, FsError, FsErrorKind, FsResult};
use soyuz_fs::RenameMode;

use super::copy;
use crate::ctx::JobCtx;
use crate::job::LogLevel;
use crate::planner::{path_key, same_path};
use crate::prompt::ConflictKind;
use crate::undo::UndoRecord;

/// The filesystem operation behind one rename: `(from, to, replace_existing)`.
pub(crate) type RenamePrim<'a> = dyn Fn(&JobCtx, &Path, &Path, bool) -> FsResult<()> + 'a;
/// Journal record for a completed pair `(original_from, final_to)`; `None` = don't record.
pub(crate) type RecordFn<'a> = dyn Fn(&Path, &Path) -> Option<UndoRecord> + 'a;

pub(crate) fn run_rename(ctx: &JobCtx, pairs: &[(PathBuf, PathBuf)]) -> FsResult<()> {
    ctx.set_totals(0, pairs.len() as u64);
    ctx.set_running();
    let prim = |_: &JobCtx, a: &Path, b: &Path, replace: bool| {
        soyuz_fs::rename(a, b, if replace { RenameMode::Replace } else { RenameMode::NoReplace })
    };
    let rec = |a: &Path, b: &Path| Some(UndoRecord::Renamed { from: a.to_path_buf(), to: b.to_path_buf() });
    run_pairs(ctx, pairs, &prim, &rec)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    Pending,
    Done,
    Failed,
}

struct Pair {
    orig: PathBuf,
    to: PathBuf,
    /// Current location of the source (the temp name while parked).
    cur: PathBuf,
    st: St,
    dep: Option<usize>,
}

pub(crate) fn run_pairs(
    ctx: &JobCtx,
    pairs: &[(PathBuf, PathBuf)],
    prim: &RenamePrim<'_>,
    rec: &RecordFn<'_>,
) -> FsResult<()> {
    let keys: Vec<Vec<String>> = pairs.iter().map(|(a, _)| path_key(a)).collect();
    let mut ps: Vec<Pair> = pairs
        .iter()
        .enumerate()
        .map(|(i, (a, b))| {
            let tk = path_key(b);
            let dep = keys.iter().enumerate().find(|(j, k)| *j != i && **k == tk).map(|(j, _)| j);
            Pair { orig: a.clone(), to: b.clone(), cur: a.clone(), st: St::Pending, dep }
        })
        .collect();
    let r = schedule(ctx, &mut ps, prim, rec);
    // Never leave anything parked under a temporary name.
    for p in ps.iter().filter(|p| p.st == St::Pending && p.cur != p.orig) {
        unpark(ctx, p, prim, rec);
    }
    r
}

fn schedule(ctx: &JobCtx, ps: &mut [Pair], prim: &RenamePrim<'_>, rec: &RecordFn<'_>) -> FsResult<()> {
    let mut parked = 0usize;
    loop {
        let pending: Vec<usize> = (0..ps.len()).filter(|i| ps[*i].st == St::Pending).collect();
        if pending.is_empty() {
            return Ok(());
        }
        let mut progressed = false;
        for i in pending.iter().copied() {
            let blocked = match ps[i].dep {
                // Still occupies our target (pending and not parked).
                Some(j) => ps[j].st == St::Pending && ps[j].cur == ps[j].orig,
                None => false,
            };
            if blocked {
                continue;
            }
            ctx.checkpoint()?;
            run_one(ctx, ps, i, prim, rec)?;
            progressed = true;
        }
        if progressed {
            continue;
        }
        // Only cycles remain: park one member under a temporary name.
        let Some(i) = pending.iter().copied().find(|i| ps[*i].cur == ps[*i].orig) else {
            // Cannot happen (a parked pair unblocks its dependent); run the rest plainly.
            for i in pending {
                run_one(ctx, ps, i, prim, rec)?;
            }
            return Ok(());
        };
        ctx.checkpoint()?;
        parked += 1;
        let tmp = temp_name(&ps[i].orig, ctx.id.0, parked);
        let orig = ps[i].orig.clone();
        match ctx.attempt(&orig, || prim(ctx, &orig, &tmp, false))? {
            Ok(()) => {
                ctx.log(LogLevel::Info, Some(orig.clone()), format!("parked as {}", tmp.display()));
                ps[i].cur = tmp;
            }
            Err(e) => {
                ps[i].st = St::Failed;
                ctx.item_failed(&orig, e, 0);
            }
        }
    }
}

fn temp_name(orig: &Path, job: u64, n: usize) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    orig.with_file_name(format!(".soyuz-rename-{job}-{n}-{nanos}"))
}

/// Move a parked pair back to its original name (best effort, journaled if it fails).
fn unpark(ctx: &JobCtx, p: &Pair, prim: &RenamePrim<'_>, rec: &RecordFn<'_>) {
    if soyuz_fs::exists_nofollow(&p.orig) || prim(ctx, &p.cur, &p.orig, false).is_err() {
        ctx.log(LogLevel::Warn, Some(p.orig.clone()), format!("left under temporary name {}", p.cur.display()));
        if let Some(r) = rec(&p.orig, &p.cur) {
            ctx.record(r);
        }
    }
}

fn run_one(ctx: &JobCtx, ps: &mut [Pair], i: usize, prim: &RenamePrim<'_>, rec: &RecordFn<'_>) -> FsResult<()> {
    let orig = ps[i].orig.clone();
    let cur = ps[i].cur.clone();
    let mut to = ps[i].to.clone();
    ctx.begin_item(&orig, 0);
    if to == orig {
        ps[i].st = St::Done;
        ctx.item_ok(0);
        return Ok(());
    }
    let mut replace = false;
    let finish_failed = |ctx: &JobCtx, ps: &mut [Pair], e: FsError| {
        ps[i].st = St::Failed;
        if cur != orig {
            unpark(ctx, &ps[i], prim, rec);
        }
        ctx.item_failed(&orig, e, 0);
    };
    loop {
        ctx.checkpoint()?;
        if !soyuz_fs::exists_nofollow(&to) {
            break;
        }
        // Case-only rename of the same entry (case-insensitive filesystems).
        if cur == orig && same_path(&orig, &to) && orig.parent() == to.parent() {
            let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match ctx.attempt(&orig, || soyuz_fs::rename_case_only(&orig, &name))? {
                Ok(()) => {
                    ps[i].st = St::Done;
                    if let Some(r) = rec(&orig, &to) {
                        ctx.record(r);
                    }
                    ctx.item_ok(0);
                }
                Err(e) => finish_failed(ctx, ps, e),
            }
            return Ok(());
        }
        let src_entry = soyuz_fs::stat(&cur, false)
            .map(|mut e| {
                e.path = orig.clone();
                e.name = soyuz_core::entry::display_name(&orig);
                e
            })
            .unwrap_or_else(|_| soyuz_core::EntryInfo::minimal(orig.clone(), EntryKind::Unknown));
        let info = copy::conflict_info(&src_entry, &to);
        let kind = info.kind;
        let res = match ctx.resolve_conflict(info) {
            Ok(r) => r,
            Err(e) => {
                if cur != orig {
                    unpark(ctx, &ps[i], prim, rec);
                    ps[i].cur = orig.clone();
                }
                return Err(e);
            }
        };
        match res {
            ConflictResolution::Skip | ConflictResolution::MergeDir => {
                ps[i].st = St::Failed;
                if cur != orig {
                    unpark(ctx, &ps[i], prim, rec);
                }
                ctx.item_skipped(&orig, 0, "target exists");
                return Ok(());
            }
            ConflictResolution::KeepBoth => to = copy::unique_path(&to),
            ConflictResolution::Rename(n) => to = to.with_file_name(n),
            ConflictResolution::Cancel => return Err(ctx.cancel_now()),
            _ if kind != ConflictKind::FileExists => {
                let err = FsError::with_message(
                    FsErrorKind::AlreadyExists,
                    "rename",
                    Some(&to),
                    "cannot replace a folder (or with a folder)",
                );
                finish_failed(ctx, ps, err);
                return Ok(());
            }
            _ => {
                replace = true;
                break;
            }
        }
    }
    match ctx.attempt(&orig, || prim(ctx, &cur, &to, replace))? {
        Ok(()) => {
            ps[i].st = St::Done;
            if replace {
                ctx.record(UndoRecord::Irreversible(to.clone()));
            } else if let Some(r) = rec(&orig, &to) {
                ctx.record(r);
            }
            ctx.item_ok(0);
        }
        Err(e) => finish_failed(ctx, ps, e),
    }
    Ok(())
}

//! Per-kind job executors. Every executor plans (sets progress totals), switches the job
//! to `Running`, executes item by item with `JobCtx` (checkpoints between items and inside
//! every byte loop, conflicts/errors through the prompt protocol) and journals undo records.

pub mod attrs;
pub mod compress;
pub mod copy;
pub mod create;
pub mod delete;
pub mod edit_archive;
pub mod extract;
pub mod hash;
pub mod mv;
pub mod remote;
pub mod rename;
pub mod sync;
pub mod trash;

use std::path::PathBuf;

use soyuz_core::FsResult;

use crate::ctx::JobCtx;
use crate::engine::Exec;
use crate::job::{JobKind, JobSummary, LogLevel};
use crate::planner::is_within;

/// Execute one job on its worker thread.
pub(crate) fn run(ctx: &JobCtx, exec: &Exec) -> FsResult<()> {
    ctx.log(LogLevel::Info, None, "started".to_string());
    match exec {
        Exec::Undo { records, stamps } => crate::undo::run_undo(ctx, records, stamps)?,
        Exec::Kind(kind) => run_kind(ctx, kind)?,
    }
    ctx.checkpoint()?;
    ctx.set_finishing();
    ctx.progress(|p| p.current = None);
    Ok(())
}

fn run_kind(ctx: &JobCtx, kind: &JobKind) -> FsResult<()> {
    match kind {
        JobKind::Copy { sources, dest_dir, rename_to } => copy::run_copy(ctx, sources, dest_dir, rename_to.as_deref()),
        JobKind::Move { sources, dest_dir, rename_to } => mv::run_move(ctx, sources, dest_dir, rename_to.as_deref()),
        JobKind::Delete { paths, mode } => delete::run_delete(ctx, paths, *mode),
        JobKind::Rename { pairs } => rename::run_rename(ctx, pairs),
        JobKind::Create { path, kind } => create::run_create(ctx, path, kind),
        JobKind::SetMeta { paths, times, attrs, mode } => {
            attrs::run_set_meta(ctx, paths, times.as_ref(), attrs.as_ref(), *mode)
        }
        JobKind::Hash { paths, algo } => hash::run_hash(ctx, paths, *algo),
        JobKind::Extract { archive, entries, dest_dir, password, strip_components } => {
            extract::run_extract(ctx, archive, entries.as_deref(), dest_dir, password.as_deref(), *strip_components)
        }
        JobKind::Compress { sources, dest, format, level, password } => {
            compress::run_compress(ctx, sources, dest, *format, *level, password.as_deref())
        }
        JobKind::RestoreTrash { items, to } => trash::run_restore(ctx, items, to.as_deref()),
        JobKind::EmptyTrash => trash::run_empty(ctx),
        JobKind::PurgeTrash { items } => trash::run_purge(ctx, items),
        JobKind::Sync { plan } => sync::run_sync(ctx, plan),
        JobKind::EditArchive { archive, edits, level } => edit_archive::run_edit_archive(ctx, archive, edits, *level),
        JobKind::Remote(op) => remote::run_remote(ctx, op),
    }
}

/// The part of `job` that failed according to `summary.errors`, as a new job kind.
pub(crate) fn retry_kind(job: &JobKind, summary: &JobSummary) -> Option<JobKind> {
    let failed: Vec<PathBuf> = summary.errors.iter().map(|(p, _)| p.clone()).collect();
    let hit = |root: &PathBuf| failed.iter().any(|f| is_within(f, root));
    let keep = |v: &[PathBuf]| -> Vec<PathBuf> { v.iter().filter(|p| hit(p)).cloned().collect() };
    let non_empty = |v: Vec<PathBuf>| if v.is_empty() { None } else { Some(v) };
    Some(match job {
        JobKind::Copy { sources, dest_dir, rename_to } => {
            let s = non_empty(keep(sources))?;
            let rename_to = if s.len() == 1 { rename_to.clone() } else { None };
            JobKind::Copy { sources: s, dest_dir: dest_dir.clone(), rename_to }
        }
        JobKind::Move { sources, dest_dir, rename_to } => {
            let s = non_empty(keep(sources))?;
            let rename_to = if s.len() == 1 { rename_to.clone() } else { None };
            JobKind::Move { sources: s, dest_dir: dest_dir.clone(), rename_to }
        }
        JobKind::Delete { paths, mode } => JobKind::Delete { paths: non_empty(keep(paths))?, mode: *mode },
        JobKind::SetMeta { paths, times, attrs, mode } => {
            JobKind::SetMeta { paths: non_empty(keep(paths))?, times: *times, attrs: *attrs, mode: *mode }
        }
        JobKind::Hash { algo, .. } => JobKind::Hash { paths: non_empty(failed)?, algo: *algo },
        JobKind::Rename { pairs } => {
            let p: Vec<_> = pairs.iter().filter(|(a, _)| failed.iter().any(|f| f == a)).cloned().collect();
            if p.is_empty() {
                return None;
            }
            JobKind::Rename { pairs: p }
        }
        JobKind::RestoreTrash { items, to } => {
            let it: Vec<_> = items.iter().filter(|i| failed.contains(&i.original)).cloned().collect();
            if it.is_empty() {
                return None;
            }
            JobKind::RestoreTrash { items: it, to: to.clone() }
        }
        JobKind::PurgeTrash { items } => {
            let it: Vec<_> = items.iter().filter(|i| failed.contains(&i.original)).cloned().collect();
            if it.is_empty() {
                return None;
            }
            JobKind::PurgeTrash { items: it }
        }
        // Create / Extract / Compress / EmptyTrash / Sync: re-run as a whole (the
        // engine pre-answers conflicts with Skip for extract).
        other => other.clone(),
    })
}

//! `hash` job executor. Results go to the job log (`Info`, path + hex digest) and to
//! `JobSummary::notes` as `"<hex>  <path>"` lines (sha*sum format).

use std::path::PathBuf;

use soyuz_core::{ByteProgress, Control, FsResult, HashAlgo};
use soyuz_fs::{WalkItem, WalkOptions};

use crate::ctx::JobCtx;
use crate::job::LogLevel;
use crate::planner::{self, has_bytes};

pub(crate) fn run_hash(ctx: &JobCtx, paths: &[PathBuf], algo: HashAlgo) -> FsResult<()> {
    // Expand folders into their files.
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    for (p, e) in planner::stat_all(ctx, paths)? {
        if e.is_dir_like() {
            soyuz_fs::walk(&p, &WalkOptions::default(), ctx.ctl(), &mut |item| {
                if let WalkItem::Entry { entry, .. } = item
                    && has_bytes(entry.kind)
                {
                    files.push((entry.path, entry.size));
                }
                Control::Continue
            })?;
        } else {
            files.push((p, e.size));
        }
    }
    let failed = ctx.summary_counts().0;
    ctx.set_totals(files.iter().map(|f| f.1).sum(), files.len() as u64 + failed);
    ctx.set_running();
    for (p, size) in &files {
        ctx.checkpoint()?;
        ctx.begin_item(p, *size);
        let r = ctx.attempt(p, || {
            let mut reported = 0u64;
            let res = {
                let mut sink = |bp: ByteProgress| {
                    reported += bp.delta;
                    ctx.on_bytes(bp);
                };
                soyuz_fs::hash_file(p, algo, ctx.ctl(), &mut sink)
            };
            if res.is_err() {
                ctx.credit_bytes(-(reported as i64));
            }
            res
        })?;
        match r {
            Ok(hex) => {
                let reported = ctx.progress(|pr| pr.current_bytes);
                if *size > reported {
                    ctx.credit_bytes((*size - reported) as i64);
                }
                ctx.log(LogLevel::Info, Some(p.clone()), hex.clone());
                ctx.note(format!("{hex}  {}", p.display()));
                ctx.item_ok(*size);
            }
            Err(e) => ctx.item_failed(p, e, *size),
        }
    }
    Ok(())
}

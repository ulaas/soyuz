//! F2: jobs that involve remote locations (`soyuz-vfs` providers): streaming copies and
//! moves between any two locations (local↔remote, remote↔remote) with conflicts,
//! progress, pause / cancel and resume of partial files (every retry of a failed file
//! continues where it stopped when both ends support it), plus remote delete, new
//! folder and rename.
//!
//! Undo: files and folders created on a server are journaled as `RemoteCreated` (undo
//! deletes them, identity-checked by size); downloads as `Copied` (undo trashes them);
//! local sources of a move go to the trash (`Trashed`); same-server moves / renames are
//! `RemoteRenamed`. Deleting or replacing something on a server cannot be undone
//! (servers have no trash), so those are journaled `Irreversible`.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use soyuz_core::{ByteProgress, ConflictResolution, EntryInfo, FsError, FsErrorKind, FsResult};
use soyuz_vfs::provider::{delete_tree, stat_opt};
use soyuz_vfs::transfer::{FileCopy, TransferHooks, copy_file, discard_partial};
use soyuz_vfs::{Provider, RemoteEntry, RemoteUri};

use crate::ctx::JobCtx;
use crate::job::RemoteOp;
use crate::prompt::{ConflictInfo, ConflictKind};
use crate::undo::UndoRecord;

/// One end of a transfer: a provider and a provider path.
#[derive(Clone)]
struct End {
    prov: Arc<dyn Provider>,
    path: String,
    /// `Some` for remote locations.
    uri: Option<RemoteUri>,
}

impl End {
    fn resolve(p: &Path) -> FsResult<End> {
        let uri = RemoteUri::from_path(p);
        let (prov, path) = soyuz_vfs::registry::resolve(p)?;
        Ok(End { prov, path, uri })
    }

    fn is_remote(&self) -> bool {
        self.uri.is_some()
    }

    /// The job-level path (URI or native path) for logs, progress and undo records.
    fn display(&self) -> PathBuf {
        match &self.uri {
            Some(u) => u.with_path(&self.path).to_path_buf(),
            None => PathBuf::from(&self.path),
        }
    }

    fn child(&self, name: &str) -> End {
        let path = match &self.uri {
            Some(_) => soyuz_vfs::uri::join_path(&self.path, name),
            None => Path::new(&self.path).join(name).to_string_lossy().into_owned(),
        };
        End { prov: self.prov.clone(), path, uri: self.uri.clone() }
    }

    fn sibling(&self, name: &str) -> End {
        match &self.uri {
            Some(_) => {
                let parent = soyuz_vfs::uri::parent_path(&self.path).unwrap_or_else(|| "/".into());
                End { prov: self.prov.clone(), path: soyuz_vfs::uri::join_path(&parent, name), uri: self.uri.clone() }
            }
            None => {
                let p = Path::new(&self.path).with_file_name(name);
                End { prov: self.prov.clone(), path: p.to_string_lossy().into_owned(), uri: None }
            }
        }
    }

    fn name(&self) -> &str {
        soyuz_vfs::uri::name_of(&self.path)
    }

    fn same_server(&self, other: &End) -> bool {
        match (&self.uri, &other.uri) {
            (Some(a), Some(b)) => a.conn_key() == b.conn_key(),
            _ => false,
        }
    }

    fn entry_info(&self, e: &RemoteEntry) -> EntryInfo {
        let mut info = e.to_entry_info(self.display());
        info.name = self.name().to_string();
        info
    }

    fn stat_opt(&self) -> FsResult<Option<RemoteEntry>> {
        stat_opt(&*self.prov, &self.path)
    }
}

/// A free name next to `dst` ("name (2).ext" …), checked on the destination provider.
fn unique_sibling(dst: &End) -> End {
    let name = dst.name().to_string();
    let exists = |p: &Path| {
        let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        dst.sibling(&n).stat_opt().ok().flatten().is_some()
    };
    let p = soyuz_core::unique_name(Path::new(""), &name, "", &exists);
    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
    dst.sibling(&n)
}

fn conflict(src: &End, se: &RemoteEntry, dst: &End, de: &RemoteEntry) -> ConflictInfo {
    let kind = match (se.is_dir, de.is_dir) {
        (true, true) => ConflictKind::DirExists,
        (false, false) => ConflictKind::FileExists,
        _ => ConflictKind::FileVsDir,
    };
    let suggested_name = unique_sibling(dst).name().to_string();
    ConflictInfo { src: src.entry_info(se), dst: dst.entry_info(de), kind, suggested_name }
}

/// Progress bridge: forwards byte counts to the job, correcting for restarts.
struct Hooks<'a> {
    ctx: &'a JobCtx,
    last: Cell<u64>,
}

impl TransferHooks for Hooks<'_> {
    fn checkpoint(&self) -> FsResult<()> {
        self.ctx.checkpoint()
    }

    fn on_bytes(&self, _delta: u64, done: u64, total: u64) {
        let last = self.last.get();
        if done >= last {
            self.ctx.on_bytes(ByteProgress { done, total, delta: done - last });
        } else {
            // A retry started over: take back what the failed attempt counted.
            self.ctx.credit_bytes(-((last - done) as i64));
        }
        self.last.set(done);
    }
}

pub(crate) fn run_remote(ctx: &JobCtx, op: &RemoteOp) -> FsResult<()> {
    match op {
        RemoteOp::Transfer { sources, dest_dir, mv } => run_transfer(ctx, sources, dest_dir, *mv),
        RemoteOp::Delete { paths } => run_delete(ctx, paths),
        RemoteOp::Mkdir { path } => run_mkdir(ctx, path),
        RemoteOp::Rename { from, to } => run_rename(ctx, from, to),
    }
}

/// Count files and bytes below `e` (for the progress totals).
fn count(ctx: &JobCtx, end: &End, e: &RemoteEntry, bytes: &mut u64, items: &mut u64) -> FsResult<()> {
    ctx.checkpoint()?;
    *items += 1;
    if !e.is_dir {
        *bytes += e.size;
        return Ok(());
    }
    if let Ok(children) = end.prov.list(&end.path) {
        for c in children {
            count(ctx, &end.child(&c.name), &c, bytes, items)?;
        }
    }
    Ok(())
}

fn run_transfer(ctx: &JobCtx, sources: &[PathBuf], dest_dir: &Path, mv: bool) -> FsResult<()> {
    let dest = match ctx.attempt(dest_dir, || End::resolve(dest_dir))? {
        Ok(d) => d,
        Err(e) => {
            ctx.set_running();
            for s in sources {
                ctx.item_failed(s, e.clone(), 0);
            }
            return Ok(());
        }
    };
    let mut plan: Vec<(End, RemoteEntry)> = Vec::new();
    let (mut bytes, mut items) = (0u64, 0u64);
    for s in sources {
        ctx.checkpoint()?;
        let r = ctx.attempt(s, || {
            let end = End::resolve(s)?;
            let mut st = end.prov.stat(&end.path)?;
            st.name = end.name().to_string();
            Ok((end, st))
        })?;
        match r {
            Ok((end, st)) => {
                count(ctx, &end, &st, &mut bytes, &mut items)?;
                plan.push((end, st));
            }
            Err(e) => ctx.item_failed(s, e, 0),
        }
    }
    ctx.set_totals(bytes, items);
    ctx.set_running();
    for (src, st) in plan {
        ctx.checkpoint()?;
        let dst = dest.child(&st.name);
        if is_within(&dst, &src) {
            let e = FsError::with_message(
                FsErrorKind::InvalidName,
                "copy",
                Some(&dst.display()),
                "cannot copy a folder into itself",
            );
            ctx.item_failed(&src.display(), e, st.size);
            continue;
        }
        if mv && src.same_server(&dest) {
            move_same_server(ctx, &src, &st, dst)?;
            continue;
        }
        let before = ctx.summary_counts();
        copy_entry(ctx, &src, &st, dst, false)?;
        if mv && ctx.summary_counts() == before {
            remove_source(ctx, &src, &st)?;
        } else if mv {
            ctx.log(
                crate::job::LogLevel::Warn,
                Some(src.display()),
                "not everything was copied; the source was kept".into(),
            );
        }
    }
    Ok(())
}

fn is_within(dst: &End, src: &End) -> bool {
    match (&dst.uri, &src.uri) {
        (Some(a), Some(b)) => {
            a.conn_key() == b.conn_key()
                && (dst.path == src.path || dst.path.starts_with(&format!("{}/", src.path.trim_end_matches('/'))))
        }
        (None, None) => Path::new(&dst.path).starts_with(&src.path),
        _ => false,
    }
}

/// Resolve a conflict at `dst`; `Ok(None)` = skip this item.
fn settle(ctx: &JobCtx, src: &End, se: &RemoteEntry, mut dst: End) -> FsResult<Option<(End, bool)>> {
    loop {
        ctx.checkpoint()?;
        let existing = match ctx.attempt(&dst.display(), || dst.stat_opt())? {
            Ok(e) => e,
            Err(e) => {
                ctx.item_failed(&src.display(), e, if se.is_dir { 0 } else { se.size });
                return Ok(None);
            }
        };
        let Some(de) = existing else { return Ok(Some((dst, false))) };
        if se.is_dir && de.is_dir && ctx.opts.copy.merge_dirs && ctx.opts.conflict.is_none() {
            return Ok(Some((dst, true)));
        }
        let res = ctx.resolve_conflict(conflict(src, se, &dst, &de))?;
        let credit = if se.is_dir { 0 } else { se.size };
        match res {
            ConflictResolution::Skip => {
                ctx.item_skipped(&src.display(), credit, "destination exists");
                return Ok(None);
            }
            ConflictResolution::KeepBoth => dst = unique_sibling(&dst),
            ConflictResolution::Rename(n) => dst = dst.sibling(&n),
            ConflictResolution::Cancel => return Err(ctx.cancel_now()),
            _ if se.is_dir && de.is_dir => return Ok(Some((dst, true))),
            _ if se.is_dir != de.is_dir => {
                let e = FsError::with_message(
                    FsErrorKind::AlreadyExists,
                    "copy",
                    Some(&dst.display()),
                    "a file and a folder have the same name",
                );
                ctx.item_failed(&src.display(), e, credit);
                return Ok(None);
            }
            ConflictResolution::OverwriteIfNewer if !(se.modified > de.modified) => {
                ctx.item_skipped(&src.display(), credit, "destination is newer");
                return Ok(None);
            }
            ConflictResolution::OverwriteIfLarger if se.size <= de.size => {
                ctx.item_skipped(&src.display(), credit, "destination is larger");
                return Ok(None);
            }
            ConflictResolution::OverwriteIfDifferent if se.size == de.size => {
                ctx.item_skipped(&src.display(), credit, "same size");
                return Ok(None);
            }
            _ => return Ok(Some((dst, true))),
        }
    }
}

/// Copy `src` (file or tree) to `dst`. `parent_new`: the parent folder was created by
/// this job (its record covers everything below).
fn copy_entry(ctx: &JobCtx, src: &End, se: &RemoteEntry, dst: End, parent_new: bool) -> FsResult<()> {
    if se.is_dir {
        ctx.begin_item(&src.display(), 0);
        let Some((dst, exists)) = settle(ctx, src, se, dst)? else { return Ok(()) };
        let created = if exists {
            false
        } else {
            match ctx.attempt(&dst.display(), || dst.prov.mkdir(&dst.path))? {
                Ok(()) => {
                    if !parent_new {
                        ctx.record(created_record(&dst, true, 0));
                    }
                    true
                }
                Err(e) => {
                    ctx.item_failed(&src.display(), e, 0);
                    return Ok(());
                }
            }
        };
        ctx.item_ok(0);
        let children = match ctx.attempt(&src.display(), || src.prov.list(&src.path))? {
            Ok(c) => c,
            Err(e) => {
                ctx.item_failed(&src.display(), e, 0);
                return Ok(());
            }
        };
        for c in children {
            ctx.checkpoint()?;
            copy_entry(ctx, &src.child(&c.name), &c, dst.child(&c.name), parent_new || created)?;
        }
        return Ok(());
    }
    ctx.begin_item(&src.display(), se.size);
    let Some((dst, replace)) = settle(ctx, src, se, dst)? else { return Ok(()) };
    if replace && !parent_new {
        // The old destination file is lost on a server; locally it goes to the trash.
        match trash_local(ctx, &dst) {
            Some(item) => ctx.record(UndoRecord::Trashed(item)),
            None => ctx.record(UndoRecord::Irreversible(dst.display())),
        }
    }
    let hooks = Hooks { ctx, last: Cell::new(0) };
    let fc = FileCopy {
        src: &*src.prov,
        src_path: &src.path,
        dst: &*dst.prov,
        dst_path: &dst.path,
        size: se.size,
        modified: se.modified,
        replace,
    };
    let r = ctx.attempt(&src.display(), || copy_file(&fc, &hooks));
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            // Cancelled (or aborted): never leave a partial file behind on cancel.
            if e.kind == FsErrorKind::Interrupted {
                discard_partial(&*dst.prov, &dst.path);
            }
            return Err(e);
        }
    };
    match r {
        Ok(stats) => {
            if stats.resumed_from > 0 {
                ctx.log(
                    crate::job::LogLevel::Info,
                    Some(dst.display()),
                    format!("resumed at {} bytes", stats.resumed_from),
                );
            }
            if !parent_new {
                ctx.record(created_record(&dst, false, se.size));
            }
            ctx.item_ok(se.size);
        }
        Err(e) => {
            let credit = se.size.saturating_sub(hooks.last.get());
            ctx.item_failed(&src.display(), e, credit);
        }
    }
    Ok(())
}

/// Move a local item to the trash when its volume has one (`None` otherwise or on error).
fn trash_local(ctx: &JobCtx, end: &End) -> Option<soyuz_fs::TrashItem> {
    if end.is_remote() {
        return None;
    }
    let p = Path::new(&end.path);
    match ctx.probe().trash_support(p) {
        soyuz_fs::TrashSupport::Native => soyuz_fs::trash::trash(p).ok(),
        soyuz_fs::TrashSupport::Unsupported(_) => None,
    }
}

fn created_record(dst: &End, is_dir: bool, size: u64) -> UndoRecord {
    if dst.is_remote() {
        UndoRecord::RemoteCreated { path: dst.display(), is_dir, size }
    } else {
        UndoRecord::Copied { dst: dst.display() }
    }
}

/// A move between two places on the same server: one rename.
fn move_same_server(ctx: &JobCtx, src: &End, se: &RemoteEntry, dst: End) -> FsResult<()> {
    ctx.begin_item(&src.display(), se.size);
    let Some((dst, replace)) = settle(ctx, src, se, dst)? else { return Ok(()) };
    if replace {
        // Same-server replace: remove the old one first (cannot be undone).
        if let Some(old) = dst.stat_opt()? {
            let check = || ctx.checkpoint();
            if let Err(e) = delete_tree(&*dst.prov, &dst.path, old.is_dir, &check) {
                ctx.item_failed(&src.display(), e, se.size);
                return Ok(());
            }
            ctx.record(UndoRecord::Irreversible(dst.display()));
        }
    }
    match ctx.attempt(&src.display(), || src.prov.rename(&src.path, &dst.path))? {
        Ok(()) => {
            ctx.record(UndoRecord::RemoteRenamed { from: src.display(), to: dst.display() });
            ctx.on_bytes(ByteProgress { done: se.size, total: se.size, delta: se.size });
            ctx.item_ok(se.size);
        }
        Err(e) => ctx.item_failed(&src.display(), e, se.size),
    }
    Ok(())
}

/// After a fully successful copy of a move: local sources go to the trash, remote ones
/// are deleted (not undoable).
fn remove_source(ctx: &JobCtx, src: &End, se: &RemoteEntry) -> FsResult<()> {
    let path = src.display();
    if !src.is_remote() {
        if let Some(item) = trash_local(ctx, src) {
            ctx.record(UndoRecord::Trashed(item));
            return Ok(());
        }
        // No trash on that volume: ask before deleting the source permanently.
        if !ctx.trash_unsupported(&path, "the source would be deleted permanently after the move")? {
            ctx.item_skipped(&path, 0, "source kept: the trash is not available");
            return Ok(());
        }
        match ctx.attempt(&path, || crate::kinds::delete::remove_path_quiet(ctx, &path))? {
            Ok(()) => ctx.record(UndoRecord::Irreversible(path)),
            Err(e) => ctx.item_failed(&path, e, 0),
        }
        return Ok(());
    }
    let check = || ctx.checkpoint();
    match ctx.attempt(&path, || delete_tree(&*src.prov, &src.path, se.is_dir, &check))? {
        Ok(()) => ctx.record(UndoRecord::Irreversible(path)),
        Err(e) => ctx.item_failed(&path, e, 0),
    }
    Ok(())
}

fn run_delete(ctx: &JobCtx, paths: &[PathBuf]) -> FsResult<()> {
    ctx.set_totals(0, paths.len() as u64);
    ctx.set_running();
    for p in paths {
        ctx.checkpoint()?;
        ctx.begin_item(p, 0);
        let r = ctx.attempt(p, || {
            let end = End::resolve(p)?;
            let st = end.prov.stat(&end.path)?;
            let check = || ctx.checkpoint();
            delete_tree(&*end.prov, &end.path, st.is_dir && !st.is_link, &check)
        })?;
        match r {
            Ok(()) => {
                ctx.record(UndoRecord::Irreversible(p.clone()));
                ctx.item_ok(0);
            }
            Err(e) => ctx.item_failed(p, e, 0),
        }
    }
    Ok(())
}

fn run_mkdir(ctx: &JobCtx, path: &Path) -> FsResult<()> {
    ctx.set_totals(0, 1);
    ctx.set_running();
    ctx.begin_item(path, 0);
    match ctx.attempt(path, || {
        let end = End::resolve(path)?;
        end.prov.mkdir(&end.path)
    })? {
        Ok(()) => {
            ctx.record(UndoRecord::RemoteCreated { path: path.to_path_buf(), is_dir: true, size: 0 });
            ctx.item_ok(0);
        }
        Err(e) => ctx.item_failed(path, e, 0),
    }
    Ok(())
}

fn run_rename(ctx: &JobCtx, from: &Path, to: &Path) -> FsResult<()> {
    ctx.set_totals(0, 1);
    ctx.set_running();
    ctx.begin_item(from, 0);
    let r = ctx.attempt(from, || {
        let (a, b) = (End::resolve(from)?, End::resolve(to)?);
        if b.stat_opt()?.is_some() {
            return Err(FsError::with_message(FsErrorKind::AlreadyExists, "rename", Some(to), "the new name is taken"));
        }
        a.prov.rename(&a.path, &b.path)
    })?;
    match r {
        Ok(()) => {
            ctx.record(UndoRecord::RemoteRenamed { from: from.to_path_buf(), to: to.to_path_buf() });
            ctx.item_ok(0);
        }
        Err(e) => ctx.item_failed(from, e, 0),
    }
    Ok(())
}

/// Undo of [`UndoRecord::RemoteCreated`]. Returns whether the item is gone afterwards.
pub(crate) fn undo_created(ctx: &JobCtx, path: &Path, is_dir: bool, size: u64) -> FsResult<bool> {
    ctx.begin_item(path, 0);
    let end = match End::resolve(path) {
        Ok(e) => e,
        Err(e) => {
            ctx.item_failed(path, e, 0);
            return Ok(false);
        }
    };
    let st = match ctx.attempt(path, || end.stat_opt())? {
        Ok(Some(st)) => st,
        Ok(None) => {
            ctx.item_skipped(path, 0, "already gone");
            return Ok(true);
        }
        Err(e) => {
            ctx.item_failed(path, e, 0);
            return Ok(false);
        }
    };
    if st.is_dir != is_dir || (!is_dir && st.size != size) {
        let e = FsError::with_message(
            FsErrorKind::Io,
            "undo",
            Some(path),
            "it changed after the operation (different item or size), so undo left it alone",
        );
        ctx.item_failed(path, e, 0);
        return Ok(false);
    }
    if is_dir {
        let non_empty = end.prov.list(&end.path).is_ok_and(|c| !c.is_empty());
        if non_empty
            && !ctx.trash_unsupported(
                path,
                "servers have no trash; undo deletes this folder and its contents permanently",
            )?
        {
            ctx.item_skipped(path, 0, "kept");
            return Ok(false);
        }
    }
    let check = || ctx.checkpoint();
    match ctx.attempt(path, || delete_tree(&*end.prov, &end.path, is_dir, &check))? {
        Ok(()) => {
            ctx.item_ok(0);
            Ok(true)
        }
        Err(e) => {
            ctx.item_failed(path, e, 0);
            Ok(false)
        }
    }
}

/// Undo of [`UndoRecord::RemoteRenamed`]: rename `to` back to `from`.
pub(crate) fn undo_renamed(ctx: &JobCtx, from: &Path, to: &Path) -> FsResult<bool> {
    ctx.begin_item(to, 0);
    let r = ctx.attempt(to, || {
        let (a, b) = (End::resolve(to)?, End::resolve(from)?);
        if b.stat_opt()?.is_some() {
            return Err(FsError::with_message(FsErrorKind::AlreadyExists, "undo", Some(from), "the old name is taken"));
        }
        a.prov.rename(&a.path, &b.path)
    })?;
    match r {
        Ok(()) => {
            ctx.item_ok(0);
            Ok(true)
        }
        Err(e) => {
            ctx.item_failed(to, e, 0);
            Ok(false)
        }
    }
}

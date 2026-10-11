//! Metadata actions (Properties, checksums, batch rename, bulk attributes / timestamps,
//! touch, links): which ones exist, what they work on, and how their results are
//! applied — `SetMeta` / `Rename` / `Create` jobs (undoable, in the activity panel),
//! chown, and the sequenced Linux inode-flag changes ([`FlagPlan`]). The dialogs that
//! edit the values are frontend widgets (`ui::dialogs::properties` and friends); the
//! core asks for them through [`FrontendCmd`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;
use soyuz_core::config::Action;
use soyuz_core::{EntryInfo, FsError, FsErrorKind, HashAlgo, Location};
use soyuz_fs::attrs::InodeFlags;
use soyuz_fs::{AttrSet, TimesSet};
use soyuz_jobs::JobKind;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::host::Host;
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;
use crate::services::jobs::{JobMeta, OpKind};
use crate::state::pane::PaneId;

/// One `SetMeta` job.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaJob {
    pub paths: Vec<PathBuf>,
    pub times: Option<TimesSet>,
    pub attrs: Option<AttrSet>,
    pub mode: Option<u32>,
}

/// Commands produced by the metadata dialogs (applied by [`apply`] with app access).
#[derive(Clone, Debug)]
pub enum MetaCmd {
    /// Submit `SetMeta` jobs (attributes / times / mode).
    SetMeta(Vec<MetaJob>),
    /// Change owner / group (unix; not undoable).
    Chown { paths: Vec<PathBuf>, uid: Option<u32>, gid: Option<u32> },
    /// Submit a batch rename job.
    BatchRename { pane: PaneId, pairs: Vec<(PathBuf, PathBuf)> },
    /// Create a link (`Create` job) at `path` inside `dir`.
    Create { dir: PathBuf, path: PathBuf, kind: soyuz_jobs::CreateKind },
    /// Linux: an Apply that switches inode flags on / off, sequenced with the other
    /// changes of the same Apply (see [`FlagPlan`]).
    InodeFlags(Box<FlagPlan>),
}

/// One Apply of the Properties dialog that changes Linux inode flags. The flags are not a
/// job kind (they are quick ioctls with no undo, like chown), so they are ordered around
/// the `SetMeta` / `Rename` jobs of the same Apply instead: flags are cleared first (an
/// immutable / append-only file can't be renamed or touched), then the jobs run (meta
/// jobs, then the rename), then flags are set on the paths as they are after the rename.
/// Each step runs off the UI thread; [`pump`] advances it once per frame.
#[derive(Clone, Debug)]
pub struct FlagPlan {
    /// The items as they are now (flags are cleared here).
    pub paths: Vec<PathBuf>,
    pub set: InodeFlags,
    pub clear: InodeFlags,
    pub meta: Vec<MetaJob>,
    pub chown: Option<(Vec<PathBuf>, Option<u32>, Option<u32>)>,
    /// `(pane, from, new name)`.
    pub rename: Option<(PaneId, PathBuf, String)>,
    /// Set once everything ran (the dialog re-reads the flags from disk then).
    pub done: Arc<std::sync::atomic::AtomicBool>,
}

impl FlagPlan {
    pub fn has_jobs(&self) -> bool {
        !self.meta.is_empty() || self.chown.is_some() || self.rename.is_some()
    }

    /// The paths after a successful rename.
    pub fn renamed_paths(&self) -> Vec<PathBuf> {
        let mut out = self.paths.clone();
        if let Some((_, from, to)) = &self.rename {
            for p in &mut out {
                if p == from {
                    *p = from.with_file_name(to);
                }
            }
        }
        out
    }
}

/// A value computed on a worker thread (the frontend is woken when it lands).
pub struct Bg<T> {
    slot: Arc<Mutex<Option<T>>>,
}

impl<T: Send + 'static> Bg<T> {
    pub fn spawn(host: &dyn Host, f: impl FnOnce() -> T + Send + 'static) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        let wake = host.wake();
        let _ = std::thread::Builder::new().name("soyuz-props".into()).spawn(move || {
            let v = f();
            *s2.lock() = Some(v);
            wake();
        });
        Bg { slot }
    }

    pub fn with<R>(&self, f: impl FnOnce(Option<&T>) -> R) -> R {
        f(self.slot.lock().as_ref())
    }
}

/// Where a [`FlagPlan`] is.
enum FlagStage {
    /// Clearing flags (or, with no jobs, setting and clearing in one go).
    Clear(Bg<Vec<Option<FsError>>>),
    /// Waiting for the `SetMeta` jobs.
    Meta(Vec<soyuz_jobs::JobId>),
    /// Waiting for the rename job.
    Rename(soyuz_jobs::JobId),
    /// Setting flags on `paths`.
    Set(Bg<Vec<Option<FsError>>>),
}

struct FlagRun {
    plan: FlagPlan,
    stage: FlagStage,
    /// Per path: the first failure.
    failed: Vec<Option<FsError>>,
    /// The paths the flags are set on (post-rename once the rename succeeded).
    targets: Vec<PathBuf>,
}

static FLAG_RUNS: Mutex<Vec<FlagRun>> = parking_lot::const_mutex(Vec::new());

/// Change flags on every path (continuing past failures); `None` per success.
fn flag_worker(host: &dyn Host, paths: Vec<PathBuf>, set: InodeFlags, clear: InodeFlags) -> Bg<Vec<Option<FsError>>> {
    Bg::spawn(host, move || {
        paths
            .iter()
            .map(|p| {
                if set.is_empty() && clear.is_empty() {
                    return None;
                }
                soyuz_fs::attrs::set_inode_flags(p, set, clear).err()
            })
            .collect()
    })
}

/// Take a finished worker's results into `failed`; `false` while it still runs.
fn take_results(bg: &Bg<Vec<Option<FsError>>>, failed: &mut [Option<FsError>]) -> bool {
    bg.with(|r| match r {
        None => false,
        Some(r) => {
            for (slot, e) in failed.iter_mut().zip(r) {
                if slot.is_none() {
                    *slot = e.clone();
                }
            }
            true
        }
    })
}

fn submit_meta_jobs(app: &mut SoyuzApp, jobs: &[MetaJob]) -> Vec<soyuz_jobs::JobId> {
    let mut ids = Vec::new();
    for j in jobs {
        if j.paths.is_empty() {
            continue;
        }
        let kind = JobKind::SetMeta { paths: j.paths.clone(), times: j.times, attrs: j.attrs, mode: j.mode };
        let spec = app.fileops.jobs.spec(kind);
        let meta = JobMeta::new(OpKind::SetMeta, &j.paths, None);
        ids.push(app.fileops.jobs.submit(spec, meta));
    }
    ids
}

fn submit_rename(app: &mut SoyuzApp, pane: PaneId, from: &std::path::Path, to: &str) -> soyuz_jobs::JobId {
    // Same job as `FileOpCmd::Rename` (inline rename), with its id kept for sequencing.
    let to_path = from.with_file_name(to);
    let spec = app.fileops.jobs.spec(JobKind::Rename { pairs: vec![(from.to_path_buf(), to_path)] });
    let meta = JobMeta::new(OpKind::Rename, &[from.to_path_buf()], Some(PathBuf::from(to)))
        .after(crate::services::jobs::AfterJob::Select { pane, name: to.to_string() })
        .quiet();
    app.fileops.jobs.submit(spec, meta)
}

fn job_finished(app: &SoyuzApp, id: soyuz_jobs::JobId) -> bool {
    app.fileops.jobs.snapshot(id).is_none_or(|s| s.state.is_terminal())
}

fn job_succeeded(app: &SoyuzApp, id: soyuz_jobs::JobId) -> bool {
    app.fileops.jobs.snapshot(id).is_some_and(|s| matches!(s.state, soyuz_jobs::JobState::Done(_)))
}

/// Per frame: advance the running [`FlagPlan`]s (from the frame loop).
pub fn pump(app: &mut SoyuzApp, host: &dyn Host) {
    let mut runs = std::mem::take(&mut *FLAG_RUNS.lock());
    if runs.is_empty() {
        return;
    }
    let mut keep = Vec::new();
    for mut run in runs.drain(..) {
        loop {
            let next = match &run.stage {
                FlagStage::Clear(bg) => {
                    if !take_results(bg, &mut run.failed) {
                        break;
                    }
                    if !run.plan.has_jobs() {
                        None
                    } else {
                        if let Some((paths, uid, gid)) = run.plan.chown.clone() {
                            apply(app, MetaCmd::Chown { paths, uid, gid }, host);
                        }
                        Some(FlagStage::Meta(submit_meta_jobs(app, &run.plan.meta)))
                    }
                }
                FlagStage::Meta(ids) => {
                    if !ids.iter().all(|id| job_finished(app, *id)) {
                        break;
                    }
                    match run.plan.rename.clone() {
                        Some((pane, from, to)) => Some(FlagStage::Rename(submit_rename(app, pane, &from, &to))),
                        None => Some(FlagStage::Set(flag_worker(
                            host,
                            run.targets.clone(),
                            run.plan.set,
                            InodeFlags::empty(),
                        ))),
                    }
                }
                FlagStage::Rename(id) => {
                    if !job_finished(app, *id) {
                        break;
                    }
                    if job_succeeded(app, *id) {
                        run.targets = run.plan.renamed_paths();
                    }
                    Some(FlagStage::Set(flag_worker(host, run.targets.clone(), run.plan.set, InodeFlags::empty())))
                }
                FlagStage::Set(bg) => {
                    if !take_results(bg, &mut run.failed) {
                        break;
                    }
                    None
                }
            };
            match next {
                Some(stage) => run.stage = stage,
                None => {
                    finish_flags(app, &run);
                    break;
                }
            }
        }
        if !run.plan.done.load(std::sync::atomic::Ordering::Relaxed) {
            keep.push(run);
        }
    }
    FLAG_RUNS.lock().extend(keep);
}

/// Plain-language reason for a failed inode-flag change.
pub fn apply_reason(e: &FsError, touched: InodeFlags) -> String {
    match e.kind {
        FsErrorKind::PermissionDenied if touched.intersects(InodeFlags::IMMUTABLE | InodeFlags::APPEND) => {
            tr!("dialogs-inode-needs-root").to_string()
        }
        FsErrorKind::PermissionDenied => tr!("dialogs-inode-denied").to_string(),
        FsErrorKind::Unsupported => tr!("dialogs-inode-unsupported").to_string(),
        FsErrorKind::ReadOnlyFs => tr!("dialogs-inode-read-only-fs").to_string(),
        _ => e.to_string(),
    }
}

/// Report a finished [`FlagPlan`] (a toast counting the failures) and let the dialog
/// re-read the flags.
fn finish_flags(app: &mut SoyuzApp, run: &FlagRun) {
    let failures: Vec<&FsError> = run.failed.iter().flatten().collect();
    let toast = match failures.first() {
        None => Toast::success(tr!("dialogs-inode-changed")),
        Some(e) => {
            let reason = apply_reason(e, run.plan.set | run.plan.clear);
            let title = if run.failed.len() > 1 {
                tr!("dialogs-inode-failed-some", failed = failures.len(), total = run.failed.len()).to_string()
            } else {
                tr!("dialogs-inode-failed").to_string()
            };
            Toast::error(title).body(reason)
        }
    };
    app.ui.toasts.push(toast);
    run.plan.done.store(true, std::sync::atomic::Ordering::Relaxed);
    refresh_panes(app);
}

/// Actions handled here (routed by `actions::execute` before the file-op module).
pub fn handles(app: &SoyuzApp, a: &Action) -> bool {
    use Action as A;
    match a {
        A::Properties
        | A::BatchRename
        | A::SetAttributes
        | A::TouchTimestamps
        | A::Checksum(_)
        | A::CompareChecksum
        | A::NewSymlink
        | A::NewHardlink
        | A::NewJunction => true,
        // F2 on a multiple selection opens the batch rename dialog.
        A::Rename => matches!(app.ws.pane().location, Location::Dir(_)) && app.ws.pane().selection.len() > 1,
        _ => false,
    }
}

/// The kind of link a "new link" dialog creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Symlink,
    Hardlink,
    Junction,
}

/// A metadata dialog the frontend opens on the focused pane's selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaDialog {
    BatchRename,
    Attributes,
    Link(LinkKind),
}

pub fn execute(app: &mut SoyuzApp, action: Action, _host: &dyn Host) {
    use Action as A;
    match action {
        A::Properties => open(app, Tab::General, None),
        A::Checksum(algo) => open(app, Tab::Checksums, Some(Start::Hash(algo))),
        A::CompareChecksum => open(app, Tab::Checksums, Some(Start::Compare)),
        A::BatchRename | A::Rename => app.frontend.push(FrontendCmd::MetaDialog(MetaDialog::BatchRename)),
        A::SetAttributes => app.frontend.push(FrontendCmd::MetaDialog(MetaDialog::Attributes)),
        A::TouchTimestamps => touch(app),
        A::NewSymlink => app.frontend.push(FrontendCmd::MetaDialog(MetaDialog::Link(LinkKind::Symlink))),
        A::NewHardlink => app.frontend.push(FrontendCmd::MetaDialog(MetaDialog::Link(LinkKind::Hardlink))),
        A::NewJunction => app.frontend.push(FrontendCmd::MetaDialog(MetaDialog::Link(LinkKind::Junction))),
        _ => {}
    }
}

/// Apply a [`MetaCmd`].
pub fn apply(app: &mut SoyuzApp, cmd: MetaCmd, host: &dyn Host) {
    match cmd {
        MetaCmd::SetMeta(jobs) => {
            for j in jobs {
                if j.paths.is_empty() {
                    continue;
                }
                let kind = JobKind::SetMeta { paths: j.paths.clone(), times: j.times, attrs: j.attrs, mode: j.mode };
                let spec = app.fileops.jobs.spec(kind);
                let meta = JobMeta::new(OpKind::SetMeta, &j.paths, None);
                app.fileops.jobs.submit(spec, meta);
            }
            refresh_panes(app);
        }
        MetaCmd::Chown { paths, uid, gid } => {
            let mut failed = Vec::new();
            for p in &paths {
                if let Err(e) = soyuz_fs::chown(p, uid, gid) {
                    failed.push(e.to_string());
                }
            }
            match failed.first() {
                None => {
                    app.ui.toasts.push(Toast::success(tr!("dialogs-props-owner-changed")));
                }
                Some(e) => {
                    app.ui.toasts.push(Toast::error(tr!("dialogs-props-owner-failed")).body(e.clone()));
                }
            }
            refresh_panes(app);
        }
        MetaCmd::BatchRename { pane, pairs } => {
            if pairs.is_empty() {
                return;
            }
            let from: Vec<PathBuf> = pairs.iter().map(|(a, _)| a.clone()).collect();
            let first_new = pairs[0].1.file_name().map(|n| n.to_string_lossy().into_owned());
            let spec = app.fileops.jobs.spec(JobKind::Rename { pairs });
            let mut meta = JobMeta::new(OpKind::Rename, &from, None);
            if let Some(name) = first_new {
                meta = meta.after(crate::services::jobs::AfterJob::Select { pane, name });
            }
            app.fileops.jobs.submit(spec, meta);
        }
        MetaCmd::InodeFlags(plan) => {
            let plan = *plan;
            // With jobs to run: clear first, set after them. Alone: one call per path.
            let (set, clear) = if plan.has_jobs() { (InodeFlags::empty(), plan.clear) } else { (plan.set, plan.clear) };
            let stage = FlagStage::Clear(flag_worker(host, plan.paths.clone(), set, clear));
            let failed = vec![None; plan.paths.len()];
            let targets = plan.paths.clone();
            FLAG_RUNS.lock().push(FlagRun { plan, stage, failed, targets });
        }
        MetaCmd::Create { dir, path, kind } => {
            let spec = app.fileops.jobs.spec(JobKind::Create { path: path.clone(), kind });
            let meta = JobMeta::new(OpKind::Link, std::slice::from_ref(&path), Some(dir));
            app.fileops.jobs.submit(spec, meta);
        }
    }
}

fn refresh_panes(app: &mut SoyuzApp) {
    // Watchers report attribute / time changes inconsistently across platforms; mark the
    // panes dirty so the next frame re-reads them.
    for t in &mut app.ws.tabs {
        for p in &mut t.panes {
            p.dirty = true;
        }
    }
}

/// Entries the metadata actions work on: the selection of the focused pane, or the
/// folder itself when nothing is selected. Archives / trash / computer are refused.
pub fn targets(app: &SoyuzApp) -> Result<(PaneId, Vec<EntryInfo>), String> {
    let pane = app.ws.pane();
    match &pane.location {
        Location::Dir(_) | Location::Search(_) | Location::Recent | Location::Tags(_) => {}
        _ => return Err(tr!("dialogs-props-unavailable").into()),
    }
    let mut sel: Vec<EntryInfo> = pane.selected_entries().into_iter().cloned().collect();
    if sel.is_empty()
        && let Location::Dir(d) = &pane.location
    {
        sel.push(soyuz_fs::stat(d, false).map_err(|e| e.to_string())?);
    }
    if sel.is_empty() {
        return Err(tr!("dialogs-props-select-first").into());
    }
    Ok((pane.id, sel))
}

/// "Touch": set modified + accessed to now on the selection (undoable job).
fn touch(app: &mut SoyuzApp) {
    match targets(app) {
        Ok((_, entries)) => {
            let now = SystemTime::now();
            let t = TimesSet { modified: Some(now), accessed: Some(now), created: None };
            let paths: Vec<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
            let jobs = vec![MetaJob { paths, times: Some(t), attrs: None, mode: None }];
            app.cmds.push(Cmd::Meta(MetaCmd::SetMeta(jobs)));
        }
        Err(e) => {
            app.ui.toasts.push(Toast::info(e));
        }
    }
}

/// Tabs of the Properties dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    General,
    Attributes,
    Security,
    Checksums,
    Details,
    /// Earlier versions from shadow copies.
    Versions,
}

/// What to do right after opening.
#[derive(Clone, Copy, Debug)]
pub enum Start {
    Hash(HashAlgo),
    Compare,
}

/// Open the Properties dialog on the focused pane's selection (a toast when there is
/// nothing it can show).
pub fn open(app: &mut SoyuzApp, tab: Tab, start: Option<Start>) {
    match targets(app) {
        Ok((pane, entries)) => app.frontend.push(FrontendCmd::Properties { pane, entries, tab, start }),
        Err(e) => {
            app.ui.toasts.push(Toast::info(e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn plan(paths: Vec<PathBuf>, rename: Option<(PathBuf, String)>) -> FlagPlan {
        FlagPlan {
            paths,
            set: InodeFlags::NODUMP,
            clear: InodeFlags::empty(),
            meta: Vec::new(),
            chown: None,
            rename: rename.map(|(f, t)| (PaneId(1), f, t)),
            done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[test]
    fn flags_follow_the_rename() {
        let a = PathBuf::from("/tmp/x/a.txt");
        let p = plan(vec![a.clone()], Some((a.clone(), "b.txt".into())));
        assert!(p.has_jobs());
        assert_eq!(p.renamed_paths(), vec![PathBuf::from("/tmp/x/b.txt")]);
        let p = plan(vec![a.clone(), PathBuf::from("/tmp/x/c")], None);
        assert!(!p.has_jobs(), "flags alone run in one step");
        assert_eq!(p.renamed_paths(), p.paths);
    }

    #[test]
    fn flag_failures_are_counted_per_item_and_do_not_stop_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("ok.txt");
        std::fs::write(&ok, b"x").unwrap();
        let missing = dir.path().join("missing.txt");
        let host = crate::host::NullHost::default();
        // An empty change touches nothing and never fails.
        let bg = flag_worker(&host, vec![missing.clone(), ok.clone()], InodeFlags::empty(), InodeFlags::empty());
        let mut failed = vec![None, None];
        while !take_results(&bg, &mut failed) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(failed.iter().all(Option::is_none));
        // The missing item fails; the worker still reaches the second one.
        let bg = flag_worker(&host, vec![missing, ok], InodeFlags::NODUMP, InodeFlags::empty());
        let mut failed = vec![None, None];
        while !take_results(&bg, &mut failed) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(failed[0].is_some(), "a missing file is reported");
        if cfg!(not(target_os = "linux")) {
            assert!(failed[1].is_some(), "unsupported off Linux");
        }
    }
}

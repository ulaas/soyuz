//! File operations integration: every copy / move / delete / rename / create is a
//! background job on the app's [`JobsService`]. This module owns the glue: the internal
//! clipboard, drag and drop, inline rename, the prompt queue, completion toasts with
//! Undo, undo/redo, and the per-frame hooks (`begin_frame` / `end_frame`) called by
//! `SoyuzApp`. Widgets reach it through [`FileOpCmd`] (wrapped in `Cmd::FileOp`) and
//! `actions::execute` forwards file-op actions to [`execute`].

pub mod clipboard;
pub mod dnd;
pub mod dragout;
pub mod linux_dnd;
pub mod names;
pub mod prompts;
pub mod rename;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use egui::{Id, Pos2};
use soyuz_core::config::{Action, Config, PasteConflict, TrashUnsupportedPolicy};
use soyuz_core::{ConflictResolution, Location};
use soyuz_fs::{LinkKind, TrashItem};
use soyuz_jobs::{
    CreateKind, DeleteMode, HistoryId, JobEvent, JobId, JobKind, JobPrompt, JobState, JobSummary, PromptResponse,
};

use crate::app::SoyuzApp;
use crate::services::jobs::{AfterJob, JobMeta, JobsService, OpKind};
use crate::state::pane::PaneId;
use crate::ui::dialogs::conflict::{PromptDialog, PromptShared};
use crate::ui::dialogs::delete_confirm::DeleteConfirmDialog;
use crate::ui::dialogs::new_item::{LinkChoice, LinkDialog, TransferDialog};
use crate::ui::panels::activity::{ActivityState, ActivityTab};
use crate::ui::toasts::Toast;
use crate::ui::views::trash::TrashState;

use clipboard::{ClipMode, Clipboard};
use dnd::{DndFrame, DropEffect};
use prompts::PromptQueue;
use rename::InlineRename;

/// Commands produced by widgets and dialogs (applied after the UI pass).
#[derive(Clone, Debug)]
pub enum FileOpCmd {
    /// An internal drag was released over `dest`; `effect = None` asks.
    Drop {
        paths: Vec<PathBuf>,
        dest: PathBuf,
        effect: Option<DropEffect>,
        pos: Pos2,
    },
    /// Copy / move / link `paths` into `dest`.
    Transfer {
        paths: Vec<PathBuf>,
        dest: PathBuf,
        effect: DropEffect,
    },
    /// Move to the trash (sidebar Trash drop target).
    TrashPaths(Vec<PathBuf>),
    /// Confirmed delete.
    Delete {
        paths: Vec<PathBuf>,
        mode: DeleteMode,
    },
    /// Inline rename committed.
    Rename {
        pane: PaneId,
        from: PathBuf,
        to: String,
    },
    /// Paste the clipboard into a folder (context menu on a folder / other pane).
    PasteInto(PathBuf),
    /// Create a link `dir/name` → `target`.
    CreateLink {
        dir: PathBuf,
        name: String,
        target: PathBuf,
        kind: LinkChoice,
    },
    /// Answer a job's prompt.
    Respond {
        job: JobId,
        response: PromptResponse,
    },
    /// Show a deferred prompt again.
    Resolve(JobId),
    CancelJob(JobId),
    /// Ask, then empty the trash.
    ConfirmEmptyTrash,
    /// Confirmed "Empty trash".
    EmptyTrash,
    /// Ask, then delete trashed items permanently.
    ConfirmPurge(Vec<TrashItem>),
    TrashRestore(Vec<TrashItem>),
    /// Confirmed permanent delete of trashed items.
    TrashPurge(Vec<TrashItem>),
}

pub struct FileOps {
    pub jobs: JobsService,
    pub clipboard: Clipboard,
    pub rename: Option<InlineRename>,
    pub prompts: PromptQueue,
    pub dnd: DndFrame,
    pub trash: TrashState,
    pub activity: ActivityState,
    prompt_dialog: Option<(JobId, Rc<RefCell<PromptShared>>)>,
    pending_rename: Option<(PaneId, PathBuf, Instant)>,
    auto_opened: HashSet<JobId>,
    /// History entries undone through Ctrl+Z (redo pops them).
    undo_stack: Vec<HistoryId>,
}

impl FileOps {
    pub fn new(ctx: &egui::Context, config: &Config) -> Self {
        FileOps {
            jobs: JobsService::new(ctx, config),
            clipboard: Clipboard::default(),
            rename: None,
            prompts: PromptQueue::default(),
            dnd: DndFrame::default(),
            trash: TrashState::new(ctx),
            activity: ActivityState::default(),
            prompt_dialog: None,
            pending_rename: None,
            auto_opened: HashSet::new(),
            undo_stack: Vec::new(),
        }
    }

    /// The item at `path` is being renamed inline in `pane`.
    pub fn renaming(&self, pane: PaneId, name: &str) -> bool {
        self.rename.as_ref().is_some_and(|r| r.pane == pane && r.key == name)
    }
}

/// Actions this module implements (routed here by `actions::execute`).
pub fn handles(a: &Action) -> bool {
    use Action as A;
    matches!(
        a,
        A::Copy
            | A::Cut
            | A::Paste
            | A::PasteAsLink
            | A::PasteShortcut
            | A::PasteAndVerify
            | A::CopyToOtherPane
            | A::MoveToOtherPane
            | A::CopyTo
            | A::MoveTo
            | A::Duplicate
            | A::Rename
            | A::NewFolder
            | A::NewFile
            | A::NewSymlink
            | A::NewHardlink
            | A::NewJunction
            | A::Delete
            | A::DeletePermanent
            | A::SecureWipe
            | A::Undo
            | A::Redo
            | A::EmptyTrash
            | A::RestoreFromTrash
            | A::JobsPauseAll
            | A::JobsResumeAll
            | A::JobsCancelAll
            | A::JobsClearFinished
            | A::JobsShowHistory
    )
}

/// Focused pane: id, its folder (when it is a real directory) and the selected paths.
fn focused(app: &SoyuzApp) -> (PaneId, Option<PathBuf>, Vec<PathBuf>) {
    let pane = app.ws.pane();
    let dir = match &pane.location {
        Location::Dir(d) => Some(d.clone()),
        _ => None,
    };
    let sel = if dir.is_some() { pane.selected_entries().iter().map(|e| e.path.clone()).collect() } else { Vec::new() };
    (pane.id, dir, sel)
}

fn other_pane_dir(app: &SoyuzApp) -> Option<PathBuf> {
    app.ws.tab().other().and_then(|p| match &p.location {
        Location::Dir(d) => Some(d.clone()),
        _ => None,
    })
}

pub fn execute(app: &mut SoyuzApp, action: Action, ctx: &egui::Context) {
    use Action as A;
    let (pane_id, dir, sel) = focused(app);
    let in_trash = matches!(app.ws.pane().location, Location::Trash);
    match action {
        A::Copy | A::Cut => {
            if sel.is_empty() {
                return;
            }
            let mode = if action == A::Cut { ClipMode::Cut } else { ClipMode::Copy };
            // The OS clipboard gets the files in the native format (Explorer / Finder /
            // Nautilus can paste them) plus the paths as text; without a native
            // mechanism, just the text (Ctrl+V then still reaches the app).
            if !crate::services::os_clipboard::set_files(mode, &sel) {
                ctx.copy_text(Clipboard::os_text(&sel));
            }
            let n = sel.len();
            app.fileops.clipboard.set(mode, sel);
            let title =
                if mode == ClipMode::Cut { tr!("ops-clip-cut", count = n) } else { tr!("ops-clip-copied", count = n) };
            let mut t = Toast::info(title).body(tr!("ops-clip-paste-hint"));
            t.duration = Some(2.0);
            app.ui.toasts.push(t);
        }
        A::Paste => {
            if let Some(d) = dir {
                paste_into(app, d);
            }
        }
        A::PasteAndVerify => {
            if let Some(d) = dir {
                paste_into_with(app, d, true);
            }
        }
        A::PasteAsLink | A::PasteShortcut => {
            let Some(d) = dir else { return };
            let Some((_, paths)) = app.fileops.clipboard.for_paste() else {
                app.ui.toasts.push(Toast::info(tr!("ops-nothing-to-paste")));
                return;
            };
            transfer(app, paths, d, DropEffect::Link);
        }
        A::CopyToOtherPane | A::MoveToOtherPane => match other_pane_dir(app) {
            Some(dest) if !sel.is_empty() => {
                let effect = if action == A::CopyToOtherPane { DropEffect::Copy } else { DropEffect::Move };
                transfer(app, sel, dest, effect);
            }
            Some(_) => {}
            None if action == A::CopyToOtherPane && !app.ws.tab().dual => crate::ops::refresh(app),
            None => {
                app.ui.toasts.push(Toast::info(tr!("ops-no-other-pane")).body(tr!("ops-no-other-pane-body")));
            }
        },
        A::CopyTo | A::MoveTo => {
            if sel.is_empty() {
                return;
            }
            let start = other_pane_dir(app).or(dir).unwrap_or_default();
            let mv = action == A::MoveTo;
            app.ui.dialogs.push(TransferDialog::new(sel, start, mv));
        }
        A::Duplicate => {
            if let Some(d) = dir
                && !sel.is_empty()
            {
                let kind = JobKind::Copy { sources: sel.clone(), dest_dir: d.clone(), rename_to: None };
                let mut spec = app.fileops.jobs.spec(kind);
                spec.options.conflict = Some(ConflictResolution::KeepBoth);
                let meta = JobMeta::new(OpKind::Duplicate, &sel, Some(d));
                app.fileops.jobs.submit(spec, meta);
            }
        }
        A::Rename => start_rename(app, pane_id),
        A::NewFolder | A::NewFile => {
            let Some(d) = dir else { return };
            let folder = action == A::NewFolder;
            let pane = app.ws.pane();
            let names: Vec<&str> = pane.entries.iter().map(|e| e.name.as_str()).collect();
            let base = if folder { tr!("ops-default-folder-name") } else { tr!("ops-default-file-name") };
            let name = names::unique_in(&names, base, names::case_insensitive_fs());
            let path = d.join(&name);
            let kind = if folder { CreateKind::Dir } else { CreateKind::File { template: None } };
            let spec = app.fileops.jobs.spec(JobKind::Create { path: path.clone(), kind });
            let op = if folder { OpKind::NewFolder } else { OpKind::NewFile };
            let meta = JobMeta::new(op, std::slice::from_ref(&path), Some(d))
                .after(AfterJob::SelectAndRename { pane: pane_id, path })
                .quiet();
            app.fileops.jobs.submit(spec, meta);
        }
        A::NewSymlink | A::NewHardlink | A::NewJunction => {
            let Some(d) = dir else { return };
            let choice = match action {
                A::NewHardlink => LinkChoice::Hardlink,
                A::NewJunction => LinkChoice::Junction,
                _ => LinkChoice::Symlink,
            };
            let names: Vec<String> = app.ws.pane().entries.iter().map(|e| e.name.clone()).collect();
            let windows_rules =
                rename::windows_rules_for(app.config.general.strict_windows_names, &app.ui.volumes, Some(&d));
            app.ui.dialogs.push(LinkDialog::new(d, sel.first().cloned(), choice, names).windows_rules(windows_rules));
        }
        A::Delete | A::DeletePermanent | A::SecureWipe => {
            if in_trash {
                let items = app.fileops.trash.selected_items();
                if !items.is_empty() {
                    app.ui.dialogs.push(DeleteConfirmDialog::purge(items));
                }
                return;
            }
            if sel.is_empty() {
                return;
            }
            let mode = match action {
                A::Delete => DeleteMode::Trash,
                A::DeletePermanent => DeleteMode::Permanent,
                _ => DeleteMode::Wipe { passes: 1 },
            };
            let confirm = match mode {
                DeleteMode::Trash => app.config.general.confirm_delete,
                DeleteMode::Permanent => app.config.general.confirm_permanent_delete,
                DeleteMode::Wipe { .. } => true,
            };
            if confirm {
                app.ui.dialogs.push(DeleteConfirmDialog::new(sel, mode));
            } else {
                delete(app, sel, mode);
            }
        }
        A::Undo => undo(app),
        A::Redo => redo(app),
        A::EmptyTrash => app.ui.dialogs.push(DeleteConfirmDialog::empty_trash(app.fileops.trash.items().len())),
        A::RestoreFromTrash => {
            let items = app.fileops.trash.selected_items();
            if !items.is_empty() {
                restore(app, items);
            } else if !in_trash {
                crate::ops::navigate(app, pane_id, Location::Trash, crate::ops::NavOpts::user());
            }
        }
        A::JobsPauseAll => app.fileops.jobs.engine().pause_all(),
        A::JobsResumeAll => app.fileops.jobs.engine().resume_all(),
        A::JobsCancelAll => app.fileops.jobs.engine().cancel_all(),
        A::JobsClearFinished => app.fileops.jobs.engine().clear_finished(),
        A::JobsShowHistory => {
            app.ui.bottom.open_activity();
            app.fileops.activity.tab = ActivityTab::History;
        }
        _ => {}
    }
}

fn start_rename(app: &mut SoyuzApp, pane_id: PaneId) {
    let selects = app.config.behaviour.rename_selects;
    let pane = app.ws.pane_mut();
    if !matches!(pane.location, Location::Dir(_)) {
        return;
    }
    let target =
        pane.cursor.as_deref().and_then(|c| pane.entry(c)).filter(|e| pane.selection.contains(&e.name)).cloned();
    let target = target.or_else(|| pane.selected_entries().first().map(|e| (*e).clone()));
    let Some(e) = target else { return };
    if let Some(pos) = pane.visible_pos(&e.name) {
        pane.move_cursor(pos, false, false);
    }
    app.fileops.rename = Some(InlineRename::new(pane_id, &e, selects));
    app.ui.focus = crate::app::Focus::List;
}

/// Paste the internal clipboard into `dest`.
pub fn paste_into(app: &mut SoyuzApp, dest: PathBuf) {
    paste_into_with(app, dest, false);
}

/// Paste; `verify` hash-compares every copied file ("Copy and verify"), whatever
/// `jobs.verify_after_copy` says.
/// The system clipboard is read first (files copied in another application win); it
/// lives in another process on Linux, so the paste may run a few frames later
/// ([`crate::services::os_clipboard::with_files`]).
pub fn paste_into_with(app: &mut SoyuzApp, dest: PathBuf, verify: bool) {
    crate::services::os_clipboard::with_files(app, Box::new(move |app, os| paste_files(app, os, dest, verify)));
}

fn paste_files(app: &mut SoyuzApp, os: crate::services::os_clipboard::Files, dest: PathBuf, verify: bool) {
    let Some((mode, paths)) = os.or_else(|| app.fileops.clipboard.for_paste()) else {
        app.ui.toasts.push(Toast::info(tr!("ops-nothing-to-paste")).body(tr!("ops-nothing-to-paste-body")));
        return;
    };
    let effect = if mode == ClipMode::Cut { DropEffect::Move } else { DropEffect::Copy };
    let id = transfer_with(app, paths, dest, effect, verify);
    if mode == ClipMode::Cut
        && let Some(id) = id
        && let Some(m) = app.fileops.jobs.meta(id).cloned()
    {
        app.fileops.jobs.track(id, m.after(AfterJob::ClearClipboard));
    }
}

/// Submit a copy / move / link of `paths` into `dest`.
pub fn transfer(app: &mut SoyuzApp, paths: Vec<PathBuf>, dest: PathBuf, effect: DropEffect) -> Option<JobId> {
    transfer_with(app, paths, dest, effect, false)
}

/// [`transfer`]; `verify` forces hash verification of the copies for this job.
pub fn transfer_with(
    app: &mut SoyuzApp,
    paths: Vec<PathBuf>,
    dest: PathBuf,
    effect: DropEffect,
    verify: bool,
) -> Option<JobId> {
    // Remote sources or destination stream through a `JobKind::Remote` job.
    if crate::services::remote::is_remote_path(&dest)
        || paths.iter().any(|p| crate::services::remote::is_remote_path(p))
    {
        return crate::ui::remote::transfer(app, paths, dest, effect == DropEffect::Move, false);
    }
    if paths.is_empty() || !dnd::valid_target(&paths, &dest) {
        if !paths.is_empty() {
            app.ui.toasts.push(Toast::warning(tr!("ops-drop-into-itself")));
        }
        return None;
    }
    let same_dir = paths.iter().all(|p| p.parent() == Some(dest.as_path()));
    match effect {
        DropEffect::Copy | DropEffect::Move => {
            if effect == DropEffect::Move && same_dir {
                return None;
            }
            let kind = if effect == DropEffect::Copy {
                JobKind::Copy { sources: paths.clone(), dest_dir: dest.clone(), rename_to: None }
            } else {
                JobKind::Move { sources: paths.clone(), dest_dir: dest.clone(), rename_to: None }
            };
            let mut spec = app.fileops.jobs.spec(kind);
            if verify {
                spec.options.copy.verify.get_or_insert(soyuz_core::HashAlgo::Blake3);
            }
            spec.options.conflict = if same_dir {
                Some(ConflictResolution::KeepBoth) // copy into the same folder duplicates
            } else {
                match app.config.behaviour.paste_conflict {
                    PasteConflict::Ask => None,
                    PasteConflict::Overwrite => Some(ConflictResolution::Overwrite),
                    PasteConflict::Skip => Some(ConflictResolution::Skip),
                    PasteConflict::KeepBoth => Some(ConflictResolution::KeepBoth),
                }
            };
            let op = match (effect, same_dir) {
                (DropEffect::Copy, true) => OpKind::Duplicate,
                (DropEffect::Copy, false) => OpKind::Copy,
                _ => OpKind::Move,
            };
            Some(app.fileops.jobs.submit(spec, JobMeta::new(op, &paths, Some(dest))))
        }
        DropEffect::Link => {
            let mut last = None;
            for p in paths {
                let name = if p.parent() == Some(dest.as_path()) {
                    names::link_base(&p)
                } else {
                    soyuz_core::entry::display_name(&p)
                };
                last = Some(create_link(app, &dest, &name, p, LinkChoice::Symlink));
            }
            last
        }
    }
}

fn create_link(app: &mut SoyuzApp, dir: &Path, name: &str, target: PathBuf, kind: LinkChoice) -> JobId {
    let path = dir.join(name);
    let ck = match kind {
        LinkChoice::Symlink => {
            // File vs folder link is resolved by the job (no stat on the UI thread).
            CreateKind::Symlink { kind: LinkKind::File, target }
        }
        LinkChoice::Hardlink => CreateKind::Hardlink { src: target },
        LinkChoice::Junction => CreateKind::Junction { target },
    };
    let spec = app.fileops.jobs.spec(JobKind::Create { path: path.clone(), kind: ck });
    let meta = JobMeta::new(OpKind::Link, std::slice::from_ref(&path), Some(dir.to_path_buf()));
    app.fileops.jobs.submit(spec, meta)
}

fn delete(app: &mut SoyuzApp, paths: Vec<PathBuf>, mode: DeleteMode) {
    if paths.is_empty() {
        return;
    }
    let op = match mode {
        DeleteMode::Trash => OpKind::Trash,
        DeleteMode::Permanent => OpKind::Delete,
        DeleteMode::Wipe { .. } => OpKind::Wipe,
    };
    let spec = app.fileops.jobs.spec(JobKind::Delete { paths: paths.clone(), mode });
    app.fileops.jobs.submit(spec, JobMeta::new(op, &paths, None));
}

fn restore(app: &mut SoyuzApp, items: Vec<TrashItem>) {
    let paths: Vec<PathBuf> = items.iter().map(|i| i.original.clone()).collect();
    let spec = app.fileops.jobs.spec(JobKind::RestoreTrash { items, to: None });
    app.fileops.jobs.submit(spec, JobMeta::new(OpKind::Restore, &paths, None));
}

/// Ctrl+Z: only operations from this session (older ones stay available in the activity
/// panel's history, where the entry is visible before it is undone).
fn undo(app: &mut SoyuzApp) {
    let entry =
        app.fileops.jobs.history().iter().rev().find(|h| h.this_session && !h.undone && h.undoable).map(|h| h.id);
    let Some(h) = entry else {
        app.ui.toasts.push(Toast::info(tr!("ops-nothing-to-undo")));
        return;
    };
    match app.fileops.jobs.engine().undo(h) {
        Some(id) => {
            app.fileops.undo_stack.push(h);
            app.fileops.jobs.track(id, JobMeta::new(OpKind::Undo, &[], None));
        }
        None => {
            app.ui.toasts.push(Toast::info(tr!("ops-still-undoing")));
        }
    }
}

fn redo(app: &mut SoyuzApp) {
    let hist = app.fileops.jobs.history();
    let undone = |id: HistoryId| hist.iter().any(|h| h.id == id && h.undone);
    let from_stack = app.fileops.undo_stack.iter().rev().copied().find(|id| undone(*id));
    let h = from_stack.or_else(|| hist.iter().rev().find(|h| h.undone && h.this_session).map(|h| h.id));
    let Some(h) = h else {
        app.ui.toasts.push(Toast::info(tr!("ops-nothing-to-redo")));
        return;
    };
    app.fileops.undo_stack.retain(|x| *x != h);
    if let Some(id) = app.fileops.jobs.engine().redo(h) {
        app.fileops.jobs.track(id, JobMeta::new(OpKind::Redo, &[], None));
    }
}

/// Undo / redo / retry one history entry (activity panel history list).
pub fn history_action(app: &mut SoyuzApp, h: HistoryId, what: HistoryAction) {
    let (id, op) = match what {
        HistoryAction::Undo => (app.fileops.jobs.engine().undo(h), OpKind::Undo),
        HistoryAction::Redo => (app.fileops.jobs.engine().redo(h), OpKind::Redo),
        HistoryAction::Retry => (app.fileops.jobs.engine().retry_failed(h), OpKind::Retry),
    };
    match id {
        Some(id) => {
            if what == HistoryAction::Undo {
                app.fileops.undo_stack.push(h);
            }
            app.fileops.jobs.track(id, JobMeta::new(op, &[], None));
        }
        None => {
            app.ui.toasts.push(Toast::info(tr!("ops-not-available")));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryAction {
    Undo,
    Redo,
    Retry,
}

pub fn apply(app: &mut SoyuzApp, cmd: FileOpCmd, _ctx: &egui::Context) {
    match cmd {
        FileOpCmd::Drop { paths, dest, effect, pos } => match effect {
            Some(e) => {
                transfer(app, paths, dest, e);
            }
            None => app.fileops.dnd.ask = Some(dnd::AskMenu { paths, dest, pos }),
        },
        FileOpCmd::Transfer { paths, dest, effect } => {
            transfer(app, paths, dest, effect);
        }
        FileOpCmd::TrashPaths(paths) => delete(app, paths, DeleteMode::Trash),
        FileOpCmd::Delete { paths, mode } => delete(app, paths, mode),
        FileOpCmd::Rename { pane, from, to } => {
            let to_path = from.with_file_name(&to);
            let spec = app.fileops.jobs.spec(JobKind::Rename { pairs: vec![(from.clone(), to_path)] });
            let meta = JobMeta::new(OpKind::Rename, std::slice::from_ref(&from), Some(PathBuf::from(&to)))
                .after(AfterJob::Select { pane, name: to })
                .quiet();
            app.fileops.jobs.submit(spec, meta);
        }
        FileOpCmd::PasteInto(dest) => paste_into(app, dest),
        FileOpCmd::CreateLink { dir, name, target, kind } => {
            create_link(app, &dir, &name, target, kind);
        }
        FileOpCmd::Respond { job, response } => {
            app.fileops.jobs.engine().respond(job, response);
            app.fileops.prompts.remove(job);
            app.fileops.jobs.refresh_snapshots();
        }
        FileOpCmd::Resolve(job) => app.fileops.prompts.undefer(job),
        FileOpCmd::CancelJob(job) => {
            app.fileops.jobs.engine().cancel(job);
            app.fileops.prompts.remove(job);
            app.fileops.jobs.refresh_snapshots();
        }
        FileOpCmd::EmptyTrash => {
            let spec = app.fileops.jobs.spec(JobKind::EmptyTrash);
            app.fileops.jobs.submit(spec, JobMeta::new(OpKind::EmptyTrash, &[], None));
        }
        FileOpCmd::ConfirmEmptyTrash => {
            app.ui.dialogs.push(DeleteConfirmDialog::empty_trash(app.fileops.trash.items().len()));
        }
        FileOpCmd::ConfirmPurge(items) => {
            if !items.is_empty() {
                app.ui.dialogs.push(DeleteConfirmDialog::purge(items));
            }
        }
        FileOpCmd::TrashRestore(items) => restore(app, items),
        FileOpCmd::TrashPurge(items) => {
            if !items.is_empty() {
                let paths: Vec<PathBuf> = items.iter().map(|i| i.original.clone()).collect();
                let spec = app.fileops.jobs.spec(JobKind::PurgeTrash { items });
                app.fileops.jobs.submit(spec, JobMeta::new(OpKind::Delete, &paths, None));
            }
        }
    }
}

/// Start of every frame (after the mailbox): drain engine events, toasts, prompts,
/// auto-open of the activity panel, external drops and deferred inline renames.
pub fn begin_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    let events = app.fileops.jobs.poll(ctx);
    for ev in events {
        match ev {
            JobEvent::Prompt(id) => on_prompt(app, id),
            JobEvent::Finished(id, summary) => on_finished(app, id, &summary),
            _ => {}
        }
    }
    // Keep tags keyed by path attached to items renamed / moved in the app (and undo).
    for (from, to) in app.fileops.jobs.take_moves() {
        crate::services::tags::rename(&from, &to);
    }
    // Show what finished jobs changed without waiting for (or relying on) the watcher.
    let affected = app.fileops.jobs.take_affected();
    if !affected.is_empty() {
        crate::ops::refresh_affected(app, &affected);
    }
    reconcile_prompts(app);
    auto_open_activity(app);
    external_drops(app, ctx);
    pending_rename(app);
    check_rename(app);
    app.fileops.trash.poll();
    let trash_shown =
        app.ws.tab().panes.iter().take(app.ws.tab().shown().max(1)).any(|p| p.location == Location::Trash);
    if trash_shown {
        app.fileops.trash.ensure_loaded();
    }
}

/// End of every frame: drag badge and the "ask" drop menu.
pub fn end_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    dragout::end_frame(app, ctx);
    dnd::paint_badge(ctx);
    if let Some(menu) = app.fileops.dnd.ask.clone() {
        let mut close = false;
        let mut chosen = None;
        let resp = egui::Area::new(Id::new("soyuz.dnd.ask")).order(egui::Order::Foreground).fixed_pos(menu.pos).show(
            ctx,
            |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    use crate::theme::icons::ph;
                    use crate::ui::widgets::{menu_item, menu_separator};
                    if menu_item(ui, Some(ph::ARROW_BEND_DOWN_RIGHT), tr!("ops-dnd-move-here"), None, true).clicked() {
                        chosen = Some(DropEffect::Move);
                    }
                    if menu_item(ui, Some(ph::COPY), tr!("ops-dnd-copy-here"), None, true).clicked() {
                        chosen = Some(DropEffect::Copy);
                    }
                    if menu_item(ui, Some(ph::LINK), tr!("ops-dnd-link-here"), None, true).clicked() {
                        chosen = Some(DropEffect::Link);
                    }
                    menu_separator(ui);
                    if menu_item(ui, Some(ph::X), tr!("common-cancel"), None, true).clicked() {
                        close = true;
                    }
                });
            },
        );
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
            && !ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| resp.response.rect.contains(p));
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || clicked_outside {
            close = true;
        }
        if let Some(effect) = chosen {
            app.fileops.dnd.ask = None;
            transfer(app, menu.paths, menu.dest, effect);
        } else if close {
            app.fileops.dnd.ask = None;
        }
    }
    // Pane rects are re-recorded by the next frame's draw.
    let ask = app.fileops.dnd.ask.take();
    let panes = std::mem::take(&mut app.fileops.dnd.panes);
    app.fileops.dnd = DndFrame { panes, item_claimed: false, ask };
}

fn on_prompt(app: &mut SoyuzApp, id: JobId) {
    let prompt = app.fileops.jobs.snapshot(id).and_then(|s| s.pending_prompt.clone());
    if let Some(JobPrompt::TrashUnsupported { .. }) = prompt {
        let auto = match app.config.jobs.trash_unsupported {
            TrashUnsupportedPolicy::Ask => None,
            TrashUnsupportedPolicy::Permanent => Some(true),
            TrashUnsupportedPolicy::Skip => Some(false),
        };
        if let Some(permanent) = auto {
            app.fileops.jobs.engine().respond(id, PromptResponse::TrashUnsupported { permanent, apply_to_all: true });
            app.fileops.jobs.refresh_snapshots();
            return;
        }
    }
    app.fileops.prompts.push(id);
}

fn reconcile_prompts(app: &mut SoyuzApp) {
    let fo = &mut app.fileops;
    // Robust against missed events: queue every waiting job, drop the others.
    let waiting: Vec<JobId> = fo.jobs.snapshots().iter().filter(|s| s.pending_prompt.is_some()).map(|s| s.id).collect();
    for id in &waiting {
        fo.prompts.push(*id);
    }
    fo.prompts.retain(|id| waiting.contains(&id));

    if let Some((job, shared)) = fo.prompt_dialog.clone() {
        let (alive, answered) = {
            let s = shared.borrow();
            (s.alive, s.answered)
        };
        if !alive {
            if !answered {
                fo.prompts.defer(job);
            }
            fo.prompt_dialog = None;
        } else {
            let mut s = shared.borrow_mut();
            s.stale = !waiting.contains(&job);
            s.total = fo.prompts.visible_len();
        }
    }
    if fo.prompt_dialog.is_none()
        && let Some(job) = fo.prompts.current()
        && let Some(snap) = fo.jobs.snapshot(job)
        && let Some(prompt) = snap.pending_prompt.clone()
    {
        let title = match fo.jobs.meta(job) {
            Some(m) => {
                names::job_title(m, true, snap.progress.totals_known.then_some(snap.progress.items_total), &snap.label)
            }
            None => snap.label.clone(),
        };
        let shared = Rc::new(RefCell::new(PromptShared {
            alive: true,
            answered: false,
            stale: false,
            total: fo.prompts.visible_len(),
        }));
        fo.prompt_dialog = Some((job, shared.clone()));
        // Rename-on-conflict names follow the target volume's rules.
        let windows_rules = match &prompt {
            JobPrompt::Conflict(c) => {
                rename::windows_rules_for(app.config.general.strict_windows_names, &app.ui.volumes, c.dst.path.parent())
            }
            _ => cfg!(windows),
        };
        app.ui.dialogs.push(PromptDialog::new(job, prompt, title, shared).windows_rules(windows_rules));
    }
}

fn on_finished(app: &mut SoyuzApp, id: JobId, summary: &JobSummary) {
    let Some(snap) = app.fileops.jobs.snapshot(id).cloned() else { return };
    let meta =
        app.fileops.jobs.meta(id).cloned().unwrap_or_else(|| JobMeta::new(OpKind::from_brief(snap.kind), &[], None));
    let total = snap.progress.totals_known.then_some(snap.progress.items_total);
    let title = names::job_title(&meta, false, total, &snap.label);
    let purge = meta.op == OpKind::Delete && snap.kind == soyuz_jobs::JobKindBrief::EmptyTrash;
    if purge || matches!(meta.op, OpKind::Trash | OpKind::Restore | OpKind::EmptyTrash | OpKind::Undo | OpKind::Redo) {
        app.fileops.trash.mark_stale();
    }
    match &snap.state {
        JobState::Done(_) => {
            match &meta.after {
                AfterJob::SelectAndRename { pane, path } => {
                    if summary.items_ok > 0 {
                        if let Some(p) = app.ws.find_pane_mut(*pane) {
                            p.pending_cursor = path.file_name().map(|n| n.to_string_lossy().into_owned());
                        }
                        app.fileops.pending_rename = Some((*pane, path.clone(), Instant::now()));
                    }
                }
                AfterJob::Select { pane, name } => {
                    if let Some(p) = app.ws.find_pane_mut(*pane) {
                        p.pending_cursor = Some(name.clone());
                    }
                }
                AfterJob::ClearClipboard => {
                    if summary.items_failed == 0 {
                        app.fileops.clipboard.clear();
                        crate::services::os_clipboard::clear_cut();
                    }
                }
                AfterJob::None => {}
            }
            if !meta.toast {
                if summary.items_failed > 0 {
                    app.ui.toasts.push(failure_toast(&title, summary));
                }
                return;
            }
            if summary.items_failed > 0 {
                app.ui.toasts.push(failure_toast(&title, summary));
            } else if summary.items_ok == 0 && summary.items_skipped > 0 {
                app.ui.toasts.push(Toast::info(title).body(names::summary_line(summary)));
            } else {
                let mut t = Toast::success(title).body(names::summary_line(summary));
                if meta.op.undoable() {
                    t = t.action(tr!("common-undo"), Action::Undo);
                    t.duration = Some(7.0);
                }
                app.ui.toasts.push(t);
            }
        }
        JobState::Failed(e) => {
            let what = names::job_title(&meta, true, total, &snap.label);
            app.ui.toasts.push(
                Toast::error(tr!("ops-job-failed", what = lower_first(&what)))
                    .body(e.to_string())
                    .action(tr!("ops-details"), Action::JobsShowHistory),
            );
        }
        JobState::Cancelled if meta.toast => {
            let what = names::job_title(&meta, true, total, &snap.label);
            let mut t = Toast::info(tr!("ops-outcome-cancelled")).body(what);
            t.duration = Some(3.0);
            app.ui.toasts.push(t);
        }
        _ => {}
    }
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn failure_toast(title: &str, s: &JobSummary) -> Toast {
    let mut body = names::summary_line(s);
    if let Some((p, e)) = s.errors.first() {
        body.push_str(&format!("\n{}: {e}", soyuz_core::entry::display_name(p)));
    }
    Toast::warning(title).body(body).action(tr!("ops-details"), Action::JobsShowHistory)
}

fn auto_open_activity(app: &mut SoyuzApp) {
    if !app.config.jobs.auto_open_activity {
        return;
    }
    let mut open = false;
    for s in app.fileops.jobs.snapshots() {
        if !s.state.is_active() || app.fileops.auto_opened.contains(&s.id) {
            continue;
        }
        let quick = app
            .fileops
            .jobs
            .meta(s.id)
            .is_some_and(|m| matches!(m.op, OpKind::Rename | OpKind::NewFolder | OpKind::NewFile | OpKind::Link));
        if !quick && s.elapsed > Duration::from_secs(1) {
            app.fileops.auto_opened.insert(s.id);
            open = true;
        }
    }
    if open && !app.ui.bottom.activity_shown() {
        app.ui.bottom.open_activity();
        app.fileops.activity.tab = ActivityTab::Jobs;
    }
}

fn external_drops(app: &mut SoyuzApp, ctx: &egui::Context) {
    let (dropped, pointer) = ctx.input(|i| {
        (
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect::<Vec<_>>(),
            i.pointer.latest_pos(),
        )
    });
    if dropped.is_empty() {
        return;
    }
    let focused = app.ws.pane().id;
    match dnd::external_target(&app.fileops.dnd, pointer, focused) {
        Some(dest) => {
            transfer(app, dropped, dest, DropEffect::Copy);
        }
        None => {
            app.ui.toasts.push(Toast::info(tr!("ops-drop-onto-folder")));
        }
    }
}

fn pending_rename(app: &mut SoyuzApp) {
    let Some((pane_id, path, at)) = app.fileops.pending_rename.clone() else { return };
    if at.elapsed() > Duration::from_secs(5) {
        app.fileops.pending_rename = None;
        return;
    }
    let selects = app.config.behaviour.rename_selects;
    let Some(pane) = app.ws.find_pane_mut(pane_id) else {
        app.fileops.pending_rename = None;
        return;
    };
    let name = soyuz_core::entry::display_name(&path);
    if let Some(e) = pane.entry(&name).cloned() {
        if let Some(pos) = pane.visible_pos(&name) {
            pane.move_cursor(pos, false, false);
        }
        app.fileops.rename = Some(InlineRename::new(pane_id, &e, selects));
        app.fileops.pending_rename = None;
    }
}

/// Drop an inline rename whose item disappeared or whose pane navigated away.
fn check_rename(app: &mut SoyuzApp) {
    if let Some(r) = &app.fileops.rename
        && let Some(p) = app.ws.find_pane(r.pane)
        && p.entry(&r.key).is_none()
        && !p.is_loading()
    {
        app.fileops.rename = None;
    }
}

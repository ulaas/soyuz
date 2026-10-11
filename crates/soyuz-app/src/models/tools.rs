//! Power tools (frontend-agnostic part): the deferred [`ToolCmd`] queue, the tool
//! actions (what they work on, toasts when there is nothing to work on), tags / recent /
//! previous-version commands, the drop shelf's contents and whether a (modal) tool
//! window is open. The tool windows themselves are frontend widgets (`ui::tools`).
//!
//! Like the windows, this state is UI-thread-local: widgets that live in shared modules
//! (context menu, sidebar, status bar) reach it through [`queue`] (applied next frame).

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_core::config::Action;
use soyuz_core::{EntryInfo, Location};
use soyuz_jobs::JobKind;

use crate::app::SoyuzApp;
use crate::host::{Host, Wake};
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;
use crate::services::jobs::{JobMeta, OpKind};

/// Deferred tool commands from shared widgets (context menu, sidebar, status bar).
#[derive(Clone, Debug)]
pub enum ToolCmd {
    ShelfAdd(Vec<PathBuf>),
    Treemap(PathBuf),
    Compare(PathBuf, Option<PathBuf>),
    Duplicates(Vec<PathBuf>),
    Split(PathBuf),
    Join(PathBuf),
    ToggleTag {
        paths: Vec<PathBuf>,
        tag: u8,
    },
    ClearTags(Vec<PathBuf>),
    /// Rename color tag `tag` (`[tags] names`); an empty name restores the color name.
    RenameTag {
        tag: u8,
        name: String,
    },
    ForgetRecent(Vec<PathBuf>),
    /// The sync profiles window (schedules, reports).
    SyncProfiles,
    /// Open Compare & sync with a saved profile.
    CompareProfile(String),
    /// "Restore previous version…" for a path (is_dir, current size / modified).
    Versions(PathBuf, bool, Option<(u64, Option<std::time::SystemTime>)>),
    /// A previous-versions action from a panel without app access (Properties tab).
    VersionAction(VersionCmd),
}

/// What the user asked for in a previous-versions panel (applied by [`apply_version`]).
#[derive(Clone, Debug)]
pub enum VersionCmd {
    /// Copy the version back over the original (conflict prompt, undoable).
    Restore { version: PathBuf, original: PathBuf },
    /// Copy the version into a chosen folder.
    CopyTo { version: PathBuf, start: PathBuf },
    /// Open: a folder is browsed in the pane, a file opens from a read-only temp copy.
    Open { version: PathBuf, is_dir: bool, label: String },
    /// Start an elevated Soyuz (shadow copies need administrator rights).
    Elevate,
}

thread_local! {
    static QUEUE: RefCell<Vec<ToolCmd>> = const { RefCell::new(Vec::new()) };
    static WAKE: RefCell<Option<Wake>> = const { RefCell::new(None) };
    static SHELF: RefCell<ShelfData> = RefCell::new(ShelfData::default());
    static WINDOW_OPEN: Cell<bool> = const { Cell::new(false) };
}

/// Queue a tool command (applied at the start of the next frame).
pub fn queue(c: ToolCmd) {
    QUEUE.with_borrow_mut(|q| q.push(c));
    WAKE.with_borrow(|w| {
        if let Some(w) = w {
            w();
        }
    });
}

/// The queued commands (emptied).
pub fn take_queue() -> Vec<ToolCmd> {
    QUEUE.with_borrow_mut(std::mem::take)
}

/// Keep the frontend's wake-up for [`queue`] and the tool workers (first call wins).
pub fn init(host: &dyn Host) {
    WAKE.with_borrow_mut(|w| {
        if w.is_none() {
            *w = Some(host.wake());
        }
    });
}

/// Repaint hook for workers.
pub fn wake() -> Wake {
    WAKE.with_borrow(|w| w.clone()).unwrap_or_else(crate::host::no_wake)
}

/// Whether a tool window is open. They are modal: the panes underneath get no input and
/// the keymap does not run while one is open.
pub fn any_window_open() -> bool {
    WINDOW_OPEN.get()
}

/// The frontend reports whether a tool window is open (after it opened / closed one).
pub fn set_window_open(open: bool) {
    WINDOW_OPEN.set(open);
}

// ---------------------------------------------------------------------------------------
// Drop shelf.

pub struct ShelfItem {
    pub path: PathBuf,
    pub entry: Option<EntryInfo>,
    pub missing: bool,
}

/// What is on the shelf (references only; persisted in the session).
pub struct ShelfData {
    pub open: bool,
    pub items: Vec<ShelfItem>,
    stat_tx: Sender<(PathBuf, Option<EntryInfo>)>,
    stat_rx: Receiver<(PathBuf, Option<EntryInfo>)>,
}

impl Default for ShelfData {
    fn default() -> Self {
        let (stat_tx, stat_rx) = unbounded();
        ShelfData { open: false, items: Vec::new(), stat_tx, stat_rx }
    }
}

impl ShelfData {
    /// Add paths (duplicates ignored) and stat them in the background.
    pub fn add(&mut self, paths: Vec<PathBuf>) {
        let fresh: Vec<PathBuf> =
            paths.into_iter().filter(|p| !self.items.iter().any(|i| &i.path == p)).collect::<Vec<_>>();
        if fresh.is_empty() {
            return;
        }
        for p in &fresh {
            self.items.push(ShelfItem { path: p.clone(), entry: None, missing: false });
        }
        self.stat(fresh);
    }

    /// Stat `paths` on a worker; results arrive through `poll`.
    pub fn stat(&self, fresh: Vec<PathBuf>) {
        let tx = self.stat_tx.clone();
        let wake = wake();
        let _ = std::thread::Builder::new().name("soyuz-shelf-stat".into()).spawn(move || {
            for p in fresh {
                let e = soyuz_fs::stat(&p, false).ok();
                if tx.send((p, e)).is_err() {
                    break;
                }
            }
            wake();
        });
    }

    /// Take finished stats.
    pub fn poll(&mut self) {
        for (p, e) in self.stat_rx.try_iter() {
            if let Some(item) = self.items.iter_mut().find(|i| i.path == p) {
                item.missing = e.is_none();
                item.entry = e;
            }
        }
    }

    /// Remove the items at `idx`.
    pub fn remove(&mut self, idx: &BTreeSet<usize>) {
        let mut i = 0;
        self.items.retain(|_| {
            let keep = !idx.contains(&i);
            i += 1;
            keep
        });
    }

    /// Paths an action applies to: the `selected` items, else everything present.
    pub fn targets(&self, selected: &BTreeSet<usize>) -> Vec<PathBuf> {
        if selected.is_empty() {
            self.items.iter().filter(|i| !i.missing).map(|i| i.path.clone()).collect()
        } else {
            selected.iter().filter_map(|&i| self.items.get(i)).map(|i| i.path.clone()).collect()
        }
    }
}

/// Lend the shelf out (a frontend drawing it); give it back with [`put_shelf`].
pub fn take_shelf() -> ShelfData {
    SHELF.with_borrow_mut(std::mem::take)
}

/// Put a lent-out shelf back (keeping anything added meanwhile).
pub fn put_shelf(mut d: ShelfData) {
    SHELF.with_borrow_mut(|slot| {
        let fresh = std::mem::take(&mut slot.items);
        let added: Vec<PathBuf> = fresh.into_iter().map(|i| i.path).collect();
        *slot = std::mem::take(&mut d);
        if !added.is_empty() {
            slot.add(added);
        }
    });
}

/// Run `f` on the shelf.
pub fn with_shelf<R>(f: impl FnOnce(&mut ShelfData) -> R) -> R {
    SHELF.with_borrow_mut(f)
}

/// Number of items on the shelf (status bar badge).
pub fn shelf_count() -> usize {
    SHELF.with_borrow(|s| s.items.len())
}

/// Paths on the shelf (persisted in the session).
pub fn shelf_paths() -> Vec<PathBuf> {
    SHELF.with_borrow(|s| s.items.iter().map(|i| i.path.clone()).collect())
}

/// Put the saved shelf contents back (closed; missing items show as missing).
pub fn restore_shelf(paths: Vec<PathBuf>) {
    if !paths.is_empty() {
        SHELF.with_borrow_mut(|s| s.add(paths));
    }
}

pub fn shelf_open() -> bool {
    SHELF.with_borrow(|s| s.open)
}

// ---------------------------------------------------------------------------------------
// Actions.

/// Actions implemented here (routed by `actions::execute`).
pub fn handles(a: &Action) -> bool {
    use Action as A;
    matches!(
        a,
        A::DiskUsageTreemap
            | A::FindDuplicates
            | A::CompareFolders
            | A::SyncFolders
            | A::SplitFile
            | A::JoinFiles
            | A::AddTag(_)
            | A::ClearTags
            | A::OpenRecent
            | A::ToggleShelf
            | A::SyncProfiles
            | A::PreviousVersions
    )
}

/// Selection of the focused pane (paths), its folder and the other pane's folder.
fn context(app: &SoyuzApp) -> (Vec<(PathBuf, bool)>, Option<PathBuf>, Option<PathBuf>) {
    let pane = app.ws.pane();
    let sel: Vec<(PathBuf, bool)> = pane.selected_entries().iter().map(|e| (e.path.clone(), e.is_dir_like())).collect();
    let here =
        pane.location.as_fs_path().map(|p| p.to_path_buf()).filter(|_| matches!(pane.location, Location::Dir(_)));
    let tab = app.ws.tab();
    let other = if tab.dual && tab.panes.len() == 2 {
        match &tab.panes[1 - tab.focused.min(1)].location {
            Location::Dir(d) => Some(d.clone()),
            _ => None,
        }
    } else {
        None
    };
    (sel, here, other)
}

/// `name.ext.003` → (`name.ext`, 3).
pub fn split_part(path: &Path) -> Option<(PathBuf, usize)> {
    let ext = path.extension()?.to_str()?;
    if ext.len() < 3 || !ext.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: usize = ext.parse().ok()?;
    let base = path.with_extension("");
    base.file_name()?;
    Some((base, n))
}

/// Looks like a split part (`.001`, `.002`, …).
pub fn is_part(path: &Path) -> bool {
    split_part(path).is_some_and(|(_, n)| n >= 1)
}

pub fn execute(app: &mut SoyuzApp, action: Action, host: &dyn Host) {
    use Action as A;
    init(host);
    let (sel, here, other) = context(app);
    let sel_paths: Vec<PathBuf> = sel.iter().map(|(p, _)| p.clone()).collect();
    match action {
        A::DiskUsageTreemap => {
            let target = match sel.as_slice() {
                [(p, true)] => Some(p.clone()),
                _ => here.clone(),
            };
            match target {
                Some(t) => queue(ToolCmd::Treemap(t)),
                None => {
                    app.ui.toasts.push(Toast::info(tr!("tools-toast-open-folder-treemap")));
                }
            }
        }
        A::FindDuplicates => {
            let dirs: Vec<PathBuf> = sel.iter().filter(|(_, d)| *d).map(|(p, _)| p.clone()).collect();
            let roots = if dirs.is_empty() { here.into_iter().collect() } else { dirs };
            queue(ToolCmd::Duplicates(roots));
        }
        A::CompareFolders | A::SyncFolders => {
            let dirs: Vec<PathBuf> = sel.iter().filter(|(_, d)| *d).map(|(p, _)| p.clone()).collect();
            let (l, r) = if dirs.len() == 2 { (Some(dirs[0].clone()), Some(dirs[1].clone())) } else { (here, other) };
            match l {
                Some(l) => queue(ToolCmd::Compare(l, r)),
                None => {
                    app.ui.toasts.push(Toast::info(tr!("tools-toast-open-folder-compare")));
                }
            }
        }
        A::SplitFile => match sel.as_slice() {
            [(p, false)] => queue(ToolCmd::Split(p.clone())),
            _ => {
                app.ui.toasts.push(Toast::info(tr!("tools-toast-select-file-split")));
            }
        },
        A::JoinFiles => match sel.iter().find(|(p, d)| !d && is_part(p)) {
            Some((p, _)) => queue(ToolCmd::Join(p.clone())),
            None => {
                app.ui.toasts.push(Toast::info(tr!("tools-toast-select-first-part")));
            }
        },
        A::AddTag(n) => {
            if !sel_paths.is_empty() && (1..=8).contains(&n) {
                queue(ToolCmd::ToggleTag { paths: sel_paths, tag: n - 1 });
            }
        }
        A::ClearTags => {
            if !sel_paths.is_empty() {
                queue(ToolCmd::ClearTags(sel_paths));
            }
        }
        A::OpenRecent => {
            let id = app.ws.pane().id;
            crate::ops::navigate(app, id, Location::Recent, crate::ops::NavOpts::user());
        }
        A::ToggleShelf => SHELF.with_borrow_mut(|s| s.open = !s.open),
        A::SyncProfiles => queue(ToolCmd::SyncProfiles),
        A::PreviousVersions => {
            let pane = app.ws.pane();
            match pane.selected_entries().as_slice() {
                [e] => queue(ToolCmd::Versions(e.path.clone(), e.is_dir_like(), Some((e.size, e.modified)))),
                _ => match &pane.location {
                    Location::Dir(d) => queue(ToolCmd::Versions(d.clone(), true, None)),
                    _ => {
                        app.ui.toasts.push(Toast::info(tr!("tools-toast-select-versions")));
                    }
                },
            }
        }
        _ => {}
    }
}

/// Per frame (before the frontend applies the window commands): service pumps for git
/// status, tags and recent files, and the OS tags of the visible folders.
pub fn pump(app: &mut SoyuzApp, host: &dyn Host) {
    init(host);
    let wake = wake();
    crate::services::git::init(&app.wake, app.config.git.status_column);
    let git_pending = crate::services::git::pump(app.config.git.refresh_secs);
    let tags_arrived = crate::services::tags::pump(&wake);
    let recent_arrived = crate::services::recent::pump(&wake);
    if tags_arrived {
        refresh_tag_panes(app);
    }
    if recent_arrived {
        refresh_location(app, |l| matches!(l, Location::Recent));
    }
    if git_pending || crate::services::tags::pending_save() || crate::services::recent::pending_save() {
        host.redraw_after(std::time::Duration::from_millis(500));
    }
}

/// OS tags for the visible folders (macOS Finder tags, Linux `user.xdg.tags`).
pub fn request_os_tags(app: &SoyuzApp) {
    if crate::services::tags::os_tags_enabled() {
        let wake = wake();
        for p in app.ws.tab().panes.iter() {
            if let Location::Dir(d) = &p.location {
                crate::services::tags::request_finder(d, &wake);
            }
        }
    }
}

/// Apply the commands that need no tool window; returns the others (for the frontend).
pub fn apply_model_cmds(app: &mut SoyuzApp, host: &dyn Host, cmds: Vec<ToolCmd>) -> Vec<ToolCmd> {
    let mut rest = Vec::new();
    for c in cmds {
        match c {
            ToolCmd::ShelfAdd(paths) => SHELF.with_borrow_mut(|s| {
                s.open = true;
                s.add(paths);
            }),
            ToolCmd::Versions(..) if !app.config.sync.previous_versions => {
                app.ui
                    .toasts
                    .push(Toast::info(tr!("tools-toast-versions-off")).body(tr!("tools-toast-versions-off-body")));
            }
            ToolCmd::VersionAction(cmd) => apply_version(app, cmd),
            ToolCmd::ToggleTag { paths, tag } => {
                let added = crate::services::tags::toggle_many(&paths, tag);
                let name = crate::services::tags::name(tag);
                let n = paths.len();
                app.ui.toasts.push(Toast::info(if added {
                    tr!("tools-toast-tagged", count = n, tag = name)
                } else {
                    tr!("tools-toast-untagged", count = n, tag = name)
                }));
                refresh_tag_panes(app);
            }
            ToolCmd::ClearTags(paths) => {
                crate::services::tags::clear_many(&paths);
                refresh_tag_panes(app);
            }
            ToolCmd::RenameTag { tag, name } => rename_tag(app, host, tag, &name),
            ToolCmd::ForgetRecent(paths) => {
                crate::services::recent::forget(&paths);
                refresh_location(app, |l| matches!(l, Location::Recent));
            }
            other => rest.push(other),
        }
    }
    rest
}

/// Store a tag's display name in `config.toml` (applied live, saved like a settings
/// edit, so an open settings window picks it up instead of overwriting it).
fn rename_tag(app: &mut SoyuzApp, host: &dyn Host, tag: u8, name: &str) {
    let i = tag as usize;
    if i >= crate::services::tags::TAG_COUNT {
        return;
    }
    let mut clean = soyuz_core::config::clean_tag_name(name);
    if clean == crate::services::tags::default_name(tag) {
        clean.clear();
    }
    crate::models::settings::update_config(app, host, |c| {
        let names = &mut c.tags.names;
        if names.len() <= i {
            names.resize(crate::services::tags::TAG_COUNT.max(i + 1), String::new());
        }
        names[i] = clean;
    });
}

/// Re-list panes showing a tag listing (membership changed).
fn refresh_tag_panes(app: &mut SoyuzApp) {
    refresh_location(app, |l| matches!(l, Location::Tags(_)));
}

fn refresh_location(app: &mut SoyuzApp, pred: impl Fn(&Location) -> bool) {
    for t in &mut app.ws.tabs {
        for p in &mut t.panes {
            if pred(&p.location) {
                crate::ops::start_listing(p, &mut app.services, true);
            }
        }
    }
}

/// Whether `path` is shown by a virtual listing (Recent / Tags) and may be dragged.
pub fn is_virtual_listing(loc: &Location) -> bool {
    matches!(loc, Location::Recent | Location::Tags(_) | Location::Timeline)
}

/// Apply a [`VersionCmd`] (UI thread; disk work goes to jobs / workers).
pub fn apply_version(app: &mut SoyuzApp, cmd: VersionCmd) {
    match cmd {
        VersionCmd::Restore { version, original } => {
            let Some(dest) = original.parent().map(Path::to_path_buf) else { return };
            let name = soyuz_core::entry::display_name(&original);
            let kind =
                JobKind::Copy { sources: vec![version.clone()], dest_dir: dest.clone(), rename_to: Some(name.clone()) };
            let mut spec = app.fileops.jobs.spec(kind);
            // Ask before replacing the current file (the prompt offers keep both / replace).
            spec.options.conflict = None;
            spec.options.label = Some(tr!("tools-versions-job-label", name = &name));
            let meta = JobMeta::new(OpKind::Restore, std::slice::from_ref(&version), Some(dest));
            app.fileops.jobs.submit(spec, meta);
        }
        VersionCmd::CopyTo { version, start } => {
            app.frontend.push(FrontendCmd::Transfer { paths: vec![version], start, mv: false });
        }
        VersionCmd::Elevate => {
            let postman = app.services.mailbox.postman();
            let _ = std::thread::Builder::new().name("soyuz-elevate".into()).spawn(move || {
                let res = std::env::current_exe()
                    .map_err(|e| e.to_string())
                    .and_then(|exe| soyuz_fs::launch::run_elevated(&exe, &[]).map_err(|e| e.to_string()));
                if let Err(e) = res {
                    postman.send(crate::services::mailbox::UiMsg::Notice(
                        Toast::error(tr!("tools-versions-elevate-failed")).body(e),
                    ));
                }
            });
        }
        VersionCmd::Open { version, is_dir: true, .. } => {
            let id = app.ws.pane().id;
            crate::ops::navigate(app, id, Location::Dir(version), crate::ops::NavOpts::user());
        }
        VersionCmd::Open { version, is_dir: false, label } => {
            let postman = app.services.mailbox.postman();
            let spawned = std::thread::Builder::new().name("soyuz-version-open".into()).spawn(move || {
                if let Err(e) = open_temp_copy(&version, &label) {
                    postman.send(crate::services::mailbox::UiMsg::Notice(
                        Toast::error(tr!("tools-versions-open-failed")).body(e),
                    ));
                }
            });
            if let Err(e) = spawned {
                app.ui.toasts.push(Toast::error(tr!("tools-versions-open-failed")).body(e.to_string()));
            }
        }
    }
}

/// Copy a snapshot file to a read-only temp file and open it with its default app
/// (apps often can't open `\\?\GLOBALROOT` paths themselves). Worker thread.
fn open_temp_copy(version: &Path, label: &str) -> Result<(), String> {
    let stamp: String = label.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
    let dir = std::env::temp_dir().join("soyuz-versions").join(if stamp.is_empty() { "v".into() } else { stamp });
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = version.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "version".into());
    let dst = dir.join(name);
    if dst.exists() {
        let mut perm = std::fs::metadata(&dst).map_err(|e| e.to_string())?.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        let _ = std::fs::set_permissions(&dst, perm);
    }
    std::fs::copy(version, &dst).map_err(|e| e.to_string())?;
    if let Ok(m) = std::fs::metadata(&dst) {
        let mut perm = m.permissions();
        perm.set_readonly(true);
        let _ = std::fs::set_permissions(&dst, perm);
    }
    soyuz_fs::launch::open_default(&dst).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_targets_and_remove() {
        let items = ["a", "b", "c"]
            .iter()
            .map(|n| ShelfItem { path: PathBuf::from(n), entry: None, missing: *n == "b" })
            .collect();
        let mut s = ShelfData { items, ..ShelfData::default() };
        assert_eq!(s.targets(&BTreeSet::new()), vec![PathBuf::from("a"), PathBuf::from("c")]);
        assert_eq!(s.targets(&BTreeSet::from([1])), vec![PathBuf::from("b")]);
        s.remove(&BTreeSet::from([0, 2]));
        assert_eq!(s.items.len(), 1);
        assert!(is_virtual_listing(&Location::Recent) && !is_virtual_listing(&Location::Trash));
    }
}

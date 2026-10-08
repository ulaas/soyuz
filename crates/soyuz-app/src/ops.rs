//! Operations that combine state and services: navigation, listing lifecycle, message
//! handling, opening entries, tabs/panes management and [`Cmd`] application.

use std::path::Path;
use std::time::{Duration, Instant};

use soyuz_core::config::{Action, OpenFoldersIn, TypeAhead};
use soyuz_core::{Location, OpControl};
use soyuz_fs::WatchKind;

use crate::app::{Focus, SoyuzApp};
use crate::cmd::Cmd;
use crate::services::mailbox::UiMsg;
use crate::services::{AuxState, Services};
use crate::state::history::HistoryEntry;
use crate::state::pane::{ListingStatus, Pane, PaneId};
use crate::state::session::{FavoriteEntry, PaneSession, TabSession};
use crate::state::workspace::{ClosedTab, Tab, Workspace};
use crate::ui::toasts::Toast;

/// Options for [`navigate`].
#[derive(Clone, Debug, Default)]
pub struct NavOpts {
    /// Record the current place in the back history.
    pub push_history: bool,
    /// Put the cursor on this name once listed ("go up selects the folder you came from").
    pub select: Option<String>,
    /// Restore cursor/scroll from a history entry.
    pub restore: Option<HistoryEntry>,
    /// Apply synchronized navigation to the other pane.
    pub sync: bool,
}

impl NavOpts {
    pub fn user() -> Self {
        NavOpts { push_history: true, sync: true, ..Default::default() }
    }
}

/// (Re)start the listing of a pane. `soft` keeps the current contents until the new
/// listing is complete (refresh / watcher rescan).
pub fn start_listing(pane: &mut Pane, services: &mut Services, soft: bool) {
    if let Some(c) = pane.ctl.take() {
        c.cancel();
    }
    pane.generation += 1;
    match pane.location.clone() {
        Location::Dir(path) => {
            let ctl = OpControl::new();
            pane.ctl = Some(ctl.clone());
            let soft = soft && matches!(pane.status, ListingStatus::Ready);
            if soft {
                pane.refreshing = true;
            } else {
                pane.clear_listing();
                pane.status = ListingStatus::Loading { since: Instant::now() };
                services.watcher.watch(pane.id, &path);
            }
            services.lister.list(pane.id, pane.generation, &path, ctl, soft);
        }
        Location::Archive { archive, inner } => {
            pane.clear_listing();
            pane.status = ListingStatus::Loading { since: Instant::now() };
            services.watcher.unwatch(pane.id);
            crate::services::archive::list(pane.id, pane.generation, &archive, &inner, services.mailbox.postman());
        }
        Location::Search(id) => {
            pane.clear_listing();
            services.watcher.unwatch(pane.id);
            crate::services::search::attach(pane, id);
        }
        _ => {
            pane.clear_listing();
            pane.status = ListingStatus::Virtual;
            services.watcher.unwatch(pane.id);
        }
    }
}

/// Start a pane's first listing lazily (restored background tabs list on first show).
pub fn ensure_started(pane: &mut Pane, services: &mut Services) {
    if pane.generation == 0 {
        start_listing(pane, services, false);
    }
}

/// Stop background work for a pane that is going away.
pub fn dispose_pane(pane: &mut Pane, services: &mut Services) {
    if let Some(c) = pane.ctl.take() {
        c.cancel();
    }
    services.watcher.unwatch(pane.id);
}

pub fn navigate(app: &mut SoyuzApp, id: PaneId, loc: Location, opts: NavOpts) {
    let view = app.view_for(&loc);
    let Some((ti, pi)) = app.ws.locate(id) else { return };
    let old = app.ws.tabs[ti].panes[pi].location.clone();
    {
        let pane = &mut app.ws.tabs[ti].panes[pi];
        if opts.push_history {
            let here = pane.here();
            pane.history.push(here, &loc);
        }
        pane.location = loc.clone();
        pane.view = view;
        pane.pending_selection.clear();
        pane.collapsed_groups.clear();
        pane.pending_cursor = opts.select.clone().or_else(|| opts.restore.as_ref().and_then(|r| r.cursor.clone()));
        pane.restore_scroll = Some(opts.restore.as_ref().map_or(0.0, |r| r.scroll));
        start_listing(pane, &mut app.services, false);
    }
    app.ui.address.editing = false;
    if matches!(loc, Location::Computer) {
        app.services.places.refresh();
    }
    if opts.sync && app.ws.tabs[ti].sync_nav && app.ws.tabs[ti].dual {
        sync_other(app, ti, pi, &old, &loc);
    }
}

/// Synchronized navigation: entering a child folder enters the same-named child in the
/// other pane (if it exists); going up goes up there too.
fn sync_other(app: &mut SoyuzApp, ti: usize, pi: usize, old: &Location, new: &Location) {
    let Some(other) = app.ws.tabs[ti].panes.get(1 - pi.min(1)) else { return };
    let other_id = other.id;
    let Location::Dir(other_dir) = other.location.clone() else { return };
    if new.parent().as_ref() == Some(old) {
        if let Location::Dir(p) = new
            && let Some(name) = p.file_name()
        {
            app.services.lister.probe(other_id, &other_dir.join(name));
        }
    } else if old.parent().as_ref() == Some(new)
        && let Some(parent) = Location::Dir(other_dir.clone()).parent()
    {
        let select = other_dir.file_name().map(|n| n.to_string_lossy().into_owned());
        navigate(app, other_id, parent, NavOpts { push_history: true, select, ..Default::default() });
    }
}

pub fn go_up(app: &mut SoyuzApp, id: PaneId) {
    let Some(pane) = app.ws.find_pane(id) else { return };
    let Some(parent) = pane.location.parent() else { return };
    let select = match &pane.location {
        Location::Dir(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
        Location::Archive { archive, inner } if inner.is_empty() => {
            archive.file_name().map(|n| n.to_string_lossy().into_owned())
        }
        Location::Archive { inner, .. } => inner.rsplit('/').next().map(str::to_string),
        _ => None,
    };
    navigate(app, id, parent, NavOpts { push_history: true, select, sync: true, ..Default::default() });
}

pub fn history_jump(app: &mut SoyuzApp, id: PaneId, back: bool, n: usize) {
    let Some(pane) = app.ws.find_pane_mut(id) else { return };
    let here = pane.here();
    let entry = if back { pane.history.back_n(n, here) } else { pane.history.forward_n(n, here) };
    if let Some(e) = entry {
        let loc = e.location.clone();
        navigate(app, id, loc, NavOpts { restore: Some(e), ..Default::default() });
    }
}

/// Activate an entry: enter folders (respecting `open_folders_in`), launch files.
pub fn open_entry(app: &mut SoyuzApp, id: PaneId, key: &str) {
    let Some(pane) = app.ws.find_pane(id) else { return };
    let Some(e) = pane.entry(key) else { return };
    let path = e.path.clone();
    let is_dir = e.is_dir_like();
    // Archives open as folders; entries inside archives navigate / extract-and-open.
    if crate::ui::archive::open_entry(app, id, &path, is_dir) {
        return;
    }
    if is_dir {
        let loc = Location::Dir(path);
        if app.config.general.open_folders_in == OpenFoldersIn::NewTab {
            open_in_new_tab(app, loc, true);
        } else {
            navigate(app, id, loc, NavOpts::user());
        }
        return;
    }
    launch(app, &path);
}

/// Launch a file with its default application on a short-lived thread (association
/// lookup and process spawn never block the UI).
pub fn launch(app: &mut SoyuzApp, path: &Path) {
    let postman = app.services.mailbox.postman();
    let path = path.to_path_buf();
    let spawned = std::thread::Builder::new().name("soyuz-launch".into()).spawn(move || {
        if let Ok(Some(target)) = soyuz_fs::launch::shortcut_target(&path)
            && target.is_dir()
        {
            postman.send(UiMsg::OpenDir(target));
            return;
        }
        if let Err(e) = soyuz_fs::launch::open_default(&path) {
            postman.send(UiMsg::Notice(
                Toast::error(format!("Couldn't open {}", soyuz_core::entry::display_name(&path))).body(e.to_string()),
            ));
        }
    });
    if let Err(e) = spawned {
        app.ui.toasts.push(Toast::error("Couldn't start the launcher").body(e.to_string()));
    }
}

/// Open the selection (Enter): one entry is activated; several files are all launched
/// and several folders open in background tabs.
pub fn open_selection(app: &mut SoyuzApp) {
    let id = app.ws.pane().id;
    let pane = app.ws.pane();
    let sel: Vec<(String, bool, std::path::PathBuf)> =
        pane.selected_entries().iter().map(|e| (e.name.clone(), e.is_dir_like(), e.path.clone())).collect();
    match sel.len() {
        0 => {}
        1 => open_entry(app, id, &sel[0].0),
        _ => {
            let mut files = 0;
            for (_, is_dir, path) in sel {
                if is_dir {
                    open_in_new_tab(app, Location::Dir(path), false);
                } else if files < 32 {
                    files += 1;
                    launch(app, &path);
                }
            }
        }
    }
}

pub fn open_in_new_tab(app: &mut SoyuzApp, loc: Location, activate: bool) {
    let mut pane = Pane::new(loc.clone(), app.view_for(&loc));
    start_listing(&mut pane, &mut app.services, false);
    let idx = app.ws.insertion_index();
    app.ws.tabs.insert(idx, Tab::new(pane));
    if activate {
        app.ws.active = idx;
    } else if idx <= app.ws.active {
        app.ws.active += 1;
    }
}

/// Make sure the active tab shows two panes; returns the other pane's id.
pub fn ensure_dual(app: &mut SoyuzApp) -> PaneId {
    let loc = app.ws.pane().location.clone();
    let view = app.view_for(&loc);
    let tab = app.ws.tab_mut();
    if tab.panes.len() < 2 {
        let mut p = Pane::new(loc, view);
        start_listing(&mut p, &mut app.services, false);
        tab.panes.push(p);
    }
    tab.dual = true;
    let other = 1 - tab.focused.min(1);
    ensure_started(&mut tab.panes[other], &mut app.services);
    tab.panes[other].id
}

pub fn open_in_other_pane(app: &mut SoyuzApp, loc: Location) {
    let other = ensure_dual(app);
    navigate(app, other, loc, NavOpts { push_history: true, ..Default::default() });
}

pub fn focus_pane(app: &mut SoyuzApp, id: PaneId) {
    if let Some((ti, pi)) = app.ws.locate(id) {
        app.ws.active = ti;
        app.ws.tabs[ti].focused = pi;
        app.ui.focus = Focus::List;
    }
}

pub fn tab_session(t: &Tab) -> TabSession {
    TabSession {
        pinned: t.pinned,
        dual: t.dual,
        orientation: t.orientation,
        split: t.split,
        focused: t.focused,
        sync_nav: t.sync_nav,
        panes: t
            .panes
            .iter()
            .map(|p| PaneSession {
                location: p.location.display(),
                cursor: p.cursor.as_deref().map(str::to_string),
                selection: Vec::new(),
                scroll: p.scroll_y,
            })
            .collect(),
    }
}

pub fn close_tab(app: &mut SoyuzApp, index: usize) {
    if app.ws.tabs.len() <= 1 || index >= app.ws.tabs.len() {
        return;
    }
    let mut tab = app.ws.tabs.remove(index);
    app.ws.closed.push(ClosedTab { index, session: tab_session(&tab) });
    if app.ws.closed.len() > Workspace::MAX_CLOSED {
        app.ws.closed.remove(0);
    }
    for p in &mut tab.panes {
        dispose_pane(p, &mut app.services);
    }
    if app.ws.active > index || app.ws.active >= app.ws.tabs.len() {
        app.ws.active = app.ws.active.saturating_sub(1);
    }
}

pub fn reopen_closed_tab(app: &mut SoyuzApp) {
    let Some(closed) = app.ws.closed.pop() else { return };
    let mut panes = Vec::new();
    for ps in &closed.session.panes {
        if let Some(loc) = Location::parse(&ps.location) {
            let mut p = Pane::new(loc.clone(), app.view_for(&loc));
            p.pending_cursor = ps.cursor.clone();
            p.restore_scroll = Some(ps.scroll);
            start_listing(&mut p, &mut app.services, false);
            panes.push(p);
        }
    }
    if panes.is_empty() {
        return;
    }
    let mut tab = Tab::new(panes.remove(0));
    tab.panes.extend(panes);
    tab.dual = closed.session.dual && tab.panes.len() > 1;
    tab.focused = closed.session.focused.min(tab.panes.len() - 1);
    tab.orientation = closed.session.orientation;
    tab.split = closed.session.split;
    tab.pinned = closed.session.pinned;
    tab.sync_nav = closed.session.sync_nav;
    let idx = closed.index.min(app.ws.tabs.len());
    app.ws.tabs.insert(idx, tab);
    app.ws.active = idx;
}

pub fn duplicate_tab(app: &mut SoyuzApp, index: usize) {
    let Some(t) = app.ws.tabs.get(index) else { return };
    let s = tab_session(t);
    let mut panes = Vec::new();
    for (src, ps) in t.panes.iter().zip(&s.panes) {
        let loc = src.location.clone();
        let mut p = Pane::new(loc, src.view.clone());
        p.pending_cursor = ps.cursor.clone();
        p.restore_scroll = Some(ps.scroll);
        p.history = src.history.clone();
        panes.push(p);
    }
    for p in &mut panes {
        start_listing(p, &mut app.services, false);
    }
    let mut tab = Tab::new(panes.remove(0));
    tab.panes.extend(panes);
    tab.dual = s.dual;
    tab.focused = s.focused;
    tab.orientation = s.orientation;
    tab.split = s.split;
    tab.sync_nav = s.sync_nav;
    let pinned = app.ws.tabs.iter().filter(|t| t.pinned).count();
    let idx = (index + 1).max(pinned).min(app.ws.tabs.len());
    app.ws.tabs.insert(idx, tab);
    app.ws.active = idx;
}

pub fn new_tab(app: &mut SoyuzApp) {
    let loc = Location::Dir(soyuz_fs::home());
    open_in_new_tab(app, loc, true);
}

/// Type-ahead: jump to the first matching name, or feed the live filter.
pub fn type_ahead(app: &mut SoyuzApp, text: &str) {
    let now = Instant::now();
    let filter_mode = app.config.behaviour.type_ahead == TypeAhead::Filter;
    let pane = app.ws.pane_mut();
    if text.trim().is_empty() && !pane.type_ahead.is_active(now) {
        return;
    }
    if filter_mode {
        let f = format!("{}{text}", pane.filter);
        pane.set_filter(&f);
        app.ui.filter_focus = true;
        return;
    }
    let buf = pane.type_ahead.push(text, now).to_string();
    if let Some(pos) = pane.find_prefix(&buf) {
        pane.move_cursor(pos, false, false);
    }
}

pub fn drain_mailbox(app: &mut SoyuzApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_millis(8);
    let mut handled = 0usize;
    while let Some(msg) = app.services.mailbox.try_recv() {
        handle_msg(app, msg);
        handled += 1;
        if handled.is_multiple_of(16) && Instant::now() > deadline {
            ctx.request_repaint();
            break;
        }
    }
}

pub fn handle_msg(app: &mut SoyuzApp, msg: UiMsg) {
    match msg {
        UiMsg::Listing { pane, generation, chunk, done, err } => {
            if let Some(p) = app.ws.find_pane_mut(pane)
                && p.generation == generation
            {
                p.on_listing(chunk, done, err);
            }
        }
        UiMsg::Patched { pane, generation, updated, removed } => {
            if let Some(p) = app.ws.find_pane_mut(pane)
                && p.generation == generation
                && !p.is_loading()
            {
                p.on_patch(updated, &removed);
            }
        }
        UiMsg::Aux { path, result } => {
            let state = match result {
                Ok(mut entries) => {
                    soyuz_core::sort_entries(&mut entries, &soyuz_core::SortSpec::default());
                    AuxState::Ready { entries: std::sync::Arc::new(entries), at: Instant::now() }
                }
                Err(e) => AuxState::Failed(e),
            };
            app.services.aux.insert(path, state);
        }
        UiMsg::Resolved { pane, generation, location } => {
            let current = app.ws.find_pane(pane).filter(|p| p.generation == generation).map(|p| p.location.clone());
            if let Some(lost) = current {
                app.ui.toasts.push(
                    Toast::warning("Folder no longer exists")
                        .body(format!("{} was removed or unmounted.", crate::state::crumbs::location_title(&lost))),
                );
                navigate(app, pane, location, NavOpts::default());
            }
        }
        UiMsg::Probed { pane, location, exists } => {
            if exists {
                navigate(app, pane, location, NavOpts { push_history: true, ..Default::default() });
            }
        }
        UiMsg::Volumes(v) => app.ui.volumes = v,
        UiMsg::CloudPlaces(c) => app.ui.cloud = c,
        UiMsg::Notice(t) => {
            app.ui.toasts.push(t);
        }
        UiMsg::OpenDir(p) => {
            let id = app.ws.pane().id;
            navigate(app, id, Location::Dir(p), NavOpts::user());
        }
    }
}

pub fn drain_watcher(app: &mut SoyuzApp) {
    if !app.config.behaviour.auto_refresh {
        let _ = app.services.watcher.drain();
        return;
    }
    for (id, kind) in app.services.watcher.drain() {
        let Some(pane) = app.ws.find_pane_mut(id) else { continue };
        let Location::Dir(dir) = pane.location.clone() else { continue };
        match kind {
            WatchKind::Changed(paths) => {
                if matches!(pane.status, ListingStatus::Ready) && !pane.refreshing {
                    app.services.lister.stat(id, pane.generation, &dir, paths);
                }
            }
            WatchKind::Rescan => start_listing(pane, &mut app.services, true),
            WatchKind::Lost => app.services.lister.resolve(id, pane.generation, &dir),
        }
    }
}

pub fn refresh(app: &mut SoyuzApp) {
    let pane = app.ws.pane_mut();
    start_listing(pane, &mut app.services, true);
    app.services.aux.clear();
    app.services.places.refresh();
}

fn coming_soon(app: &mut SoyuzApp, action: &Action) {
    app.ui.toasts.push(
        Toast::info(format!("{} is coming soon", action.title()))
            .body("File operations arrive with the jobs engine integration."),
    );
}

pub fn apply(app: &mut SoyuzApp, cmd: Cmd, ctx: &egui::Context) {
    match cmd {
        Cmd::Action(a) => crate::actions::execute(app, a, ctx),
        Cmd::PaneAction(id, a) => {
            focus_pane(app, id);
            crate::actions::execute(app, a, ctx);
        }
        Cmd::Navigate { pane, location, select } => {
            navigate(app, pane, location, NavOpts { select, ..NavOpts::user() });
        }
        Cmd::Open { pane, key } => open_entry(app, pane, &key),
        Cmd::OpenInNewTab { location, activate } => open_in_new_tab(app, location, activate),
        Cmd::OpenInOtherPane(loc) => open_in_other_pane(app, loc),
        Cmd::OpenWith { path, app: info } => {
            if let Err(e) = soyuz_fs::launch::open_with(&path, &info) {
                app.ui.toasts.push(Toast::error(format!("Couldn't open with {}", info.name)).body(e.to_string()));
            }
        }
        Cmd::HistoryJump { pane, back, n } => history_jump(app, pane, back, n),
        Cmd::FocusPane(id) => focus_pane(app, id),
        Cmd::CopyText(t) => {
            let n = t.lines().count();
            ctx.copy_text(t);
            app.ui.toasts.push(Toast::success(if n > 1 {
                format!("Copied {n} paths")
            } else {
                "Copied to clipboard".into()
            }));
        }
        Cmd::Toast(t) => {
            app.ui.toasts.push(t);
        }
        Cmd::ViewChanged(id) => {
            if app.config.behaviour.remember_folder_views
                && let Some(p) = app.ws.find_pane(id)
            {
                let (loc, view) = (p.location.clone(), p.view.clone());
                app.views.remember(&loc, &view);
            }
        }
        Cmd::ComingSoon(a) => coming_soon(app, &a),
        Cmd::RevealInSystem(p) => {
            if let Err(e) = soyuz_fs::launch::reveal_in_system(&p) {
                app.ui.toasts.push(Toast::error("Couldn't reveal in the system file manager").body(e.to_string()));
            }
        }
        Cmd::TerminalAt(dir) => {
            let preferred = (!app.config.terminal.shell.is_empty()).then(|| app.config.terminal.shell.clone());
            if let Err(e) = soyuz_fs::launch::open_terminal_at(&dir, preferred.as_deref()) {
                app.ui.toasts.push(Toast::error("Couldn't open a terminal").body(e.to_string()));
            }
        }
        Cmd::AddFavorite(path) => {
            let s = &mut app.ui.sidebar;
            if !s.favorites.iter().any(|f| f.path == path) {
                let name = soyuz_core::entry::display_name(&path);
                s.favorites.push(FavoriteEntry { name: name.clone(), path });
                s.favorites_customized = true;
                app.ui.toasts.push(Toast::success(format!("Added “{name}” to Favorites")));
            }
        }
        Cmd::RemoveFavorite(path) => {
            let s = &mut app.ui.sidebar;
            s.favorites.retain(|f| f.path != path);
            s.favorites_customized = true;
        }
        Cmd::MoveFavorite { from, to } => {
            let s = &mut app.ui.sidebar;
            if from < s.favorites.len() {
                let f = s.favorites.remove(from);
                let to = if to > from { to - 1 } else { to };
                s.favorites.insert(to.min(s.favorites.len()), f);
                s.favorites_customized = true;
            }
        }
        Cmd::SelectTab(i) => {
            if i < app.ws.tabs.len() {
                app.ws.active = i;
            }
        }
        Cmd::CloseTab(i) => close_tab(app, i),
        Cmd::CloseOtherTabs(i) => {
            let keep = app.ws.tabs.get(i).map(|t| t.id);
            let mut idx = app.ws.tabs.len();
            while idx > 0 {
                idx -= 1;
                if Some(app.ws.tabs[idx].id) != keep && !app.ws.tabs[idx].pinned {
                    close_tab(app, idx);
                }
            }
            if let Some(k) = keep {
                app.ws.active = app.ws.tabs.iter().position(|t| t.id == k).unwrap_or(0);
            }
        }
        Cmd::CloseTabsToRight(i) => {
            while app.ws.tabs.len() > i + 1 {
                let last = app.ws.tabs.len() - 1;
                close_tab(app, last);
            }
        }
        Cmd::DuplicateTab(i) => duplicate_tab(app, i),
        Cmd::PinTab(i, pinned) => app.ws.set_pinned(i, pinned),
        Cmd::MoveTab { from, to } => app.ws.move_tab(from, to),
        Cmd::NewTabAfter(i) => {
            app.ws.active = i.min(app.ws.tabs.len() - 1);
            new_tab(app);
        }
        Cmd::SetTheme(pref) => {
            ctx.set_theme(pref);
            app.ui.theme = pref;
        }
        Cmd::ApplyViewToAll(id) => {
            if let Some(p) = app.ws.find_pane(id) {
                let view = p.view.clone();
                app.views.apply_to_all(&view);
                app.ui.toasts.push(Toast::success("View applied to all folders"));
            }
        }
        Cmd::SetSplit(f) => app.ws.tab_mut().split = f.clamp(0.15, 0.85),
        Cmd::OpenTerminalPanel(dir) => crate::ui::panels::terminal::open_at(app, ctx, dir),
        Cmd::SubmitJob(spec) => {
            // Minimal handler until the app-wide jobs service lands (see services::archive).
            if crate::services::archive::submit_job(ctx, *spec).is_none() {
                app.ui.toasts.push(Toast::error("Couldn't start the job"));
            }
        }
        Cmd::ExtractEntries { archive, items, strip, dest } => {
            crate::ui::archive::extract_entries(app, archive, items, strip, dest)
        }
        Cmd::RunSavedSearch(i) => crate::ui::search::run_saved(app, i),
        Cmd::RemoveSavedSearch(i) => crate::ui::search::remove_saved(app, i),
        Cmd::RevealPath(path) => {
            let select = path.file_name().map(|n| n.to_string_lossy().into_owned());
            if let Some(parent) = path.parent() {
                let id = app.ws.pane().id;
                navigate(app, id, Location::Dir(parent.to_path_buf()), NavOpts { select, ..NavOpts::user() });
            }
        }
    }
}

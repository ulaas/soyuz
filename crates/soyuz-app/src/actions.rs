//! `execute(app, Action)`: the single dispatcher used by key bindings, menus, toolbar
//! buttons and toasts. Actions that are not implemented yet show a
//! "coming soon" toast so every binding gives feedback.

use soyuz_core::config::{Action, ActionCategory, TypeAhead};
use soyuz_core::{Column, GroupBy, Location, SortDir, ViewMode};

use crate::cmd::Cmd;
use crate::core::{AppCore, Focus};
use crate::host::Host;
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;
use crate::ops::{self, NavOpts};
use crate::state::layout::NavDir;
use crate::state::view_memory::{ZOOM_MAX, ZOOM_MIN};

pub fn execute(app: &mut AppCore, action: Action, host: &dyn Host) {
    use Action as A;
    if crate::ai::handles(&action) {
        crate::ai::execute_action(app, action, host);
        return;
    }
    // Tab groups, workspaces, quad layout, windows, history graph, timeline.
    if crate::models::workspaces::action(app, &action) {
        return;
    }
    let pane_id = app.ws.pane().id;
    match action {
        // ---- navigation ----
        A::Back => ops::history_jump(app, pane_id, true, 1),
        A::Forward => ops::history_jump(app, pane_id, false, 1),
        A::Up => ops::go_up(app, pane_id),
        A::Home => ops::navigate(app, pane_id, Location::Dir(soyuz_fs::home()), NavOpts::user()),
        A::Root => {
            if let Location::Dir(p) = &app.ws.pane().location {
                let root: std::path::PathBuf = p.ancestors().last().map(|r| r.to_path_buf()).unwrap_or_default();
                if !root.as_os_str().is_empty() {
                    ops::navigate(app, pane_id, Location::Dir(root), NavOpts::user());
                }
            }
        }
        A::Computer => ops::navigate(app, pane_id, Location::Computer, NavOpts::user()),
        A::OpenTrash => ops::navigate(app, pane_id, Location::Trash, NavOpts::user()),
        A::GoTo | A::EditPath | A::FocusAddressBar => {
            let loc = app.ws.pane().location.display();
            app.ui.address.editing = true;
            app.ui.address.text = loc;
            app.ui.address.request_focus = true;
            app.ui.address.completion = None;
        }
        A::Refresh => ops::refresh(app),
        A::ToggleSidebar => app.ui.sidebar_visible = !app.ui.sidebar_visible,
        A::ToggleTreeSidebar => crate::models::tree::toggle(app, host),
        A::FocusSidebar => {
            app.ui.sidebar_visible = true;
            app.ui.focus = Focus::Sidebar;
        }
        A::FocusList => {
            app.ui.focus = Focus::List;
            app.frontend.push(FrontendCmd::ReleaseFocus);
        }
        A::FocusOtherPane => {
            let tab = app.ws.tab_mut();
            // Dual: the other pane; quad: the next one clockwise in reading order.
            tab.focused = tab.next_visible();
            app.ui.focus = Focus::List;
        }
        A::NextTab => {
            let n = app.ws.tabs.len();
            app.ws.active = (app.ws.active + 1) % n;
        }
        A::PrevTab => {
            let n = app.ws.tabs.len();
            app.ws.active = (app.ws.active + n - 1) % n;
        }
        A::SelectTab(i) => {
            let n = app.ws.tabs.len();
            app.ws.active = if i >= 9 { n - 1 } else { (i as usize).saturating_sub(1).min(n - 1) };
        }
        A::OpenBookmark(i) => {
            if let Some(b) = app.config.bookmarks.get((i as usize).saturating_sub(1)).cloned() {
                ops::navigate(app, pane_id, Location::Dir(b.path), NavOpts::user());
            }
        }

        // ---- tabs & panes ----
        A::NewTab => ops::new_tab(app),
        A::CloseTab => {
            let i = app.ws.active;
            ops::close_tab(app, i);
        }
        A::ReopenClosedTab => ops::reopen_closed_tab(app),
        A::DuplicateTab => {
            let i = app.ws.active;
            ops::duplicate_tab(app, i);
        }
        A::PinTab => {
            let i = app.ws.active;
            let pinned = app.ws.tabs[i].pinned;
            app.ws.set_pinned(i, !pinned);
        }
        A::MoveTabLeft => {
            let i = app.ws.active;
            app.ws.move_tab(i, i.saturating_sub(1));
        }
        A::MoveTabRight => {
            let i = app.ws.active;
            app.ws.move_tab(i, i + 1);
        }
        A::ToggleDualPane => {
            if app.ws.tab().dual {
                let tab = app.ws.tab_mut();
                tab.set_layout(crate::state::workspace::PaneLayout::Single);
            } else {
                ops::ensure_dual(app);
            }
        }
        A::SwapPanes => {
            let tab = app.ws.tab_mut();
            if tab.dual && tab.panes.len() == 2 {
                tab.panes.swap(0, 1);
            }
        }
        A::MirrorPaneToOther => {
            let loc = app.ws.pane().location.clone();
            let other = ops::ensure_dual(app);
            ops::navigate(app, other, loc, NavOpts { push_history: true, ..Default::default() });
        }
        A::OpenInOtherPane => {
            let pane = app.ws.pane();
            let target = pane
                .selected_entries()
                .first()
                .filter(|e| e.is_dir_like())
                .map(|e| Location::Dir(e.path.clone()))
                .unwrap_or_else(|| pane.location.clone());
            ops::open_in_other_pane(app, target);
        }
        A::NewWindow => {
            let loc = app.ws.pane().location.clone();
            app.cmds.push(Cmd::NewWindow(loc));
        }
        A::TabToNewWindow => ops::window::tab_to_new_window(app),
        // Closes this window (the main window hands over to another one, if any).
        A::CloseWindow => host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Close),
        A::SyncNavigation => {
            let tab = app.ws.tab_mut();
            tab.sync_nav = !tab.sync_nav;
            let on = tab.sync_nav;
            app.ui.toasts.push(Toast::info(if on { tr!("actions-sync-nav-on") } else { tr!("actions-sync-nav-off") }));
        }

        // ---- view ----
        A::ViewDetails => set_mode(app, ViewMode::Details),
        A::ViewGrid => set_mode(app, ViewMode::Grid),
        A::ViewList => set_mode(app, ViewMode::List),
        A::ViewMiller => set_mode(app, ViewMode::Miller),
        A::ViewGallery => set_mode(app, ViewMode::Gallery),
        A::ZoomIn | A::ZoomOut | A::ZoomReset => {
            let def = app.default_view.zoom;
            let pane = app.ws.pane_mut();
            let z = pane.view.zoom as f32;
            pane.view.zoom = match action {
                A::ZoomIn => (z * 1.25).round() as u16,
                A::ZoomOut => (z / 1.25).round() as u16,
                _ => def,
            }
            .clamp(ZOOM_MIN, ZOOM_MAX);
            if matches!(pane.view.mode, ViewMode::Details | ViewMode::List | ViewMode::Miller) && action != A::ZoomReset
            {
                pane.view.mode = ViewMode::Grid;
            }
            app.cmds.push(Cmd::ViewChanged(pane_id));
        }
        A::ToggleHidden => {
            app.ui.show_hidden = !app.ui.show_hidden;
            for t in &mut app.ws.tabs {
                for p in &mut t.panes {
                    p.dirty = true;
                }
            }
            let on = app.ui.show_hidden;
            app.ui.toasts.push(Toast::info(if on {
                tr!("actions-showing-hidden")
            } else {
                tr!("actions-hiding-hidden")
            }));
        }
        A::SortBy(col) => sort_by(app, col),
        A::ToggleSortDir => {
            let pane = app.ws.pane_mut();
            pane.view.sort.dir = match pane.view.sort.dir {
                SortDir::Asc => SortDir::Desc,
                SortDir::Desc => SortDir::Asc,
            };
            pane.resort();
            app.cmds.push(Cmd::ViewChanged(pane_id));
        }
        A::SetGroupBy(g) => {
            let pane = app.ws.pane_mut();
            pane.view.group = g;
            pane.collapsed_groups.clear();
            pane.dirty = true;
            app.cmds.push(Cmd::ViewChanged(pane_id));
        }
        A::ChooseColumns => app.frontend.push(FrontendCmd::ChooseColumns(pane_id)),
        A::ResetFolderView => {
            let loc = app.ws.pane().location.clone();
            app.views.reset(&loc);
            let view = app.view_for(&loc);
            let pane = app.ws.pane_mut();
            pane.view = view;
            pane.resort();
            app.ui.toasts.push(Toast::info(tr!("actions-folder-view-reset")));
        }
        A::ApplyViewToAll => {
            app.frontend.push(FrontendCmd::Confirm {
                title: tr!("actions-apply-view-title").into(),
                message: tr!("actions-apply-view-message").into(),
                confirm: tr!("actions-apply-view-confirm").into(),
                on_confirm: Cmd::ApplyViewToAll(pane_id),
            });
        }
        A::ToggleStatusBar => app.ui.status_visible = !app.ui.status_visible,
        A::ToggleCheckboxes => app.ui.checkbox_mode = !app.ui.checkbox_mode,
        A::ToggleTerminal => crate::models::terminal::toggle(app, host),
        A::FocusTerminal => crate::models::terminal::focus(app, host),
        A::TogglePreviewPane => app.ui.preview.toggle(),
        A::ToggleInfoPane => app.ui.preview.toggle_info(),
        A::ToggleFlatView => crate::models::search::toggle_flat_view(app),
        A::ToggleHexPreview => app.ui.preview.toggle_hex(),
        A::ToggleActivityPanel => app.ui.bottom.toggle(crate::models::panels::BottomTab::Activity),

        // ---- selection ----
        A::SelectAll => {
            let pane = app.ws.pane_mut();
            let order = pane.visible_keys();
            pane.selection.select_all(&order);
        }
        A::SelectNone => {
            let pane = app.ws.pane_mut();
            if pane.filter_active() {
                pane.set_filter("");
            } else {
                pane.selection.clear();
            }
        }
        A::InvertSelection => {
            let pane = app.ws.pane_mut();
            let order = pane.visible_keys();
            pane.selection.invert(&order);
        }
        A::SelectSameExtension | A::SelectSameName => {
            let pane = app.ws.pane_mut();
            let Some(cur) = pane.cursor.as_deref().and_then(|c| pane.entry(c)).cloned() else { return };
            let by_ext = action == A::SelectSameExtension;
            let keys: Vec<_> = pane
                .visible
                .iter()
                .filter(|&&i| {
                    let e = &pane.entries[i];
                    if by_ext {
                        e.extension().map(str::to_lowercase) == cur.extension().map(str::to_lowercase)
                    } else {
                        e.stem().eq_ignore_ascii_case(cur.stem())
                    }
                })
                .map(|&i| pane.keys[i].clone())
                .collect();
            for k in keys {
                pane.selection.insert(k);
            }
        }
        A::SelectByPattern => app.frontend.push(FrontendCmd::SelectByPattern(pane_id)),
        A::MarkToggle | A::MarkAndMoveDown => {
            let pane = app.ws.pane_mut();
            if let Some(k) = pane.cursor.clone() {
                pane.selection.toggle(k);
            }
            if action == A::MarkAndMoveDown {
                cursor(app, NavDir::Down, false, true);
            }
        }
        A::CursorUp => cursor(app, NavDir::Up, false, false),
        A::CursorDown => cursor(app, NavDir::Down, false, false),
        A::CursorPageUp => cursor(app, NavDir::PageUp, false, false),
        A::CursorPageDown => cursor(app, NavDir::PageDown, false, false),
        A::CursorHome => cursor(app, NavDir::Home, false, false),
        A::CursorEnd => cursor(app, NavDir::End, false, false),
        A::ExtendUp => cursor(app, NavDir::Up, true, false),
        A::ExtendDown => cursor(app, NavDir::Down, true, false),
        A::Filter => app.ui.filter_focus = true,
        A::Search => crate::models::search::open(app, false),
        A::SearchContent => crate::models::search::open(app, true),
        A::FolderSize => crate::models::search::folder_sizes(app),
        A::ClearFilter => app.ws.pane_mut().set_filter(""),
        A::TypeAheadToggle => {
            let b = &mut app.config.behaviour;
            b.type_ahead = if b.type_ahead == TypeAhead::Jump { TypeAhead::Filter } else { TypeAhead::Jump };
            let msg = if b.type_ahead == TypeAhead::Jump {
                tr!("actions-type-ahead-jump")
            } else {
                tr!("actions-type-ahead-filter")
            };
            app.ui.toasts.push(Toast::info(msg));
        }

        // ---- file actions available now ----
        A::Open => ops::open_selection(app),
        A::OpenWith => {
            let file = app.ws.pane().selected_entries().first().filter(|e| !e.is_dir_like()).map(|e| e.path.clone());
            match file {
                Some(path) => app.cmds.push(Cmd::ChooseOpenWith(path)),
                None => {
                    app.ui.toasts.push(Toast::info(tr!("actions-open-with-needs-file")));
                }
            }
        }
        A::RunElevated => ops::window::run_elevated(app),
        A::OpenInNewTab => {
            let dirs: Vec<Location> = app
                .ws
                .pane()
                .selected_entries()
                .iter()
                .filter(|e| e.is_dir_like())
                .map(|e| Location::Dir(e.path.clone()))
                .collect();
            for d in dirs {
                ops::open_in_new_tab(app, d, false);
            }
        }
        A::OpenTerminalHere | A::OpenInTerminal => {
            let pane = app.ws.pane();
            let dir = match (&pane.location, action == A::OpenInTerminal) {
                (_, true) => pane.selected_entries().first().filter(|e| e.is_dir_like()).map(|e| e.path.clone()),
                _ => None,
            }
            .or_else(|| pane.location.as_fs_path().map(|p| p.to_path_buf()));
            if let Some(d) = dir {
                app.cmds.push(Cmd::OpenTerminalPanel(d));
            }
        }
        A::RevealInSystem => {
            let pane = app.ws.pane();
            let target = pane
                .selected_entries()
                .first()
                .map(|e| e.path.clone())
                .or_else(|| pane.location.as_fs_path().map(|p| p.to_path_buf()));
            if let Some(t) = target {
                app.cmds.push(Cmd::RevealInSystem(t));
            }
        }
        A::CopyPath | A::CopyPathPosix | A::CopyPathQuoted | A::CopyName | A::CopyAsFileList => {
            let pane = app.ws.pane();
            let mut items: Vec<String> = pane
                .selected_entries()
                .iter()
                .map(|e| {
                    let p = soyuz_fs::strip_verbatim(&e.path).to_string_lossy().into_owned();
                    match action {
                        A::CopyName => e.name.clone(),
                        A::CopyPathPosix => p.replace('\\', "/"),
                        A::CopyPathQuoted => format!("\"{p}\""),
                        _ => p,
                    }
                })
                .collect();
            if items.is_empty()
                && let Some(p) = pane.location.as_fs_path()
            {
                items.push(soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned());
            }
            if !items.is_empty() {
                let sep = if action == A::CopyAsFileList || action == A::CopyPath { "\n" } else { " " };
                app.cmds.push(Cmd::CopyText(items.join(sep)));
            }
        }

        // ---- tools ----
        A::Quit => {
            // Every window: close the root viewport for real (with the job check).
            app.workspaces.quitting = true;
            host.window(crate::host::WindowId::Root, crate::host::WindowCmd::Close);
        }

        A::About => app.frontend.push(FrontendCmd::About),
        A::Welcome => app.ui.welcome = true,
        A::UiZoomIn | A::UiZoomOut | A::UiZoomReset => {
            let cur = if app.config.appearance.ui_scale > 0.25 { app.config.appearance.ui_scale } else { 1.0 };
            let next = match action {
                A::UiZoomIn => ui_zoom_step(cur, 1),
                A::UiZoomOut => ui_zoom_step(cur, -1),
                _ => 1.0,
            };
            crate::models::settings::update_config(app, host, |c| c.appearance.ui_scale = next);
            let percent = format!("{:.0}", next * 100.0);
            let mut t = Toast::info(tr!("actions-ui-zoom")).body(tr!("actions-ui-zoom-body", percent = percent));
            t.duration = Some(1.5);
            app.ui.toasts.push_replacing(t);
        }
        A::ToggleHighContrast => {
            let on = !app.config.appearance.high_contrast;
            crate::models::settings::update_config(app, host, |c| c.appearance.high_contrast = on);
            app.ui.toasts.push(Toast::info(if on {
                tr!("actions-high-contrast-on")
            } else {
                tr!("actions-high-contrast-off")
            }));
        }
        A::ShowKeybindings => {
            app.frontend.push(FrontendCmd::OpenSettings(Some(crate::models::settings::Section::Keyboard)))
        }
        A::CommandPalette => crate::palette::open(app, crate::palette::query::Mode::Commands, ""),
        A::JumpToFolderDialog | A::OpenRecent => crate::palette::open(app, crate::palette::query::Mode::Folders, ""),
        A::Settings => app.frontend.push(FrontendCmd::OpenSettings(None)),
        A::CustomCommand(name) => crate::commands::run_named(app, &name, host),
        A::RunScript(id) => crate::automation::run_action(app, &id),
        A::OpenScripts => app.frontend.push(FrontendCmd::OpenSettings(Some(crate::models::settings::Section::Scripts))),
        A::OpenAutomations => {
            app.frontend.push(FrontendCmd::OpenSettings(Some(crate::models::settings::Section::Automations)))
        }
        A::ReloadScripts => crate::automation::reload_scripts(app),
        A::EditConfigFile => {
            let path = soyuz_core::config::default_path();
            if !path.exists()
                && let Err(e) = soyuz_core::config::save_pretty(&path, &app.config)
            {
                app.ui.toasts.push(Toast::error(tr!("actions-config-create-failed")).body(e.to_string()));
                return;
            }
            ops::launch(app, &path);
        }
        A::ReloadConfig => crate::services::config_watch::reload_now(app, host),
        A::QuickLook | A::QuickLookNext | A::QuickLookPrev => {
            if !app.ws.pane().type_ahead.is_active(std::time::Instant::now()) {
                crate::models::quicklook::toggle(app);
            }
        }
        A::CreateArchive => crate::models::archive::create_dialog(app),
        A::ExtractHere => crate::models::archive::extract_here(app),
        A::ExtractToFolder => crate::models::archive::extract_to_folder(app),
        A::ExtractTo => crate::models::archive::extract_to_dialog(app),
        A::TestArchive => crate::models::archive::test_archive(app),
        a if crate::models::remote::handles(app, &a) => crate::models::remote::execute(app, a, host),
        a if crate::models::properties::handles(app, &a) => crate::models::properties::execute(app, a, host),
        a if crate::models::tools::handles(&a) => crate::models::tools::execute(app, a, host),
        a if crate::models::archive::handles_edit(app, &a) => crate::models::archive::execute_edit(app, a),
        a if crate::editors::handles(&a) => crate::editors::execute(app, a),
        a if crate::ops::fileops::handles(&a) => crate::ops::fileops::execute(app, a, host),
        other => {
            if other.category() == ActionCategory::FileOps || other.category() == ActionCategory::Jobs {
                app.cmds.push(Cmd::ComingSoon(other));
            } else {
                app.ui
                    .toasts
                    .push(Toast::info(tr!("actions-coming-soon", action = crate::palette::catalog::title(&other))));
            }
        }
    }
}

/// Actions without an implementation yet: hidden from the command palette and the
/// keyboard settings (a binding in `config.toml` still answers with a "coming soon").
pub fn unavailable(a: &Action) -> bool {
    // Every action is implemented (including the folder tree); NTFS junctions exist
    // on Windows only.
    matches!(a, Action::NewJunction) && !cfg!(windows)
}

fn set_mode(app: &mut AppCore, mode: ViewMode) {
    let id = app.ws.pane().id;
    let default_zoom = app.default_view.zoom;
    let pane = app.ws.pane_mut();
    if pane.view.mode != mode {
        let from = std::mem::replace(&mut pane.view.mode, mode);
        // The gallery is about big previews.
        if mode == ViewMode::Gallery && pane.view.zoom < 160 {
            pane.view.zoom = 192;
        }
        // …and the grid should not inherit the gallery's giant tiles.
        if from == ViewMode::Gallery && mode == ViewMode::Grid && pane.view.zoom >= 160 {
            pane.view.zoom = default_zoom;
        }
        pane.scroll_to_cursor = true;
        app.cmds.push(Cmd::ViewChanged(id));
    }
}

/// Sort by `col`; the same column again flips the direction. Size and dates start
/// descending (largest / newest first), text columns ascending.
pub fn sort_by(app: &mut AppCore, col: Column) {
    let id = app.ws.pane().id;
    let pane = app.ws.pane_mut();
    if pane.view.sort.column == col {
        pane.view.sort.dir = match pane.view.sort.dir {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        };
    } else {
        pane.view.sort.column = col;
        pane.view.sort.dir = default_dir(col);
    }
    pane.resort();
    pane.scroll_to_cursor = true;
    app.cmds.push(Cmd::ViewChanged(id));
}

pub fn default_dir(col: Column) -> SortDir {
    match col {
        Column::Size | Column::Modified | Column::Created | Column::Accessed | Column::ItemCount => SortDir::Desc,
        _ => SortDir::Asc,
    }
}

/// Keyboard cursor movement using the pane's current layout.
pub fn cursor(app: &mut AppCore, dir: NavDir, extend: bool, keep: bool) {
    let pane = app.ws.pane_mut();
    let from = pane.cursor_pos();
    let target = match &pane.layout {
        Some(l) if l.n == pane.visible.len() => l.nav(from, dir, pane.viewport_h),
        _ => {
            let n = pane.visible.len();
            if n == 0 {
                None
            } else {
                let i = from.unwrap_or(0);
                Some(match dir {
                    NavDir::Up | NavDir::Left => i.saturating_sub(1),
                    NavDir::Down | NavDir::Right => (i + 1).min(n - 1),
                    NavDir::PageUp => i.saturating_sub(pane.page_rows),
                    NavDir::PageDown => (i + pane.page_rows).min(n - 1),
                    NavDir::Home => 0,
                    NavDir::End => n - 1,
                })
            }
        }
    };
    if let Some(t) = target {
        pane.move_cursor(t, extend, keep);
    }
    app.ui.focus = Focus::List;
}

/// Group-by choices shown in menus (English names; the UI shows
/// [`crate::palette::catalog::group_label`]).
pub const GROUPS: [(GroupBy, &str); 6] = [
    (GroupBy::None, "None"),
    (GroupBy::Kind, "Type"),
    (GroupBy::Date, "Date modified"),
    (GroupBy::Size, "Size"),
    (GroupBy::Extension, "Extension"),
    (GroupBy::FirstLetter, "First letter"),
];

/// Interface zoom presets (browser-like steps).
const UI_ZOOM_STEPS: [f32; 13] = [0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

/// The next preset above (`dir > 0`) or below the current scale.
pub fn ui_zoom_step(cur: f32, dir: i32) -> f32 {
    if dir > 0 {
        UI_ZOOM_STEPS.iter().copied().find(|s| *s > cur + 0.001).unwrap_or(3.0)
    } else {
        UI_ZOOM_STEPS.iter().rev().copied().find(|s| *s < cur - 0.001).unwrap_or(0.5)
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::ui_zoom_step;

    #[test]
    fn ui_zoom_steps() {
        assert_eq!(ui_zoom_step(1.0, 1), 1.1);
        assert_eq!(ui_zoom_step(1.0, -1), 0.9);
        assert_eq!(ui_zoom_step(1.05, -1), 1.0);
        assert_eq!(ui_zoom_step(3.0, 1), 3.0);
        assert_eq!(ui_zoom_step(0.5, -1), 0.5);
    }
}

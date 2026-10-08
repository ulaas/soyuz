//! Context menus (items / background / column header) and the reusable View, Sort and
//! Group-by menu sections shared with the toolbar. Every entry dispatches through
//! `Cmd::PaneAction`, so menus, keys and toolbar behave identically.

use egui::Ui;
use soyuz_core::config::{Action, Keymap};
use soyuz_core::{Column, Location, SortDir, ViewMode};

use crate::actions::GROUPS;
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::state::pane::{ContextTarget, Pane, PaneId};
use crate::theme::icons::ph;
use crate::ui::views::ViewCtx;
use crate::ui::widgets::{menu_caption, menu_check, menu_item, menu_separator, submenu};

/// A menu row bound to an action (shortcut hint from the keymap).
fn action_item(ui: &mut Ui, km: &Keymap, cmds: &mut Vec<Cmd>, pane: PaneId, icon: &str, label: &str, action: Action) {
    let sc = chord_label(km, &action);
    if menu_item(ui, Some(icon), label, sc.as_deref(), true).clicked() {
        cmds.push(Cmd::PaneAction(pane, action));
        ui.close();
    }
}

fn soon_item(ui: &mut Ui, km: &Keymap, cmds: &mut Vec<Cmd>, icon: &str, label: &str, action: Action) {
    let sc = chord_label(km, &action);
    if menu_item(ui, Some(icon), label, sc.as_deref(), true).clicked() {
        cmds.push(Cmd::ComingSoon(action));
        ui.close();
    }
}

pub fn pane_menu(ui: &mut Ui, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    ui.set_width(260.0);
    match pane.context {
        ContextTarget::Items if !pane.selection.is_empty() => items_menu(ui, pane, v),
        _ => background_menu(ui, pane, v),
    }
}

fn items_menu(ui: &mut Ui, pane: &Pane, v: &mut ViewCtx<'_>) {
    let km = v.keymap;
    let id = pane.id;
    let sel = pane.selected_entries();
    let single = sel.len() == 1;
    let all_dirs = sel.iter().all(|e| e.is_dir_like());
    let first = sel.first().map(|e| (e.path.clone(), e.is_dir_like()));
    let cmds = &mut *v.cmds;

    action_item(ui, km, cmds, id, if all_dirs { ph::FOLDER_OPEN } else { ph::ARROW_SQUARE_OUT }, "Open", Action::Open);
    if all_dirs {
        action_item(ui, km, cmds, id, ph::BROWSERS, "Open in new tab", Action::OpenInNewTab);
        if single
            && let Some((path, _)) = &first
            && menu_item(ui, Some(ph::SQUARE_SPLIT_HORIZONTAL), "Open in other pane", None, true).clicked()
        {
            cmds.push(Cmd::OpenInOtherPane(Location::Dir(path.clone())));
            ui.close();
        }
    }
    if single && let Some((path, false)) = &first {
        let path = path.clone();
        submenu(ui, Some(ph::APP_WINDOW), "Open with", |ui| {
            ui.set_width(220.0);
            let apps = apps_for_cached(ui, &path);
            if apps.is_empty() {
                menu_item(ui, Some(ph::INFO), "No registered apps found", None, false);
            }
            for app in apps {
                if menu_item(ui, Some(ph::APP_WINDOW), &app.name, None, app.launchable).clicked() {
                    cmds.push(Cmd::OpenWith { path: path.clone(), app: app.clone() });
                    ui.close();
                }
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::DOTS_THREE), "Choose another app…", None, true).clicked() {
                cmds.push(Cmd::ComingSoon(Action::OpenWith));
                ui.close();
            }
        });
    }
    menu_separator(ui);
    soon_item(ui, km, cmds, ph::SCISSORS, "Cut", Action::Cut);
    soon_item(ui, km, cmds, ph::COPY, "Copy", Action::Copy);
    soon_item(ui, km, cmds, ph::PENCIL_SIMPLE, "Rename", Action::Rename);
    soon_item(ui, km, cmds, ph::COPY_SIMPLE, "Duplicate", Action::Duplicate);
    soon_item(ui, km, cmds, ph::TRASH, "Move to trash", Action::Delete);
    menu_separator(ui);
    action_item(
        ui,
        km,
        cmds,
        id,
        ph::CLIPBOARD_TEXT,
        if single { "Copy path" } else { "Copy paths" },
        Action::CopyPath,
    );
    action_item(ui, km, cmds, id, ph::TEXT_AA, if single { "Copy name" } else { "Copy names" }, Action::CopyName);
    if single {
        action_item(
            ui,
            km,
            cmds,
            id,
            ph::ARROW_SQUARE_UP_RIGHT,
            "Reveal in system file manager",
            Action::RevealInSystem,
        );
    }
    if single && let Some((path, true)) = &first {
        action_item(ui, km, cmds, id, ph::TERMINAL_WINDOW, "Open terminal here", Action::OpenInTerminal);
        if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), "Open external terminal here", None, true).clicked() {
            cmds.push(Cmd::TerminalAt(path.clone()));
            ui.close();
        }
        if menu_item(ui, Some(ph::STAR), "Add to favorites", None, true).clicked() {
            cmds.push(Cmd::AddFavorite(path.clone()));
            ui.close();
        }
    }
    let any_archive = sel.iter().any(|e| !e.is_dir_like() && crate::services::archive::is_browsable(&e.path));
    crate::ui::archive::context_items(ui, cmds, id, &pane.location, any_archive, !sel.is_empty());
    if all_dirs {
        action_item(ui, km, cmds, id, ph::CHART_PIE, "Calculate folder sizes", Action::FolderSize);
    }
    menu_separator(ui);
    action_item(ui, km, cmds, id, ph::INFO, "Properties", Action::Properties);
}

fn background_menu(ui: &mut Ui, pane: &Pane, v: &mut ViewCtx<'_>) {
    let km = v.keymap;
    let id = pane.id;
    let cmds = &mut *v.cmds;
    submenu(ui, Some(ph::SQUARES_FOUR), "View", |ui| {
        ui.set_width(220.0);
        view_mode_items(ui, km, cmds, pane);
    });
    submenu(ui, Some(ph::SORT_ASCENDING), "Sort by", |ui| {
        ui.set_width(220.0);
        sort_items(ui, cmds, pane);
    });
    submenu(ui, Some(ph::ROWS), "Group by", |ui| {
        ui.set_width(200.0);
        group_items(ui, cmds, pane);
    });
    menu_separator(ui);
    action_item(ui, km, cmds, id, ph::ARROW_CLOCKWISE, "Refresh", Action::Refresh);
    soon_item(ui, km, cmds, ph::CLIPBOARD, "Paste", Action::Paste);
    soon_item(ui, km, cmds, ph::FOLDER_PLUS, "New folder", Action::NewFolder);
    soon_item(ui, km, cmds, ph::FILE_PLUS, "New file", Action::NewFile);
    menu_separator(ui);
    if let Location::Dir(dir) = &pane.location {
        action_item(ui, km, cmds, id, ph::TERMINAL_WINDOW, "Open terminal here", Action::OpenTerminalHere);
        if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), "Open external terminal here", None, true).clicked() {
            cmds.push(Cmd::TerminalAt(dir.clone()));
            ui.close();
        }
        action_item(
            ui,
            km,
            cmds,
            id,
            ph::ARROW_SQUARE_UP_RIGHT,
            "Reveal in system file manager",
            Action::RevealInSystem,
        );
        action_item(ui, km, cmds, id, ph::CLIPBOARD_TEXT, "Copy folder path", Action::CopyPath);
        if menu_item(ui, Some(ph::STAR), "Add to favorites", None, true).clicked() {
            cmds.push(Cmd::AddFavorite(dir.clone()));
            ui.close();
        }
        menu_separator(ui);
    }
    if matches!(pane.location, Location::Archive { .. }) {
        crate::ui::archive::context_items(ui, cmds, id, &pane.location, false, false);
        menu_separator(ui);
    }
    if menu_check(ui, v.show_hidden, "Show hidden files", chord_label(km, &Action::ToggleHidden).as_deref()).clicked() {
        cmds.push(Cmd::PaneAction(id, Action::ToggleHidden));
        ui.close();
    }
    if menu_check(ui, v.checkbox_mode, "Item checkboxes", None).clicked() {
        cmds.push(Cmd::PaneAction(id, Action::ToggleCheckboxes));
        ui.close();
    }
    action_item(ui, km, cmds, id, ph::SELECTION_ALL, "Select all", Action::SelectAll);
    action_item(ui, km, cmds, id, ph::CHART_PIE, "Calculate folder sizes", Action::FolderSize);
    menu_separator(ui);
    action_item(ui, km, cmds, id, ph::INFO, "Properties", Action::Properties);
}

pub const MODES: [(ViewMode, &str, &str, Action); 5] = [
    (ViewMode::Details, ph::LIST, "Details", Action::ViewDetails),
    (ViewMode::List, ph::LIST_DASHES, "List", Action::ViewList),
    (ViewMode::Grid, ph::SQUARES_FOUR, "Icons", Action::ViewGrid),
    (ViewMode::Gallery, ph::IMAGES, "Gallery", Action::ViewGallery),
    (ViewMode::Miller, ph::COLUMNS, "Columns", Action::ViewMiller),
];

pub fn view_mode_items(ui: &mut Ui, km: &Keymap, cmds: &mut Vec<Cmd>, pane: &Pane) {
    for (mode, icon, label, action) in MODES {
        let checked = pane.view.mode == mode;
        let sc = chord_label(km, &action);
        let r = menu_item(ui, Some(if checked { ph::CHECK } else { icon }), label, sc.as_deref(), true);
        if r.clicked() {
            cmds.push(Cmd::PaneAction(pane.id, action));
            ui.close();
        }
    }
}

const SORT_COLUMNS: [Column; 7] =
    [Column::Name, Column::Modified, Column::Size, Column::Kind, Column::Extension, Column::Created, Column::Accessed];

pub fn sort_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, pane: &Pane) {
    for col in SORT_COLUMNS {
        if menu_check(ui, pane.view.sort.column == col, col.label(), None).clicked() {
            if pane.view.sort.column != col {
                cmds.push(Cmd::PaneAction(pane.id, Action::SortBy(col)));
            }
            ui.close();
        }
    }
    menu_separator(ui);
    for (dir, label) in [(SortDir::Asc, "Ascending"), (SortDir::Desc, "Descending")] {
        if menu_check(ui, pane.view.sort.dir == dir, label, None).clicked() {
            if pane.view.sort.dir != dir {
                cmds.push(Cmd::PaneAction(pane.id, Action::ToggleSortDir));
            }
            ui.close();
        }
    }
}

pub fn group_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, pane: &Pane) {
    for (g, label) in GROUPS {
        if menu_check(ui, pane.view.group == g, label, None).clicked() {
            cmds.push(Cmd::PaneAction(pane.id, Action::SetGroupBy(g)));
            ui.close();
        }
    }
}

/// Column chooser (details header right-click).
pub fn column_menu(ui: &mut Ui, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    ui.set_width(220.0);
    menu_caption(ui, "Columns");
    let visible: Vec<Column> = pane.view.visible_columns().iter().map(|c| c.column).collect();
    for col in Column::ALL {
        if matches!(
            col,
            Column::Tags
                | Column::GitStatus
                | Column::Dimensions
                | Column::Duration
                | Column::Checksum
                | Column::ItemCount
                | Column::LinkTarget
        ) {
            continue; // data sources arrive in later milestones
        }
        let on = visible.contains(&col);
        let enabled = col != Column::Name;
        let r = crate::ui::widgets::menu_item(ui, Some(if on { ph::CHECK } else { "" }), col.label(), None, enabled);
        if r.clicked() {
            pane.view.set_column_visible(col, !on);
            pane.layout_sig = None;
            v.cmds.push(Cmd::ViewChanged(pane.id));
        }
    }
    menu_separator(ui);
    if menu_item(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), "Reset columns", None, true).clicked() {
        pane.view.columns = crate::state::view_memory::ViewState::from_config(v.config).columns;
        pane.layout_sig = None;
        v.cmds.push(Cmd::ViewChanged(pane.id));
        ui.close();
    }
}

/// "Open with" candidates, cached per extension for the menu's lifetime.
fn apps_for_cached(ui: &Ui, path: &std::path::Path) -> Vec<soyuz_fs::AppInfo> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let id = egui::Id::new(("soyuz.openwith", ext));
    if let Some(v) = ui.ctx().data(|d| d.get_temp::<Vec<soyuz_fs::AppInfo>>(id)) {
        return v;
    }
    let apps = soyuz_fs::launch::apps_for(path);
    ui.ctx().data_mut(|d| d.insert_temp(id, apps.clone()));
    apps
}

//! Workspace power-ups, GUI side: the menus of the tab strip, group chips, app menu and
//! layout button, the workspace dialogs, the history graph and the in-process windows'
//! viewports. The state, commands and rules are in `models::workspaces` /
//! `models::windows` (re-exported here).

pub mod dialogs;
pub mod history;
pub mod windows;

use egui::Ui;
use soyuz_core::Location;
use soyuz_core::config::Action;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::state::pane::PaneId;
use crate::state::workspace::{GroupId, PaneLayout, TabGroup};
use crate::state::workspace_windows::WindowId;
use crate::theme::icons::ph;
use crate::ui::widgets::{menu_item, menu_separator, submenu};

pub use crate::models::workspaces::{WsCmd, overlays_here, palette_shows, palette_title, store_snapshot};

/// A modal owns the keyboard / overlays (don't move the overlay owner then).
pub fn modal_open(app: &SoyuzApp) -> bool {
    app.dialog_open() || app.core.palette.open || app.gui.settings.is_open()
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

/// Group and window entries of a tab's context menu.
pub fn tab_menu_extra(ui: &mut Ui, app: &SoyuzApp, cmds: &mut Vec<Cmd>, i: usize) {
    let Some(tab) = app.core.ws.tabs.get(i) else { return };
    let ws = &app.core.ws;
    menu_separator(ui);
    if !tab.pinned {
        if menu_item(ui, Some(ph::RECTANGLE_DASHED), tr!("workspaces-menu-add-new-group"), None, true).clicked() {
            cmds.push(Cmd::Workspace(WsCmd::NewGroup(i)));
            ui.close();
        }
        let others: Vec<&TabGroup> = ws.groups.iter().filter(|g| Some(g.id) != tab.group).collect();
        if !others.is_empty() {
            submenu(ui, Some(ph::STACK), tr!("workspaces-menu-add-to-group"), |ui| {
                for g in others {
                    let label =
                        if g.name.is_empty() { tr!("workspaces-unnamed-group").to_string() } else { g.name.clone() };
                    if dot_item(ui, g.color, &label).clicked() {
                        cmds.push(Cmd::Workspace(WsCmd::AddToGroup(i, g.id)));
                        ui.close();
                    }
                }
            });
        }
        if tab.group.is_some()
            && menu_item(ui, Some(ph::RECTANGLE), tr!("workspaces-menu-remove-from-group"), None, true).clicked()
        {
            cmds.push(Cmd::Workspace(WsCmd::RemoveFromGroup(i)));
            ui.close();
        }
    }
    menu_separator(ui);
    let can_move = ws.tabs.len() > 1;
    if menu_item(ui, Some(ph::APP_WINDOW), tr!("workspaces-menu-move-new-window"), None, can_move).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::TabToNewWindow(i)));
        ui.close();
    }
    let targets = crate::models::windows::window_list(&app.core);
    let here = app.core.workspaces.current;
    let others: Vec<&(WindowId, String)> = targets.iter().filter(|(id, _)| *id != here).collect();
    if !others.is_empty() {
        submenu(ui, Some(ph::BROWSERS), tr!("workspaces-menu-move-to-window"), |ui| {
            for (id, title) in others {
                if menu_item(ui, Some(ph::APP_WINDOW), title, None, can_move).clicked() {
                    cmds.push(Cmd::Workspace(WsCmd::TabToWindow(i, *id)));
                    ui.close();
                }
            }
        });
    }
    let km = &app.core.keymap;
    let sc = crate::keys::chord_label(km, &Action::TabHistory);
    if menu_item(ui, Some(ph::GIT_FORK), tr!("workspaces-history-title"), sc.as_deref(), true).clicked() {
        cmds.push(Cmd::Action(Action::TabHistory));
        ui.close();
    }
}

/// A menu row with a colored dot.
pub fn dot_item(ui: &mut Ui, color: u8, label: &str) -> egui::Response {
    let p = crate::theme::palette(ui.ctx());
    let r = menu_item(ui, Some(""), label, None, true);
    let c = p.tags[(color as usize).min(7)];
    ui.painter().circle_filled(egui::pos2(r.rect.left() + 17.0, r.rect.center().y), 5.0, c);
    r
}

/// Display name of tag color `c` (group colors).
fn color_name(c: usize) -> &'static str {
    match c {
        0 => tr!("workspaces-color-red"),
        1 => tr!("workspaces-color-orange"),
        2 => tr!("workspaces-color-yellow"),
        3 => tr!("workspaces-color-green"),
        4 => tr!("workspaces-color-cyan"),
        5 => tr!("workspaces-color-blue"),
        6 => tr!("workspaces-color-purple"),
        _ => tr!("workspaces-color-gray"),
    }
}

/// Context menu of a group chip.
pub fn group_menu(ui: &mut Ui, app: &SoyuzApp, cmds: &mut Vec<Cmd>, g: GroupId) {
    ui.set_width(220.0);
    let Some(group) = app.core.ws.group(g) else { return };
    if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("workspaces-group-rename"), None, true).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::StartRenameGroup(g)));
        ui.close();
    }
    // Color swatches.
    let p = crate::theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        for (c, color) in p.tags.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(20.0, 22.0), egui::Sense::click());
            let center = r.center();
            ui.painter().circle_filled(center, 7.0, *color);
            if group.color as usize == c {
                ui.painter().circle_stroke(center, 9.5, egui::Stroke::new(1.5, p.text_primary));
            } else if resp.hovered() {
                ui.painter().circle_stroke(center, 9.0, egui::Stroke::new(1.0, p.border_strong));
            }
            let name = color_name(c);
            crate::ui::a11y::button(&resp, name);
            if resp.on_hover_text(name).clicked() {
                cmds.push(Cmd::Workspace(WsCmd::SetGroupColor(g, c as u8)));
                ui.close();
            }
        }
    });
    menu_separator(ui);
    let label = if group.collapsed { tr!("workspaces-group-expand") } else { tr!("workspaces-group-collapse") };
    let icon = if group.collapsed { ph::CARET_DOWN } else { ph::CARET_RIGHT };
    if menu_item(ui, Some(icon), label, None, true).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::ToggleGroup(g)));
        ui.close();
    }
    if menu_item(ui, Some(ph::PLUS), tr!("workspaces-group-new-tab"), None, true).clicked() {
        if let Some(span) = app.core.ws.group_span(g) {
            cmds.push(Cmd::NewTabAfter(span.end - 1));
        }
        ui.close();
    }
    if menu_item(ui, Some(ph::RECTANGLE), tr!("workspaces-group-ungroup"), None, true).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::Ungroup(g)));
        ui.close();
    }
    menu_separator(ui);
    let can_move = app.core.ws.group_tab_count(g) < app.core.ws.tabs.len();
    if menu_item(ui, Some(ph::APP_WINDOW), tr!("workspaces-group-to-new-window"), None, can_move).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::GroupToNewWindow(g)));
        ui.close();
    }
    if menu_item(ui, Some(ph::X), tr!("workspaces-group-close"), None, true).clicked() {
        cmds.push(Cmd::Workspace(WsCmd::CloseGroup(g)));
        ui.close();
    }
}

/// "Workspaces" submenu (app menu): open / save / manage.
pub fn workspaces_menu(ui: &mut Ui, app: &SoyuzApp, cmds: &mut Vec<Cmd>) {
    submenu(ui, Some(ph::STACK), tr!("workspaces-menu"), |ui| {
        ui.set_width(240.0);
        let km = &app.core.keymap;
        for (i, w) in app.core.workspaces.store.workspaces.iter().enumerate() {
            let sc = (i < 9).then(|| crate::keys::chord_label(km, &Action::OpenWorkspace(i as u8 + 1))).flatten();
            let current = app.core.workspaces.last.as_deref() == Some(w.name.as_str());
            let icon = if current { ph::CHECK } else { ph::STACK };
            if menu_item(ui, Some(icon), &w.name, sc.as_deref(), true).clicked() {
                cmds.push(Cmd::Workspace(WsCmd::Open(i)));
                ui.close();
            }
        }
        if !app.core.workspaces.store.workspaces.is_empty() {
            menu_separator(ui);
        }
        if menu_item(ui, Some(ph::FLOPPY_DISK), tr!("workspaces-menu-save-as"), None, true).clicked() {
            cmds.push(Cmd::Action(Action::SaveWorkspace));
            ui.close();
        }
        let has_last = app.core.workspaces.last.is_some();
        if menu_item(ui, Some(ph::ARROWS_CLOCKWISE), tr!("workspaces-menu-update"), None, has_last).clicked() {
            cmds.push(Cmd::Action(Action::UpdateWorkspace));
            ui.close();
        }
        if menu_item(ui, Some(ph::GEAR_SIX), tr!("workspaces-menu-manage"), None, true).clicked() {
            cmds.push(Cmd::Workspace(WsCmd::Manage));
            ui.close();
        }
    });
}

/// The Timeline row (sidebar, under Recent).
pub fn timeline_row(ui: &mut Ui, current: &Location, pane: PaneId, cmds: &mut Vec<Cmd>) {
    let active = matches!(current, Location::Timeline);
    let r = crate::ui::widgets::NavRow {
        icon: ph::CALENDAR_DOTS,
        label: tr!("workspaces-timeline"),
        active,
        icon_color: None,
        usage: None,
    }
    .show(ui);
    let r = r.on_hover_text(tr!("workspaces-timeline-tip"));
    if r.clicked() {
        cmds.push(Cmd::FocusPane(pane));
        cmds.push(Cmd::Navigate { pane, location: Location::Timeline, select: None });
    }
    if r.middle_clicked() {
        cmds.push(Cmd::OpenInNewTab { location: Location::Timeline, activate: false });
    }
}

/// Layout entries for the dual-pane button menu.
pub fn layout_menu(ui: &mut Ui, app: &SoyuzApp, cmds: &mut Vec<Cmd>) {
    use crate::ui::widgets::menu_check;
    use soyuz_core::config::Orientation;
    let cur = app.core.ws.tab().layout();
    for (layout, label) in [
        (PaneLayout::Single, tr!("workspaces-layout-single")),
        (PaneLayout::Dual(Orientation::Vertical), tr!("workspaces-layout-dual")),
        (PaneLayout::Dual(Orientation::Horizontal), tr!("workspaces-layout-stacked")),
        (PaneLayout::Quad, tr!("workspaces-layout-quad")),
    ] {
        let sc = match layout {
            PaneLayout::Quad => crate::keys::chord_label(&app.core.keymap, &Action::ToggleQuadPane),
            _ => None,
        };
        if menu_check(ui, cur == layout, label, sc.as_deref()).clicked() {
            cmds.push(Cmd::Workspace(WsCmd::SetLayout(layout)));
            ui.close();
        }
    }
}

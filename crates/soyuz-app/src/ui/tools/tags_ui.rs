//! Menus and sidebar pieces for the tools: the "Tag" submenu and tool entries of the
//! item / background context menus, the sidebar Tags section (drop files on a tag to
//! tag them) and the Recent row.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::{Align2, CornerRadius, DragAndDrop, FontId, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_core::Location;
use soyuz_core::config::{Action, Keymap};

use super::{ToolCmd, queue};
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::ops::fileops::dnd::{self, DragPayload};
use crate::services::tags::{self, TAG_COUNT, TAG_NAMES};
use crate::state::pane::PaneId;
use crate::theme::tokens::{radius, size};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::{NavRow, collapsible_header, menu_item, menu_separator, submenu, trunc_galley};

/// A menu row with a colored dot instead of an icon; `checked` draws a check mark.
fn tag_row(ui: &mut Ui, tag: u8, checked: bool, shortcut: Option<&str>) -> egui::Response {
    let p = palette(ui.ctx());
    let resp = menu_item(ui, None, TAG_NAMES[tag as usize], shortcut, true);
    let c = pos2(resp.rect.left() + 17.0, resp.rect.center().y);
    ui.painter().circle_filled(c, 5.5, p.tags[tag as usize]);
    if checked {
        ui.painter().text(c, Align2::CENTER_CENTER, ph::CHECK, icons::font(9.0), egui::Color32::WHITE);
    }
    resp
}

/// Extra rows for the items context menu (after the archive rows).
pub fn items_menu(
    ui: &mut Ui,
    km: &Keymap,
    cmds: &mut Vec<Cmd>,
    pane: PaneId,
    location: &Location,
    sel: &[(PathBuf, bool)],
) {
    if sel.is_empty() {
        return;
    }
    let paths: Vec<PathBuf> = sel.iter().map(|(p, _)| p.clone()).collect();
    menu_separator(ui);
    if matches!(location, Location::Recent) {
        recent_items_menu(ui, &paths);
    }
    let masks: Vec<u8> = paths.iter().map(|p| tags::tags_of(p)).collect();
    submenu(ui, Some(ph::TAG), "Tags", |ui| {
        ui.set_width(200.0);
        for t in 0..TAG_COUNT as u8 {
            let all = masks.iter().all(|m| m & (1 << t) != 0);
            let sc = chord_label(km, &Action::AddTag(t + 1));
            if tag_row(ui, t, all, sc.as_deref()).clicked() {
                queue(ToolCmd::ToggleTag { paths: paths.clone(), tag: t });
                ui.close();
            }
        }
        menu_separator(ui);
        let any = masks.iter().any(|m| *m != 0);
        if menu_item(ui, Some(ph::X_CIRCLE), "Clear tags", None, any).clicked() {
            queue(ToolCmd::ClearTags(paths.clone()));
            ui.close();
        }
    });
    if menu_item(ui, Some(ph::TRAY_ARROW_DOWN), "Add to shelf", None, true).clicked() {
        queue(ToolCmd::ShelfAdd(paths.clone()));
        ui.close();
    }
    let dirs: Vec<&PathBuf> = sel.iter().filter(|(_, d)| *d).map(|(p, _)| p).collect();
    let act = |ui: &mut Ui, cmds: &mut Vec<Cmd>, icon: &str, label: &str, a: Action| {
        let sc = chord_label(km, &a);
        if menu_item(ui, Some(icon), label, sc.as_deref(), true).clicked() {
            cmds.push(Cmd::PaneAction(pane, a));
            ui.close();
        }
    };
    if dirs.len() == 1 && sel.len() == 1 {
        act(ui, cmds, ph::CHART_PIE_SLICE, "Analyze disk usage", Action::DiskUsageTreemap);
    }
    if !dirs.is_empty() && dirs.len() == sel.len() {
        act(ui, cmds, ph::FILES, "Find duplicates", Action::FindDuplicates);
    }
    if dirs.len() == 2 && sel.len() == 2 {
        act(ui, cmds, ph::GIT_DIFF, "Compare folders", Action::CompareFolders);
    }
    if let [(p, false)] = sel {
        if crate::ui::tools::split::is_part(p) {
            act(ui, cmds, ph::ARROWS_MERGE, "Join parts…", Action::JoinFiles);
        } else {
            act(ui, cmds, ph::ARROWS_SPLIT, "Split file…", Action::SplitFile);
        }
    }
}

/// Extra rows for the background context menu of a folder.
pub fn background_menu(ui: &mut Ui, km: &Keymap, cmds: &mut Vec<Cmd>, pane: PaneId, location: &Location, dual: bool) {
    let act = |ui: &mut Ui, cmds: &mut Vec<Cmd>, icon: &str, label: &str, a: Action| {
        let sc = chord_label(km, &a);
        if menu_item(ui, Some(icon), label, sc.as_deref(), true).clicked() {
            cmds.push(Cmd::PaneAction(pane, a));
            ui.close();
        }
    };
    match location {
        Location::Dir(_) => {
            act(ui, cmds, ph::CHART_PIE_SLICE, "Analyze disk usage", Action::DiskUsageTreemap);
            act(ui, cmds, ph::FILES, "Find duplicates", Action::FindDuplicates);
            act(
                ui,
                cmds,
                ph::GIT_DIFF,
                if dual { "Compare with other pane" } else { "Compare folders…" },
                Action::CompareFolders,
            );
            menu_separator(ui);
        }
        Location::Recent => {
            if menu_item(ui, Some(ph::BROOM), "Clear recent items", None, true).clicked() {
                crate::services::recent::clear();
                cmds.push(Cmd::PaneAction(pane, Action::Refresh));
                ui.close();
            }
            menu_separator(ui);
        }
        _ => {}
    }
}

/// Rows for items shown in the Recent location.
pub fn recent_items_menu(ui: &mut Ui, paths: &[PathBuf]) {
    if menu_item(ui, Some(ph::CLOCK_COUNTER_CLOCKWISE), "Remove from Recent", None, true).clicked() {
        queue(ToolCmd::ForgetRecent(paths.to_vec()));
        ui.close();
    }
}

/// The sidebar Tags section: one row per color with its item count. Dropping files on
/// a row tags them.
pub fn sidebar_section(
    ui: &mut Ui,
    collapsed: &mut BTreeSet<String>,
    current: &Location,
    pane: PaneId,
    cmds: &mut Vec<Cmd>,
) {
    let name = "Tags";
    let mut open = !collapsed.contains(name);
    collapsible_header(ui, name, &mut open);
    if open {
        collapsed.remove(name);
    } else {
        collapsed.insert(name.to_string());
        return;
    }
    let p = palette(ui.ctx());
    let counts = tags::counts();
    for t in 0..TAG_COUNT as u8 {
        let slug = tags::slug(t);
        let active = matches!(current, Location::Tags(s) if s.eq_ignore_ascii_case(&slug));
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW_COZY), Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect.expand(3.0));
            let cr = CornerRadius::same(radius::ROW);
            let hover = ui.ctx().animate_bool_with_time(
                resp.id.with("hover"),
                resp.hovered(),
                crate::theme::anim(ui.ctx(), 0.10),
            );
            if active {
                painter.rect_filled(rect, cr, p.accent_subtle);
            } else if hover > 0.0 {
                painter.rect_filled(rect, cr, p.bg_surface2.gamma_multiply(hover));
            }
            let c = pos2(rect.left() + 18.0, rect.center().y);
            painter.circle_filled(c, 5.0, p.tags[t as usize]);
            if active {
                painter.circle_stroke(c, 7.5, Stroke::new(1.0, p.tags[t as usize].gamma_multiply(0.6)));
            }
            let label_color = if active { p.text_primary } else { p.text_primary.gamma_multiply(0.9) };
            let g = trunc_galley(
                &painter,
                TAG_NAMES[t as usize],
                FontId::proportional(13.0),
                label_color,
                rect.width() - 70.0,
            );
            painter.galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, label_color);
            let n = counts[t as usize];
            if n > 0 {
                painter.text(
                    pos2(rect.right() - 10.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    crate::ui::statusbar::group_thousands(n),
                    FontId::proportional(11.5),
                    p.text_tertiary,
                );
            }
        }
        if resp.clicked() {
            cmds.push(Cmd::FocusPane(pane));
            cmds.push(Cmd::Navigate { pane, location: Location::Tags(slug.clone()), select: None });
        }
        if resp.middle_clicked() {
            cmds.push(Cmd::OpenInNewTab { location: Location::Tags(slug.clone()), activate: false });
        }
        tag_drop_target(ui, &resp, t);
    }
}

/// Dropping dragged files on a tag row tags them.
fn tag_drop_target(ui: &Ui, resp: &egui::Response, tag: u8) {
    let ctx = ui.ctx();
    let Some(_) = dnd::payload(ctx) else { return };
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return };
    if !resp.rect.contains(pos) {
        return;
    }
    dnd::highlight(ui, resp.rect, radius::ROW);
    let p = palette(ctx);
    ui.painter().rect_stroke(
        resp.rect,
        CornerRadius::same(radius::ROW),
        Stroke::new(1.0, p.tags[tag as usize]),
        StrokeKind::Inside,
    );
    if ctx.input(|i| i.pointer.any_released())
        && let Some(payload) = DragAndDrop::take_payload::<DragPayload>(ctx)
    {
        let mut paths = payload.paths.clone();
        // Only add (never remove) when dropping.
        paths.retain(|p| tags::tags_of(p) & (1 << tag) == 0);
        if !paths.is_empty() {
            queue(ToolCmd::ToggleTag { paths, tag });
        }
    }
}

/// The Recent row (sidebar "Other" section).
pub fn recent_row(ui: &mut Ui, current: &Location, pane: PaneId, cmds: &mut Vec<Cmd>) {
    let active = matches!(current, Location::Recent);
    let r =
        NavRow { icon: ph::CLOCK_COUNTER_CLOCKWISE, label: "Recent", active, icon_color: None, usage: None }.show(ui);
    if r.clicked() {
        cmds.push(Cmd::FocusPane(pane));
        cmds.push(Cmd::Navigate { pane, location: Location::Recent, select: None });
    }
    if r.middle_clicked() {
        cmds.push(Cmd::OpenInNewTab { location: Location::Recent, activate: false });
    }
}

/// Whether `path` is shown by a virtual listing (Recent / Tags) and may be dragged.
pub fn is_virtual_listing(loc: &Location) -> bool {
    matches!(loc, Location::Recent | Location::Tags(_))
}

/// Display text for a path in tool windows (no verbatim prefix).
pub fn show_path(p: &Path) -> String {
    soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned()
}

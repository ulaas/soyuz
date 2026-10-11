//! Menus and sidebar pieces for the tools: the "Tag" submenu and tool entries of the
//! item / background context menus, the sidebar Tags section (drop files on a tag to
//! tag them) and the Recent row.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    Align2, CornerRadius, DragAndDrop, FontId, Key, Margin, Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2,
};
use soyuz_core::Location;
use soyuz_core::config::{Action, Keymap, TAG_NAME_MAX_CHARS};

use super::{ToolCmd, queue};
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::ops::fileops::dnd::{self, DragPayload};
use crate::services::tags::{self, TAG_COUNT};
use crate::state::pane::PaneId;
use crate::theme::tokens::{radius, size};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::{NavRow, collapsible_header, menu_item, menu_separator, submenu, trunc_galley};

/// Display name of a tag: the user's name for it (`[tags] names`), else the translated
/// color name (the English `TAG_NAMES` stay the slugs / config values).
pub fn tag_name(tag: u8) -> String {
    tags::name(tag)
}

/// The sidebar row being renamed in place.
struct Renaming {
    tag: u8,
    text: String,
    /// Select the whole name until the editor owns the focus.
    select_all: bool,
}

thread_local! {
    static RENAMING: RefCell<Option<Renaming>> = const { RefCell::new(None) };
}

/// Start renaming tag `tag` in its sidebar row.
pub fn begin_rename(tag: u8) {
    let text = tags::custom_name(tag).unwrap_or_else(|| tags::default_name(tag).to_string());
    RENAMING.with_borrow_mut(|r| *r = Some(Renaming { tag, text, select_all: true }));
}

/// The in-place name editor over a sidebar tag row (`rect` = the label area). Enter or
/// clicking away commits, Esc cancels.
fn rename_editor(ui: &mut Ui, rect: egui::Rect, tag: u8) {
    let Some(mut r) = RENAMING.with_borrow_mut(Option::take) else { return };
    let p = palette(ui.ctx());
    let id = egui::Id::new(("soyuz.tag_rename", tag));
    let has_focus = ui.memory(|m| m.has_focus(id));
    let (esc, enter) = if has_focus || r.select_all {
        ui.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)))
    } else {
        (false, false)
    };
    if r.select_all {
        let n = r.text.chars().count();
        let mut st = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(n))));
        st.store(ui.ctx(), id);
    }
    // The editor takes the whole row height (so the rows below keep their place); the
    // box is drawn slightly inset.
    let frame = rect.shrink2(vec2(0.0, 3.0)).expand(1.0);
    ui.painter().rect_filled(frame, CornerRadius::same(radius::CONTROL), p.bg_surface1);
    let te = TextEdit::singleline(&mut r.text)
        .id(id)
        .font(FontId::proportional(13.0))
        .hint_text(tags::default_name(tag))
        .char_limit(TAG_NAME_MAX_CHARS)
        .frame(egui::Frame::NONE)
        .vertical_align(egui::Align::Center)
        .margin(Margin::symmetric(8, 2))
        .desired_width(rect.width() - 16.0);
    let out = ui.put(rect, te);
    crate::ui::a11y::name(&out, tr!("tools-tags-rename"));
    if r.select_all {
        if has_focus {
            r.select_all = false;
        } else {
            out.request_focus();
        }
    }
    ui.painter().rect_stroke(
        frame,
        CornerRadius::same(radius::CONTROL),
        Stroke::new(1.5, p.accent),
        StrokeKind::Outside,
    );
    if esc {
        return;
    }
    if (enter && has_focus) || out.lost_focus() {
        if tags::resolve_name(&r.text, tags::default_name(tag)) != tags::name(tag) {
            queue(ToolCmd::RenameTag { tag, name: r.text });
        }
        return;
    }
    RENAMING.with_borrow_mut(|slot| *slot = Some(r));
}

/// A menu row with a colored dot instead of an icon; `checked` draws a check mark.
fn tag_row(ui: &mut Ui, tag: u8, checked: bool, shortcut: Option<&str>) -> egui::Response {
    let p = palette(ui.ctx());
    let resp = menu_item(ui, None, &tag_name(tag), shortcut, true);
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
    submenu(ui, Some(ph::TAG), tr!("tools-tags-menu"), |ui| {
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
        if menu_item(ui, Some(ph::X_CIRCLE), tr!("tools-tags-clear"), None, any).clicked() {
            queue(ToolCmd::ClearTags(paths.clone()));
            ui.close();
        }
    });
    if menu_item(ui, Some(ph::TRAY_ARROW_DOWN), tr!("tools-menu-add-to-shelf"), None, true).clicked() {
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
        act(ui, cmds, ph::CHART_PIE_SLICE, tr!("tools-menu-analyze-disk-usage"), Action::DiskUsageTreemap);
    }
    if !dirs.is_empty() && dirs.len() == sel.len() {
        act(ui, cmds, ph::FILES, tr!("tools-menu-find-duplicates"), Action::FindDuplicates);
    }
    if dirs.len() == 2 && sel.len() == 2 {
        act(ui, cmds, ph::GIT_DIFF, tr!("tools-menu-compare-folders"), Action::CompareFolders);
    }
    if let [(p, false)] = sel {
        if crate::ui::tools::split::is_part(p) {
            act(ui, cmds, ph::ARROWS_MERGE, tr!("tools-menu-join-parts"), Action::JoinFiles);
        } else {
            act(ui, cmds, ph::ARROWS_SPLIT, tr!("tools-menu-split-file"), Action::SplitFile);
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
            act(ui, cmds, ph::CHART_PIE_SLICE, tr!("tools-menu-analyze-disk-usage"), Action::DiskUsageTreemap);
            act(ui, cmds, ph::FILES, tr!("tools-menu-find-duplicates"), Action::FindDuplicates);
            act(
                ui,
                cmds,
                ph::GIT_DIFF,
                if dual { tr!("tools-menu-compare-other-pane") } else { tr!("tools-menu-compare-folders-dialog") },
                Action::CompareFolders,
            );
            menu_separator(ui);
        }
        Location::Recent => {
            if menu_item(ui, Some(ph::BROOM), tr!("tools-menu-clear-recent"), None, true).clicked() {
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
    if menu_item(ui, Some(ph::CLOCK_COUNTER_CLOCKWISE), tr!("tools-menu-remove-from-recent"), None, true).clicked() {
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
    // The collapsed-state key stays English (persisted in the session).
    let name = "Tags";
    let mut open = !collapsed.contains(name);
    collapsible_header(ui, tr!("tools-tags-section"), &mut open);
    if open {
        collapsed.remove(name);
    } else {
        collapsed.insert(name.to_string());
        return;
    }
    let p = palette(ui.ctx());
    let counts = tags::counts();
    let editing = RENAMING.with_borrow(|r| r.as_ref().map(|r| r.tag));
    for t in 0..TAG_COUNT as u8 {
        let slug = tags::slug(t);
        let active = matches!(current, Location::Tags(s) if s.eq_ignore_ascii_case(&slug));
        let renaming = editing == Some(t);
        let sense = if renaming { Sense::hover() } else { Sense::click() };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), size::ROW_COZY), sense);
        let name = tag_name(t);
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
            if !renaming {
                let g = trunc_galley(&painter, &name, FontId::proportional(13.0), label_color, rect.width() - 70.0);
                painter.galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, label_color);
            }
            let n = counts[t as usize];
            if n > 0 && !renaming {
                painter.text(
                    pos2(rect.right() - 10.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    crate::ui::statusbar::group_thousands(n),
                    FontId::proportional(11.5),
                    p.text_tertiary,
                );
            }
        }
        if renaming {
            let edit = egui::Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), rect.max - vec2(6.0, 0.0));
            rename_editor(ui, edit, t);
            continue;
        }
        let resp = match tags::custom_name(t) {
            Some(_) => resp.on_hover_text(tr!("tools-tags-color-tip", color = tags::default_name(t))),
            None => resp,
        };
        if resp.clicked() {
            cmds.push(Cmd::FocusPane(pane));
            cmds.push(Cmd::Navigate { pane, location: Location::Tags(slug.clone()), select: None });
        }
        if resp.middle_clicked() {
            cmds.push(Cmd::OpenInNewTab { location: Location::Tags(slug.clone()), activate: false });
        }
        crate::ui::native::popup::context_menu(&resp, |ui| tag_row_menu(ui, cmds, t, &slug));
        tag_drop_target(ui, &resp, t);
    }
}

/// Right-click menu of a sidebar tag row.
fn tag_row_menu(ui: &mut Ui, cmds: &mut Vec<Cmd>, tag: u8, slug: &str) {
    ui.set_width(220.0);
    if menu_item(ui, Some(ph::BROWSERS), tr!("app-ctx-open-in-new-tab"), None, true).clicked() {
        cmds.push(Cmd::OpenInNewTab { location: Location::Tags(slug.to_string()), activate: true });
        ui.close();
    }
    if menu_item(ui, Some(ph::SQUARE_SPLIT_HORIZONTAL), tr!("app-ctx-open-in-other-pane"), None, true).clicked() {
        cmds.push(Cmd::OpenInOtherPane(Location::Tags(slug.to_string())));
        ui.close();
    }
    menu_separator(ui);
    if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("tools-tags-rename"), None, true).clicked() {
        begin_rename(tag);
        ui.close();
    }
    if tags::custom_name(tag).is_some()
        && menu_item(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("tools-tags-reset-name"), None, true).clicked()
    {
        queue(ToolCmd::RenameTag { tag, name: String::new() });
        ui.close();
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
        NavRow { icon: ph::CLOCK_COUNTER_CLOCKWISE, label: tr!("tools-recent"), active, icon_color: None, usage: None }
            .show(ui);
    if r.clicked() {
        cmds.push(Cmd::FocusPane(pane));
        cmds.push(Cmd::Navigate { pane, location: Location::Recent, select: None });
    }
    if r.middle_clicked() {
        cmds.push(Cmd::OpenInNewTab { location: Location::Recent, activate: false });
    }
}

pub use crate::models::tools::is_virtual_listing;

/// Display text for a path in tool windows (no verbatim prefix).
pub fn show_path(p: &Path) -> String {
    soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned()
}

//! Folder tree sidebar (drawing and egui input): a lazily loaded tree of the drives and
//! the home folder. The state, the background listing, follow-the-pane, filtering and
//! keyboard logic live in [`crate::models::tree`]; this module draws the rows, filters as
//! you type, expands on drag-hover and accepts drops (same effect rules and jobs as every
//! other drop target), and feeds keyboard input (arrows, Left/Right collapse/expand,
//! Home/End, PageUp/PageDown, type-ahead) to the model.
//!
//! Where it appears is `[sidebar] mode`: `places` (no tree), `tree` (the tree fills the
//! sidebar) or `both` (the tree is the `tree` section among the places sections).

pub use crate::models::tree::model;
pub use crate::models::tree::{TreeArgs, TreeCmd, TreeIo, TreeState};

use std::path::Path;
use std::time::Duration;

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Rect, Response, Sense, Stroke, TextEdit, Ui,
    pos2, vec2,
};
use soyuz_core::Location;
use soyuz_core::config::DragDefault;
use soyuz_fs::VolumeInfo;

use crate::cmd::Cmd;
use crate::models::tree::KeyInput;
use crate::ops::fileops::dnd::{self, DragPayload};
use crate::theme::tokens::{alpha, radius, size};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::{focus_ring, menu_item, menu_separator};
use model::{Nav, RootKind};

/// The filter box's widget id.
fn filter_id() -> Id {
    Id::new("soyuz.tree.filter")
}

/// Indentation per tree level (compressed down to `INDENT_MIN` for deep trees so
/// labels keep at least `LABEL_MIN` points).
const INDENT: f32 = 12.0;
const INDENT_MIN: f32 = 5.0;
const LABEL_MIN: f32 = 110.0;

/// Indentation for a view whose deepest visible row has `max_depth`.
pub fn indent_for(width: f32, max_depth: usize) -> f32 {
    if max_depth == 0 {
        return INDENT;
    }
    ((width - 48.0 - LABEL_MIN) / max_depth as f32).clamp(INDENT_MIN, INDENT)
}

/// The filter box ("Filter folders"), with a clear button while it has text.
pub fn filter_box(ui: &mut Ui, s: &mut TreeState) {
    let p = palette(ui.ctx());
    let id = filter_id();
    ui.add_space(2.0);
    let h = 26.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let rect = rect.shrink2(vec2(4.0, 0.0));
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(radius::ROW), p.bg_surface2);
    if focused {
        painter.rect_stroke(
            rect,
            CornerRadius::same(radius::ROW),
            Stroke::new(1.0, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    painter.text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        ph::FUNNEL_SIMPLE,
        icons::font(13.0),
        if s.filter.is_empty() { p.text_tertiary } else { p.accent },
    );
    let clear_w = if s.filter.is_empty() { 0.0 } else { 22.0 };
    let text_rect =
        Rect::from_min_max(pos2(rect.left() + 26.0, rect.top()), pos2(rect.right() - clear_w - 4.0, rect.bottom()));
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(text_rect).layout(egui::Layout::left_to_right(Align::Center)));
    let edit = TextEdit::singleline(&mut s.filter)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text(egui::RichText::new(tr!("views-tree-filter")).color(p.text_tertiary))
        .font(FontId::proportional(12.5))
        .desired_width(text_rect.width())
        .margin(egui::Margin::ZERO);
    let resp = child.add(edit);
    crate::ui::a11y::with(ui.ctx(), resp.id, |n| n.set_label(tr!("views-tree-filter")));
    if !s.filter.is_empty() {
        let cr = Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), vec2(18.0, 18.0));
        let c = ui.interact(cr, id.with("clear"), Sense::click());
        crate::ui::a11y::button(&c, tr!("views-tree-clear-filter"));
        if c.hovered() {
            ui.painter().rect_filled(cr, CornerRadius::same(radius::CONTROL), p.bg_surface3);
        }
        ui.painter().text(cr.center(), Align2::CENTER_CENTER, ph::X, icons::font(11.0), p.text_secondary);
        if c.clicked() {
            s.filter.clear();
        }
    }
    ui.add_space(4.0);
}

/// Draw the rows (virtualized: only the visible ones are laid out) and handle input.
pub fn show(ui: &mut Ui, a: &mut TreeArgs<'_>, io: &mut TreeIo<'_>) {
    let ctx = ui.ctx().clone();
    crate::models::tree::pump(&ctx, a, io, crate::ui::views::computer::volume_label);
    let filter_focused = ctx.memory(|m| m.has_focus(filter_id()));
    let h = (size::ROW_COMPACT + 2.0).max(ui.spacing().interact_size.y + 2.0);
    if a.keyboard || filter_focused {
        keyboard(&ctx, a, io, filter_focused, h);
    }
    let s = &mut *a.state;
    let n = s.rows().len();
    let width = ui.available_width();
    let (all, _) = ui.allocate_exact_size(vec2(width, (n as f32 * h).max(h)), Sense::hover());
    let container = Id::new("soyuz.tree");
    crate::ui::a11y::container(&ctx, container, Role::Tree, tr!("views-tree-a11y"));
    if n == 0 {
        let p = palette(&ctx);
        let msg = if s.filter.trim().is_empty() { tr!("common-loading") } else { tr!("views-tree-no-matches") };
        ui.painter().text(
            pos2(all.left() + 12.0, all.center().y),
            Align2::LEFT_CENTER,
            msg,
            FontId::proportional(12.0),
            p.text_tertiary,
        );
        return;
    }
    let selected_idx = s.selected.as_ref().and_then(|k| s.row_index(k));
    if s.take_scroll()
        && let Some(i) = selected_idx
    {
        let r = Rect::from_min_size(pos2(all.left(), all.top() + i as f32 * h), vec2(width, h));
        ui.scroll_to_rect(r, Some(Align::Center));
    }
    let clip = ui.clip_rect();
    let first = (((clip.top() - all.top()) / h).floor().max(0.0) as usize).min(n);
    let last = (((clip.bottom() - all.top()) / h).ceil().max(0.0) as usize).min(n);
    let current = match io.current {
        Location::Dir(d) => Some(d.clone()),
        _ => None,
    };
    let mut hovering_drop = false;
    let max_depth = s.rows()[first..last].iter().map(|r| r.depth).max().unwrap_or(0);
    let target = indent_for(width, max_depth);
    // Ease the indentation change when deeper rows scroll in or out.
    let indent = ctx.animate_value_with_time(container.with("indent"), target, crate::theme::anim(&ctx, 0.15));
    for i in first..last {
        let rect = Rect::from_min_size(pos2(all.left(), all.top() + i as f32 * h), vec2(width, h));
        let row = s.rows()[i].clone();
        let is_current = current.as_deref().is_some_and(|c| c == row.key.path);
        let is_selected = selected_idx == Some(i);
        let cursor = is_selected && (a.keyboard || filter_focused);
        let resp = row_ui(ui, rect, &row, (is_current, cursor), indent, s, io.volumes);
        // Chevron hit area: toggles without navigating.
        let on_chevron =
            resp.interact_pointer_pos().is_some_and(|pos| pos.x < rect.left() + 8.0 + row.depth as f32 * indent + 18.0);
        if resp.clicked() {
            if on_chevron && row.expandable {
                s.model.toggle(&row.key.path);
            } else {
                s.open_row(row.key.clone(), io.pane, io.cmds);
            }
            io.cmds.push(Cmd::Tree(TreeCmd::Focus));
        }
        if resp.double_clicked() && !on_chevron && row.expandable {
            s.model.toggle(&row.key.path);
        }
        if resp.middle_clicked() {
            io.cmds.push(Cmd::OpenInNewTab { location: Location::Dir(row.key.path.clone()), activate: false });
        }
        if resp.drag_started() && row.depth > 0 {
            start_drag(&ctx, &row.key.path, a.drag_default);
        }
        // Drop target; hovering a collapsed folder expands it after a delay.
        if dnd::target(ui, &resp, &row.key.path, io.cmds) {
            hovering_drop = true;
            if row.expandable
                && !row.expanded
                && let Some(wait) = s.drag_hover(&row.key.path, a.drag_expand)
            {
                ctx.request_repaint_after(wait);
            }
        }
        let path = row.key.path.clone();
        let is_root = row.depth == 0;
        let expanded = row.expanded;
        let cmds = &mut *io.cmds;
        let mut toggle = false;
        crate::ui::native::popup::context_menu(&resp, |ui| {
            ui.set_width(230.0);
            if menu_item(ui, Some(ph::FOLDER_OPEN), tr!("views-tree-open"), None, true).clicked() {
                cmds.push(Cmd::Navigate { pane: io.pane, location: Location::Dir(path.clone()), select: None });
                ui.close();
            }
            if menu_item(ui, Some(ph::BROWSERS), tr!("views-tree-open-new-tab"), None, true).clicked() {
                cmds.push(Cmd::OpenInNewTab { location: Location::Dir(path.clone()), activate: true });
                ui.close();
            }
            if menu_item(ui, Some(ph::SQUARE_SPLIT_HORIZONTAL), tr!("views-tree-open-other-pane"), None, true).clicked()
            {
                cmds.push(Cmd::OpenInOtherPane(Location::Dir(path.clone())));
                ui.close();
            }
            if row.expandable
                && menu_item(
                    ui,
                    Some(ph::CARET_DOWN),
                    if expanded { tr!("views-tree-collapse") } else { tr!("views-tree-expand") },
                    None,
                    true,
                )
                .clicked()
            {
                toggle = true;
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::FOLDER_PLUS), tr!("views-tree-new-folder"), None, true).clicked() {
                cmds.push(Cmd::Tree(TreeCmd::NewFolder(path.clone())));
                ui.close();
            }
            if menu_item(ui, Some(ph::CLIPBOARD_TEXT), tr!("views-tree-copy-path"), None, true).clicked() {
                cmds.push(Cmd::CopyText(soyuz_fs::strip_verbatim(&path).to_string_lossy().into_owned()));
                ui.close();
            }
            if !is_root && menu_item(ui, Some(ph::STAR), tr!("views-tree-add-favorite"), None, true).clicked() {
                cmds.push(Cmd::AddFavorite(path.clone()));
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::INFO), tr!("views-tree-properties"), None, true).clicked() {
                cmds.push(Cmd::Tree(TreeCmd::Properties(path.clone())));
                ui.close();
            }
        });
        if toggle {
            s.model.toggle(&row.key.path);
        }
    }
    if !hovering_drop && dnd::payload(&ctx).is_none() {
        s.drag_left();
    }
}

/// Start an internal drag of a tree folder (moves / copies like any dragged item).
fn start_drag(ctx: &egui::Context, path: &Path, default: DragDefault) {
    egui::DragAndDrop::set_payload(ctx, DragPayload { paths: vec![path.to_path_buf()], source_pane: None, default });
}

fn row_ui(
    ui: &mut Ui,
    rect: Rect,
    row: &model::Row,
    (current, cursor): (bool, bool),
    indent: f32,
    s: &TreeState,
    volumes: &[VolumeInfo],
) -> Response {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = Id::new(("soyuz.tree.row", row.key.root, &row.key.path));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    crate::ui::a11y::with(&ctx, resp.id, |n| {
        n.set_role(Role::TreeItem);
        n.set_label(row.name.as_str());
        n.set_level(row.depth + 1);
        n.set_selected(current || cursor);
        if row.expandable {
            n.set_expanded(row.expanded);
        }
    });
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let painter = ui.painter_at(rect.expand(3.0));
    let rounding = CornerRadius::same(radius::ROW);
    let hover = ctx.animate_bool_with_time(id.with("hover"), resp.hovered(), crate::theme::anim(&ctx, 0.10));
    if current {
        painter.rect_filled(rect, rounding, p.accent_subtle);
    } else if cursor {
        painter.rect_filled(rect, rounding, p.bg_surface2);
    } else if hover > 0.0 {
        painter.rect_filled(rect, rounding, p.bg_surface2.gamma_multiply(hover));
    }
    if cursor {
        focus_ring(ui, rect.shrink(1.0));
    }
    let x0 = rect.left() + 8.0 + row.depth as f32 * indent;
    let cy = rect.center().y;
    // Indent guides for nested levels.
    for d in 1..=row.depth {
        let gx = rect.left() + 8.0 + (d as f32 - 1.0) * indent + 7.0;
        painter.line_segment(
            [pos2(gx, rect.top()), pos2(gx, rect.bottom())],
            Stroke::new(1.0, p.border_subtle.gamma_multiply(0.55)),
        );
    }
    // Chevron (rotates with the expand animation) or a spinner while listing.
    let cc = pos2(x0 + 7.0, cy);
    if row.loading && row.expanded {
        spinner(&painter, cc, 4.5, p.text_tertiary, ctx.input(|i| i.time) as f32);
        ctx.request_repaint_after(Duration::from_millis(33));
    } else if row.expandable {
        let t = ctx.animate_bool_with_time(id.with("open"), row.expanded, crate::theme::anim(&ctx, 0.14));
        let color = if hover > 0.5 { p.text_secondary } else { p.text_tertiary };
        chevron(&painter, cc, 3.5, t * std::f32::consts::FRAC_PI_2, color);
    }
    // Icon.
    let (glyph, fill) = match (&row.depth, s.root_kind(row)) {
        (0, Some(RootKind::Home)) => (ph::HOUSE, current),
        (0, Some(RootKind::Volume(i))) => {
            (volumes.get(i).map_or(ph::HARD_DRIVE, crate::ui::views::computer::volume_glyph), current)
        }
        _ if row.expanded && row.expandable => (ph::FOLDER_OPEN, current),
        _ => (ph::FOLDER_SIMPLE, current),
    };
    let icon_color = if row.depth == 0 { if current { p.accent } else { p.text_secondary } } else { p.glyph_folder };
    let icon_color = if row.hidden { icon_color.gamma_multiply(alpha::HIDDEN) } else { icon_color };
    let font = if fill { icons::font_fill(15.0) } else { icons::font(15.0) };
    painter.text(pos2(x0 + 18.0, cy), Align2::LEFT_CENTER, glyph, font, icon_color);
    // Label (filter matches in the accent color).
    let mut color = if current { p.text_primary } else { p.text_primary.gamma_multiply(0.9) };
    if row.hidden {
        color = color.gamma_multiply(alpha::HIDDEN);
    }
    if row.failed {
        color = p.text_tertiary;
    }
    let lx = x0 + 40.0;
    let max_w = (rect.right() - lx - if row.failed { 24.0 } else { 6.0 }).max(8.0);
    let font = FontId::proportional(12.5);
    let galley = match row.matched {
        Some((a, b)) if b > a && b <= row.name.len() => {
            let mut job = LayoutJob { break_on_newline: false, ..Default::default() };
            let base = TextFormat::simple(font.clone(), color);
            let hit = TextFormat { color: p.accent, underline: Stroke::new(1.0, p.accent), ..base.clone() };
            job.append(&row.name[..a], 0.0, base.clone());
            job.append(&row.name[a..b], 0.0, hit);
            job.append(&row.name[b..], 0.0, base);
            job.wrap = TextWrapping::truncate_at_width(max_w);
            painter.layout_job(job)
        }
        _ => crate::ui::widgets::trunc_galley(&painter, &row.name, font, color, max_w),
    };
    painter.galley(pos2(lx, cy - galley.size().y / 2.0), galley, color);
    if row.failed {
        painter.text(
            pos2(rect.right() - 10.0, cy),
            Align2::RIGHT_CENTER,
            ph::WARNING_CIRCLE,
            icons::font(12.0),
            p.warning,
        );
    }
    let tip = match s.model.node(&row.key.path).map(|n| &n.load) {
        Some(model::Load::Failed(e)) => format!("{}\n{e}", soyuz_fs::strip_verbatim(&row.key.path).display()),
        _ => soyuz_fs::strip_verbatim(&row.key.path).display().to_string(),
    };
    resp.on_hover_text_at_pointer(tip)
}

/// A chevron pointing right, rotated by `angle` (0 → right, π/2 → down).
fn chevron(painter: &egui::Painter, c: egui::Pos2, r: f32, angle: f32, color: Color32) {
    let rot = |x: f32, y: f32| {
        let (s, co) = angle.sin_cos();
        pos2(c.x + x * co - y * s, c.y + x * s + y * co)
    };
    let pts = [rot(-r * 0.5, -r), rot(r * 0.5, 0.0), rot(-r * 0.5, r)];
    let stroke = Stroke::new(1.4, color);
    painter.line_segment([pts[0], pts[1]], stroke);
    painter.line_segment([pts[1], pts[2]], stroke);
}

fn spinner(painter: &egui::Painter, c: egui::Pos2, r: f32, color: Color32, time: f32) {
    let start = time * 6.0;
    let pts: Vec<egui::Pos2> = (0..=16)
        .map(|i| {
            let a = start + i as f32 / 16.0 * std::f32::consts::PI * 1.4;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.4, color)));
}

/// Arrow keys, Home/End, PageUp/PageDown, Enter, Escape and type-ahead: collected
/// here, carried out by [`crate::models::tree::keyboard`].
fn keyboard(ctx: &egui::Context, a: &mut TreeArgs<'_>, io: &mut TreeIo<'_>, filter_focused: bool, h: f32) {
    let page = ((ctx.content_rect().height() * 0.6) / h).max(1.0) as usize;
    let mut k = KeyInput { filter_focused, ..KeyInput::default() };
    ctx.input_mut(|i| {
        let mut take = |key: Key| i.consume_key(Modifiers::NONE, key);
        for (key, n) in [(Key::ArrowUp, Nav::Up), (Key::ArrowDown, Nav::Down)] {
            if take(key) {
                k.steps.push(n);
            }
        }
        if !filter_focused {
            for (key, n) in [
                (Key::ArrowLeft, Nav::Left),
                (Key::ArrowRight, Nav::Right),
                (Key::Home, Nav::Home),
                (Key::End, Nav::End),
                (Key::PageUp, Nav::PageUp(page)),
                (Key::PageDown, Nav::PageDown(page)),
            ] {
                if take(key) {
                    k.steps.push(n);
                }
            }
        }
        k.enter = take(Key::Enter);
        k.escape = take(Key::Escape);
    });
    if !filter_focused {
        k.typed = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) if !i.modifiers.ctrl && !i.modifiers.alt && !i.modifiers.command => {
                        Some(t.clone())
                    }
                    _ => None,
                })
                .collect()
        });
    }
    if crate::models::tree::keyboard(a.state, k, io) {
        ctx.memory_mut(|m| m.surrender_focus(filter_id()));
    }
}

/// `TreeCmd::Properties` (through `FrontendCmd::Properties`): the properties dialog of
/// a tree folder.
pub fn properties(app: &mut crate::app::SoyuzApp, dir: &Path) {
    match soyuz_fs::stat(dir, false) {
        Ok(e) => {
            let pane = app.ws.pane().id;
            let size_style = app.config.general.size_style;
            let d = crate::ui::dialogs::properties::PropertiesDialog::new(
                pane,
                vec![e],
                size_style,
                crate::ui::dialogs::properties::Tab::General,
                None,
            )
            .naming(app);
            app.ui.dialogs.push(d);
        }
        Err(e) => {
            app.ui.toasts.push(crate::models::toasts::Toast::error(tr!("views-tree-read-failed")).body(e.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_trees_compress_their_indentation() {
        assert_eq!(indent_for(220.0, 0), INDENT);
        assert_eq!(indent_for(400.0, 3), INDENT, "shallow: full indent");
        let i = indent_for(220.0, 10);
        assert!((INDENT_MIN..INDENT).contains(&i));
        assert!(220.0 - 48.0 - i * 10.0 >= LABEL_MIN - 0.01, "labels keep their minimum width");
        assert_eq!(indent_for(200.0, 40), INDENT_MIN, "never below the minimum");
    }
}

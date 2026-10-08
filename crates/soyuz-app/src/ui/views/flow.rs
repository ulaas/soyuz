//! The flow view: details (table), grid/gallery (icons + thumbnails), list (compact,
//! multi-column) and Miller's current column. Rows are painted directly (no widget per
//! cell) and a single interaction surface handles clicks, double-clicks, the context
//! menu, rubber-band selection with auto-scroll and middle-click.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Id, Painter, PointerButton, Pos2, Rect, Response,
    ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};
use soyuz_core::config::{Action, MiddleClick};
use soyuz_core::{Column, EntryInfo, SortDir};

use super::ViewCtx;
use super::columns::{self, ColGeom};
use super::icons;
use super::layout::{FlowLayout, LineKind};
use crate::cmd::Cmd;
use crate::services::thumbs::{self, ThumbKey};
use crate::state::pane::{Band, ContextTarget, LayoutSig, Pane};
use crate::state::selection::{BandMode, ClickMods, Selection};
use crate::theme::fonts::SEMIBOLD;
use crate::theme::tokens::{alpha, radius};
use crate::theme::{Palette, icons as ticons, icons::ph, palette};
use crate::ui::context_menu;
use crate::ui::widgets::{trunc_galley, wrapped_galley};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Details,
    Grid,
    Gallery,
    List,
    /// Miller columns: the pane's own (current) column.
    Column,
}

impl Style {
    fn code(self) -> u8 {
        self as u8
    }
}

const PAD_X: f32 = 6.0;
const HEADER_H: f32 = 30.0;
const GROUP_H: f32 = 32.0;
const LIST_CELL_W: f32 = 250.0;
const CHECK: f32 = 16.0;

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Selection fill for focused / unfocused panes.
pub fn selection_fill(p: &Palette, focused: bool) -> Color32 {
    if focused {
        p.accent_subtle
    } else if p.dark {
        p.bg_surface3
    } else {
        p.bg_sunken
    }
}

struct Geometry {
    cols: Vec<ColGeom>,
    content_w: f32,
    cell: Vec2,
    gap: Vec2,
    pad: Vec2,
    header_h: f32,
}

fn geometry(style: Style, pane: &Pane, avail_w: f32, row_h: f32) -> Geometry {
    let zoom = pane.view.zoom as f32;
    let mut cols = Vec::new();
    let mut content_w = avail_w;
    let (cell, gap, pad) = match style {
        Style::Details => {
            let (c, w) = columns::geometry(&pane.view.visible_columns(), avail_w, PAD_X);
            cols = c;
            content_w = w;
            (vec2(w - 2.0 * PAD_X, row_h), Vec2::ZERO, vec2(PAD_X, 4.0))
        }
        Style::Grid | Style::Gallery => {
            let (base, gap, pad) = if style == Style::Grid {
                (vec2((zoom + 30.0).max(92.0), zoom + 18.0 + 36.0), vec2(4.0, 6.0), vec2(12.0, 10.0))
            } else {
                (vec2(zoom + 24.0, zoom + 40.0), vec2(10.0, 10.0), vec2(16.0, 12.0))
            };
            // Stretch cells so each row fills the width evenly (bounded).
            let usable = avail_w - 2.0 * pad.x;
            let n = (((usable + gap.x) / (base.x + gap.x)).floor()).max(1.0);
            let stretched = ((usable - (n - 1.0) * gap.x) / n - 0.01).clamp(base.x.min(usable), base.x * 1.45);
            (vec2(stretched, base.y), gap, pad)
        }
        Style::List => (vec2(LIST_CELL_W, row_h), vec2(6.0, 0.0), vec2(8.0, 6.0)),
        Style::Column => (vec2((avail_w - 8.0).max(60.0), 26.0), Vec2::ZERO, vec2(4.0, 4.0)),
    };
    let header_h = if style == Style::Details { HEADER_H } else { 0.0 };
    Geometry { cols, content_w, cell, gap, pad, header_h }
}

/// Draw the pane's listing into `rect`.
pub fn show(ui: &mut Ui, rect: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>, style: Style) {
    let row_h = v.config.appearance.density.row_height();
    let g = geometry(style, pane, rect.width(), row_h);
    let sig = LayoutSig { vis_version: pane.vis_version, width: rect.width(), cell: g.cell, mode: style.code() };
    if pane.layout_sig != Some(sig) || pane.layout.is_none() {
        pane.layout =
            Some(FlowLayout::new(pane.visible.len(), &pane.groups, g.content_w, g.cell, g.gap, g.pad, GROUP_H));
        pane.layout_sig = Some(sig);
    }
    let Some(layout) = pane.layout.take() else { return };

    let mut scroll = ScrollArea::new([style == Style::Details, true])
        .id_salt(("soyuz.flow", pane.id.0, style.code()))
        .auto_shrink([false, false]);
    if let Some(y) = pane.restore_scroll.take() {
        scroll = scroll.vertical_scroll_offset(y.max(0.0));
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    let out = scroll.show_viewport(&mut child, |ui, viewport| {
        body(ui, viewport, pane, v, style, &g, &layout);
    });
    pane.scroll_y = out.state.offset.y;
    pane.viewport_h = (out.inner_rect.height() - g.header_h).max(1.0);
    pane.page_rows = layout.rows_per_page(pane.viewport_h);
    pane.layout = Some(layout);
}

#[allow(clippy::too_many_lines)]
fn body(
    ui: &mut Ui,
    viewport: Rect,
    pane: &mut Pane,
    v: &mut ViewCtx<'_>,
    style: Style,
    g: &Geometry,
    layout: &FlowLayout,
) {
    let p = palette(ui.ctx());
    let origin = ui.min_rect().min;
    ui.set_min_size(vec2(g.content_w, g.header_h + layout.total_h));
    let vis = Rect::from_min_size(origin + viewport.min.to_vec2(), viewport.size());
    let list_top = origin.y + g.header_h;
    let to_layout = |pos: Pos2| pos2(pos.x - origin.x, pos.y - list_top);
    let to_screen = |r: Rect| r.translate(vec2(origin.x, list_top));
    let id = Id::new(("soyuz.flow.bg", pane.id.0));
    let bg = ui.interact(vis, id, Sense::click_and_drag());
    let list_area = Rect::from_min_max(pos2(vis.left(), vis.top() + g.header_h), vis.max);

    let hover_pos = bg.hover_pos().filter(|pos| list_area.contains(*pos));
    let hovered = if pane.band.is_none() { hover_pos.and_then(|pos| layout.hit(to_layout(pos))) } else { None };

    // ---- paint ----
    let painter = ui.painter_at(list_area);
    let y0 = viewport.min.y - g.header_h;
    let y1 = viewport.max.y - g.header_h;
    let cursor_pos = pane.cursor_pos();
    let thumbs_on = v.config.appearance.show_thumbnails;
    let ppp = ui.ctx().pixels_per_point();
    for line in &layout.lines()[layout.lines_in(y0, y1)] {
        match line.kind {
            LineKind::Header(gi) => {
                let r = to_screen(Rect::from_min_size(
                    pos2(g.pad.x, line.top),
                    vec2(g.content_w - 2.0 * g.pad.x, line.height),
                ));
                if let Some(span) = pane.groups.get(gi) {
                    let collapsed = pane.collapsed_groups.contains(&span.label);
                    let hot = hover_pos.is_some_and(|hp| r.contains(hp));
                    paint_group_header(&painter, &p, r, &span.label, span.count, collapsed, hot);
                }
            }
            LineKind::Items { start, len } => {
                for i in start..start + len {
                    let Some(cell) = layout.item_rect(i) else { continue };
                    let r = to_screen(cell);
                    let idx = pane.visible[i];
                    let e = &pane.entries[idx];
                    let key = &pane.keys[idx];
                    let st = ItemState {
                        selected: pane.selection.contains(key),
                        cursor: cursor_pos == Some(i) && v.list_focus && v.focused,
                        hovered: hovered == Some(i),
                        focused_pane: v.focused,
                        checkbox: v.checkbox_mode,
                    };
                    match style {
                        Style::Details => paint_details_row(&painter, &p, r, e, &st, &g.cols, origin.x, v),
                        Style::Grid | Style::Gallery => {
                            let tex = if thumbs_on && thumbs::is_thumbnailable(&e.path) {
                                let px = thumbs::bucket(pane.view.zoom as f32, ppp);
                                v.services.thumbs.get(&ThumbKey {
                                    path: e.path.clone(),
                                    mtime: e.modified,
                                    size: e.size,
                                    px,
                                })
                            } else {
                                None
                            };
                            paint_grid_cell(
                                ui,
                                &painter,
                                &p,
                                r,
                                e,
                                &st,
                                pane.view.zoom as f32,
                                tex,
                                style == Style::Gallery,
                            );
                        }
                        Style::List => paint_list_cell(&painter, &p, r, e, &st, false),
                        Style::Column => paint_list_cell(&painter, &p, r, e, &st, true),
                    }
                }
            }
        }
    }

    // ---- interaction ----
    let mods = ui.input(|i| i.modifiers);
    let click_mods = ClickMods { toggle: mods.command, range: mods.shift };
    let press = bg.interact_pointer_pos();
    let item_at = |pos: Option<Pos2>| pos.filter(|p| list_area.contains(*p)).and_then(|p| layout.hit(to_layout(p)));

    if bg.clicked() || bg.secondary_clicked() || bg.clicked_by(PointerButton::Middle) {
        v.cmds.push(Cmd::FocusPane(pane.id));
    }
    if bg.clicked() {
        let pos = press.filter(|p| list_area.contains(*p));
        if let Some(pos) = pos
            && let Some(gi) = layout.header_at(to_layout(pos))
        {
            if let Some(span) = pane.groups.get(gi) {
                let label = span.label.clone();
                if !pane.collapsed_groups.remove(&label) {
                    pane.collapsed_groups.insert(label);
                }
                pane.dirty = true;
            }
        } else if let Some(i) = item_at(pos) {
            let key = pane.key_at(i).cloned();
            if let Some(key) = key {
                let on_checkbox = v.checkbox_mode
                    && pos.is_some_and(|pp| {
                        checkbox_rect(style, to_screen(layout.item_rect(i).unwrap_or(Rect::NOTHING)), &g.cols, origin.x)
                            .contains(pp)
                    });
                let single_open = v.config.general.single_click_open
                    && !mods.any()
                    && pane.entry(&key).is_some_and(|e| e.is_dir_like());
                if on_checkbox {
                    pane.selection.toggle(key.clone());
                    pane.cursor = Some(key);
                } else if single_open {
                    v.cmds.push(Cmd::Open { pane: pane.id, key });
                } else {
                    let order = pane.visible_keys();
                    pane.selection.click(&order, key.clone(), click_mods);
                    pane.cursor = Some(key);
                }
            }
        } else if !click_mods.toggle && !click_mods.range {
            pane.selection.clear();
        }
    }
    if bg.double_clicked()
        && let Some(i) = item_at(press)
        && let Some(key) = pane.key_at(i).cloned()
    {
        v.cmds.push(Cmd::Open { pane: pane.id, key });
    }
    if bg.clicked_by(PointerButton::Middle)
        && let Some(i) = item_at(press)
        && let Some(e) = pane.entry_at(i)
        && e.is_dir_like()
    {
        let loc = soyuz_core::Location::Dir(e.path.clone());
        match v.config.behaviour.middle_click {
            MiddleClick::NewTab => v.cmds.push(Cmd::OpenInNewTab { location: loc, activate: false }),
            MiddleClick::OtherPane => v.cmds.push(Cmd::OpenInOtherPane(loc)),
            MiddleClick::Nothing => {}
        }
    }
    if bg.secondary_clicked() {
        match item_at(press).and_then(|i| pane.key_at(i).cloned()) {
            Some(key) => {
                if !pane.selection.contains(&key) {
                    pane.selection.select_only(key.clone());
                }
                pane.cursor = Some(key);
                pane.context = ContextTarget::Items;
            }
            None => {
                if !click_mods.toggle {
                    pane.selection.clear();
                }
                pane.context = ContextTarget::Background;
            }
        }
    }

    // Rubber band: starts on empty space (or outside the name in details view).
    if bg.drag_started_by(PointerButton::Primary) {
        let origin_pos = ui.input(|i| i.pointer.press_origin()).filter(|p| list_area.contains(*p));
        if let Some(op) = origin_pos {
            let on_item = item_at(Some(op)).is_some_and(|i| match style {
                Style::Details => {
                    in_name_extent(&painter, op, i, pane, layout, &g.cols, origin.x, list_top, v.checkbox_mode)
                }
                _ => true,
            });
            if on_item {
                // Press-and-drag on an item selects it (drag-and-drop arrives with the
                // jobs integration).
                if let Some(key) = item_at(Some(op)).and_then(|i| pane.key_at(i).cloned()) {
                    if !pane.selection.contains(&key) && !click_mods.toggle {
                        pane.selection.select_only(key.clone());
                    }
                    pane.cursor = Some(key);
                }
                v.cmds.push(Cmd::FocusPane(pane.id));
            } else {
                let mode = BandMode::from_mods(click_mods);
                let base = if mode == BandMode::Replace { Selection::default() } else { pane.selection.clone() };
                let start = to_layout(op);
                pane.band = Some(Band { start, current: start, base, mode });
                v.cmds.push(Cmd::FocusPane(pane.id));
            }
        }
    }
    if let Some(mut band) = pane.band.take() {
        if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
            let mut cur = to_layout(pos);
            cur.x = cur.x.clamp(0.0, g.content_w);
            cur.y = cur.y.clamp(0.0, layout.total_h);
            band.current = cur;
            // Auto-scroll near / beyond the edges.
            let top = list_area.top() + 24.0;
            let bottom = list_area.bottom() - 24.0;
            let delta = if pos.y < top {
                ((top - pos.y) * 0.35).clamp(3.0, 50.0)
            } else if pos.y > bottom {
                -((pos.y - bottom) * 0.35).clamp(3.0, 50.0)
            } else {
                0.0
            };
            if delta != 0.0 {
                ui.scroll_with_delta(vec2(0.0, delta));
                ui.ctx().request_repaint();
            }
        }
        let band_rect = Rect::from_two_pos(band.start, band.current);
        let hits: Vec<_> =
            layout.items_in_rect(band_rect).into_iter().filter_map(|i| pane.key_at(i).cloned()).collect();
        let under = layout.hit(band.current).and_then(|i| pane.key_at(i).cloned());
        pane.selection = Selection::from_band(&band.base, &hits, band.mode);
        if let Some(k) = under.or_else(|| hits.last().cloned()) {
            pane.cursor = Some(k);
        }
        let screen = to_screen(band_rect);
        let painter = ui.painter_at(list_area);
        painter.rect_filled(screen, CornerRadius::same(3), p.accent.gamma_multiply(0.10));
        painter.rect_stroke(
            screen,
            CornerRadius::same(3),
            Stroke::new(1.0, p.accent.gamma_multiply(0.8)),
            StrokeKind::Inside,
        );
        if bg.dragged() && !bg.drag_stopped() {
            pane.band = Some(band);
        }
    }

    // Ctrl + wheel zooms the icon views.
    if bg.hovered() && matches!(style, Style::Grid | Style::Gallery) {
        let zd = ui.input(|i| i.zoom_delta());
        if (zd - 1.0).abs() > 0.001 {
            let z = (pane.view.zoom as f32 * zd).round().clamp(16.0, 256.0) as u16;
            if z != pane.view.zoom {
                pane.view.zoom = z;
                v.cmds.push(Cmd::ViewChanged(pane.id));
            }
        }
    }

    // Keep the cursor visible after keyboard moves / view switches.
    if pane.scroll_to_cursor {
        pane.scroll_to_cursor = false;
        if let Some(r) = pane.cursor_pos().and_then(|c| layout.item_rect(c)) {
            let r = to_screen(r);
            let target =
                Rect::from_min_max(pos2(r.left(), r.top() - g.header_h - 6.0), pos2(r.right(), r.bottom() + 6.0));
            ui.scroll_to_rect(target, None);
        }
    }

    if style == Style::Details {
        header(ui, &p, vis, pane, v, &g.cols, origin.x);
    }

    bg.context_menu(|ui| context_menu::pane_menu(ui, pane, v));
}

struct ItemState {
    selected: bool,
    cursor: bool,
    hovered: bool,
    focused_pane: bool,
    checkbox: bool,
}

fn paint_item_bg(painter: &Painter, p: &Palette, r: Rect, st: &ItemState, rounding: u8) {
    let cr = CornerRadius::same(rounding);
    if st.selected {
        painter.rect_filled(r, cr, selection_fill(p, st.focused_pane));
        if st.hovered {
            painter.rect_filled(r, cr, p.accent.gamma_multiply(0.06));
        }
    } else if st.hovered {
        painter.rect_filled(r, cr, p.bg_surface2);
    }
    if st.cursor {
        painter.rect_stroke(r.shrink(0.5), cr, Stroke::new(1.0, p.accent.gamma_multiply(0.75)), StrokeKind::Inside);
    }
}

fn text_alpha(e: &EntryInfo) -> f32 {
    if e.is_hidden() { alpha::HIDDEN } else { 1.0 }
}

fn paint_checkbox(painter: &Painter, p: &Palette, r: Rect, checked: bool) {
    let cr = CornerRadius::same(4);
    if checked {
        painter.rect_filled(r, cr, p.accent);
        painter.text(r.center(), Align2::CENTER_CENTER, ph::CHECK, ticons::font(11.0), Color32::WHITE);
    } else {
        painter.rect_filled(r, cr, p.bg_surface1);
        painter.rect_stroke(r, cr, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    }
}

/// Checkbox hit area of an item (screen coordinates).
fn checkbox_rect(style: Style, item: Rect, cols: &[ColGeom], origin_x: f32) -> Rect {
    match style {
        Style::Details => {
            let name_x = cols.iter().find(|c| c.column == Column::Name).map_or(item.left(), |c| origin_x + c.x);
            Rect::from_center_size(pos2(name_x + 8.0 + CHECK / 2.0, item.center().y), vec2(CHECK + 6.0, CHECK + 6.0))
        }
        Style::Grid | Style::Gallery => Rect::from_min_size(item.min + vec2(4.0, 4.0), vec2(CHECK + 6.0, CHECK + 6.0)),
        Style::List | Style::Column => Rect::from_center_size(
            pos2(item.left() + 6.0 + CHECK / 2.0, item.center().y),
            vec2(CHECK + 6.0, CHECK + 6.0),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_details_row(
    painter: &Painter,
    p: &Palette,
    r: Rect,
    e: &EntryInfo,
    st: &ItemState,
    cols: &[ColGeom],
    origin_x: f32,
    v: &ViewCtx<'_>,
) {
    if !painter.clip_rect().intersects(r) {
        return;
    }
    paint_item_bg(painter, p, r, st, radius::ROW);
    let a = text_alpha(e);
    let cy = r.center().y;
    for c in cols {
        let x0 = origin_x + c.x;
        let x1 = x0 + c.w;
        if x1 < painter.clip_rect().left() || x0 > painter.clip_rect().right() {
            continue;
        }
        if c.column == Column::Name {
            let mut x = x0 + 10.0;
            if st.checkbox {
                let cb = Rect::from_center_size(pos2(x0 + 8.0 + CHECK / 2.0, cy), vec2(CHECK, CHECK));
                if st.selected || st.hovered {
                    paint_checkbox(painter, p, cb, st.selected);
                } else {
                    painter.rect_stroke(
                        cb,
                        CornerRadius::same(4),
                        Stroke::new(1.0, p.border_subtle),
                        StrokeKind::Inside,
                    );
                }
                x = x0 + 8.0 + CHECK + 10.0;
            }
            let glyph_font = if e.is_dir_like() { ticons::font_fill(16.0) } else { ticons::font(16.0) };
            painter.text(
                pos2(x + 8.0, cy),
                Align2::CENTER_CENTER,
                icons::glyph(e),
                glyph_font,
                icons::color(e, p).gamma_multiply(a),
            );
            let tx = x + 24.0;
            let color = p.text_primary.gamma_multiply(a);
            let galley = trunc_galley(painter, &e.name, FontId::proportional(13.0), color, x1 - tx - 8.0);
            painter.galley(pos2(tx, cy - galley.size().y / 2.0), galley, color);
        } else {
            let text = columns::cell_text(c.column, e, v.config);
            if text.is_empty() {
                continue;
            }
            let color = p.text_secondary.gamma_multiply(a);
            let galley = trunc_galley(painter, &text, FontId::proportional(12.5), color, c.w - 18.0);
            let x = if columns::right_aligned(c.column) { x1 - 10.0 - galley.size().x } else { x0 + 10.0 };
            painter.galley(pos2(x, cy - galley.size().y / 2.0), galley, color);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_grid_cell(
    ui: &Ui,
    painter: &Painter,
    p: &Palette,
    r: Rect,
    e: &EntryInfo,
    st: &ItemState,
    zoom: f32,
    tex: Option<egui::TextureHandle>,
    gallery: bool,
) {
    if !painter.clip_rect().intersects(r) {
        return;
    }
    paint_item_bg(painter, p, r, st, radius::CARD);
    let a = text_alpha(e);
    let icon_box = Rect::from_center_size(pos2(r.center().x, r.top() + 8.0 + zoom / 2.0), vec2(zoom, zoom));
    match tex {
        Some(t) => {
            let size = t.size_vec2();
            let scale = (zoom / size.x).min(zoom / size.y);
            let img = Rect::from_center_size(icon_box.center(), size * scale);
            let rounding = CornerRadius::same(if zoom >= 64.0 { 6 } else { 3 });
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), size))
                .corner_radius(rounding)
                .tint(Color32::WHITE.gamma_multiply(a))
                .paint_at(ui, img);
            painter.rect_stroke(img, rounding, Stroke::new(1.0, p.border_subtle), StrokeKind::Outside);
        }
        None => {
            let g = icons::glyph(e);
            let font = ticons::font_fill(zoom * if e.is_dir_like() { 0.86 } else { 0.8 });
            painter.text(icon_box.center(), Align2::CENTER_CENTER, g, font, icons::color(e, p).gamma_multiply(a));
        }
    }
    if st.checkbox && (st.selected || st.hovered) {
        paint_checkbox(painter, p, Rect::from_min_size(r.min + vec2(7.0, 7.0), vec2(CHECK, CHECK)), st.selected);
    }
    let color = p.text_primary.gamma_multiply(a);
    let font = FontId::proportional(if zoom < 48.0 { 12.0 } else { 12.5 });
    let rows = if gallery { 1 } else { 2 };
    let galley = wrapped_galley(painter, &e.name, font, color, r.width() - 10.0, rows);
    let gy = icon_box.bottom() + 6.0;
    painter.galley(pos2(r.center().x, gy), galley, color);
}

fn paint_list_cell(painter: &Painter, p: &Palette, r: Rect, e: &EntryInfo, st: &ItemState, miller: bool) {
    if !painter.clip_rect().intersects(r) {
        return;
    }
    paint_item_bg(painter, p, r, st, radius::ROW);
    let a = text_alpha(e);
    let cy = r.center().y;
    let mut x = r.left() + 8.0;
    if st.checkbox {
        let cb = Rect::from_center_size(pos2(r.left() + 6.0 + CHECK / 2.0, cy), vec2(CHECK, CHECK));
        if st.selected || st.hovered {
            paint_checkbox(painter, p, cb, st.selected);
            x = cb.right() + 6.0;
        }
    }
    let glyph_font = if e.is_dir_like() { ticons::font_fill(16.0) } else { ticons::font(16.0) };
    painter.text(
        pos2(x + 8.0, cy),
        Align2::CENTER_CENTER,
        icons::glyph(e),
        glyph_font,
        icons::color(e, p).gamma_multiply(a),
    );
    let right_pad = if miller && e.is_dir_like() { 24.0 } else { 8.0 };
    let color = p.text_primary.gamma_multiply(a);
    let galley = trunc_galley(painter, &e.name, FontId::proportional(13.0), color, r.right() - x - 24.0 - right_pad);
    painter.galley(pos2(x + 24.0, cy - galley.size().y / 2.0), galley, color);
    if miller && e.is_dir_like() {
        painter.text(
            pos2(r.right() - 10.0, cy),
            Align2::RIGHT_CENTER,
            ph::CARET_RIGHT,
            ticons::font(11.0),
            p.text_tertiary,
        );
    }
}

fn paint_group_header(painter: &Painter, p: &Palette, r: Rect, label: &str, count: usize, collapsed: bool, hot: bool) {
    let cy = r.center().y + 2.0;
    let caret = if collapsed { ph::CARET_RIGHT } else { ph::CARET_DOWN };
    let c = if hot { p.text_primary } else { p.text_secondary };
    painter.text(pos2(r.left() + 10.0, cy), Align2::CENTER_CENTER, caret, ticons::font(11.0), p.text_tertiary);
    let lg = painter.layout_no_wrap(label.to_string(), semibold(12.5), c);
    let lx = r.left() + 22.0;
    let lw = lg.size().x;
    painter.galley(pos2(lx, cy - lg.size().y / 2.0), lg, c);
    let cg = painter.layout_no_wrap(count.to_string(), FontId::proportional(11.5), p.text_tertiary);
    let cx = lx + lw + 8.0;
    let cw = cg.size().x;
    painter.galley(pos2(cx, cy - cg.size().y / 2.0), cg, p.text_tertiary);
    let line_x = cx + cw + 12.0;
    if line_x < r.right() - 8.0 {
        painter.line_segment([pos2(line_x, cy), pos2(r.right() - 8.0, cy)], Stroke::new(1.0, p.border_subtle));
    }
}

/// Is `pos` over the icon + name text of item `i` (details view)? Drags that start
/// there are reserved for drag-and-drop instead of the rubber band.
#[allow(clippy::too_many_arguments)]
fn in_name_extent(
    painter: &Painter,
    pos: Pos2,
    i: usize,
    pane: &Pane,
    layout: &FlowLayout,
    cols: &[ColGeom],
    origin_x: f32,
    list_top: f32,
    checkbox: bool,
) -> bool {
    let Some(name) = cols.iter().find(|c| c.column == Column::Name) else { return false };
    let Some(e) = pane.entry_at(i) else { return false };
    let Some(r) = layout.item_rect(i) else { return false };
    let x0 = origin_x + name.x + if checkbox { 8.0 + CHECK + 10.0 } else { 10.0 };
    let w = painter.layout_no_wrap(e.name.clone(), FontId::proportional(13.0), Color32::WHITE).size().x;
    let x1 = (x0 + 24.0 + w + 6.0).min(origin_x + name.x + name.w);
    let y = pos.y - list_top;
    pos.x >= x0 && pos.x <= x1 && y >= r.top() && y <= r.bottom()
}

/// Sticky column header: sort on click, resize by the right edge, drag to reorder,
/// right-click for the column chooser.
fn header(ui: &mut Ui, p: &Palette, vis: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>, cols: &[ColGeom], origin_x: f32) {
    let hr = Rect::from_min_size(vis.min, vec2(vis.width(), HEADER_H));
    let painter = ui.painter_at(hr);
    painter.rect_filled(hr, CornerRadius::ZERO, p.bg_surface1);
    painter.line_segment(
        [pos2(hr.left(), hr.bottom() - 0.5), pos2(hr.right(), hr.bottom() - 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );
    let sort = pane.view.sort.clone();
    let mut chooser: Option<Response> = None;
    let mut drop_target: Option<usize> = None;
    for (ci, c) in cols.iter().enumerate() {
        let r = Rect::from_min_max(pos2(origin_x + c.x, hr.top()), pos2(origin_x + c.x + c.w, hr.bottom()));
        if r.right() < hr.left() || r.left() > hr.right() {
            continue;
        }
        let resp = ui.interact(r, Id::new(("soyuz.hdr", pane.id.0, ci)), Sense::click_and_drag());
        let active = sort.column == c.column;
        if resp.hovered() && pane.col_drag.is_none() {
            painter.rect_filled(r.shrink2(vec2(2.0, 4.0)), CornerRadius::same(radius::CONTROL), p.bg_surface2);
        }
        let color = if active || resp.hovered() { p.text_primary } else { p.text_secondary };
        let label = c.column.label();
        let font = if active { semibold(12.0) } else { FontId::proportional(12.0) };
        let galley = trunc_galley(&painter, label, font, color, c.w - 34.0);
        let gw = galley.size().x;
        let right = columns::right_aligned(c.column);
        let tx = if right {
            r.right() - 12.0 - gw - if active { 14.0 } else { 0.0 }
        } else {
            r.left() + 10.0 + if c.column == Column::Name && v.checkbox_mode { CHECK + 8.0 } else { 0.0 }
        };
        painter.galley(pos2(tx, r.center().y - galley.size().y / 2.0), galley, color);
        if active {
            let caret = if sort.dir == SortDir::Asc { ph::CARET_UP } else { ph::CARET_DOWN };
            let cx = if right { r.right() - 14.0 } else { tx + gw + 10.0 };
            painter.text(pos2(cx, r.center().y), Align2::CENTER_CENTER, caret, ticons::font(10.0), p.accent);
        }
        if ci + 1 < cols.len() {
            painter.line_segment(
                [pos2(r.right() - 0.5, r.top() + 9.0), pos2(r.right() - 0.5, r.bottom() - 9.0)],
                Stroke::new(1.0, p.border_subtle),
            );
        }
        if resp.clicked() {
            v.cmds.push(Cmd::PaneAction(pane.id, Action::SortBy(c.column)));
        }
        if resp.drag_started() {
            pane.col_drag = Some((ci, resp.interact_pointer_pos().map_or(r.center().x, |p| p.x)));
        }
        if resp.secondary_clicked() || resp.context_menu_opened() {
            chooser = Some(resp.clone());
        }
        if let Some((from, x)) = pane.col_drag
            && from != ci
            && x >= r.left()
            && x < r.right()
        {
            drop_target = Some(ci);
        }
    }
    // Resize handles go on top of every header body (registered last).
    for (ci, c) in cols.iter().enumerate() {
        let r = Rect::from_min_max(pos2(origin_x + c.x, hr.top()), pos2(origin_x + c.x + c.w, hr.bottom()));
        if r.right() < hr.left() - 4.0 || r.right() > hr.right() + 4.0 {
            continue;
        }
        let handle = Rect::from_center_size(pos2(r.right(), r.center().y), vec2(8.0, HEADER_H));
        let hresp = ui.interact(handle, Id::new(("soyuz.hdr.resize", pane.id.0, ci)), Sense::drag());
        if hresp.hovered() || hresp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn);
            painter.line_segment(
                [pos2(r.right() - 0.5, r.top() + 5.0), pos2(r.right() - 0.5, r.bottom() - 5.0)],
                Stroke::new(2.0, p.accent),
            );
        }
        if hresp.dragged() {
            let dx = hresp.drag_delta().x;
            let new_w = (c.w + dx).max(if c.column == Column::Name { columns::MIN_NAME_W } else { columns::MIN_COL_W });
            pane.view.set_column_width(c.column, new_w);
            pane.layout_sig = None;
        }
        if hresp.drag_stopped() {
            v.cmds.push(Cmd::ViewChanged(pane.id));
        }
    }
    // Column reorder by dragging a header.
    if let Some((from, _)) = pane.col_drag {
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let down = ui.input(|i| i.pointer.primary_down());
        if let Some(pos) = pointer {
            pane.col_drag = Some((from, pos.x));
            if let Some(t) = drop_target {
                let c = cols[t];
                let x = if t > from { origin_x + c.x + c.w } else { origin_x + c.x };
                painter.line_segment([pos2(x, hr.top() + 4.0), pos2(x, hr.bottom() - 4.0)], Stroke::new(2.0, p.accent));
            }
            if let Some(c) = cols.get(from) {
                let ghost = Rect::from_center_size(pos2(pos.x, hr.center().y), vec2(c.w.min(160.0), HEADER_H - 8.0));
                painter.rect_filled(ghost, CornerRadius::same(radius::CONTROL), p.accent.gamma_multiply(0.15));
                painter.text(
                    ghost.center(),
                    Align2::CENTER_CENTER,
                    c.column.label(),
                    FontId::proportional(12.0),
                    p.text_primary,
                );
            }
        }
        if !down {
            if let Some(t) = drop_target {
                pane.view.move_visible_column(from, t);
                pane.layout_sig = None;
                v.cmds.push(Cmd::ViewChanged(pane.id));
            }
            pane.col_drag = None;
        }
    }
    if let Some(resp) = chooser {
        resp.context_menu(|ui| context_menu::column_menu(ui, pane, v));
    }
}

//! A virtualized, painter-only table for the monitor pages: one interaction surface for
//! all rows (like the file views), a sticky header with click-to-sort, drag-to-reorder and
//! edge-drag-to-resize columns, horizontal scrolling for wide column sets, and row callbacks
//! that paint only what is on screen. The processes table, the list pages, the comparison
//! and the details tabs all draw through it.

use egui::{
    Align2, Color32, CursorIcon, FontId, Id, Modifiers, Painter, Rect, Response, ScrollArea, Sense, Stroke, Ui, pos2,
    vec2,
};

use crate::theme::Palette;
use crate::theme::font::semibold;
use crate::theme::tokens::radius;
use crate::theme::{font, icons as ticons, icons::ph};
use crate::ui::widgets::trunc_galley;

/// One column.
#[derive(Clone, Debug)]
pub struct Col {
    pub title: String,
    pub width: f32,
    /// Right-aligned figures.
    pub numeric: bool,
    pub tip: String,
    pub resizable: bool,
}

impl Col {
    pub fn new(title: impl Into<String>, width: f32) -> Self {
        Col { title: title.into(), width, numeric: false, tip: String::new(), resizable: true }
    }

    pub fn numeric(mut self) -> Self {
        self.numeric = true;
        self
    }

    pub fn tip(mut self, t: impl Into<String>) -> Self {
        self.tip = t.into();
        self
    }

    pub fn fixed(mut self) -> Self {
        self.resizable = false;
        self
    }
}

/// The sorted column and direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub col: usize,
    pub descending: bool,
}

/// What the header reported.
#[derive(Clone, Debug, PartialEq)]
pub enum HeaderEvent {
    Sort(usize),
    /// Live, while dragging the edge.
    Resize {
        col: usize,
        width: f32,
    },
    /// The edge drag ended.
    ResizeDone {
        col: usize,
        width: f32,
    },
    Move {
        from: usize,
        to: usize,
    },
    /// Right click on a header.
    Menu(usize),
}

/// What a row callback gets.
pub struct RowCtx<'a> {
    pub painter: &'a Painter,
    pub p: &'a Palette,
    pub rect: Rect,
    pub index: usize,
    pub hovered: bool,
    /// `(x, width)` of every column, in screen coordinates.
    pub cols: &'a [(f32, f32)],
}

impl RowCtx<'_> {
    /// The cell rectangle of column `c`, inset for text.
    pub fn cell(&self, c: usize) -> Rect {
        let (x, w) = self.cols.get(c).copied().unwrap_or((self.rect.left(), 0.0));
        Rect::from_min_max(pos2(x + 8.0, self.rect.top()), pos2(x + w - 8.0, self.rect.bottom()))
    }

    /// Truncated single-line text in column `c`.
    pub fn text(&self, c: usize, text: &str, numeric: bool, font: FontId, color: Color32) {
        let r = self.cell(c);
        if r.width() < 4.0 || text.is_empty() {
            return;
        }
        let g = trunc_galley(self.painter, text, font, color, r.width());
        let x = if numeric { r.right() - g.size().x } else { r.left() };
        self.painter.galley(pos2(x, r.center().y - g.size().y / 2.0), g, color);
    }
}

/// What the table reported.
pub struct TableOut {
    pub events: Vec<HeaderEvent>,
    pub clicked: Option<(usize, Modifiers)>,
    pub double_clicked: Option<usize>,
    pub secondary: Option<usize>,
    pub hovered: Option<usize>,
    /// The rows' interaction surface (attach a context menu to it).
    pub bg: Response,
    /// The header that was right-clicked.
    pub menu: Option<Response>,
    /// Rows that fit in the viewport.
    pub page_rows: usize,
    /// First and last visible row.
    pub visible: std::ops::Range<usize>,
}

/// A table; configure, then [`Table::show`].
pub struct Table<'a> {
    pub id: Id,
    pub cols: &'a [Col],
    pub sort: Option<Sort>,
    pub row_h: f32,
    pub rows: usize,
    /// Columns can be dragged into another order.
    pub reorder: bool,
    /// The last column grows to the right edge when the columns do not fill the width.
    pub stretch_last: bool,
    /// Scroll this row into view.
    pub scroll_to: Option<usize>,
    /// Accessible name of the table.
    pub name: String,
    /// Draw the header.
    pub header: bool,
    /// Keep returning this header's response in [`TableOut::menu`] (a context menu that
    /// stays open after the click that opened it).
    pub menu_col: Option<usize>,
}

fn header_h() -> f32 {
    font::grow(28.0)
}

impl<'a> Table<'a> {
    pub fn new(id: Id, cols: &'a [Col], rows: usize, row_h: f32) -> Self {
        Table {
            id,
            cols,
            sort: None,
            row_h,
            rows,
            reorder: false,
            stretch_last: true,
            scroll_to: None,
            name: String::new(),
            header: true,
            menu_col: None,
        }
    }

    /// Paint the table in `rect`; `paint_row` draws one row and returns its accessible name.
    pub fn show(
        self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        mut paint_row: impl FnMut(&RowCtx<'_>) -> String,
    ) -> TableOut {
        let hh = if self.header { header_h() } else { 0.0 };
        let mut events = Vec::new();
        let mut out_clicked = None;
        let mut out_double = None;
        let mut out_secondary = None;
        let mut out_hovered = None;
        let mut menu = None;
        let mut bg_out: Option<Response> = None;
        let mut page_rows = 1usize;
        let mut visible = 0..0;
        let drag_id = self.id.with("coldrag");
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(self.id.with("child")));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        ScrollArea::new([true, true]).id_salt(self.id.with("scroll")).auto_shrink([false, false]).show_viewport(
            &mut child,
            |ui, viewport| {
                let origin = ui.min_rect().min;
                let avail_w = viewport.width().max(rect.width() - 14.0);
                // Column geometry.
                let natural: f32 = self.cols.iter().map(|c| c.width).sum::<f32>() + 8.0;
                let stretch = if self.stretch_last && natural < avail_w { avail_w - natural } else { 0.0 };
                let mut xs: Vec<(f32, f32)> = Vec::with_capacity(self.cols.len());
                let mut x = origin.x + 4.0;
                for (i, c) in self.cols.iter().enumerate() {
                    let w = c.width + if i + 1 == self.cols.len() { stretch } else { 0.0 };
                    xs.push((x, w));
                    x += w;
                }
                let total_w = (x - origin.x + 4.0).max(avail_w);
                let total_h = hh + self.rows as f32 * self.row_h + 6.0;
                ui.set_min_size(vec2(total_w, total_h));
                let vis = Rect::from_min_size(origin + viewport.min.to_vec2(), viewport.size());
                let list_area = Rect::from_min_max(pos2(vis.left(), vis.top() + hh), vis.max);
                page_rows = ((list_area.height() / self.row_h).floor() as usize).max(1);

                // ---- rows ----
                let bg = ui.interact(list_area, self.id.with("bg"), Sense::click());
                let row_at = |pos: egui::Pos2| -> Option<usize> {
                    if !list_area.contains(pos) {
                        return None;
                    }
                    let i = ((pos.y - (origin.y + hh)) / self.row_h).floor();
                    (i >= 0.0 && (i as usize) < self.rows).then_some(i as usize)
                };
                let hovered_row = bg.hover_pos().and_then(row_at);
                out_hovered = hovered_row;
                if let Some(pos) = bg.interact_pointer_pos() {
                    if bg.double_clicked() {
                        out_double = row_at(pos);
                    } else if bg.clicked() {
                        out_clicked = row_at(pos).map(|i| (i, ui.input(|inp| inp.modifiers)));
                    }
                    if bg.secondary_clicked() {
                        out_secondary = row_at(pos);
                    }
                }
                let first = ((viewport.top() - hh) / self.row_h).floor().max(0.0) as usize;
                let last = (((viewport.bottom() - hh) / self.row_h).ceil().max(0.0) as usize).min(self.rows);
                visible = first.min(last)..last;
                let painter = ui.painter_at(list_area);
                let a11y = crate::ui::a11y::active(ui.ctx());
                if a11y {
                    crate::ui::a11y::container(ui.ctx(), bg.id, egui::accesskit::Role::ListBox, &self.name);
                }
                for i in visible.clone() {
                    let r = Rect::from_min_size(
                        pos2(origin.x, origin.y + hh + i as f32 * self.row_h),
                        vec2(total_w, self.row_h),
                    );
                    if self.scroll_to == Some(i) && (r.top() < list_area.top() || r.bottom() > list_area.bottom()) {
                        // Only vertically: the row spans every column, which must not pan the table.
                        ui.scroll_to_rect(r.with_min_x(vis.left()).with_max_x(vis.right()), None);
                    }
                    let rc =
                        RowCtx { painter: &painter, p, rect: r, index: i, hovered: hovered_row == Some(i), cols: &xs };
                    let label = paint_row(&rc);
                    if a11y {
                        crate::ui::a11y::list_item(
                            ui,
                            self.id.with(("row", i)),
                            r.intersect(list_area),
                            &label,
                            false,
                            i,
                            self.rows,
                        );
                    }
                }
                if let Some(i) = self.scroll_to
                    && !visible.contains(&i)
                    && i < self.rows
                {
                    let r = Rect::from_min_size(
                        pos2(origin.x, origin.y + hh + i as f32 * self.row_h),
                        vec2(total_w, self.row_h),
                    );
                    ui.scroll_to_rect(r.with_min_x(vis.left()).with_max_x(vis.right()), Some(egui::Align::Center));
                }

                // ---- header (sticky) ----
                if self.header {
                    let hr = Rect::from_min_size(vis.min, vec2(vis.width().max(total_w), hh));
                    let hp = ui.painter_at(hr);
                    hp.rect_filled(hr, egui::CornerRadius::ZERO, p.bg_surface1);
                    hp.hline(hr.x_range(), hr.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
                    let mut drag: Option<(usize, f32)> = ui.ctx().data(|d| d.get_temp(drag_id));
                    let mut drop_target: Option<usize> = None;
                    for (ci, c) in self.cols.iter().enumerate() {
                        let (cx, cw) = xs[ci];
                        let r = Rect::from_min_max(pos2(cx, hr.top()), pos2(cx + cw, hr.bottom()));
                        if r.right() < hr.left() || r.left() > hr.right() {
                            continue;
                        }
                        let sense = if self.reorder { Sense::click_and_drag() } else { Sense::click() };
                        let resp = ui.interact(r, self.id.with(("hdr", ci)), sense);
                        let active = self.sort.is_some_and(|s| s.col == ci);
                        if resp.hovered() && drag.is_none() {
                            hp.rect_filled(
                                r.shrink2(vec2(2.0, 4.0)),
                                crate::theme::tokens::radius::same(radius::CONTROL),
                                p.bg_surface2,
                            );
                        }
                        let color = if active || resp.hovered() { p.text_primary } else { p.text_secondary };
                        crate::ui::a11y::with(ui.ctx(), resp.id, |n| {
                            n.set_role(egui::accesskit::Role::ColumnHeader);
                            n.set_label(c.title.as_str());
                            if let Some(s) = self.sort.filter(|s| s.col == ci) {
                                n.set_sort_direction(if s.descending {
                                    egui::accesskit::SortDirection::Descending
                                } else {
                                    egui::accesskit::SortDirection::Ascending
                                });
                            }
                        });
                        let f = if active { semibold(11.5) } else { font::ui(11.5) };
                        let g = trunc_galley(&hp, &c.title, f, color, (cw - 30.0).max(8.0));
                        let gw = g.size().x;
                        let tx = if c.numeric {
                            r.right() - 10.0 - gw - if active { 13.0 } else { 0.0 }
                        } else {
                            r.left() + 8.0
                        };
                        hp.galley(pos2(tx, r.center().y - g.size().y / 2.0), g, color);
                        if let Some(s) = self.sort.filter(|s| s.col == ci) {
                            let caret = if s.descending { ph::CARET_DOWN } else { ph::CARET_UP };
                            let ax = if c.numeric { r.right() - 12.0 } else { tx + gw + 9.0 };
                            hp.text(pos2(ax, r.center().y), Align2::CENTER_CENTER, caret, ticons::font(9.5), p.accent);
                        }
                        if ci + 1 < self.cols.len() {
                            hp.vline(
                                r.right() - 0.5,
                                (r.top() + 8.0)..=(r.bottom() - 8.0),
                                Stroke::new(1.0, p.border_subtle),
                            );
                        }
                        let resp = if c.tip.is_empty() { resp } else { resp.on_hover_text(&c.tip) };
                        if resp.clicked() {
                            events.push(HeaderEvent::Sort(ci));
                        }
                        if self.reorder && resp.drag_started() {
                            drag = Some((ci, resp.interact_pointer_pos().map_or(r.center().x, |q| q.x)));
                        }
                        if resp.secondary_clicked() || resp.context_menu_opened() {
                            events.push(HeaderEvent::Menu(ci));
                            menu = Some(resp.clone());
                        } else if self.menu_col == Some(ci) && menu.is_none() {
                            menu = Some(resp.clone());
                        }
                        if let Some((from, dx)) = drag
                            && from != ci
                            && dx >= r.left()
                            && dx < r.right()
                        {
                            drop_target = Some(ci);
                        }
                    }
                    // Resize handles on top of the header bodies.
                    for (ci, c) in self.cols.iter().enumerate() {
                        if !c.resizable {
                            continue;
                        }
                        let (cx, cw) = xs[ci];
                        let edge = cx + cw;
                        if edge < hr.left() - 4.0 || edge > hr.right() + 4.0 {
                            continue;
                        }
                        let handle = Rect::from_center_size(pos2(edge, hr.center().y), vec2(8.0, hh));
                        let hresp = ui.interact(handle, self.id.with(("resize", ci)), Sense::drag());
                        crate::ui::a11y::with(ui.ctx(), hresp.id, |n| {
                            n.set_role(egui::accesskit::Role::Splitter);
                            n.set_label(tr!("monitor-gui-resize-column", column = c.title.as_str()).as_str());
                        });
                        if hresp.hovered() || hresp.dragged() {
                            ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn);
                            hp.vline(edge - 0.5, (hr.top() + 4.0)..=(hr.bottom() - 4.0), Stroke::new(2.0, p.accent));
                        }
                        let stretched = ci + 1 == self.cols.len() && stretch > 0.0;
                        if hresp.dragged() && hresp.drag_delta().x != 0.0 {
                            let base = if stretched { cw } else { c.width };
                            let w = (base + hresp.drag_delta().x).clamp(40.0, 2000.0);
                            events.push(HeaderEvent::Resize { col: ci, width: w });
                        }
                        if hresp.drag_stopped() {
                            let w = if stretched { cw } else { c.width };
                            events.push(HeaderEvent::ResizeDone { col: ci, width: w.clamp(40.0, 2000.0) });
                        }
                    }
                    // Reorder by dragging a header.
                    if let Some((from, _)) = drag {
                        let pointer = ui.input(|i| i.pointer.latest_pos());
                        let down = ui.input(|i| i.pointer.primary_down());
                        if let Some(pos) = pointer {
                            drag = Some((from, pos.x));
                            if let Some(t) = drop_target {
                                let (tx, tw) = xs[t];
                                let x = if t > from { tx + tw } else { tx };
                                hp.vline(x, (hr.top() + 4.0)..=(hr.bottom() - 4.0), Stroke::new(2.0, p.accent));
                            }
                            if let Some(c) = self.cols.get(from) {
                                let ghost = Rect::from_center_size(
                                    pos2(pos.x, hr.center().y),
                                    vec2(c.width.min(160.0), hh - 8.0),
                                );
                                hp.rect_filled(
                                    ghost,
                                    crate::theme::tokens::radius::same(radius::CONTROL),
                                    p.accent.gamma_multiply(0.15),
                                );
                                hp.text(
                                    ghost.center(),
                                    Align2::CENTER_CENTER,
                                    &c.title,
                                    font::ui(11.5),
                                    p.text_primary,
                                );
                            }
                        }
                        if !down {
                            if let Some(t) = drop_target {
                                events.push(HeaderEvent::Move { from, to: t });
                            }
                            drag = None;
                        }
                    }
                    match drag {
                        Some(d) => {
                            ui.ctx().data_mut(|dm| dm.insert_temp(drag_id, d));
                        }
                        None => {
                            ui.ctx().data_mut(|dm| dm.remove::<(usize, f32)>(drag_id));
                        }
                    }
                }
                bg_out = Some(bg);
            },
        );
        let bg = bg_out.expect("the scroll area ran its closure");
        TableOut {
            events,
            clicked: out_clicked,
            double_clicked: out_double,
            secondary: out_secondary,
            hovered: out_hovered,
            bg,
            menu,
            page_rows,
            visible,
        }
    }
}

/// The background of a row: selection, hover, zebra-free.
pub fn row_bg(painter: &Painter, p: &Palette, rect: Rect, selected: bool, hovered: bool, focused: bool) {
    let r = rect.shrink2(vec2(4.0, 0.0));
    let cr = crate::theme::tokens::radius::same(radius::ROW);
    if selected {
        painter.rect_filled(r, cr, crate::ui::views::flow::selection_fill(p, focused));
    } else if hovered {
        painter.rect_filled(r, cr, p.bg_surface2);
    }
}

/// Small text helper for columns that are not part of a [`RowCtx`] (card tables).
pub fn text_in(painter: &Painter, rect: Rect, text: &str, numeric: bool, font: FontId, color: Color32) {
    if rect.width() < 4.0 || text.is_empty() {
        return;
    }
    let g = trunc_galley(painter, text, font, color, rect.width());
    let x = if numeric { rect.right() - g.size().x } else { rect.left() };
    painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
}

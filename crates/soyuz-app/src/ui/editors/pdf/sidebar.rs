//! The viewer's sidebar: a segmented Thumbnails / Outline switch over a virtualized
//! thumbnail list (an accent ring on the current page; it follows the page) or the
//! outline tree (compact rows, carets to expand), with an empty state when the document
//! has no outline.

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use soyuz_core::config::PdfSidebar;

use super::super::widgets::{self as ew, SCROLLBAR};
use super::{PdfEnv, PdfOutput, ViewState};
use crate::editors::pdf::PdfDoc;
use crate::services::pdf::Outline;
use crate::services::pdf::nav::Dest;
use crate::theme::tokens::{radius, size};
use crate::theme::{icons, icons::ph};

pub(super) fn show(
    doc: &mut PdfDoc,
    ui: &mut Ui,
    r: Rect,
    env: &mut PdfEnv<'_>,
    st: &mut ViewState,
    out: &mut PdfOutput,
) {
    let p = env.p;
    ui.painter().rect_filled(r, CornerRadius::ZERO, p.bg_surface1);
    ui.painter().vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, p.border_subtle));
    let header = Rect::from_min_size(r.min, vec2(r.width(), 38.0));
    let items = [(ph::SQUARES_FOUR, tr!("pdf-ui-thumbnails")), (ph::LIST_BULLETS, tr!("pdf-ui-outline"))];
    let selected = usize::from(doc.sidebar == PdfSidebar::Outline);
    let cell = vec2(34.0, 24.0);
    let seg = Rect::from_center_size(header.center(), vec2(cell.x * 2.0 + 4.0, cell.y + 4.0));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(seg).id_salt(env.id.with("pdf.side.seg")));
    if let Some(i) = crate::ui::widgets::segmented(&mut c, &items, selected, cell) {
        doc.set_sidebar(if i == 1 { PdfSidebar::Outline } else { PdfSidebar::Thumbnails });
    }
    let list = Rect::from_min_max(pos2(r.left(), header.bottom()), pos2(r.right() - 1.0, r.bottom()));
    match doc.sidebar {
        PdfSidebar::Outline => outline(doc, ui, list, env, st, out),
        _ => thumbnails(doc, ui, list, env, st),
    }
}

// ---- thumbnails ---------------------------------------------------------------------

fn thumbnails(doc: &mut PdfDoc, ui: &mut Ui, list: Rect, env: &mut PdfEnv<'_>, st: &mut ViewState) {
    let p = env.p;
    let id = env.id.with("pdf.thumbs");
    let n = doc.page_count();
    let pad = 14.0;
    let box_w = (list.width() - 2.0 * pad - SCROLLBAR).max(24.0);
    // Equal cells sized for the tallest page (aspect capped), so the list is a plain
    // virtualized grid of one column.
    let aspect = doc
        .layout()
        .rects
        .iter()
        .map(|r| (r[3] - r[1]) / (r[2] - r[0]).max(1.0))
        .fold(0.0f32, f32::max)
        .clamp(0.3, 1.8);
    let box_h = box_w * aspect;
    let label_h = 20.0;
    let cell = box_h + label_h + 12.0;
    let resp = ui.interact(list, id, Sense::hover());
    crate::ui::a11y::container(ui.ctx(), resp.id, egui::accesskit::Role::ListBox, tr!("pdf-ui-thumbnails"));
    if resp.hovered() {
        let d = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta));
        st.thumb_scroll -= d.y;
    }
    let page = doc.page();
    if st.follow_page != Some(page) {
        st.follow_page = Some(page);
        st.thumb_scroll = super::scroll_into_view(st.thumb_scroll, list.height(), cell, page);
    }
    let total = n as f32 * cell + 8.0;
    let max = (total - list.height()).max(0.0);
    let mut pos = st.thumb_scroll.clamp(0.0, max) as f64;
    let bar = Rect::from_min_max(pos2(list.right() - SCROLLBAR, list.top()), list.max);
    ew::vscroll(ui, bar, id.with("bar"), total as f64, list.height() as f64, &mut pos, p);
    st.thumb_scroll = (pos as f32).clamp(0.0, max);
    let range = super::visible_rows(st.thumb_scroll, list.height(), cell, n);
    // One more on each side, so a short scroll finds them ready.
    doc.want_thumbs(range.start.saturating_sub(1)..(range.end + 1).min(n));
    let painter = ui.painter_at(list);
    let shadow = p.shadow(crate::theme::tokens::shadow::POPUP);
    let a11y = crate::ui::a11y::active(ui.ctx());
    let mut go = None;
    for i in range {
        let top = list.top() + 4.0 + i as f32 * cell - st.thumb_scroll;
        let cell_r = Rect::from_min_size(pos2(list.left() + 4.0, top), vec2(list.width() - SCROLLBAR - 8.0, cell));
        let Some(lr) = doc.layout().rects.get(i as usize).copied() else { continue };
        let (pw, ph_) = ((lr[2] - lr[0]).max(1.0), (lr[3] - lr[1]).max(1.0));
        let s = (box_w / pw).min(box_h / ph_);
        let page_r = Rect::from_min_size(
            pos2(list.left() + pad + (box_w - pw * s) / 2.0, top + 6.0 + (box_h - ph_ * s)),
            vec2(pw * s, ph_ * s),
        );
        let resp = ui.interact(cell_r.intersect(list), id.with(("cell", i)), Sense::click());
        let current = i == page;
        if resp.hovered() {
            painter.rect_filled(cell_r, crate::theme::tokens::radius::same(radius::ROW), p.bg_surface2);
        }
        painter.add(egui::epaint::Shadow { blur: 6, offset: [0, 1], ..shadow }.as_shape(page_r, CornerRadius::ZERO));
        let paper = if doc.invert() { Color32::from_gray(24) } else { Color32::WHITE };
        painter.rect_filled(page_r, CornerRadius::ZERO, paper);
        if let Some(t) = doc.thumb(i) {
            use crate::ui::gfx::TexExt;
            painter.image(t.id(), page_r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if current {
            painter.rect_stroke(
                page_r.expand(3.0),
                crate::theme::tokens::radius::same(radius::CONTROL),
                Stroke::new(2.0, p.accent),
                egui::StrokeKind::Inside,
            );
        } else {
            painter.rect_stroke(
                page_r,
                CornerRadius::ZERO,
                Stroke::new(1.0, p.border_subtle),
                egui::StrokeKind::Outside,
            );
        }
        painter.text(
            pos2(cell_r.center().x, page_r.bottom() + 4.0 + label_h / 2.0),
            Align2::CENTER_CENTER,
            (i + 1).to_string(),
            crate::theme::font::numbers(11.5),
            if current { p.accent } else { p.text_secondary },
        );
        if a11y {
            crate::ui::a11y::list_item(
                ui,
                id.with(("a11y", i)),
                cell_r.intersect(list),
                &doc.page_label(i),
                current,
                i as usize,
                n as usize,
            );
        }
        if resp.clicked() {
            go = Some(i);
        }
    }
    if let Some(i) = go {
        doc.go_to_page(i);
        st.follow_page = Some(i);
        doc.focus_request = true;
    }
}

// ---- outline ------------------------------------------------------------------------

/// One visible outline row.
struct Row<'a> {
    node: &'a Outline,
    key: u64,
    depth: usize,
    pos: usize,
    siblings: usize,
}

/// Walks the expanded outline, counting every visible row and keeping those in
/// `want` (so a huge outline costs a walk, not a list, per frame).
struct Walk<'a, 'b> {
    expanded: &'b std::collections::HashSet<u64>,
    want: std::ops::Range<usize>,
    n: usize,
    rows: Vec<Row<'a>>,
}

impl<'a> Walk<'a, '_> {
    fn visit(&mut self, nodes: &'a [Outline], depth: usize, parent: u64) {
        for (i, node) in nodes.iter().enumerate() {
            let key = parent.wrapping_mul(1_000_003).wrapping_add(i as u64 + 1);
            if self.want.contains(&self.n) {
                self.rows.push(Row { node, key, depth, pos: i, siblings: nodes.len() });
            }
            self.n += 1;
            if !node.children.is_empty() && self.expanded.contains(&key) {
                self.visit(&node.children, depth + 1, key);
            }
        }
    }
}

fn outline(doc: &mut PdfDoc, ui: &mut Ui, list: Rect, env: &mut PdfEnv<'_>, st: &mut ViewState, out: &mut PdfOutput) {
    let p = env.p;
    let id = env.id.with("pdf.outline");
    if doc.outline().is_empty() {
        let mut c = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(list.shrink(8.0))
                .id_salt(id)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        crate::ui::empty_state::EmptyState {
            glyph: ph::LIST_BULLETS,
            glyph_color: Some(p.text_tertiary),
            title: tr!("pdf-ui-no-outline"),
            subtitle: tr!("pdf-ui-no-outline-body"),
            hints: &[],
        }
        .show(&mut c);
        return;
    }
    if !st.outline_init {
        st.outline_init = true;
        // A single top entry (the book's title) opens by itself.
        if doc.outline().len() == 1 {
            st.expanded.insert(1);
        }
    }
    let row_h = size::ROW_COMPACT;
    let resp = ui.interact(list, id, Sense::hover());
    crate::ui::a11y::container(ui.ctx(), resp.id, egui::accesskit::Role::Tree, tr!("pdf-ui-outline"));
    if resp.hovered() {
        let d = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta));
        st.outline_scroll -= d.y;
    }
    let first = (st.outline_scroll.max(0.0) / row_h) as usize;
    let count = (list.height() / row_h).ceil() as usize + 1;
    let mut walk = Walk { expanded: &st.expanded, want: first..first + count, n: 0, rows: Vec::with_capacity(count) };
    walk.visit(doc.outline(), 0, 0);
    let total = walk.n as f32 * row_h + 8.0;
    let max = (total - list.height()).max(0.0);
    let mut pos = st.outline_scroll.clamp(0.0, max) as f64;
    let bar = Rect::from_min_max(pos2(list.right() - SCROLLBAR, list.top()), list.max);
    ew::vscroll(ui, bar, id.with("bar"), total as f64, list.height() as f64, &mut pos, p);
    let scroll = (pos as f32).clamp(0.0, max);
    let painter = ui.painter_at(list);
    let mut toggle = None;
    let mut go: Option<(Option<Dest>, Option<String>)> = None;
    for (k, row) in walk.rows.iter().enumerate() {
        let top = list.top() + 4.0 + (first + k) as f32 * row_h - scroll;
        let rr = Rect::from_min_size(pos2(list.left() + 4.0, top), vec2(list.width() - SCROLLBAR - 6.0, row_h));
        let resp = ui.interact(rr.intersect(list), id.with(("row", row.key)), Sense::click());
        if resp.hovered() || resp.has_focus() {
            painter.rect_filled(rr, crate::theme::tokens::radius::same(radius::ROW), p.bg_surface2);
        }
        let x = rr.left() + 6.0 + row.depth as f32 * 14.0;
        let parent = !row.node.children.is_empty();
        let open = parent && st.expanded.contains(&row.key);
        let caret = Rect::from_min_size(pos2(x, rr.top()), vec2(16.0, row_h));
        if parent {
            painter.text(
                caret.center(),
                Align2::CENTER_CENTER,
                if open { ph::CARET_DOWN } else { ph::CARET_RIGHT },
                icons::font(11.0),
                p.text_tertiary,
            );
        }
        let title = row.node.title.replace(['\n', '\r', '\t'], " ");
        let g = crate::ui::widgets::trunc_galley(
            &painter,
            &title,
            crate::theme::font::ui(12.5),
            p.text_primary,
            (rr.right() - caret.right() - 6.0).max(10.0),
        );
        painter.galley(pos2(caret.right() + 2.0, rr.center().y - g.size().y / 2.0), g, p.text_primary);
        crate::ui::a11y::tree_item(
            ui,
            id.with(("a11y", row.key)),
            rr.intersect(list),
            &title,
            false,
            parent.then_some(open),
            row.depth,
            row.pos,
            row.siblings,
        );
        if resp.clicked() {
            let on_caret = resp.interact_pointer_pos().is_some_and(|pt| pt.x <= caret.right());
            if parent && (on_caret || (row.node.dest.is_none() && row.node.uri.is_none())) {
                toggle = Some(row.key);
            } else {
                go = Some((row.node.dest, row.node.uri.clone()));
            }
        }
        if resp.double_clicked() && parent {
            toggle = Some(row.key);
        }
    }
    drop(walk);
    st.outline_scroll = scroll;
    if let Some(k) = toggle
        && !st.expanded.remove(&k)
    {
        st.expanded.insert(k);
    }
    if let Some((dest, uri)) = go {
        if let Some(d) = dest {
            doc.go_to_dest(d);
            doc.focus_request = true;
        } else if let Some(u) = uri {
            super::canvas::web_link(doc, &u, st, out);
        }
    }
}

//! The page canvas: white pages with a popup shadow on the sunken background, the base
//! bitmap first and sharp tiles over it (a spinner only while a page has nothing yet),
//! selection / match highlights, slim scrollbars, and every pointer gesture: wheel
//! scroll, Ctrl+wheel / pinch zoom around the pointer, middle-drag pan, left-drag text
//! selection (double = word, triple = line, Shift extends), links, the context menu.
//! Also the opening / locked / failed states and the "open this link?" bar.

use egui::accesskit::Role;
use egui::{
    Color32, CornerRadius, CursorIcon, EventFilter, PointerButton, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2,
};

use super::super::widgets::{self as ew, SCROLLBAR};
use super::{PdfEnv, PdfOutput, ViewState};
use crate::editors::pdf::{Hover, LinkAction, PdfDoc, Status};
use crate::services::pdf::nav::is_web_uri;
use crate::theme::icons::ph;
use crate::ui::gfx::TexExt;
use soyuz_core::config::PdfExternalLinks;

const UV: Rect = Rect { min: Pos2::ZERO, max: pos2(1.0, 1.0) };

/// What the context menu asked for.
enum MenuCmd {
    Copy,
    SelectAll,
    CopyPage(u32),
    FindSelection,
    Goto,
}

/// A web link from the outline or a page: open, ask or refuse per `[pdf] external_links`.
pub(super) fn web_link(doc: &PdfDoc, uri: &str, st: &mut ViewState, out: &mut PdfOutput) {
    if !is_web_uri(uri) {
        // Refused there, with a notice.
        out.open_link = Some(uri.to_string());
        return;
    }
    match doc.external_links() {
        PdfExternalLinks::Off => {}
        PdfExternalLinks::Ask => st.ask_link = Some(uri.to_string()),
        PdfExternalLinks::Open => out.open_link = Some(uri.to_string()),
    }
}

fn link_result(a: LinkAction, st: &mut ViewState, out: &mut PdfOutput) {
    match a {
        LinkAction::Jumped => {}
        LinkAction::Ask(u) => st.ask_link = Some(u),
        LinkAction::Open(u) | LinkAction::Blocked(u) => out.open_link = Some(u),
    }
}

fn to_rect(origin: Pos2, r: [f32; 4]) -> Rect {
    Rect::from_min_max(origin + vec2(r[0], r[1]), origin + vec2(r[2], r[3]))
}

pub(super) fn show(
    doc: &mut PdfDoc,
    ui: &mut Ui,
    area: Rect,
    env: &mut PdfEnv<'_>,
    st: &mut ViewState,
    out: &mut PdfOutput,
) {
    let p = env.p;
    let cid = env.id.with("pdf.canvas");
    env.focus.push(cid);
    let resp = ui.interact(area, cid, Sense::click_and_drag());
    crate::ui::a11y::container(ui.ctx(), resp.id, Role::Document, &tr!("pdf-ui-document", name = env.name));
    let just_focused = std::mem::take(&mut doc.focus_request) && !resp.has_focus();
    if just_focused {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    ui.painter().rect_filled(area, CornerRadius::ZERO, p.bg_sunken);
    doc.set_viewport([area.width(), area.height()], ui.ctx().pixels_per_point());

    match doc.status().clone() {
        Status::Opening => {
            doc.pump();
            crate::ui::widgets::spinner(ui, area.center(), 12.0, p.accent);
            return;
        }
        Status::Failed(e) => {
            doc.pump();
            let id = cid.with("state");
            if let Some(wrong) = doc.locked() {
                let body = if wrong { tr!("pdf-ui-locked-wrong") } else { tr!("pdf-ui-locked-body") };
                let b = (ph::KEY, tr!("pdf-ui-unlock"));
                if super::card(ui, area, id, p, ph::LOCK_SIMPLE, p.accent, tr!("pdf-ui-locked"), body, Some(b)) {
                    out.ask_password = true;
                }
            } else {
                let b = (ph::ARROWS_CLOCKWISE, tr!("pdf-ui-retry"));
                let msg = e.to_string();
                if super::card(ui, area, id, p, ph::WARNING, p.danger, tr!("pdf-ui-failed"), &msg, Some(b)) {
                    doc.reload();
                }
            }
            return;
        }
        Status::Ready => {}
    }

    let ptr = ui.input(|i| i.pointer.hover_pos());
    let local = |pt: Pos2| [pt.x - area.left(), pt.y - area.top()];
    // ---- wheel, pinch ----
    if resp.hovered() {
        let (scroll, zoom) = ui.input_mut(|i| (std::mem::take(&mut i.smooth_scroll_delta), i.zoom_delta()));
        if (zoom - 1.0).abs() > f32::EPSILON {
            if let Some(pt) = ptr {
                doc.zoom_by(zoom, local(pt));
            }
        } else if scroll != Vec2::ZERO {
            doc.scroll_by([-scroll.x, -scroll.y]);
        }
    }
    // ---- middle-drag pan ----
    if resp.dragged_by(PointerButton::Middle) {
        let d = resp.drag_delta();
        doc.scroll_by([-d.x, -d.y]);
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    // ---- left button: links and selection ----
    let (pressed, released, down, shift, now, moved, ipos) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_released(),
            i.pointer.primary_down(),
            i.modifiers.shift,
            i.time,
            i.pointer.delta() != Vec2::ZERO,
            i.pointer.interact_pos(),
        )
    });
    if pressed
        && resp.hovered()
        && let Some(pt) = ptr
    {
        let pos = local(pt);
        if doc.hover(pos) == Hover::Link && !shift {
            st.link_press = Some(pos);
        } else {
            st.clicks = super::click_count(st.last_press, st.clicks, now, pt);
            st.last_press = Some((now, pt));
            doc.pointer_down(pos, st.clicks, shift);
            st.selecting = true;
        }
    }
    if st.selecting
        && down
        && let Some(pt) = ipos
    {
        // Past the top / bottom edge: scroll along.
        let edge = if pt.y < area.top() {
            pt.y - area.top()
        } else if pt.y > area.bottom() {
            pt.y - area.bottom()
        } else {
            0.0
        };
        if edge != 0.0 {
            doc.scroll_by([0.0, edge.clamp(-40.0, 40.0)]);
            ui.ctx().request_repaint();
        }
        if moved || edge != 0.0 {
            let clamped = pos2(pt.x.clamp(area.left(), area.right()), pt.y.clamp(area.top(), area.bottom()));
            doc.pointer_drag(local(clamped));
        }
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    if released {
        if std::mem::take(&mut st.selecting) {
            doc.pointer_up();
        }
        if let Some(pos) = st.link_press.take()
            && ipos.is_some_and(|pt| {
                let l = local(pt);
                (l[0] - pos[0]).abs() < 6.0 && (l[1] - pos[1]).abs() < 6.0
            })
            && let Some(a) = doc.click_link(pos)
        {
            link_result(a, st, out);
        }
    }
    // ---- cursor ----
    if resp.hovered()
        && !st.selecting
        && !resp.dragged_by(PointerButton::Middle)
        && let Some(pt) = ptr
    {
        match doc.hover(local(pt)) {
            Hover::Link => ui.ctx().set_cursor_icon(CursorIcon::PointingHand),
            Hover::Text => ui.ctx().set_cursor_icon(CursorIcon::Text),
            Hover::Nothing => {}
        }
    }
    // ---- keys ----
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                cid,
                EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: false },
            )
        });
        if !just_focused {
            for ev in super::super::edit_inputs(ui) {
                if let Some(t) = doc.input(ev) {
                    ui.ctx().copy_text(t);
                }
            }
        }
    }
    // ---- context menu ----
    if resp.secondary_clicked()
        && let Some(pt) = ptr
    {
        st.menu_page = doc.view_to_page(local(pt), true).map(|(pg, _)| pg);
    }
    let mut menu_cmd = None;
    let has_sel = doc.selection_len() > 0;
    let menu_page = st.menu_page;
    crate::ui::native::popup::context_menu(&resp, |ui| {
        use crate::ui::widgets::{menu_item, menu_separator};
        ui.set_width(240.0);
        let mut item = |ui: &mut Ui, glyph: &str, label: &str, key: Option<&str>, on: bool, c: MenuCmd| {
            if menu_item(ui, Some(glyph), label, key, on).clicked() {
                menu_cmd = Some(c);
                ui.close();
            }
        };
        item(ui, ph::COPY, tr!("pdf-ui-copy"), Some("Ctrl+C"), has_sel, MenuCmd::Copy);
        item(ui, ph::SELECTION_ALL, tr!("pdf-ui-select-all"), Some("Ctrl+A"), true, MenuCmd::SelectAll);
        if let Some(pg) = menu_page {
            item(ui, ph::FILE_TEXT, tr!("pdf-ui-copy-page"), None, true, MenuCmd::CopyPage(pg));
        }
        menu_separator(ui);
        item(ui, ph::MAGNIFYING_GLASS, tr!("pdf-ui-find-selection"), None, has_sel, MenuCmd::FindSelection);
        item(ui, ph::ARROW_LINE_DOWN, tr!("pdf-ui-goto"), None, true, MenuCmd::Goto);
    });
    match menu_cmd {
        Some(MenuCmd::Copy) => {
            if let Some(t) = doc.copy() {
                ui.ctx().copy_text(t);
            }
        }
        Some(MenuCmd::SelectAll) => doc.select_all(),
        Some(MenuCmd::CopyPage(pg)) => {
            doc.select_page(pg);
            if let Some(t) = doc.copy() {
                ui.ctx().copy_text(t);
            }
        }
        Some(MenuCmd::FindSelection) => doc.open_find(),
        Some(MenuCmd::Goto) => doc.open_goto(),
        None => {}
    }

    // ---- render requests, then paint ----
    doc.pump();
    paint(doc, ui, area, env, cid);
    // ---- scrollbars ----
    let size = doc.doc_size();
    let s = doc.scroll();
    let wide = size[0] > area.width() + 0.5;
    let vbar = Rect::from_min_max(
        pos2(area.right() - SCROLLBAR, area.top()),
        pos2(area.right(), area.bottom() - if wide { SCROLLBAR } else { 0.0 }),
    );
    let mut y = s[1] as f64;
    ew::vscroll(ui, vbar, cid.with("vbar"), size[1] as f64, area.height() as f64, &mut y, p);
    let mut x = s[0] as f64;
    if wide {
        let hbar = Rect::from_min_max(pos2(area.left(), area.bottom() - SCROLLBAR), pos2(vbar.left(), area.bottom()));
        ew::hscroll(ui, hbar, cid.with("hbar"), size[0] as f64, area.width() as f64, &mut x, p);
    }
    if (x as f32 - s[0]).abs() > 0.01 || (y as f32 - s[1]).abs() > 0.01 {
        doc.set_scroll([x as f32, y as f32]);
    }
    // ---- "open this link?" ----
    if let Some(url) = st.ask_link.clone() {
        let r =
            Rect::from_min_size(pos2(area.left() + 8.0, area.top() + 6.0), vec2(area.width() - 16.0 - SCROLLBAR, 30.0));
        let text = tr!("pdf-ui-link-ask", url = url.as_str());
        match ew::banner(ui, r, p, ph::LINK, p.info, &text, &[tr!("pdf-ui-link-open"), tr!("common-cancel")]) {
            Some(0) => {
                out.open_link = Some(url);
                st.ask_link = None;
            }
            Some(_) => st.ask_link = None,
            None => {}
        }
    }
}

/// Pages, highlights, per-page cards and accessibility nodes.
fn paint(doc: &mut PdfDoc, ui: &mut Ui, area: Rect, env: &mut PdfEnv<'_>, cid: egui::Id) {
    let p = env.p;
    let painter = ui.painter_at(area);
    let shadow = p.shadow(crate::theme::tokens::shadow::POPUP);
    let paper = if doc.invert() { Color32::from_gray(24) } else { Color32::WHITE };
    let sel_fill = p.accent.gamma_multiply(0.35);
    let match_fill = p.warning.gamma_multiply(0.35);
    let a11y = crate::ui::a11y::active(ui.ctx());
    let mut retry = None;
    for pv in doc.visible_pages() {
        let r = to_rect(area.min, pv.rect);
        painter.add(shadow.as_shape(r, CornerRadius::ZERO));
        painter.rect_filled(r, CornerRadius::ZERO, paper);
        if let Some(b) = &pv.base {
            painter.image(b.id(), r, UV, Color32::WHITE);
        }
        for (t, tr) in &pv.tiles {
            painter.image(t.id(), to_rect(area.min, *tr), UV, Color32::WHITE);
        }
        let vis = r.intersect(area);
        if let Some(e) = &pv.failed {
            let id = cid.with(("failed", pv.page));
            let b = (ph::ARROWS_CLOCKWISE, tr!("pdf-ui-retry"));
            let msg = e.to_string();
            if vis.height() > 120.0
                && super::card(ui, vis, id, p, ph::WARNING, p.danger, tr!("pdf-ui-page-failed"), &msg, Some(b))
            {
                retry = Some(pv.page);
            }
        } else if pv.base.is_none() && pv.tiles.is_empty() && vis.is_positive() {
            crate::ui::widgets::spinner(ui, vis.center(), 10.0, p.accent);
        }
        for m in &pv.matches {
            painter.rect_filled(to_rect(area.min, *m), CornerRadius::same(2), match_fill);
        }
        for m in &pv.current {
            let mr = to_rect(area.min, *m).expand(1.0);
            painter.rect_filled(mr, CornerRadius::same(2), match_fill);
            painter.rect_stroke(mr, CornerRadius::same(2), Stroke::new(1.5, p.accent), egui::StrokeKind::Outside);
        }
        for s in &pv.selection {
            painter.rect_filled(to_rect(area.min, *s), CornerRadius::ZERO, sel_fill);
        }
        if a11y && vis.is_positive() {
            let label = doc.page_label(pv.page);
            crate::ui::a11y::item(ui, cid.with(("page", pv.page)), vis, Role::Group, &label, doc.page_text(pv.page));
        }
    }
    if let Some(pg) = retry {
        doc.retry_page(pg);
    }
}

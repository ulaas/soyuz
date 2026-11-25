//! The viewer's toolbar: sidebar toggle, previous / next page, the editable page field
//! ("12 / 340"), zoom out / menu / in, rotate, dark pages, find and the overflow menu.
//! Narrow panels drop the zoom menu's width first, then rotate and dark pages (both
//! stay in the overflow menu).

use egui::{Align2, CornerRadius, Id, Key, Rect, Response, Sense, Stroke, Ui, pos2, vec2};
use soyuz_core::config::{Action, PdfSidebar};

use super::{PdfEnv, PdfOutput, ZoomPick};
use crate::editors::pdf::{PdfDoc, ZOOM_STEPS, Zoom};
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons, icons::ph};
use crate::ui::widgets::dropdown as dd;

const BTN: f32 = 26.0;
const STEP: f32 = 28.0;
const GAP: f32 = 8.0;

/// Which toolbar parts fit in a width. Always shown: the sidebar toggle, the page
/// field, find and the overflow menu (which also has everything that is dropped).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Plan {
    /// Width of the page field.
    field: f32,
    prev_next: bool,
    zoom_buttons: bool,
    /// Width of the zoom menu.
    zoom_menu: Option<f32>,
    rotate_dark: bool,
}

impl Plan {
    fn fit(width: f32) -> Plan {
        // Edges, sidebar toggle + gap, find, more, the gap before the zoom group.
        let mut room = width - (6.0 + STEP + GAP) - (2.0 * STEP + 6.0) - GAP;
        let field = if room >= 92.0 { 92.0 } else { 64.0 };
        room -= field;
        let mut take = |w: f32| {
            let ok = room >= w;
            if ok {
                room -= w;
            }
            ok
        };
        let prev_next = take(2.0 * STEP);
        let zoom_buttons = prev_next && take(2.0 * STEP + 8.0);
        let mut zoom_menu = (zoom_buttons && take(64.0)).then_some(64.0);
        let rotate_dark = zoom_menu.is_some() && take(2.0 * STEP + GAP);
        if let Some(z) = &mut zoom_menu {
            *z += room.clamp(0.0, 54.0);
        }
        Plan { field, prev_next, zoom_buttons, zoom_menu, rotate_dark }
    }
}

/// An icon button (`on`: a toggle and its state).
#[allow(clippy::too_many_arguments)]
fn tool_btn(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    glyph: &str,
    tip: &str,
    enabled: bool,
    on: Option<bool>,
    p: &Palette,
) -> Response {
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    match on {
        Some(on) => crate::ui::a11y::toggle(&resp, tip, on),
        None => crate::ui::a11y::button(&resp, tip),
    }
    let r = crate::theme::tokens::radius::same(radius::CONTROL);
    if on == Some(true) {
        ui.painter().rect_filled(rect, r, p.accent_subtle);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, r, p.bg_surface2);
    }
    if resp.has_focus() {
        crate::ui::widgets::focus_ring(ui, rect);
    }
    let color = if !enabled {
        p.text_tertiary.gamma_multiply(0.6)
    } else if on == Some(true) {
        p.accent
    } else if resp.hovered() {
        p.text_primary
    } else {
        p.text_secondary
    };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, icons::font(14.0), color);
    resp.on_hover_text(tip)
}

/// What a toolbar click asks for.
#[derive(Clone)]
enum Cmd {
    Action(Action),
    Zoom(ZoomPick),
    Goto,
    Copy,
    SelectAll,
    OpenHex,
    Reveal,
}

pub(super) fn show(doc: &mut PdfDoc, ui: &mut Ui, r: Rect, env: &mut PdfEnv<'_>, out: &mut PdfOutput) {
    let p = env.p;
    let id = env.id.with("pdf.tb");
    ui.painter().rect_filled(r, CornerRadius::ZERO, p.bg_surface1);
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
    let ready = doc.is_ready();
    let top = r.center().y - BTN / 2.0;
    let at = |x: f32| Rect::from_min_size(pos2(x, top), vec2(BTN, BTN));
    let mut cmds: Vec<Cmd> = Vec::new();
    let plan = Plan::fit(r.width());

    // ---- left: sidebar, pages, zoom ----
    let mut x = r.left() + 6.0;
    let sb_on = doc.sidebar != PdfSidebar::None;
    let tip = env.tip(tr!("pdf-ui-sidebar"), &Action::PdfToggleSidebar);
    if tool_btn(ui, at(x), id.with("sidebar"), ph::SIDEBAR_SIMPLE, &tip, ready, Some(sb_on), p).clicked() {
        cmds.push(Cmd::Action(Action::PdfToggleSidebar));
    }
    x += STEP + GAP;
    let page = doc.page();
    let pages = doc.page_count();
    if plan.prev_next {
        let tip = env.tip(tr!("pdf-ui-prev-page"), &Action::PdfPrevPage);
        if tool_btn(ui, at(x), id.with("prev"), ph::CARET_LEFT, &tip, ready && page > 0, None, p).clicked() {
            cmds.push(Cmd::Action(Action::PdfPrevPage));
        }
        x += STEP;
    }
    let fr = Rect::from_min_size(pos2(x + 2.0, r.center().y - 12.0), vec2(plan.field - 4.0, 24.0));
    page_field(doc, ui, fr, env, &mut cmds);
    x += plan.field;
    if plan.prev_next {
        let tip = env.tip(tr!("pdf-ui-next-page"), &Action::PdfNextPage);
        let last = page + 1 >= pages;
        if tool_btn(ui, at(x), id.with("next"), ph::CARET_RIGHT, &tip, ready && !last, None, p).clicked() {
            cmds.push(Cmd::Action(Action::PdfNextPage));
        }
        x += STEP;
    }
    x += GAP;
    if plan.zoom_buttons {
        let tip = env.tip(tr!("pdf-ui-zoom-out"), &Action::PdfZoomOut);
        if tool_btn(ui, at(x), id.with("zout"), ph::MAGNIFYING_GLASS_MINUS, &tip, ready, None, p).clicked() {
            cmds.push(Cmd::Action(Action::PdfZoomOut));
        }
        x += STEP + 4.0;
    }
    if let Some(zoom_w) = plan.zoom_menu {
        if ready {
            let zr = Rect::from_min_size(pos2(x, r.center().y - 13.0), vec2(zoom_w, 26.0));
            if let Some(z) = zoom_menu(doc, ui, zr, env) {
                cmds.push(Cmd::Zoom(z));
            }
        }
        x += zoom_w + 4.0;
    }
    if plan.zoom_buttons {
        let tip = env.tip(tr!("pdf-ui-zoom-in"), &Action::PdfZoomIn);
        if tool_btn(ui, at(x), id.with("zin"), ph::MAGNIFYING_GLASS_PLUS, &tip, ready, None, p).clicked() {
            cmds.push(Cmd::Action(Action::PdfZoomIn));
        }
    }

    // ---- right: rotate, dark pages, find, more ----
    let mut rx = r.right() - 6.0 - BTN;
    let more = tool_btn(ui, at(rx), id.with("more"), ph::DOTS_THREE_VERTICAL, tr!("pdf-ui-more"), true, None, p);
    rx -= STEP;
    let tip = env.tip(tr!("pdf-ui-find"), &Action::EditorFind);
    if tool_btn(ui, at(rx), id.with("find"), ph::MAGNIFYING_GLASS, &tip, ready, Some(doc.find.open), p).clicked() {
        if doc.find.open {
            doc.close_find();
            doc.focus_request = true;
        } else {
            cmds.push(Cmd::Action(Action::EditorFind));
        }
    }
    if plan.rotate_dark {
        rx -= STEP + GAP;
        let tip = env.tip(tr!("pdf-ui-dark-pages"), &Action::PdfToggleDarkPages);
        if tool_btn(ui, at(rx), id.with("dark"), ph::MOON, &tip, ready, Some(doc.invert()), p).clicked() {
            cmds.push(Cmd::Action(Action::PdfToggleDarkPages));
        }
        rx -= STEP;
        let tip = env.tip(tr!("pdf-ui-rotate-cw"), &Action::PdfRotateCw);
        if tool_btn(ui, at(rx), id.with("rot"), ph::ARROW_CLOCKWISE, &tip, ready, None, p).clicked() {
            cmds.push(Cmd::Action(Action::PdfRotateCw));
        }
    }
    let has_sel = doc.selection_len() > 0;
    let dark = doc.invert();
    crate::ui::native::popup::Menu::menu(&more).id(id.with("more.menu")).show(|ui| {
        use crate::ui::widgets::{menu_item, menu_separator};
        ui.set_width(240.0);
        let mut item = |ui: &mut Ui, glyph: &str, label: &str, key: Option<&str>, on: bool, c: Cmd| {
            if menu_item(ui, Some(glyph), label, key, on).clicked() {
                cmds.push(c);
                ui.close();
            }
        };
        item(ui, ph::COPY, tr!("pdf-ui-copy"), Some("Ctrl+C"), ready && has_sel, Cmd::Copy);
        item(ui, ph::SELECTION_ALL, tr!("pdf-ui-select-all"), Some("Ctrl+A"), ready, Cmd::SelectAll);
        item(ui, ph::ARROW_LINE_DOWN, tr!("pdf-ui-goto"), None, ready, Cmd::Goto);
        menu_separator(ui);
        item(ui, ph::ARROWS_OUT_LINE_HORIZONTAL, tr!("pdf-ui-fit-width"), None, ready, Cmd::Zoom(ZoomPick::Width));
        item(ui, ph::CORNERS_OUT, tr!("pdf-ui-fit-page"), None, ready, Cmd::Zoom(ZoomPick::Page));
        item(ui, ph::MAGNIFYING_GLASS, tr!("pdf-ui-actual-size"), None, ready, Cmd::Action(Action::PdfZoomActual));
        item(ui, ph::ARROW_CLOCKWISE, tr!("pdf-ui-rotate-cw"), None, ready, Cmd::Action(Action::PdfRotateCw));
        item(ui, ph::ARROW_COUNTER_CLOCKWISE, tr!("pdf-ui-rotate-ccw"), None, ready, Cmd::Action(Action::PdfRotateCcw));
        let moon = if dark { ph::CHECK } else { ph::MOON };
        item(ui, moon, tr!("pdf-ui-dark-pages"), None, ready, Cmd::Action(Action::PdfToggleDarkPages));
        menu_separator(ui);
        item(ui, ph::BINARY, tr!("editors-tab-reopen-hex"), None, true, Cmd::OpenHex);
        item(ui, ph::ARROW_SQUARE_OUT, tr!("editors-tab-reveal"), None, true, Cmd::Reveal);
    });

    for c in cmds {
        match c {
            Cmd::Action(a) => {
                doc.command(&a);
                if !matches!(a, Action::EditorFind) {
                    doc.focus_request = true;
                }
            }
            Cmd::Zoom(z) => {
                doc.zoom_to(z.zoom(), None);
                doc.focus_request = true;
            }
            Cmd::Goto => doc.open_goto(),
            Cmd::Copy => {
                if let Some(t) = doc.copy() {
                    ui.ctx().copy_text(t);
                }
            }
            Cmd::SelectAll => doc.select_all(),
            Cmd::OpenHex => out.open_hex = true,
            Cmd::Reveal => out.reveal = true,
        }
    }
}

/// "12 / 340": a button that turns into a number field (the go-to field, Ctrl+G).
fn page_field(doc: &mut PdfDoc, ui: &mut Ui, fr: Rect, env: &mut PdfEnv<'_>, cmds: &mut Vec<Cmd>) {
    let p = env.p;
    let fid = env.id.with("pdf.goto");
    let page = doc.page();
    let pages = doc.page_count();
    if doc.goto.is_some() {
        env.focus.push(fid);
        let want_focus = std::mem::take(&mut doc.goto_focus);
        let Some(text) = doc.goto.as_mut() else { return };
        if want_focus && text.is_empty() {
            *text = (page + 1).to_string();
        }
        let resp = super::super::widgets::field(ui, fr, fid, text, tr!("pdf-ui-goto-hint"), false);
        if want_focus {
            resp.request_focus();
            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), fid) {
                let n = text.chars().count();
                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(n),
                )));
                st.store(ui.ctx(), fid);
            }
        }
        doc.field_focus |= resp.has_focus();
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        if (resp.has_focus() || resp.lost_focus()) && enter {
            let t = text.clone();
            if !doc.submit_goto(&t) {
                resp.request_focus();
            }
        } else if resp.lost_focus() && !want_focus {
            doc.goto = None;
        }
        ui.painter().text(
            pos2(fr.right() - 6.0, fr.center().y),
            Align2::RIGHT_CENTER,
            format!("/ {pages}"),
            crate::theme::font::ui(11.5),
            p.text_tertiary,
        );
        return;
    }
    let resp = ui.interact(fr, fid.with("button"), if doc.is_ready() { Sense::click() } else { Sense::hover() });
    let label = if doc.is_ready() { doc.page_label(page) } else { String::new() };
    crate::ui::a11y::button(&resp, &label);
    let rr = crate::theme::tokens::radius::same(radius::CONTROL);
    let fill = if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 };
    ui.painter().rect_filled(fr, rr, fill);
    ui.painter().rect_stroke(fr, rr, Stroke::new(1.0, p.border_subtle), egui::StrokeKind::Inside);
    if resp.has_focus() {
        crate::ui::widgets::focus_ring(ui, fr);
    }
    if doc.is_ready() {
        let text = tr!("pdf-ui-page-count", page = (page + 1).to_string(), pages = pages.to_string());
        let g = crate::ui::widgets::trunc_galley(
            ui.painter(),
            &text,
            crate::theme::font::numbers(12.0),
            p.text_primary,
            fr.width() - 8.0,
        );
        ui.painter().galley(fr.center() - g.size() / 2.0, g, p.text_primary);
    }
    let tip = env.tip(tr!("pdf-ui-goto"), &Action::EditorGoTo);
    if resp.on_hover_text(tip).clicked() {
        cmds.push(Cmd::Goto);
    }
}

/// The zoom menu: fit width / fit page and the preset percentages; the button shows
/// the current zoom.
fn zoom_menu(doc: &PdfDoc, ui: &mut Ui, zr: Rect, env: &PdfEnv<'_>) -> Option<ZoomPick> {
    let mut items: Vec<dd::Item<ZoomPick>> = Vec::with_capacity(ZOOM_STEPS.len() + 2);
    items.push(dd::Item::new(ZoomPick::Width, tr!("pdf-ui-fit-width")).icon(ph::ARROWS_OUT_LINE_HORIZONTAL));
    items.push(dd::Item::new(ZoomPick::Page, tr!("pdf-ui-fit-page")).icon(ph::CORNERS_OUT));
    for s in ZOOM_STEPS {
        let n = s as u32;
        items.push(dd::Item::new(ZoomPick::Percent(n), tr!("pdf-zoom-percent", percent = n.to_string())));
    }
    // Fit modes and in-between zooms show their percentage on the button (placeholder).
    let fit = matches!(doc.zoom(), Zoom::FitWidth | Zoom::FitPage);
    let mut value = if fit { ZoomPick::Percent(0) } else { ZoomPick::of(doc.zoom(), doc.percent()) };
    let before = value;
    let mut c = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(zr)
            .id_salt(env.id.with("pdf.zoom"))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let o = dd::Dropdown::new(format!("pdf.zoom.{:?}", env.id))
        .width(zr.width())
        .name(tr!("pdf-ui-zoom"))
        .search(false)
        .placeholder(doc.zoom_label())
        .show(&mut c, &mut value, &items);
    (o.changed && value != before).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::Plan;

    #[test]
    fn toolbar_drops_parts_as_it_narrows() {
        let wide = Plan::fit(900.0);
        assert!(wide.prev_next && wide.zoom_buttons && wide.rotate_dark);
        assert_eq!((wide.field, wide.zoom_menu), (92.0, Some(118.0)));
        let mid = Plan::fit(470.0);
        assert!(mid.rotate_dark && mid.zoom_menu.is_some_and(|z| (64.0..118.0).contains(&z)));
        let narrow = Plan::fit(380.0);
        assert!(narrow.zoom_buttons && !narrow.rotate_dark, "{narrow:?}");
        let tiny = Plan::fit(200.0);
        assert!(!tiny.prev_next && tiny.zoom_menu.is_none() && tiny.field == 64.0, "{tiny:?}");
        // Nothing overlaps: what is shown fits.
        for w in (150..1000).step_by(7) {
            let p = Plan::fit(w as f32);
            let used = 6.0
                + 28.0
                + 8.0
                + p.field
                + if p.prev_next { 56.0 } else { 0.0 }
                + 8.0
                + if p.zoom_buttons { 64.0 } else { 0.0 }
                + p.zoom_menu.unwrap_or(0.0)
                + if p.rotate_dark { 64.0 } else { 0.0 }
                + 62.0;
            assert!(used <= (w as f32).max(204.0) + 0.5, "{w}: {p:?} uses {used}");
        }
    }
}

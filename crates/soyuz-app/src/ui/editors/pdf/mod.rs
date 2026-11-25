//! Draws the PDF viewer ([`crate::editors::pdf::PdfDoc`]): the toolbar ([`toolbar`]),
//! the find bar ([`findbar`]), the optional sidebar with thumbnails or the outline
//! ([`sidebar`]), the page canvas ([`canvas`]) and the status bar.
//!
//! The model does the work (layout, zoom, render requests, find, selection, links); this
//! module hands it the viewport and the input every frame and paints what
//! [`PdfDoc::visible_pages`] returns. No I/O happens here.

mod canvas;
mod findbar;
mod sidebar;
mod toolbar;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use egui::{Align2, Id, Pos2, Rect, Stroke, Ui, pos2, vec2};
use soyuz_core::config::{Action, Keymap, PdfSidebar};

use crate::editors::pdf::{PdfDoc, PdfFind, Zoom};
use crate::theme::Palette;

fn toolbar_h() -> f32 {
    crate::theme::font::grow(38.0)
}
fn bar_h() -> f32 {
    crate::theme::font::grow(34.0)
}
fn status_h() -> f32 {
    crate::theme::font::grow(24.0)
}

/// What the surrounding panel does after a frame (things that need the whole app).
#[derive(Default, Debug, PartialEq)]
pub struct PdfOutput {
    /// Ask for the password of the locked document.
    pub ask_password: bool,
    /// Open this link (confirmed, or `[pdf] external_links = "open"`; non-web links
    /// are refused there with a notice).
    pub open_link: Option<String>,
    /// Reopen the document in the hex editor.
    pub open_hex: bool,
    /// Show the file in its folder.
    pub reveal: bool,
}

/// Theme, ids and settings for one frame.
pub struct PdfEnv<'a> {
    pub p: &'a Palette,
    /// Focusable widget ids of the editor (keys go to the editor while one has focus).
    pub focus: &'a mut Vec<Id>,
    pub id: Id,
    /// `[pdf] sidebar_width` (clamped).
    pub sidebar_width: f32,
    /// The document's file name (accessibility).
    pub name: &'a str,
    pub keymap: &'a Keymap,
}

impl PdfEnv<'_> {
    /// `label` with the key bound to `a` ("Zoom in (+)").
    fn tip(&self, label: &str, a: &Action) -> String {
        match crate::keys::chord_label(self.keymap, a) {
            Some(k) => format!("{label} ({k})"),
            None => label.to_string(),
        }
    }
}

/// Per-document view state that is not part of the model (pointer gestures, sidebar
/// scroll, outline expansion, a link waiting for confirmation).
#[derive(Default)]
struct ViewState {
    /// Time and place of the last primary press (double / triple click).
    last_press: Option<(f64, Pos2)>,
    clicks: u8,
    /// A primary press started on a link (viewport coordinates).
    link_press: Option<[f32; 2]>,
    /// A text selection drag is running.
    selecting: bool,
    /// A web link waiting for "Open link" (`[pdf] external_links = "ask"`).
    ask_link: Option<String>,
    /// The page the context menu was opened on.
    menu_page: Option<u32>,
    thumb_scroll: f32,
    outline_scroll: f32,
    /// Outline nodes shown expanded (keys from their index path).
    expanded: HashSet<u64>,
    outline_init: bool,
    /// The page the thumbnail list last scrolled to.
    follow_page: Option<u32>,
}

type Shared = Arc<Mutex<ViewState>>;

fn view_state(ctx: &egui::Context, id: Id) -> Shared {
    ctx.data_mut(|d| d.get_temp_mut_or_insert_with(id.with("pdf.state"), Shared::default).clone())
}

pub fn show(doc: &mut PdfDoc, ui: &mut Ui, rect: Rect, env: &mut PdfEnv<'_>) -> PdfOutput {
    let mut out = PdfOutput::default();
    let shared = view_state(ui.ctx(), env.id);
    let mut st = shared.lock().unwrap_or_else(|e| e.into_inner());
    doc.set_theme_dark(env.p.dark);
    doc.field_focus = false;
    let p = env.p;
    // Toolbar.
    let tb = Rect::from_min_size(rect.min, vec2(rect.width(), toolbar_h()));
    toolbar::show(doc, ui, tb, env, &mut out);
    let mut top = tb.bottom();
    if doc.find.open {
        let r = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), bar_h()));
        findbar::show(doc, ui, r, env);
        top += bar_h();
    }
    let status = Rect::from_min_max(pos2(rect.left(), rect.bottom() - status_h()), rect.max);
    let mut body = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), status.top()));
    if doc.sidebar != PdfSidebar::None && doc.is_ready() && body.width() > 320.0 {
        let w = env.sidebar_width.min(body.width() * 0.45);
        let side = Rect::from_min_max(body.min, pos2(body.left() + w, body.bottom()));
        sidebar::show(doc, ui, side, env, &mut st, &mut out);
        body.min.x = side.right();
    }
    canvas::show(doc, ui, body, env, &mut st, &mut out);
    status_bar(doc, ui, status, p);
    out
}

fn status_bar(doc: &PdfDoc, ui: &mut Ui, r: Rect, p: &Palette) {
    ui.painter().rect_filled(r, egui::CornerRadius::ZERO, p.bg_surface1);
    ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border_subtle));
    if !doc.is_ready() {
        return;
    }
    let page = doc.page();
    let mut left = String::new();
    if let Some((w, h)) = doc.page_size_mm(page) {
        left.push_str(&tr!("pdf-ui-status-size", width = format!("{w:.0}"), height = format!("{h:.0}")));
        left.push_str("  ·  ");
    }
    left.push_str(&doc.zoom_label());
    if let Some(info) = doc.info() {
        left.push_str("  ·  ");
        left.push_str(info.engine.label());
    }
    let selected = doc.selection_len();
    if selected > 0 {
        left.push_str("  ·  ");
        left.push_str(&tr!("pdf-ui-status-selected", count = selected));
    }
    let font = crate::theme::font::ui(11.5);
    let right = ui.painter().layout_no_wrap(doc.page_label(page), font.clone(), p.text_secondary);
    let room = (r.width() - right.size().x - 36.0).max(40.0);
    let g = crate::ui::widgets::trunc_galley(ui.painter(), &left, font, p.text_secondary, room);
    let y = r.center().y;
    ui.painter().galley(pos2(r.left() + 10.0, y - g.size().y / 2.0), g, p.text_secondary);
    ui.painter().galley(pos2(r.right() - 10.0 - right.size().x, y - right.size().y / 2.0), right, p.text_secondary);
}

/// How "Edit" / "Open in viewer" from the preview opens `path`: PDFs in the viewer,
/// everything else as text.
pub fn edit_want(path: &std::path::Path) -> crate::editors::io::Want {
    if crate::models::archive::is_pdf(path) { crate::editors::io::Want::Auto } else { crate::editors::io::Want::Text }
}

// ---- small pure helpers (unit-tested) ------------------------------------------------

/// The zoom menu's entries.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ZoomPick {
    Width,
    Page,
    /// A fixed zoom in whole percent.
    Percent(u32),
}

impl ZoomPick {
    fn of(z: Zoom, percent: f32) -> ZoomPick {
        match z {
            Zoom::FitWidth => ZoomPick::Width,
            Zoom::FitPage => ZoomPick::Page,
            Zoom::Scale(_) => ZoomPick::Percent(percent.round().max(1.0) as u32),
        }
    }

    fn zoom(self) -> Zoom {
        match self {
            ZoomPick::Width => Zoom::FitWidth,
            ZoomPick::Page => Zoom::FitPage,
            ZoomPick::Percent(n) => Zoom::percent(n as f32),
        }
    }
}

/// Clicks in a row: a press within 0.45 s and 6 points of the previous one counts on
/// (1 → 2 → 3, then back to 1).
fn click_count(last: Option<(f64, Pos2)>, prev_count: u8, now: f64, at: Pos2) -> u8 {
    match last {
        Some((t, p)) if now - t <= 0.45 && p.distance(at) <= 6.0 && prev_count < 3 => prev_count + 1,
        _ => 1,
    }
}

/// The rows of a virtualized list of equal `cell` heights that `view` points starting at
/// `scroll` show (`n` rows in all).
fn visible_rows(scroll: f32, view: f32, cell: f32, n: u32) -> std::ops::Range<u32> {
    if n == 0 || cell <= 0.0 || view <= 0.0 {
        return 0..0;
    }
    let first = (scroll.max(0.0) / cell).floor() as u32;
    let last = ((scroll.max(0.0) + view) / cell).ceil() as u32;
    first.min(n)..last.min(n)
}

/// The scroll offset that keeps row `i` (of height `cell`) inside `view` points.
fn scroll_into_view(scroll: f32, view: f32, cell: f32, i: u32) -> f32 {
    let top = i as f32 * cell;
    if top < scroll {
        top
    } else if top + cell > scroll + view {
        (top + cell - view).max(0.0)
    } else {
        scroll
    }
}

/// The find bar's count: "3 of 12", "12 matches", "No matches" (`true`: nothing found),
/// with "…" while the sweep is still running.
fn find_status(f: &PdfFind) -> (String, bool) {
    if !f.active() {
        return (String::new(), false);
    }
    let count = f.count();
    let mut s = if count == 0 && f.done() {
        return (tr!("pdf-ui-find-none").to_string(), true);
    } else if let Some(o) = f.ordinal() {
        tr!("pdf-ui-find-of", current = o, count = count)
    } else {
        tr!("pdf-ui-find-count", count = count)
    };
    if !f.done() {
        s.push_str(" …");
    }
    (s, false)
}

/// A centred card (lock, error) with an optional button; `true` when it was clicked.
#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut Ui,
    area: Rect,
    id: Id,
    p: &Palette,
    glyph: &str,
    color: egui::Color32,
    title: &str,
    body: &str,
    button: Option<(&str, &str)>,
) -> bool {
    let w = (area.width() - 32.0).clamp(120.0, 380.0);
    let font = crate::theme::font::ui(12.0);
    let body_g = ui.painter().layout(body.to_string(), font, p.text_secondary, w - 28.0);
    let h = 14.0 + 22.0 + 6.0 + body_g.size().y + if button.is_some() { 44.0 } else { 14.0 };
    let r = Rect::from_center_size(area.center(), vec2(w, h));
    let shadow = p.shadow(crate::theme::tokens::shadow::POPUP);
    let radius = crate::theme::tokens::radius::same(crate::theme::tokens::radius::CARD);
    ui.painter().add(shadow.as_shape(r, radius));
    ui.painter().rect_filled(r, radius, p.bg_surface1);
    ui.painter().rect_stroke(r, radius, Stroke::new(1.0, p.border_subtle), egui::StrokeKind::Inside);
    let y = r.top() + 14.0 + 11.0;
    ui.painter().text(
        pos2(r.left() + 14.0, y),
        Align2::LEFT_CENTER,
        glyph,
        crate::theme::icons::font_fill(16.0),
        color,
    );
    let tg = crate::ui::widgets::trunc_galley(
        ui.painter(),
        title,
        crate::theme::font::semibold(13.0),
        p.text_primary,
        w - 50.0,
    );
    ui.painter().galley(pos2(r.left() + 38.0, y - tg.size().y / 2.0), tg, p.text_primary);
    let by = r.top() + 14.0 + 22.0 + 6.0;
    ui.painter().galley(pos2(r.left() + 14.0, by), body_g.clone(), p.text_secondary);
    let mut clicked = false;
    if let Some((icon, label)) = button {
        let bw = crate::ui::widgets::pill_width(ui, label) + 22.0;
        let br = Rect::from_min_size(pos2(r.left() + 14.0, by + body_g.size().y + 10.0), vec2(bw, 28.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(br).id_salt(id));
        clicked =
            crate::ui::widgets::pill_button(&mut c, Some(icon), label, crate::ui::widgets::Btn::Primary).clicked();
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_pick_round_trips() {
        assert_eq!(ZoomPick::of(Zoom::FitWidth, 87.0), ZoomPick::Width);
        assert_eq!(ZoomPick::of(Zoom::FitPage, 50.0), ZoomPick::Page);
        assert_eq!(ZoomPick::of(Zoom::percent(150.0), 149.6), ZoomPick::Percent(150));
        assert_eq!(ZoomPick::Percent(200).zoom(), Zoom::percent(200.0));
        assert_eq!(ZoomPick::Width.zoom(), Zoom::FitWidth);
    }

    #[test]
    fn clicks_count_up_close_together() {
        let at = pos2(10.0, 10.0);
        assert_eq!(click_count(None, 0, 1.0, at), 1);
        assert_eq!(click_count(Some((1.0, at)), 1, 1.2, at), 2);
        assert_eq!(click_count(Some((1.2, at)), 2, 1.4, pos2(12.0, 11.0)), 3);
        assert_eq!(click_count(Some((1.4, at)), 3, 1.5, at), 1, "a fourth click starts over");
        assert_eq!(click_count(Some((1.0, at)), 1, 2.0, at), 1, "too slow");
        assert_eq!(click_count(Some((1.0, at)), 1, 1.1, pos2(40.0, 10.0)), 1, "too far");
    }

    #[test]
    fn thumbnail_virtualization() {
        // 100 rows of 50 points, a 120-point window.
        assert_eq!(visible_rows(0.0, 120.0, 50.0, 100), 0..3);
        assert_eq!(visible_rows(75.0, 120.0, 50.0, 100), 1..4);
        assert_eq!(visible_rows(4950.0, 120.0, 50.0, 100), 99..100);
        assert_eq!(visible_rows(-20.0, 120.0, 50.0, 100), 0..3);
        assert_eq!(visible_rows(0.0, 120.0, 50.0, 0), 0..0);
        assert_eq!(visible_rows(0.0, 0.0, 50.0, 10), 0..0);
        // Following the current page.
        assert_eq!(scroll_into_view(0.0, 120.0, 50.0, 1), 0.0, "already visible");
        assert_eq!(scroll_into_view(0.0, 120.0, 50.0, 5), 180.0, "below: bottom-aligned");
        assert_eq!(scroll_into_view(400.0, 120.0, 50.0, 2), 100.0, "above: top-aligned");
    }

    #[test]
    fn find_status_texts() {
        let f = PdfFind::default();
        assert_eq!(find_status(&f), (String::new(), false), "no search yet");
    }
}

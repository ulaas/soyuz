//! Quick Look: the focused pane's cursor item, large, over the whole screen (Space) —
//! Phase T4.6. The GUI's `ui::quicklook` in cells.
//!
//! The loading is the core's (`models::preview_load::PreviewLoader` with `big`: bigger
//! images and rich renders), the bodies are the preview pane's (`view::preview`:
//! text and code, Markdown, hex, archive listings, folder summaries, messages); images
//! are drawn with [`crate::images::draw`], fitted in the body. [`prepare`] follows the
//! cursor and builds the model ([`QuickLookVm`], kept in the input state);
//! [`draw_quicklook`] draws a card with a one-cell margin over everything but the
//! palette, menus and dialogs:
//!
//! - the title row: `‹` (previous), the icon and name, the position (`3 / 12`) and size
//!   on the right, then `›` (next) and `×` (close);
//! - the body;
//! - the footer: type, size, dimensions and date on the left, key hints on the right.
//!
//! The whole screen is one `QuickLook` click target; [`geometry`] gives
//! `crate::input::quicklook` the card and its marks. Images under the card (the preview
//! pane's, grid thumbnails) are repaired by `crate::images::end_frame` when it closes.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::file_class::{FileClass, classify};
use soyuz_appcore::models::preview_info::{info_rows, kind_label};
use soyuz_appcore::models::preview_load::PreviewLoader;
use soyuz_appcore::services::preview::Content;
use soyuz_appcore::services::preview_gfx::Tex;
use soyuz_appcore::tr;
use soyuz_core::format_size;

use crate::dialog::safe;
use crate::images::{self, Drawn, ImageState};
use crate::input::InputState;
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};
use crate::view::preview::{self, Body, PLAIN_FG, PreviewVm};
use crate::view::text;
use crate::widgets::SelectList;

/// An image to draw in the body.
#[derive(Clone, Debug)]
pub struct QlImage {
    pub tex: Tex,
    /// The file's own size in pixels.
    pub width: u32,
    pub height: u32,
}

impl PartialEq for QlImage {
    fn eq(&self, o: &QlImage) -> bool {
        let id = |t: &Tex| t.handle::<crate::images::store::TuiTex>().map(|h| h.id());
        self.tex.size == o.tex.size && self.width == o.width && self.height == o.height && id(&self.tex) == id(&o.tex)
    }
}

impl Eq for QlImage {}

/// The Quick Look overlay as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickLookVm {
    /// The overlay is drawn.
    pub visible: bool,
    /// The item's name.
    pub title: String,
    /// Its icon's class.
    pub class: Option<FileClass>,
    /// `3 / 12`.
    pub position: String,
    /// Its size (empty for folders until their size is known).
    pub size: String,
    /// The body (in `preview.body`; drawn by the preview pane's body drawer).
    pub preview: PreviewVm,
    /// An image instead of the body.
    pub image: Option<QlImage>,
    /// The footer's summary: type, size, dimensions, duration, date.
    pub summary: String,
    /// The footer's key hints.
    pub hints: String,
}

impl QuickLookVm {
    pub fn from_core(core: &AppCore, input: &InputState) -> QuickLookVm {
        if !crate::input::quicklook::open(core, input) {
            return QuickLookVm::default();
        }
        QuickLookVm { visible: true, ..input.quicklook.vm.clone() }
    }

    /// The body's text in view (tests).
    pub fn body_text(&self) -> Vec<String> {
        self.preview.body_text()
    }
}

/// Where the parts of the overlay are on a screen `area`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    /// The card with its frame (presses outside it close Quick Look).
    pub card: Rect,
    /// The title row.
    pub header: Rect,
    pub body: Rect,
    /// The footer row (empty when there is no room).
    pub footer: Rect,
    /// The marks in the title row (empty when there is no room).
    pub prev: Rect,
    pub next: Rect,
    pub close: Rect,
}

/// The overlay's parts on `area` (the whole screen): a margin of two columns and one
/// row when there is room, a frame, the title row, the body and a footer row.
pub fn geometry(area: Rect) -> Geometry {
    let mx = if area.width >= 40 { 2 } else { 0 };
    let my = if area.height >= 12 { 1 } else { 0 };
    let card =
        Rect::new(area.x + mx, area.y + my, area.width.saturating_sub(2 * mx), area.height.saturating_sub(2 * my));
    let mut g = Geometry { card, ..Geometry::default() };
    if card.width < 3 || card.height < 3 {
        return g;
    }
    let inner = Rect::new(card.x + 1, card.y + 1, card.width - 2, card.height - 2);
    g.header = Rect { height: 1, ..inner };
    let footer = inner.height >= 4;
    g.body = Rect { y: inner.y + 1, height: inner.height - 1 - u16::from(footer), ..inner };
    if footer {
        g.footer = Rect { y: inner.bottom() - 1, height: 1, ..inner };
    }
    if inner.width >= 16 {
        let (x, y, r) = (inner.x, inner.y, inner.right());
        g.prev = Rect::new(x, y, 2, 1);
        g.next = Rect::new(r - 5, y, 2, 1);
        g.close = Rect::new(r - 2, y, 2, 1);
    }
    g
}

/// Step 1 of a frame (`view::prepare`): follow the cursor, drain the loader and build the
/// model. Lets go of everything while Quick Look is closed.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    let st = &mut input.quicklook;
    if !core.ui.quicklook.open {
        st.release();
        return;
    }
    let target = {
        let pane = core.ws.pane();
        pane.cursor.as_deref().and_then(|c| pane.entry(c)).cloned()
    };
    let Some(target) = target else {
        // Nothing under the cursor any more (the folder emptied, the pane moved away).
        soyuz_appcore::models::quicklook::close(core);
        st.release();
        return;
    };
    let (pos, count) = {
        let pane = core.ws.pane();
        (pane.cursor_pos().map_or(0, |i| i + 1), pane.visible.len())
    };
    let dark = TuiTheme::cached(core, false).is_dark();
    let loader = st.loader.get_or_insert_with(|| {
        let mut l = PreviewLoader::new(core.wake.clone(), core.uploader.clone(), true, 1.0);
        // Only pixels reach the terminal: SVG and 3D models come rasterized.
        l.terminal = true;
        l
    });
    let limits = loader.limits(&core.config, dark, PLAIN_FG);
    if loader.set_target(Some(&target), limits) {
        st.scroll = 0;
        st.archive = SelectList::new();
    }
    loader.poll();
    let g = geometry(regions.area);
    let (w, h) = (usize::from(g.body.width), usize::from(g.body.height));
    st.page = h;
    let style = core.config.general.size_style;
    let size = if target.is_dir_like() {
        loader.folder_size().map(|(b, ..)| format_size(b, style)).unwrap_or_default()
    } else {
        format_size(target.size, style)
    };
    let rows = info_rows(&target, loader.loaded.as_ref(), loader.folder_size(), &core.config);
    let mut summary: Vec<String> = Vec::new();
    for k in ["Type", "Size", "Dimensions", "Duration", "Modified"] {
        if let Some((_, v)) = rows.iter().find(|(a, _)| a == k) {
            summary.push(safe(v));
        }
    }
    let kind = loader.loaded.as_ref().map(|l| kind_label(l.kind).to_string()).unwrap_or_default();
    let mut vm = QuickLookVm {
        visible: true,
        title: safe(&target.name),
        class: Some(classify(&target)),
        position: tr!("tui-quicklook-position", pos = pos, count = count),
        size,
        summary: summary.join(" · "),
        hints: hints(),
        ..QuickLookVm::default()
    };
    vm.preview = PreviewVm {
        visible: true,
        focused: true,
        title: vm.title.clone(),
        class: vm.class,
        kind,
        hex: loader.force_hex,
        ..PreviewVm::default()
    };
    match loader.loaded.as_ref().map(|l| &l.content) {
        Some(Content::Image(img)) if !loader.force_hex => {
            vm.image = Some(QlImage { tex: img.tex.clone(), width: img.width, height: img.height });
            vm.preview.body = Body::Nothing;
        }
        _ => vm.preview.body = preview::content_body(loader, &mut st.scroll, &mut st.archive, w, h),
    }
    st.vm = vm;
}

/// The footer's key hints.
fn hints() -> String {
    [
        format!("Esc {}", tr!("views-quicklook-hint-close")),
        format!("←→ {}", tr!("views-quicklook-hint-browse")),
        format!("PgUp/PgDn {}", tr!("tui-quicklook-hint-scroll")),
        format!("Enter {}", tr!("views-quicklook-hint-open")),
    ]
    .join(" · ")
}

fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, w: u16, style: Style) {
    buf.set_stringn(x, y, s, usize::from(w), style);
}

/// Draw the overlay over `area` (the whole screen).
pub fn draw_quicklook(f: &mut Frame<'_>, area: Rect, m: &QuickLookVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || !m.visible {
        return;
    }
    hits.push(area, HitTarget::QuickLook);
    let g = geometry(area);
    // What is under the margin fades back.
    f.buffer_mut().set_style(area, Style::default().add_modifier(Modifier::DIM));
    f.render_widget(Clear, g.card);
    f.buffer_mut().set_style(g.card, theme.base().patch(theme.style(Role::Text)));
    f.render_widget(crate::borders::overlay_block(theme.borders(), theme.style(Role::BorderFocused)), g.card);
    if g.header.is_empty() {
        return;
    }
    draw_header(f.buffer_mut(), &g, m, theme);
    if !g.footer.is_empty() {
        draw_footer(f.buffer_mut(), g.footer, m, theme);
    }
    if g.body.is_empty() {
        return;
    }
    match &m.image {
        Some(img) => draw_image(f, g.body, img, theme),
        None => preview::draw_body(f.buffer_mut(), g.body, &m.preview, theme, hits),
    }
}

fn draw_header(buf: &mut Buffer, g: &Geometry, m: &QuickLookVm, theme: &TuiTheme) {
    let h = g.header;
    let mark = theme.style(Role::Accent).add_modifier(Modifier::BOLD);
    let mut left = h.x;
    let mut right = h.right();
    if !g.prev.is_empty() {
        put(buf, g.prev.x, h.y, "‹", 1, mark);
        put(buf, g.next.x + 1, h.y, "›", 1, mark);
        put(buf, g.close.x + 1, h.y, "×", 1, theme.style(Role::Muted));
        left = g.prev.right();
        right = g.next.x;
    }
    let room = usize::from(right.saturating_sub(left));
    let mut x = left;
    if let Some(c) = m.class {
        let glyph = crate::icons::class_glyph(c, theme.icons());
        let gw = text::width(glyph);
        if gw + 1 < room {
            put(buf, x, h.y, glyph, gw as u16, theme.style(Role::of_class(c)));
            x += gw as u16 + 1;
        }
    }
    let room = usize::from(right.saturating_sub(x));
    let meta = if m.size.is_empty() { m.position.clone() } else { format!("{} · {}", m.position, m.size) };
    let meta_w = text::width(&meta);
    let (title_w, meta_shown) =
        if meta_w > 0 && meta_w + 10 <= room { (room - meta_w - 2, true) } else { (room, false) };
    let title = text::truncate_name(&m.title, title_w, true);
    put(buf, x, h.y, &title, title_w as u16, theme.style(Role::Title).add_modifier(Modifier::BOLD));
    if meta_shown {
        put(buf, right - 1 - meta_w as u16, h.y, &meta, meta_w as u16, theme.style(Role::Muted));
    }
}

fn draw_footer(buf: &mut Buffer, r: Rect, m: &QuickLookVm, theme: &TuiTheme) {
    let w = usize::from(r.width);
    let hints_w = text::width(&m.hints);
    let summary_w = text::width(&m.summary);
    let hints_shown = hints_w + 2 + summary_w.min(12) <= w;
    let summary_room = if hints_shown { w - hints_w - 2 } else { w };
    let summary = text::truncate_end(&m.summary, summary_room);
    put(buf, r.x, r.y, &summary, summary_room as u16, theme.style(Role::Muted));
    if hints_shown {
        put(buf, r.right() - hints_w as u16, r.y, &m.hints, hints_w as u16, theme.style(Role::KeyHint));
    }
}

fn draw_image(f: &mut Frame<'_>, r: Rect, img: &QlImage, theme: &TuiTheme) {
    let mut state = ImageState::default();
    match images::draw(f, r, &img.tex, &mut state) {
        Drawn::Image | Drawn::Pending | Drawn::Placeholder | Drawn::NoRoom => {}
        Drawn::Missing | Drawn::Off => {
            // No image on this terminal: say what it is.
            let s = tr!("tui-quicklook-image", width = img.width, height = img.height);
            let s = text::truncate_end(&s, usize::from(r.width));
            let x = r.x + (r.width - text::width(&s) as u16) / 2;
            let y = r.y + r.height.saturating_sub(1) / 2;
            put(f.buffer_mut(), x, y, &s, r.width, theme.style(Role::Muted));
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::view::preview::PLine;

    fn draw(m: &QuickLookVm, w: u16, h: u16) -> (Vec<String>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_quicklook(f, Rect::new(0, 0, w, h), m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect();
        (lines, hits)
    }

    fn vm() -> QuickLookVm {
        QuickLookVm {
            visible: true,
            title: "notes.txt".into(),
            class: Some(FileClass::Text),
            position: "3 / 12".into(),
            size: "1 KiB".into(),
            preview: PreviewVm {
                visible: true,
                focused: true,
                body: Body::Lines {
                    lines: vec![PLine::plain("hello quick look", Role::Text)],
                    top: 0,
                    total: 1,
                    gutter: 0,
                },
                ..PreviewVm::default()
            },
            summary: "Text · 1 KiB".into(),
            hints: "Esc close".into(),
            ..QuickLookVm::default()
        }
    }

    #[test]
    fn geometry_leaves_a_margin_and_marks() {
        let g = geometry(Rect::new(0, 0, 80, 24));
        assert_eq!(g.card, Rect::new(2, 1, 76, 22));
        assert_eq!(g.header, Rect::new(3, 2, 74, 1));
        assert_eq!(g.footer, Rect::new(3, 21, 74, 1));
        assert_eq!(g.body, Rect::new(3, 3, 74, 18));
        assert_eq!(g.prev.x, 3);
        assert_eq!(g.close.right(), 77);
        assert!(g.next.right() <= g.close.x);
        let small = geometry(Rect::new(0, 0, 10, 4));
        assert_eq!(small.card, Rect::new(0, 0, 10, 4));
        assert!(small.prev.is_empty() && small.footer.is_empty());
        assert!(geometry(Rect::new(0, 0, 2, 2)).header.is_empty());
    }

    #[test]
    fn draws_title_position_body_and_footer() {
        let (lines, hits) = draw(&vm(), 60, 16);
        assert!(lines[2].contains("notes.txt") && lines[2].contains("3 / 12 · 1 KiB"), "{lines:?}");
        assert!(lines[2].contains('‹') && lines[2].contains('›') && lines[2].contains('×'), "{lines:?}");
        assert!(lines[3].contains("hello quick look"), "{lines:?}");
        assert!(lines[13].contains("Text · 1 KiB") && lines[13].contains("Esc close"), "{lines:?}");
        assert_eq!(hits.at(0, 0), Some(HitTarget::QuickLook));
        assert_eq!(hits.at(30, 8), Some(HitTarget::QuickLook));
    }

    #[test]
    fn hidden_draws_nothing() {
        let (lines, hits) = draw(&QuickLookVm::default(), 20, 4);
        assert!(lines.iter().all(|l| l.trim().is_empty()));
        assert_eq!(hits.at(1, 1), None);
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let mut m = vm();
        m.title = "日本語 👨\u{200d}👩\u{200d}👧 a very long name.txt".into();
        m.summary = "x".repeat(80);
        for w in 0..=20 {
            for h in 0..=6 {
                draw(&m, w, h);
            }
        }
    }
}

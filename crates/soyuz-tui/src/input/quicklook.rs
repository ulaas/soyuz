//! Keys and mouse of Quick Look (`TuiFocus::QuickLook`) — Phase T4.6.
//!
//! Quick Look is the core's (`models::quicklook`): the action `QuickLook` (Space; F3 in
//! the Commander preset) opens it on the focused pane's cursor item, and it follows that
//! cursor. While it is open, like the GUI (`ui::quicklook::handle_keys`):
//!
//! | Key | Does |
//! |---|---|
//! | Esc / Space | close |
//! | Left / Up, Right / Down | previous / next item (the pane's cursor moves) |
//! | Home / End | first / last item |
//! | Enter | close and open the item |
//! | PageUp / PageDown | scroll the text a page (an archive listing: its rows) |
//! | Shift+Up / Shift+Down | scroll a line |
//!
//! The `PreviewScroll*` / `PreviewPage*` actions (Alt+Shift+Up / Down, Alt+PageUp /
//! PageDown) scroll Quick Look instead of the preview pane while it is open ([`scroll`]).
//! Every other key goes to the core's bindings in the file-list context (Ctrl+Q,
//! type-ahead, the palette, …). Mouse: a press outside the card closes it, the `‹` / `›`
//! marks in its title step, `×` closes, the wheel scrolls.

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::frontend::ScrollStep;
use soyuz_appcore::models::preview_load::PreviewLoader;
use soyuz_appcore::models::quicklook::{self as ql, Step};
use soyuz_core::config::{KeyName, Mods};

use super::{InputState, TuiFocus, key_context};
use crate::app::App;
use crate::view::quicklook::{QuickLookVm, geometry};
use crate::widgets::SelectList;

/// What Quick Look keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// First line shown.
    pub scroll: usize,
    /// Cursor and scroll of an archive listing.
    pub archive: SelectList,
    /// Rows of the body last frame (a page).
    pub page: usize,
    /// The loader (`big`: Quick Look sizes), created when the overlay first opens.
    pub loader: Option<PreviewLoader>,
    /// The model built by `view::quicklook::prepare`.
    pub vm: QuickLookVm,
}

impl State {
    /// Let go of everything loaded (the overlay closed).
    pub fn release(&mut self) {
        if let Some(l) = &mut self.loader {
            l.clear();
        }
        self.vm = QuickLookVm::default();
        self.scroll = 0;
        self.archive = SelectList::new();
    }

    /// Scroll the body by `rows` (negative: up); `view::quicklook::prepare` clamps.
    pub fn scroll_by(&mut self, rows: i32) {
        match &self.vm.preview.body {
            crate::view::preview::Body::Archive { total, .. } => {
                self.archive.wheel(rows, *total, self.page.saturating_sub(2))
            }
            _ => self.scroll = self.scroll.saturating_add_signed(rows as isize),
        }
    }
}

/// Scroll Quick Look (the `PreviewScroll*` / `PreviewPage*` actions while it is open).
pub fn scroll(app: &mut App, step: ScrollStep) {
    if !app.core.ui.quicklook.open {
        return;
    }
    let st = &mut app.input.quicklook;
    let page = st.page.saturating_sub(1).max(1) as i32;
    st.scroll_by(match step {
        ScrollStep::LineUp => -1,
        ScrollStep::LineDown => 1,
        ScrollStep::PageUp => -page,
        ScrollStep::PageDown => page,
    });
}

/// Whether the overlay is open.
pub fn open(core: &AppCore, _st: &InputState) -> bool {
    core.ui.quicklook.open
}

/// Keys while it is open: its own first (in order), the rest to the core's bindings.
pub fn route(app: &mut App, events: &[Event]) {
    let mut frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    let mut i = 0;
    while i < frame.events.len() {
        if !app.core.ui.quicklook.open {
            break;
        }
        if let InputEvent::Chord(c) = &frame.events[i]
            && !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD)
            && key(app, c.key, c.mods.contains(Mods::SHIFT))
        {
            frame.consumed[i] = true;
            // Space also types a blank: not for type-ahead.
            if c.key == KeyName::Space
                && let Some(InputEvent::Text(t)) = frame.events.get(i + 1)
                && t == " "
            {
                frame.consumed[i + 1] = true;
                i += 1;
            }
        }
        i += 1;
    }
    let (kctx, text) = key_context(TuiFocus::QuickLook);
    app.core.input(&app.host, &mut frame, kctx, text);
}

/// One key (`shift`: Shift held); returns whether Quick Look used it.
fn key(app: &mut App, k: KeyName, shift: bool) -> bool {
    let st = &mut app.input.quicklook;
    let page = st.page.saturating_sub(1).max(1) as i32;
    if shift {
        match k {
            KeyName::ArrowUp => st.scroll_by(-1),
            KeyName::ArrowDown => st.scroll_by(1),
            _ => return false,
        }
        return true;
    }
    match k {
        KeyName::Escape | KeyName::Space => ql::close(&mut app.core),
        KeyName::ArrowUp | KeyName::ArrowLeft => ql::step(&mut app.core, Step::Prev),
        KeyName::ArrowDown | KeyName::ArrowRight => ql::step(&mut app.core, Step::Next),
        KeyName::Home => ql::step(&mut app.core, Step::First),
        KeyName::End => ql::step(&mut app.core, Step::Last),
        KeyName::Enter => ql::open_item(&mut app.core),
        KeyName::PageUp => st.scroll_by(-page),
        KeyName::PageDown => st.scroll_by(page),
        _ => return false,
    }
    true
}

/// A mouse event while it is open: a press outside the card closes it, the title's
/// marks step or close, the wheel scrolls.
pub fn mouse(app: &mut App, m: &MouseEvent) {
    let g = geometry(app.regions().area);
    let p = ratatui::layout::Position { x: m.column, y: m.row };
    match m.kind {
        MouseEventKind::ScrollDown => app.input.quicklook.scroll_by(super::mouse::WHEEL_ROWS),
        MouseEventKind::ScrollUp => app.input.quicklook.scroll_by(-super::mouse::WHEEL_ROWS),
        MouseEventKind::Down(MouseButton::Left) => {
            if !g.card.contains(p) || g.close.contains(p) {
                ql::close(&mut app.core);
            } else if g.prev.contains(p) {
                ql::step(&mut app.core, Step::Prev);
            } else if g.next.contains(p) {
                ql::step(&mut app.core, Step::Next);
            }
        }
        MouseEventKind::Down(_) if !g.card.contains(p) => ql::close(&mut app.core),
        _ => {}
    }
}

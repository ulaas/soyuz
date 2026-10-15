//! Keys and mouse of the preview pane (`TuiFocus::Preview`) and the preview scroll
//! actions (Phase T4).
//!
//! The pane takes the keyboard when it is clicked (like the GUI, it is not part of the
//! Tab cycle between the panes). While it has it, the core's bindings run first in the
//! global context (Alt+P, Ctrl+Q, … keep working), then:
//!
//! | Key | Archive listing | Anything else |
//! |---|---|---|
//! | Up / Down, PageUp / PageDown, Home / End | move the cursor | scroll |
//! | Enter | open the archive at that entry (the GUI's Browse) | |
//! | Esc / Tab | back to the file list | back to the file list |
//!
//! A press anywhere outside the pane gives the keyboard back to whatever was pressed.
//! The `PreviewScroll*` / `PreviewPage*` actions (Alt+Shift+Up / Down, Alt+PageUp /
//! PageDown) scroll the pane from anywhere; the wheel over it scrolls it.
//!
//! While the pane shows an open player (Phase V3b, `crate::player::active`) its core
//! context is `Player` instead of `Global`: K / J / L, ← / →, - / =, M run the `Player*`
//! actions (the global chords still work through the context's fallback). A click on the
//! status row's mark plays / pauses (`PlayerPlay`), one on the bar seeks there
//! (`PlayerSeek`).

use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::frontend::ScrollStep;
use soyuz_appcore::models::preview_load::{DiffState, PreviewLoader};
use soyuz_core::Location;
use soyuz_core::config::{KeyName, Mods};

use super::{InputState, TuiFocus, core_input};
use crate::app::App;
use crate::layout::HitTarget;
use crate::view::preview::{Body, PreviewVm};
use crate::widgets::{ListMove, SelectList};

/// What the preview pane keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The pane has the keyboard.
    pub focused: bool,
    /// First line shown.
    pub scroll: usize,
    /// Cursor and scroll of an archive listing.
    pub archive: SelectList,
    /// Rows of the body last frame (a page).
    pub page: usize,
    /// The loader (created when the pane first shows something).
    pub loader: Option<PreviewLoader>,
    /// The two-file diff and the pair it is for.
    pub diff: DiffState,
    pub diff_key: Option<(std::path::PathBuf, std::path::PathBuf)>,
    /// The model built by `view::preview::prepare`.
    pub vm: PreviewVm,
    /// The last left press on an archive row (double-click detection).
    last_press: Option<(Instant, usize)>,
}

impl State {
    /// Let go of everything loaded (the pane is hidden or has no room).
    pub fn release(&mut self) {
        if let Some(l) = &mut self.loader {
            l.clear();
        }
        if self.diff_key.take().is_some() {
            self.diff.clear();
        }
        self.vm = PreviewVm::default();
        self.scroll = 0;
        self.archive = SelectList::new();
    }
}

/// Whether the pane has the keyboard (it must be shown).
pub fn has_focus(core: &AppCore, st: &InputState) -> bool {
    st.preview.focused && crate::view::preview::shown(core)
}

/// The path the pane shows or is loading (`None` while hidden: nothing loads then).
pub fn target(app: &App) -> Option<&Path> {
    app.input.preview.loader.as_ref()?.entry.as_ref().map(|e| e.path.as_path())
}

/// Scroll the preview (the `PreviewScroll*` / `PreviewPage*` actions).
pub fn scroll(app: &mut App, step: ScrollStep) {
    if !crate::view::preview::shown(&app.core) {
        return;
    }
    let st = &mut app.input.preview;
    let page = st.page.saturating_sub(1).max(1) as i32;
    let rows = match step {
        ScrollStep::LineUp => -1,
        ScrollStep::LineDown => 1,
        ScrollStep::PageUp => -page,
        ScrollStep::PageDown => page,
    };
    scroll_by(st, rows);
}

/// Scroll the body by `rows` (negative: up); `view::preview::prepare` clamps.
fn scroll_by(st: &mut State, rows: i32) {
    match &st.vm.body {
        Body::Archive { total, .. } => st.archive.wheel(rows, *total, st.page.saturating_sub(2)),
        _ => st.scroll = st.scroll.saturating_add_signed(rows as isize),
    }
}

/// Keys while the pane has focus.
pub fn route(app: &mut App, events: &[Event]) {
    // A press elsewhere takes the keyboard away (the mouse handler ran already).
    for ev in events {
        if let Event::Mouse(m) = ev
            && matches!(m.kind, MouseEventKind::Down(_))
            && !matches!(
                app.hits.at(m.column, m.row),
                Some(
                    HitTarget::PreviewScroll | HitTarget::ArchiveRow(_) | HitTarget::PlayerPlay | HitTarget::PlayerSeek
                )
            )
        {
            app.input.preview.focused = false;
            return super::list::route(app, events);
        }
    }
    let (kctx, text) = super::key_context_of(app);
    let frame = core_input(app, events, kctx, text);
    for i in frame.live() {
        if crate::input::focus(&app.core, &app.input) != TuiFocus::Preview {
            return;
        }
        if let InputEvent::Chord(c) = &frame.events[i]
            && !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT)
        {
            key(app, c.key);
        }
    }
}

fn key(app: &mut App, k: KeyName) {
    let st = &mut app.input.preview;
    let page = st.page;
    if let Body::Archive { total, .. } = &st.vm.body {
        let total = *total;
        let rows = page.saturating_sub(2);
        let m = match k {
            KeyName::ArrowUp => Some(ListMove::Up),
            KeyName::ArrowDown => Some(ListMove::Down),
            KeyName::PageUp => Some(ListMove::PageUp),
            KeyName::PageDown => Some(ListMove::PageDown),
            KeyName::Home => Some(ListMove::Home),
            KeyName::End => Some(ListMove::End),
            _ => None,
        };
        if let Some(m) = m {
            st.archive.go(m, total, rows.saturating_sub(1));
            st.archive.show_cursor(rows);
            return;
        }
        if k == KeyName::Enter {
            let i = st.archive.cursor;
            open_row(app, i);
            return;
        }
    }
    let page = page.saturating_sub(1).max(1) as i32;
    match k {
        KeyName::ArrowUp => scroll_by(st, -1),
        KeyName::ArrowDown => scroll_by(st, 1),
        KeyName::PageUp => scroll_by(st, -page),
        KeyName::PageDown => scroll_by(st, page),
        KeyName::Home => st.scroll = 0,
        KeyName::End => st.scroll = usize::MAX,
        KeyName::Escape | KeyName::Tab => st.focused = false,
        _ => {}
    }
}

/// Open archive row `index` in the focused pane (the GUI's Browse).
fn open_row(app: &mut App, index: usize) {
    let Some(loader) = &app.input.preview.loader else { return };
    let Some((archive, inner, select)) = crate::view::preview::archive_target(loader, index) else { return };
    let pane = app.core.ws.pane().id;
    app.input.preview.focused = false;
    app.core.cmds.push(Cmd::Navigate { pane, location: Location::Archive { archive, inner }, select });
}

/// A mouse press on one of the pane's targets: it takes the keyboard; on an archive row
/// the cursor moves there and a double-click opens it; the player's mark plays / pauses
/// and its bar seeks.
pub fn click(app: &mut App, target: HitTarget, m: &MouseEvent) {
    if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }
    if player_click(app, target, m) {
        app.input.preview.focused = crate::view::preview::shown(&app.core);
        return;
    }
    let st = &mut app.input.preview;
    st.focused = true;
    if let HitTarget::ArchiveRow(i) = target {
        let total = match &st.vm.body {
            Body::Archive { total, .. } => *total,
            _ => return,
        };
        st.archive.select(i, total);
        let now = Instant::now();
        let ms = app
            .core
            .config
            .general
            .double_click_speed_ms
            .clamp(crate::input::mouse::DOUBLE_CLICK_MS_MIN, crate::input::mouse::DOUBLE_CLICK_MS_MAX);
        let double =
            st.last_press.is_some_and(|(at, j)| j == i && now.duration_since(at) <= Duration::from_millis(ms.into()));
        st.last_press = if double { None } else { Some((now, i)) };
        if double {
            open_row(app, i);
        }
    }
}

/// A left press on the player's targets (Quick Look's too): the mark toggles play /
/// pause, the bar seeks to the pressed fraction. Returns whether `target` was one.
pub fn player_click(app: &mut App, target: HitTarget, m: &MouseEvent) -> bool {
    match target {
        HitTarget::PlayerPlay => app.core.ui.player.toggle(),
        HitTarget::PlayerSeek => {
            let Some((r, HitTarget::PlayerSeek)) = super::mouse::hit(&app.hits, m.column, m.row) else { return true };
            let f = if r.width > 1 { f64::from(m.column - r.x) / f64::from(r.width - 1) } else { 0.0 };
            crate::player::seek_fraction(&app.core, f);
        }
        _ => return false,
    }
    true
}

/// The wheel over one of the pane's targets (`rows`: negative scrolls up).
pub fn wheel(app: &mut App, _target: HitTarget, rows: i32) {
    scroll_by(&mut app.input.preview, rows);
}

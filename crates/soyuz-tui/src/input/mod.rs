//! Input routing: who gets this frame's terminal events.
//!
//! The focus is derived every frame from the core's state ([`focus`]), first match wins:
//!
//! | [`TuiFocus`] | When | Key context, text field |
//! |---|---|---|
//! | `Dialog` | a core prompt is open (`dialogs.is_open()`) | `Dialog`, yes |
//! | `Palette` | `palette.open` | (the palette reads its own keys, like the GUI) |
//! | `Address` | `ui.address.editing` | `Omnibar`, yes |
//! | `Filter` | the TUI-local `filter.editing`, armed by `ui.filter_focus` | `Omnibar`, yes |
//! | `Sidebar` | `ui.focus == Focus::Sidebar` | `Sidebar`, no |
//! | `List` | otherwise | `FileList`, no |
//!
//! Each focus has its own module with a `route` function (and a `State` in
//! [`InputState`] when it keeps something between frames); the steps of Phase T2 fill
//! their own: `palette.rs` (T2.7), `address.rs` (T2.5), `filter.rs` (T2.8), `sidebar.rs`
//! (T2.6), `mouse.rs` (T2.9). Text fields use `crate::widgets::LineEdit`; see its module
//! docs for the key order.

pub mod address;
pub mod dialog;
pub mod filter;
pub mod list;
pub mod mouse;
pub mod palette;
pub mod sidebar;

use crossterm::event::Event;
use soyuz_appcore::core::{AppCore, Focus};
use soyuz_appcore::keys::input::InputFrame;
use soyuz_core::config::KeyContext;

use crate::app::App;

/// Who owns the keyboard this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiFocus {
    Dialog,
    Palette,
    Address,
    Filter,
    Sidebar,
    List,
}

/// Input state the terminal UI keeps between frames (one part per focus). The views read
/// it too (`view::prepare`, `ViewModel::from_core`): the field cursors, the sidebar cursor
/// and the scroll positions the core does not keep.
#[derive(Debug, Default)]
pub struct InputState {
    pub address: address::State,
    pub filter: filter::State,
    pub palette: palette::State,
    pub sidebar: sidebar::State,
    pub mouse: mouse::State,
}

/// The focus for the core's state and the TUI's own input state.
pub fn focus(core: &AppCore, st: &InputState) -> TuiFocus {
    if core.dialogs.is_open() {
        TuiFocus::Dialog
    } else if core.palette.open {
        TuiFocus::Palette
    } else if core.ui.address.editing {
        TuiFocus::Address
    } else if st.filter.editing {
        TuiFocus::Filter
    } else if core.ui.focus == Focus::Sidebar {
        TuiFocus::Sidebar
    } else {
        TuiFocus::List
    }
}

/// The core key context of a focus, and whether a text field owns the keyboard.
pub fn key_context(f: TuiFocus) -> (KeyContext, bool) {
    match f {
        TuiFocus::Dialog => (KeyContext::Dialog, true),
        // The palette's keys never reach the core's dispatcher (`palette::route`).
        TuiFocus::Palette => (KeyContext::Global, true),
        TuiFocus::Address | TuiFocus::Filter => (KeyContext::Omnibar, true),
        TuiFocus::Sidebar => (KeyContext::Sidebar, false),
        TuiFocus::List => (KeyContext::FileList, false),
    }
}

/// Hand this frame's `events` to whoever owns them.
pub fn route(app: &mut App, events: &[Event]) {
    if events.iter().any(|e| matches!(e, Event::Resize(..))) {
        app.hits.clear();
    }
    filter::arm(app);
    mouse::route(app, events);
    match focus(&app.core, &app.input) {
        TuiFocus::Dialog => dialog::route(app, events),
        TuiFocus::Palette => palette::route(app, events),
        TuiFocus::Address => address::route(app, events),
        TuiFocus::Filter => filter::route(app, events),
        TuiFocus::Sidebar => sidebar::route(app, events),
        TuiFocus::List => list::route(app, events),
    }
}

/// The core's key handling (bindings, sequences, type-ahead) for `events` in `kctx`.
/// Returns the frame, with what the core consumed marked, for the widget that runs next.
pub fn core_input(app: &mut App, events: &[Event], kctx: KeyContext, text_focused: bool) -> InputFrame {
    let mut frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    app.core.input(&app.host, &mut frame, kctx, text_focused);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_contexts() {
        assert_eq!(key_context(TuiFocus::List), (KeyContext::FileList, false));
        assert_eq!(key_context(TuiFocus::Address), (KeyContext::Omnibar, true));
        assert_eq!(key_context(TuiFocus::Filter), (KeyContext::Omnibar, true));
        assert_eq!(key_context(TuiFocus::Sidebar), (KeyContext::Sidebar, false));
        assert_eq!(key_context(TuiFocus::Dialog), (KeyContext::Dialog, true));
    }
}

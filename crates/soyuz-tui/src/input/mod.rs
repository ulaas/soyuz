//! Input routing: who gets this frame's terminal events.
//!
//! The focus is derived every frame from the core's state ([`focus`]), first match wins:
//!
//! | [`TuiFocus`] | When | Key context, text field |
//! |---|---|---|
//! | `Dialog` | a dialog of either stack is open (`dialogs::is_open`) | `Dialog`, yes |
//! | `Menu` | a popup menu is open (`menu::is_open`) | `Dialog`, yes |
//! | `Palette` | `palette.open` | (the palette reads its own keys, like the GUI) |
//! | `Settings` | the settings editor is open (`settings::has_focus`) | `Dialog`, yes |
//! | `Tool` | the compare screen or a tool screen shows (`compare::has_focus`, `screens::has_focus`) | `Dialog`, yes |
//! | `QuickLook` | Quick Look is open (`quicklook::open`) | `FileList`, no |
//! | `Editor` | the editor has the keyboard (`editor::has_focus`) | `Global`, yes |
//! | `Rename` | an inline rename is active (`rename::active`) | `Rename`, yes |
//! | `Address` | `ui.address.editing` | `Omnibar`, yes |
//! | `Filter` | the TUI-local `filter.editing`, armed by `ui.filter_focus` | `Omnibar`, yes |
//! | `Search` | the search bar's field (`search::has_focus`, armed by `ui.search.focus`) | `Omnibar`, yes |
//! | `Terminal` | the terminal pane (`terminal::has_focus`) | `Terminal`, yes |
//! | `Ai` | the AI panel (`ai::has_focus`) | `Global`, yes |
//! | `Jobs` | the jobs panel has the keyboard (`jobs::has_focus`) | `Global`, no |
//! | `Preview` | the preview pane has the keyboard (`preview::has_focus`) | `Global`, no |
//! | `Sidebar` | `ui.focus == Focus::Sidebar` | `Sidebar`, no |
//! | `List` | otherwise | `FileList`, no |
//!
//! Each focus has its own module with a `route` function (and a `State` in
//! [`InputState`] when it keeps something between frames); each step fills its own:
//! `palette.rs` (T2.7), `address.rs` (T2.5), `filter.rs` (T2.8), `sidebar.rs` (T2.6),
//! `mouse.rs` (T2.9); Phase T3 `dialogs/*`, `menu.rs`, `rename.rs`, `jobs.rs`; Phase T4
//! `preview.rs`, `quicklook.rs`, `editor.rs`; Phase T5 / T6 `terminal.rs`, `search.rs`,
//! `ai.rs`, `compare.rs`, `settings.rs` (each with `has_focus`, `route`, `on_mouse` and
//! `captures_mouse`); Phase T7 `screens/` (the full-screen treemap and duplicate finder,
//! the same four functions; one screen at a time, the compare screen included). Text
//! fields use `crate::widgets::LineEdit`; see its module docs for the key order.
//!
//! The full-screen overlays come before Quick Look because each one takes the keys that
//! would open the next; `Search` comes before `Terminal` so a passthrough chord from the
//! terminal lands in the bar.

pub mod address;
pub mod ai;
pub mod compare;
pub mod dialogs;
pub mod editor;
pub mod filter;
pub mod jobs;
pub mod list;
pub mod menu;
pub mod mouse;
pub mod palette;
pub mod preview;
pub mod quicklook;
pub mod rename;
pub mod screens;
pub mod search;
pub mod settings;
pub mod settings_keys;
pub mod sidebar;
pub mod terminal;

use crossterm::event::Event;
use soyuz_appcore::core::{AppCore, Focus};
use soyuz_appcore::keys::input::InputFrame;
use soyuz_core::config::KeyContext;

use crate::app::App;

/// Who owns the keyboard this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiFocus {
    Dialog,
    Menu,
    Palette,
    /// The settings editor (Phase T6).
    Settings,
    /// The compare screen (Phase T5) or a full-screen tool (Phase T7).
    Tool,
    QuickLook,
    Editor,
    Rename,
    Address,
    Filter,
    /// The search bar's field (Phase T5).
    Search,
    /// The embedded terminal pane (Phase T5).
    Terminal,
    /// The AI assistant panel (Phase T5).
    Ai,
    Jobs,
    Preview,
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
    pub dialogs: dialogs::State,
    pub menu: menu::State,
    pub quicklook: quicklook::State,
    pub editor: editor::State,
    pub rename: rename::State,
    pub jobs: jobs::State,
    pub preview: preview::State,
    pub search: search::State,
    pub terminal: terminal::State,
    pub ai: ai::State,
    pub compare: compare::State,
    pub settings: settings::State,
    /// The full-screen tools (Phase T7).
    pub screens: screens::State,
    /// The media player (Phase V3b, `crate::player`).
    pub player: crate::player::State,
}

/// The focus for the core's state and the TUI's own input state.
pub fn focus(core: &AppCore, st: &InputState) -> TuiFocus {
    if dialogs::is_open(core, &st.dialogs) {
        TuiFocus::Dialog
    } else if menu::is_open(&st.menu) {
        TuiFocus::Menu
    } else if core.palette.open {
        TuiFocus::Palette
    } else if settings::has_focus(core, st) {
        TuiFocus::Settings
    } else if compare::has_focus(core, st) || screens::has_focus(core, st) {
        TuiFocus::Tool
    } else if quicklook::open(core, st) {
        TuiFocus::QuickLook
    } else if editor::has_focus(core, st) {
        TuiFocus::Editor
    } else if rename::active(core, st) {
        TuiFocus::Rename
    } else if core.ui.address.editing {
        TuiFocus::Address
    } else if st.filter.editing {
        TuiFocus::Filter
    } else if search::has_focus(core, st) {
        TuiFocus::Search
    } else if terminal::has_focus(core, st) {
        TuiFocus::Terminal
    } else if ai::has_focus(core, st) {
        TuiFocus::Ai
    } else if jobs::has_focus(core, st) {
        TuiFocus::Jobs
    } else if preview::has_focus(core, st) {
        TuiFocus::Preview
    } else if core.ui.focus == Focus::Sidebar {
        TuiFocus::Sidebar
    } else {
        TuiFocus::List
    }
}

/// The core key context of a focus, and whether a text field owns the keyboard.
pub fn key_context(f: TuiFocus) -> (KeyContext, bool) {
    match f {
        TuiFocus::Dialog | TuiFocus::Menu => (KeyContext::Dialog, true),
        // Global chords never fire under a full-screen overlay; it reads its own keys first.
        TuiFocus::Settings | TuiFocus::Tool => (KeyContext::Dialog, true),
        // The palette's keys never reach the core's dispatcher (`palette::route`).
        TuiFocus::Palette => (KeyContext::Global, true),
        TuiFocus::QuickLook => (KeyContext::FileList, false),
        TuiFocus::Editor => (KeyContext::Global, true),
        TuiFocus::Rename => (KeyContext::Rename, true),
        TuiFocus::Address | TuiFocus::Filter | TuiFocus::Search => (KeyContext::Omnibar, true),
        TuiFocus::Terminal => (KeyContext::Terminal, true),
        // The composer has the cursor (the card list routes with `(Global, false)` itself).
        TuiFocus::Ai => (KeyContext::Global, true),
        TuiFocus::Jobs | TuiFocus::Preview => (KeyContext::Global, false),
        TuiFocus::Sidebar => (KeyContext::Sidebar, false),
        TuiFocus::List => (KeyContext::FileList, false),
    }
}

/// [`key_context`] for the app's focus, with the media player's context (Phase V3b):
/// `Player` while Quick Look or the focused preview pane shows the open player
/// (`crate::player::active`); its chords fall back to `Global`.
pub fn key_context_of(app: &App) -> (KeyContext, bool) {
    let f = focus(&app.core, &app.input);
    let (kctx, text) = key_context(f);
    if matches!(f, TuiFocus::Preview | TuiFocus::QuickLook) && crate::player::active(&app.core, &app.input.player) {
        return (KeyContext::Player, text);
    }
    (kctx, text)
}

/// Hand this frame's `events` to whoever owns them.
pub fn route(app: &mut App, events: &[Event]) {
    if events.iter().any(|e| matches!(e, Event::Resize(..))) {
        app.hits.clear();
    }
    filter::arm(app);
    search::arm(app);
    // The AI panel's every-frame work; Esc / ToggleAiPanel it takes are gone from `events`.
    let events: &[Event] = &ai::arm(app, events);
    mouse::route(app, events);
    menu::menu_key(app, events);
    match focus(&app.core, &app.input) {
        TuiFocus::Dialog => dialogs::route(app, events),
        TuiFocus::Menu => menu::route(app, events),
        TuiFocus::Palette => palette::route(app, events),
        TuiFocus::Settings => settings::route(app, events),
        TuiFocus::Tool if app.input.compare.open => compare::route(app, events),
        TuiFocus::Tool => screens::route(app, events),
        TuiFocus::QuickLook => quicklook::route(app, events),
        TuiFocus::Editor => editor::route(app, events),
        TuiFocus::Rename => rename::route(app, events),
        TuiFocus::Address => address::route(app, events),
        TuiFocus::Filter => filter::route(app, events),
        TuiFocus::Search => search::route(app, events),
        TuiFocus::Terminal => terminal::route(app, events),
        TuiFocus::Ai => ai::route(app, events),
        TuiFocus::Jobs => jobs::route(app, events),
        TuiFocus::Preview => preview::route(app, events),
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
        assert_eq!(key_context(TuiFocus::Menu), (KeyContext::Dialog, true));
        assert_eq!(key_context(TuiFocus::Rename), (KeyContext::Rename, true));
        assert_eq!(key_context(TuiFocus::Jobs), (KeyContext::Global, false));
        assert_eq!(key_context(TuiFocus::Settings), (KeyContext::Dialog, true));
        assert_eq!(key_context(TuiFocus::Tool), (KeyContext::Dialog, true));
        assert_eq!(key_context(TuiFocus::Search), (KeyContext::Omnibar, true));
        assert_eq!(key_context(TuiFocus::Terminal), (KeyContext::Terminal, true));
        assert_eq!(key_context(TuiFocus::Ai), (KeyContext::Global, true));
    }
}

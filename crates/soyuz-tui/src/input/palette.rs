//! Keys while the command palette is open (step T2.7), like the GUI
//! (`soyuz-app/src/ui/palette/mod.rs`, `read_keys`): the palette owns the keyboard, nothing
//! reaches the core's dispatcher.
//!
//! Each event in order: a palette key ([`palette_key`]: the global palette chords toggle
//! or switch the mode, Esc, Up / Down / Ctrl+J / Ctrl+K / Ctrl+N / Ctrl+P, PageUp /
//! PageDown, Enter with its modifiers, Tab) becomes a `soyuz_appcore::palette::KeyInput`
//! applied to fresh results; anything else goes to the `LineEdit` over `palette.input`
//! (which never binds Ctrl+J / K / N / P). Going event by event (the GUI reads a whole
//! frame's keys first) keeps "type, then Enter" in one terminal read in order. Events after
//! the one that closed the palette are dropped. The results are refreshed at the end.

use crossterm::event::Event;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::palette::query::Mode;
use soyuz_appcore::palette::{self, KeyInput, RunMods};
use soyuz_core::config::{Action, KeyContext, KeyName, Keymap, Mods};

use crate::app::App;
use crate::widgets::LineEdit;

/// What the palette keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// Cursor and scroll of the input field (the text is `palette.input`).
    edit: LineEdit,
    /// What the drawn palette keeps between frames (list scroll, the cursor it shows).
    pub view: crate::view::palette::Kept,
}

/// The palette key of `ev` with key map `km`, if it is one.
pub fn palette_key(km: &Keymap, ev: &InputEvent) -> Option<KeyInput> {
    let InputEvent::Chord(c) = ev else { return None };
    let mut k = KeyInput::default();
    if !c.mods.is_empty() {
        match km.lookup(KeyContext::Global, *c) {
            Some(Action::CommandPalette) => {
                k.toggle = Some(Mode::Commands);
                return Some(k);
            }
            Some(Action::JumpToFolderDialog) => {
                k.toggle = Some(Mode::Folders);
                return Some(k);
            }
            _ => {}
        }
    }
    let plain = c.mods.is_empty();
    let ctrl = c.mods == Mods::CTRL;
    match c.key {
        KeyName::Escape if plain => k.escape = true,
        KeyName::ArrowDown if plain => k.down = true,
        KeyName::J | KeyName::N if ctrl => k.down = true,
        KeyName::ArrowUp if plain => k.up = true,
        KeyName::K | KeyName::P if ctrl => k.up = true,
        KeyName::PageDown if plain => k.page_down = true,
        KeyName::PageUp if plain => k.page_up = true,
        KeyName::Enter if (Mods::CTRL | Mods::SHIFT | Mods::CMD).contains(c.mods) => {
            k.enter = Some(RunMods {
                command: c.mods.intersects(Mods::CTRL | Mods::CMD),
                shift: c.mods.contains(Mods::SHIFT),
            });
        }
        KeyName::Tab if plain => k.tab = true,
        // Swallowed, like the GUI.
        KeyName::Tab if c.mods == Mods::SHIFT => {}
        _ => return None,
    }
    Some(k)
}

/// Follow the core's requests for the field (new text from Esc, completion, a prefix).
fn sync(app: &mut App) {
    let p = &mut app.core.palette;
    if p.cursor_to_end {
        p.cursor_to_end = false;
        app.input.palette.edit = LineEdit::at_end(&p.input);
    }
    p.focus_input = false;
}

pub fn route(app: &mut App, events: &[Event]) {
    let frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    sync(app);
    for ev in &frame.events {
        if !app.core.palette.open {
            break;
        }
        if let Some(k) = palette_key(&app.core.keymap, ev) {
            palette::refresh_results(&mut app.core);
            palette::apply_keys(&mut app.core, &app.host, k);
            sync(app);
        } else {
            app.input.palette.edit.apply(&mut app.core.palette.input, ev);
        }
    }
    if app.core.palette.open {
        palette::refresh_results(&mut app.core);
        app.input.palette.view.cursor = Some(app.input.palette.edit.cursor());
    } else {
        app.input.palette.edit = LineEdit::new();
        app.input.palette.view.cursor = None;
    }
}

#[cfg(test)]
mod tests {
    use soyuz_core::config::defaults::preset;
    use soyuz_core::config::{KeyChord, KeyPreset};

    use super::*;

    fn key(km: &Keymap, s: &str) -> Option<KeyInput> {
        palette_key(km, &InputEvent::Chord(s.parse::<KeyChord>().unwrap()))
    }

    #[test]
    fn palette_keys_like_the_gui() {
        let km = preset(KeyPreset::default());
        let toggle = |m| Some(KeyInput { toggle: Some(m), ..KeyInput::default() });
        let down = Some(KeyInput { down: true, ..KeyInput::default() });
        let up = Some(KeyInput { up: true, ..KeyInput::default() });
        // The global palette chords toggle / switch modes before anything else.
        assert_eq!(key(&km, "Ctrl+K"), toggle(Mode::Commands));
        assert_eq!(key(&km, "Ctrl+Shift+P"), toggle(Mode::Commands));
        assert_eq!(key(&km, "Ctrl+P"), toggle(Mode::Folders));
        assert_eq!(key(&km, "Down"), down);
        assert_eq!(key(&km, "Ctrl+J"), down);
        assert_eq!(key(&km, "Ctrl+N"), down);
        assert_eq!(key(&km, "Up"), up);
        // Unbound: Ctrl+K / Ctrl+P move up.
        let empty = Keymap::default();
        assert_eq!(key(&empty, "Ctrl+K"), up);
        assert_eq!(key(&empty, "Ctrl+P"), up);
        assert_eq!(key(&km, "Esc").map(|k| k.escape), Some(true));
        assert_eq!(key(&km, "Enter").and_then(|k| k.enter), Some(RunMods::NONE));
        assert_eq!(key(&km, "Ctrl+Enter").and_then(|k| k.enter), Some(RunMods { command: true, shift: false }));
        assert_eq!(key(&km, "Shift+Enter").and_then(|k| k.enter), Some(RunMods { command: false, shift: true }));
        assert_eq!(key(&km, "Tab").map(|k| k.tab), Some(true));
        assert_eq!(key(&km, "Shift+Tab"), Some(KeyInput::default()));
        assert_eq!(key(&km, "PageDown").map(|k| k.page_down), Some(true));
        // Typing and editing keys are the field's.
        for s in ["H", "Shift+H", "Backspace", "Left", "Ctrl+Left", "Home", "Alt+Enter"] {
            assert_eq!(key(&km, s), None, "{s}");
        }
        assert_eq!(palette_key(&km, &InputEvent::Text("h".into())), None);
    }
}

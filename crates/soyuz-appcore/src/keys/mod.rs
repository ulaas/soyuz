//! Keyboard dispatch, toolkit-free: `KeyChord` → `Keymap` lookup → `Action`. The
//! toolkit's key events become chords in the frontend (`ui::keys` in the GUI).
//!
//! While a text field has focus only "chorded" shortcuts that are not text editing keys
//! reach the keymap.

pub mod conflicts;
pub mod input;
pub mod sequence;

use std::sync::OnceLock;

use soyuz_core::config::{Action, Config, KeyChord, KeyName, Keymap, Mods};

/// Hook a frontend installs to adjust every key map the core builds (the terminal UI adds
/// alternatives for chords a legacy terminal cannot send). Process-wide; the first
/// install wins.
static KEYMAP_OVERLAY: OnceLock<fn(&mut Keymap)> = OnceLock::new();

/// Install the [`build_keymap`] overlay (later calls are ignored).
pub fn set_keymap_overlay(overlay: fn(&mut Keymap)) {
    let _ = KEYMAP_OVERLAY.set(overlay);
}

/// The key map for `config` (`soyuz_core::config::keymap`) plus the frontend's overlay,
/// if one is installed. Every place the core (re)builds its key map goes through here;
/// workspace and script hotkeys are bound on top afterwards.
pub fn build_keymap(config: &Config) -> (Keymap, Vec<String>) {
    let (mut km, warnings) = soyuz_core::config::keymap(config);
    if let Some(overlay) = KEYMAP_OVERLAY.get() {
        overlay(&mut km);
    }
    (km, warnings)
}

/// Text-editing chords a focused text field must keep.
pub fn is_text_editing(chord: KeyChord) -> bool {
    use KeyName as N;
    let plain = chord.mods.is_empty() || chord.mods == Mods::SHIFT;
    if plain {
        return true;
    }
    let ctrl = chord.mods.contains(Mods::CTRL) || chord.mods.contains(Mods::CMD);
    ctrl && !chord.mods.contains(Mods::ALT)
        && matches!(
            chord.key,
            N::A | N::C
                | N::V
                | N::X
                | N::Z
                | N::Y
                | N::ArrowLeft
                | N::ArrowRight
                | N::Backspace
                | N::Delete
                | N::Home
                | N::End
        )
}

/// One keyboard event turned into an action.
#[derive(Clone, Debug, PartialEq)]
pub struct Dispatched {
    pub action: Action,
    pub chord: KeyChord,
}

/// Pretty chord text for menus/tooltips (`Ctrl+Shift+P`).
pub fn chord_label(keymap: &Keymap, action: &Action) -> Option<String> {
    keymap.chords_for(action).first().map(|c| c.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::{Config, KeyContext, keymap};

    #[test]
    fn text_fields_keep_editing_keys() {
        assert!(is_text_editing(KeyChord::new(Mods::CTRL, KeyName::A)));
        assert!(is_text_editing(KeyChord::new(Mods::empty(), KeyName::Backspace)));
        assert!(is_text_editing(KeyChord::new(Mods::SHIFT, KeyName::Home)));
        assert!(!is_text_editing(KeyChord::new(Mods::CTRL, KeyName::T)));
        assert!(!is_text_editing(KeyChord::new(Mods::CTRL | Mods::SHIFT, KeyName::P)));
    }

    #[test]
    fn default_keymap_is_reachable() {
        let (km, _) = keymap(&Config::default());
        assert_eq!(km.lookup(KeyContext::Global, KeyChord::new(Mods::CTRL, KeyName::T)), Some(&Action::NewTab));
    }
}

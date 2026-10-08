//! Keyboard dispatch: egui key events → core `KeyChord` → `Keymap` lookup → `Action`.
//!
//! Matched events are removed from the input queue so widgets do not react to them a
//! second time. While a text field has focus only "chorded" shortcuts that are not text
//! editing keys reach the keymap.

use egui::{Event, Key, Modifiers};
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, Keymap, Mods};

/// `egui::Key` → core `KeyName`.
pub fn key_name(k: Key) -> Option<KeyName> {
    use KeyName as N;
    Some(match k {
        Key::A => N::A,
        Key::B => N::B,
        Key::C => N::C,
        Key::D => N::D,
        Key::E => N::E,
        Key::F => N::F,
        Key::G => N::G,
        Key::H => N::H,
        Key::I => N::I,
        Key::J => N::J,
        Key::K => N::K,
        Key::L => N::L,
        Key::M => N::M,
        Key::N => N::N,
        Key::O => N::O,
        Key::P => N::P,
        Key::Q => N::Q,
        Key::R => N::R,
        Key::S => N::S,
        Key::T => N::T,
        Key::U => N::U,
        Key::V => N::V,
        Key::W => N::W,
        Key::X => N::X,
        Key::Y => N::Y,
        Key::Z => N::Z,
        Key::Num0 => N::Num0,
        Key::Num1 => N::Num1,
        Key::Num2 => N::Num2,
        Key::Num3 => N::Num3,
        Key::Num4 => N::Num4,
        Key::Num5 => N::Num5,
        Key::Num6 => N::Num6,
        Key::Num7 => N::Num7,
        Key::Num8 => N::Num8,
        Key::Num9 => N::Num9,
        Key::F1 => N::F1,
        Key::F2 => N::F2,
        Key::F3 => N::F3,
        Key::F4 => N::F4,
        Key::F5 => N::F5,
        Key::F6 => N::F6,
        Key::F7 => N::F7,
        Key::F8 => N::F8,
        Key::F9 => N::F9,
        Key::F10 => N::F10,
        Key::F11 => N::F11,
        Key::F12 => N::F12,
        Key::F13 => N::F13,
        Key::F14 => N::F14,
        Key::F15 => N::F15,
        Key::F16 => N::F16,
        Key::F17 => N::F17,
        Key::F18 => N::F18,
        Key::F19 => N::F19,
        Key::F20 => N::F20,
        Key::F21 => N::F21,
        Key::F22 => N::F22,
        Key::F23 => N::F23,
        Key::F24 => N::F24,
        Key::Enter => N::Enter,
        Key::Escape => N::Escape,
        Key::Tab => N::Tab,
        Key::Space => N::Space,
        Key::Backspace => N::Backspace,
        Key::Delete => N::Delete,
        Key::Insert => N::Insert,
        Key::Home => N::Home,
        Key::End => N::End,
        Key::PageUp => N::PageUp,
        Key::PageDown => N::PageDown,
        Key::ArrowUp => N::ArrowUp,
        Key::ArrowDown => N::ArrowDown,
        Key::ArrowLeft => N::ArrowLeft,
        Key::ArrowRight => N::ArrowRight,
        Key::Backtick => N::Backtick,
        Key::Minus => N::Minus,
        Key::Plus => N::Plus,
        Key::Equals => N::Equals,
        Key::Comma => N::Comma,
        Key::Period => N::Period,
        Key::Slash => N::Slash,
        Key::Backslash => N::Backslash,
        Key::Semicolon => N::Semicolon,
        Key::Quote => N::Quote,
        Key::OpenBracket => N::OpenBracket,
        Key::CloseBracket => N::CloseBracket,
        _ => return None,
    })
}

/// egui modifiers → core `Mods` (Cmd on macOS maps to `CMD`, normalized later).
pub fn mods(m: Modifiers) -> Mods {
    let mut out = Mods::empty();
    if m.ctrl {
        out |= Mods::CTRL;
    }
    if m.shift {
        out |= Mods::SHIFT;
    }
    if m.alt {
        out |= Mods::ALT;
    }
    if m.mac_cmd {
        out |= Mods::CMD;
    }
    out
}

/// Text-editing chords a focused text field must keep.
fn is_text_editing(chord: KeyChord) -> bool {
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

/// Map a raw input event to a chord (`Copy`/`Cut`/`Paste` come pre-translated from
/// egui-winit and are mapped back to their chords).
fn event_chord(ev: &Event, current: Modifiers, cmd_is_ctrl: bool) -> Option<KeyChord> {
    let chord = match ev {
        Event::Key { key, pressed: true, modifiers, .. } => KeyChord::new(mods(*modifiers), key_name(*key)?),
        Event::Copy if current.shift && current.command => KeyChord::new(Mods::CTRL | Mods::SHIFT, KeyName::C),
        Event::Copy => KeyChord::new(Mods::CTRL, KeyName::C),
        Event::Cut if current.shift && !current.ctrl => KeyChord::new(Mods::SHIFT, KeyName::Delete),
        Event::Cut => KeyChord::new(Mods::CTRL, KeyName::X),
        Event::Paste(_) if current.shift && !current.ctrl => KeyChord::new(Mods::SHIFT, KeyName::Insert),
        Event::Paste(_) if current.shift && current.command => KeyChord::new(Mods::CTRL | Mods::SHIFT, KeyName::V),
        Event::Paste(_) => KeyChord::new(Mods::CTRL, KeyName::V),
        _ => return None,
    };
    Some(chord.normalized(cmd_is_ctrl))
}

/// Collect and consume the key events of this frame that map to actions in `ctx_kind`.
/// `text_focused`: a text field owns the keyboard (only non-editing chords dispatch).
pub fn dispatch(
    ctx: &egui::Context,
    keymap: &Keymap,
    ctx_kind: KeyContext,
    text_focused: bool,
    cmd_is_ctrl: bool,
) -> Vec<Dispatched> {
    let mut out = Vec::new();
    ctx.input_mut(|i| {
        let current = i.modifiers;
        i.events.retain(|ev| {
            let Some(chord) = event_chord(ev, current, cmd_is_ctrl) else { return true };
            if text_focused && is_text_editing(chord) {
                return true;
            }
            match keymap.lookup(ctx_kind, chord) {
                Some(action) => {
                    out.push(Dispatched { action: action.clone(), chord });
                    false
                }
                None => true,
            }
        });
    });
    out
}

/// Printable text typed this frame (for type-ahead), without consuming it.
pub fn typed_text(ctx: &egui::Context) -> String {
    ctx.input(|i| {
        if i.modifiers.ctrl || i.modifiers.alt || i.modifiers.mac_cmd {
            return String::new();
        }
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    })
}

/// Pretty chord text for menus/tooltips (`Ctrl+Shift+P`).
pub fn chord_label(keymap: &Keymap, action: &Action) -> Option<String> {
    keymap.chords_for(action).first().map(|c| c.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::{Config, keymap};

    fn key(k: Key, m: Modifiers) -> Event {
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m }
    }

    #[test]
    fn chords_from_events() {
        let c = event_chord(&key(Key::T, Modifiers::CTRL), Modifiers::CTRL, true).unwrap();
        assert_eq!(c.to_string(), "Ctrl+T");
        assert_eq!(event_chord(&Event::Copy, Modifiers::CTRL, true).unwrap().to_string(), "Ctrl+C");
        assert_eq!(event_chord(&Event::Cut, Modifiers::SHIFT, true).unwrap().to_string(), "Shift+Del");
        let released =
            Event::Key { key: Key::A, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE };
        assert!(event_chord(&released, Modifiers::NONE, true).is_none());
        let mac = Modifiers { mac_cmd: true, command: true, ..Default::default() };
        assert_eq!(event_chord(&key(Key::W, mac), mac, true).unwrap().to_string(), "Ctrl+W");
    }

    #[test]
    fn text_fields_keep_editing_keys() {
        assert!(is_text_editing(KeyChord::new(Mods::CTRL, KeyName::A)));
        assert!(is_text_editing(KeyChord::new(Mods::empty(), KeyName::Backspace)));
        assert!(is_text_editing(KeyChord::new(Mods::SHIFT, KeyName::Home)));
        assert!(!is_text_editing(KeyChord::new(Mods::CTRL, KeyName::T)));
        assert!(!is_text_editing(KeyChord::new(Mods::CTRL | Mods::SHIFT, KeyName::P)));
    }

    #[test]
    fn every_key_name_has_an_egui_key() {
        for name in KeyName::ALL {
            assert!(Key::ALL.iter().any(|k| key_name(*k) == Some(*name)), "{name:?} unreachable");
        }
        assert_eq!(key_name(Key::ShiftLeft), None);
    }

    #[test]
    fn default_keymap_is_reachable() {
        let (km, _) = keymap(&Config::default());
        assert_eq!(km.lookup(KeyContext::Global, KeyChord::new(Mods::CTRL, KeyName::T)), Some(&Action::NewTab));
    }
}

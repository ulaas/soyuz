//! Key map layers of the terminal UI, installed with
//! `soyuz_appcore::keys::set_keymap_overlay` (the GUI's key map is never touched):
//!
//! - [`tui_defaults`], always: defaults only the terminal UI has. `FocusSidebar` (no
//!   default chord in any preset; the GUI reaches the sidebar with the mouse and Tab) gets
//!   F12, which no preset binds. Like the alternatives below, a default is only added
//!   while its chord is free, so a user binding of F12 wins.
//! - [`apply`], on legacy terminals (no kitty keyboard protocol): the alternatives below.
//!   [`legacy`] runs both.
//!
//! A legacy terminal sends Ctrl+letter as one control byte, so Ctrl+Shift+letter looks
//! exactly like Ctrl+letter, Ctrl+I like Tab, Ctrl+Tab like Tab, and Ctrl with most
//! punctuation either sends nothing or another key (Ctrl+/ is `0x1F`, which crossterm
//! reports as Ctrl+7). [`apply`] gives such bindings a second chord every legacy terminal
//! can send: Alt is sent as an Esc prefix, and Ctrl+PageUp / PageDown as a CSI sequence
//! with a modifier parameter.
//!
//! An alternative is only added where its chord is still free in the whole key map (no
//! binding in any context, no sequence starting with it), so the overlay never creates a
//! duplicate, a shadowed global or a dead sequence; user rebinds of the original chord get
//! the alternative too. Workspace and script hotkeys are bound afterwards and take a chord
//! from the overlay if they name it.
//!
//! The view modes (Ctrl+Shift+1…5 arrive as plain digits or nothing; Alt+Shift+digit types
//! `!`, `@`, … and Alt+digit opens bookmarks) get Ctrl+F1…F5, which every xterm-style
//! terminal sends as a CSI sequence with a modifier parameter.
//!
//! Not covered (no free, sendable alternative): Ctrl+digit (Select tab n, Ctrl+0; Alt+digit
//! opens bookmarks), Ctrl+Enter (Alt+Enter is Properties), Ctrl+[ / Ctrl+] (Finder Back /
//! Forward; Alt+Left / Right do the same, and Alt+[ starts a CSI sequence), Ctrl+Plus /
//! Minus / Equals (GUI zoom). The palette's own chords are sendable (Ctrl+K, Ctrl+P) or
//! covered (Ctrl+Shift+P).

use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, Keymap, Mods};

/// `(chord a legacy terminal cannot send, the alternative)`. Ctrl+Shift+letter gets
/// Alt+Shift+letter, so plain Ctrl+letter keeps its own Alt+letter for Ctrl+I.
pub const ALTERNATIVES: &[(&str, &str)] = &[
    // Ctrl+Shift+letter arrives as Ctrl+letter.
    ("Ctrl+Shift+C", "Alt+Shift+C"),
    ("Ctrl+Shift+E", "Alt+Shift+E"),
    ("Ctrl+Shift+F", "Alt+Shift+F"),
    ("Ctrl+Shift+G", "Alt+Shift+G"),
    ("Ctrl+Shift+H", "Alt+Shift+H"),
    ("Ctrl+Shift+I", "Alt+Shift+I"),
    ("Ctrl+Shift+J", "Alt+Shift+J"),
    ("Ctrl+Shift+N", "Alt+Shift+N"),
    ("Ctrl+Shift+P", "Alt+Shift+P"),
    ("Ctrl+Shift+R", "Alt+Shift+R"),
    ("Ctrl+Shift+T", "Alt+Shift+T"),
    ("Ctrl+Shift+V", "Alt+Shift+V"),
    ("Ctrl+Shift+Z", "Alt+Shift+Z"),
    // Ctrl+Tab / Ctrl+Shift+Tab arrive as Tab / Shift+Tab.
    ("Ctrl+Tab", "Ctrl+PageDown"),
    ("Ctrl+Shift+Tab", "Ctrl+PageUp"),
    // Ctrl+I arrives as Tab.
    ("Ctrl+I", "Alt+I"),
    // Ctrl+` sends NUL (Ctrl+Space), Ctrl+/ sends 0x1F (Ctrl+7), Ctrl+, sends nothing.
    ("Ctrl+`", "Alt+`"),
    ("Ctrl+/", "Alt+/"),
    ("Ctrl+,", "Alt+,"),
    // Ctrl+Backspace often sends 0x08 (Ctrl+H).
    ("Ctrl+Backspace", "Alt+Backspace"),
    // Ctrl+Shift+digit: views (Details, Grid, List, Miller columns, Gallery).
    ("Ctrl+Shift+1", "Ctrl+F1"),
    ("Ctrl+Shift+2", "Ctrl+F2"),
    ("Ctrl+Shift+3", "Ctrl+F3"),
    ("Ctrl+Shift+4", "Ctrl+F4"),
    ("Ctrl+Shift+5", "Ctrl+F5"),
    // Ctrl+. sends nothing (Finder: show hidden files); Alt+> arrives as Alt+Shift+.
    ("Ctrl+Shift+.", "Alt+Shift+."),
];

/// `(action, chord, context)` defaults of the terminal UI only ([`tui_defaults`]).
pub const TUI_DEFAULTS: &[(Action, &str, KeyContext)] = &[(Action::FocusSidebar, "F12", KeyContext::Global)];

fn chord(s: &str) -> KeyChord {
    s.parse().unwrap_or(KeyChord::new(Mods::empty(), KeyName::F24))
}

/// `chord` is not bound in any context and starts no sequence.
fn is_free(km: &Keymap, chord: KeyChord) -> bool {
    !km.bindings.values().any(|v| v.contains(&chord))
        && !km.sequences.values().any(|seqs| seqs.iter().any(|s| s.first() == Some(&chord)))
}

/// Add the [`TUI_DEFAULTS`] whose chord is free and whose action has no chord or sequence
/// yet. (An action unbound with an empty `[keys.bindings]` list looks the same as one that
/// was never bound, so it gets the default too; binding the chord elsewhere frees it.)
pub fn tui_defaults(km: &mut Keymap) {
    for (action, c, ctx) in TUI_DEFAULTS {
        let c = chord(c);
        let bound = !km.chords_for(action).is_empty() || km.sequences.keys().any(|(_, a)| a == action);
        if is_free(km, c) && !bound {
            km.bind(*ctx, action.clone(), c);
        }
    }
}

/// The overlay of a legacy terminal: [`tui_defaults`], then [`apply`].
pub fn legacy(km: &mut Keymap) {
    tui_defaults(km);
    apply(km);
}

/// Add the [`ALTERNATIVES`] to `km` (see the module docs).
pub fn apply(km: &mut Keymap) {
    for (from, to) in ALTERNATIVES {
        let (from, to) = (chord(from), chord(to));
        if !is_free(km, to) {
            continue;
        }
        let mut targets: Vec<_> = km
            .bindings
            .iter()
            .filter(|(_, v)| v.contains(&from))
            .map(|((ctx, action), _)| (*ctx, action.clone()))
            .collect();
        // A global chord a specific context shadows (Finder's Ctrl+I: Toggle AI panel,
        // Properties in the file list) gives the alternative to the specific contexts only,
        // so it does not copy the shadowing.
        if targets.len() > 1 {
            targets.retain(|(ctx, _)| *ctx != KeyContext::Global);
        }
        for (ctx, action) in targets {
            km.bind(ctx, action, to);
        }
    }
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::keys::conflicts::analyze;
    use soyuz_core::config::{Action, Config, KeyPreset, defaults, keymap};

    use super::*;

    #[test]
    fn alternatives_parse_and_differ() {
        for (from, to) in ALTERNATIVES {
            let (f, t): (KeyChord, KeyChord) = (from.parse().unwrap(), to.parse().unwrap());
            assert_ne!(f, t);
            assert_eq!(f.to_string().to_lowercase(), from.to_lowercase(), "{from}");
            // Alternatives never need Ctrl+Shift or Ctrl + an aliased key.
            assert!(!(t.mods.contains(Mods::CTRL) && t.mods.contains(Mods::SHIFT)), "{to}");
        }
    }

    #[test]
    fn overlay_adds_no_conflicts_to_any_preset() {
        for p in KeyPreset::ALL {
            let mut c = Config::default();
            c.keys.preset = p;
            let (km, _) = keymap(&c);
            let mut over = km.clone();
            apply(&mut over);
            assert_eq!(analyze(&over), analyze(&km), "{p:?}");
            assert!(
                over.bindings.values().map(Vec::len).sum::<usize>() > km.bindings.values().map(Vec::len).sum::<usize>()
            );
            // Idempotent.
            let mut twice = over.clone();
            apply(&mut twice);
            assert_eq!(twice, over);
        }
    }

    #[test]
    fn overlay_binds_sendable_alternatives() {
        let mut km = defaults::preset(KeyPreset::Explorer);
        apply(&mut km);
        let at = |ctx, s: &str| km.lookup(ctx, s.parse().unwrap()).cloned();
        assert_eq!(at(KeyContext::Global, "Alt+Shift+T"), Some(Action::ReopenClosedTab));
        assert_eq!(at(KeyContext::Global, "Ctrl+PageDown"), Some(Action::NextTab));
        assert_eq!(at(KeyContext::Global, "Ctrl+PageUp"), Some(Action::PrevTab));
        assert_eq!(at(KeyContext::Global, "Alt+I"), Some(Action::ToggleAiPanel));
        assert_eq!(at(KeyContext::FileList, "Alt+Shift+N"), Some(Action::NewFolder));
        assert_eq!(at(KeyContext::FileList, "Alt+Shift+C"), Some(Action::CopyPath));
        // Finder: Ctrl+I is Properties in the file list, Toggle AI globally.
        let mut f = defaults::preset(KeyPreset::Finder);
        apply(&mut f);
        assert_eq!(f.lookup(KeyContext::FileList, "Alt+I".parse().unwrap()), Some(&Action::Properties));
        assert_eq!(f.lookup(KeyContext::FileList, "Alt+Backspace".parse().unwrap()), Some(&Action::Delete));
    }

    #[test]
    fn view_modes_get_function_keys() {
        let mut km = defaults::preset(KeyPreset::Explorer);
        apply(&mut km);
        let at = |s: &str| km.lookup(KeyContext::Global, s.parse().unwrap()).cloned();
        assert_eq!(at("Ctrl+F1"), Some(Action::ViewDetails));
        assert_eq!(at("Ctrl+F3"), Some(Action::ViewList));
        assert_eq!(at("Ctrl+F4"), Some(Action::ViewMiller));
        // Finder binds the views to Ctrl+digit (sendable): nothing to add there.
        let mut f = defaults::preset(KeyPreset::Finder);
        apply(&mut f);
        assert_eq!(f.lookup(KeyContext::Global, "Ctrl+F1".parse().unwrap()), None);
        assert_eq!(f.lookup(KeyContext::Global, "Alt+Shift+.".parse().unwrap()), Some(&Action::ToggleHidden));
    }

    #[test]
    fn tui_defaults_bind_free_chords_only() {
        let f12: KeyChord = "F12".parse().unwrap();
        for p in KeyPreset::ALL {
            let mut c = Config::default();
            c.keys.preset = p;
            let (km, _) = keymap(&c);
            assert!(is_free(&km, f12), "{p:?} binds F12: pick another TUI default");
            assert!(km.chords_for(&Action::FocusSidebar).is_empty(), "{p:?}");
            let mut over = km.clone();
            legacy(&mut over);
            assert_eq!(over.lookup(KeyContext::FileList, f12), Some(&Action::FocusSidebar), "{p:?}");
            assert_eq!(analyze(&over), analyze(&km), "{p:?}");
        }
        // A user binding wins: F12 taken, or the action bound elsewhere.
        let mut km = defaults::preset(KeyPreset::Explorer);
        km.bind(KeyContext::FileList, Action::Refresh, f12);
        tui_defaults(&mut km);
        assert!(km.chords_for(&Action::FocusSidebar).is_empty());
        let mut km = defaults::preset(KeyPreset::Explorer);
        km.bind(KeyContext::Global, Action::FocusSidebar, "Alt+S".parse().unwrap());
        tui_defaults(&mut km);
        assert_eq!(km.chords_for(&Action::FocusSidebar), vec!["Alt+S".parse::<KeyChord>().unwrap()]);
    }

    #[test]
    fn taken_alternatives_are_left_alone() {
        let mut km = defaults::preset(KeyPreset::Explorer);
        km.bind(KeyContext::FileList, Action::Refresh, "Alt+Shift+T".parse().unwrap());
        apply(&mut km);
        assert_eq!(km.lookup(KeyContext::Global, "Alt+Shift+T".parse().unwrap()), None);
        assert_eq!(km.lookup(KeyContext::FileList, "Alt+Shift+T".parse().unwrap()), Some(&Action::Refresh));
        assert!(!km.chords_for(&Action::ReopenClosedTab).contains(&"Alt+Shift+T".parse().unwrap()));
    }
}

//! Key map overlay for legacy terminals (no kitty keyboard protocol), installed with
//! `soyuz_appcore::keys::set_keymap_overlay`.
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
//! Not covered (no free, sendable alternative): Ctrl+digit (Select tab n, Ctrl+0; Alt+digit
//! opens bookmarks), Ctrl+Shift+digit (views; use the palette), Ctrl+Enter (Alt+Enter is
//! Properties), Ctrl+[ / Ctrl+] (Finder Back / Forward; Alt+Left / Right do the same, and
//! Alt+[ starts a CSI sequence), Ctrl+Plus / Minus / Equals (GUI zoom).

use soyuz_core::config::{KeyChord, KeyContext, KeyName, Keymap, Mods};

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
];

fn chord(s: &str) -> KeyChord {
    s.parse().unwrap_or(KeyChord::new(Mods::empty(), KeyName::F24))
}

/// `chord` is not bound in any context and starts no sequence.
fn is_free(km: &Keymap, chord: KeyChord) -> bool {
    !km.bindings.values().any(|v| v.contains(&chord))
        && !km.sequences.values().any(|seqs| seqs.iter().any(|s| s.first() == Some(&chord)))
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
    fn taken_alternatives_are_left_alone() {
        let mut km = defaults::preset(KeyPreset::Explorer);
        km.bind(KeyContext::FileList, Action::Refresh, "Alt+Shift+T".parse().unwrap());
        apply(&mut km);
        assert_eq!(km.lookup(KeyContext::Global, "Alt+Shift+T".parse().unwrap()), None);
        assert_eq!(km.lookup(KeyContext::FileList, "Alt+Shift+T".parse().unwrap()), Some(&Action::Refresh));
        assert!(!km.chords_for(&Action::ReopenClosedTab).contains(&"Alt+Shift+T".parse().unwrap()));
    }
}

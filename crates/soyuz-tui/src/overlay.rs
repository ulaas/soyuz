//! Key map layers of the terminal UI, installed with
//! `soyuz_appcore::keys::set_keymap_overlay` (the GUI's key map is never touched):
//!
//! - [`tui_defaults`], always: defaults only the terminal UI has. `FocusSidebar` (no
//!   default chord in any preset; the GUI reaches the sidebar with the mouse and Tab) gets
//!   F12, which no preset binds; `FocusActivityPanel` (the jobs panel) gets Alt+J. Like
//!   the alternatives below, a default is only added while its chord is free, so a user
//!   binding of F12 wins. `FocusEditor` gets Alt+E. The editors' own commands
//!   ([`EDITOR_DEFAULTS`], `KeyContext::Editor`) get the terminal's chords: Replace is
//!   Ctrl+R (Ctrl+H is Backspace or Ctrl+Backspace in many terminals, so the editor
//!   deletes the word before the caret with it), plus save as, wrap, maximize, the dock
//!   side, the notice's action and the find bar's toggles. Editor chords shadow the global
//!   ones only while a document has the keyboard, so they never count as taken for the
//!   alternatives below ([`is_free`]).
//! - [`apply`], on legacy terminals (no kitty keyboard protocol): the alternatives below.
//!   [`legacy`] runs both.
//! - [`windows_terminal`] / [`windows_terminal_legacy`], inside Windows Terminal (see
//!   "Windows Terminal" below).
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
//!
//! ## Windows Terminal
//!
//! Windows Terminal (detected by `WT_SESSION`, [`in_windows_terminal`]) runs its own key
//! bindings before the application sees a key: its defaults ([`WT_BLOCKED`], from WT's
//! `defaults.json` plus the `userDefaults.json` a new `settings.json` starts from) never
//! reach Soyuz. [`windows_terminal`] keeps the user's bindings and the GUI untouched, and
//! for every binding that uses such a chord:
//! 1. when the action has no other usable chord in that context, adds the first free
//!    stand-in from [`ALTERNATIVES`] and [`WT_ALTERNATIVES`] that WT does not take;
//! 2. moves the blocked chords to the end of the binding's list (nothing is removed), so
//!    the first chord — the one `chord_label` shows in the palette, menus and toast hints —
//!    is one that works (Command palette: Ctrl+K instead of Ctrl+Shift+P).
//!
//! WT's paste chords ([`WT_PASTE`]) are special: WT pastes the clipboard's text (or the
//! paths of files copied in Explorer) as a bracketed paste, and a paste into the file
//! list runs Soyuz's Paste (`crate::input::list`). That only works where crossterm
//! reports bracketed pastes (`Event::Paste`): unix terminals, so WSL inside WT. The
//! Windows console reader gets a paste as typed keys, so there the paste chords count as
//! blocked and Paste gets Alt+V.
//!
//! Chords WT only takes in some states fall through otherwise and are left alone
//! ([`WT_CONDITIONAL`]): copy (Ctrl+C, Ctrl+Shift+C, Ctrl+Insert, Enter) only with a
//! selection in the terminal, pane focus / resize (Alt+arrows, Ctrl+Alt+Left,
//! Alt+Shift+arrows) only with split panes. Not covered: UI zoom (Ctrl+= / - / 0 /
//! Plus: the terminal's own font size is the zoom there), Ctrl+Alt+digit,
//! Ctrl+Shift+6…9, Ctrl+Shift+, / Space, Alt+Shift+D / - / Plus, Alt+F4, Alt+Space, F11
//! and the Ctrl+Shift scroll keys (no preset binds them; a user binding keeps its chord).

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
pub const TUI_DEFAULTS: &[(Action, &str, KeyContext)] = &[
    (Action::FocusSidebar, "F12", KeyContext::Global),
    // The jobs panel (Phase T3); Ctrl+J is Enter on legacy terminals.
    (Action::FocusActivityPanel, "Alt+J", KeyContext::Global),
    // Back into the editor after Esc (Phase T4).
    (Action::FocusEditor, "Alt+E", KeyContext::Global),
];

/// `(editor command, chords)` the terminal UI binds in `KeyContext::Editor`
/// ([`editor_defaults`]). Ctrl+Shift+S arrives as Ctrl+S in legacy terminals: save as
/// has Alt+Shift+S too.
pub const EDITOR_DEFAULTS: &[(Action, &[&str])] = &[
    (Action::EditorReplace, &["Ctrl+R"]),
    (Action::EditorSaveAs, &["Ctrl+Shift+S", "Alt+Shift+S"]),
    (Action::EditorToggleWrap, &["Alt+Z"]),
    (Action::EditorMaximize, &["Alt+M"]),
    (Action::EditorToggleDock, &["Alt+Shift+M"]),
    (Action::EditorNoticeAction, &["Alt+E"]),
    (Action::EditorMatchCase, &["Alt+C"]),
    (Action::EditorWholeWord, &["Alt+W"]),
    (Action::EditorRegex, &["Alt+R"]),
    (Action::EditorReplaceAll, &["Alt+A"]),
    (Action::EditorResizeImage, &["Alt+S"]),
];

/// The chord the terminal UI never sees as itself in the editor (Ctrl+H deletes the word
/// before the caret, `input::editor`): taken off the editor's bindings.
pub const EDITOR_UNSENDABLE: &str = "Ctrl+H";

/// Chords Windows Terminal's default key bindings always consume, as Soyuz spells them
/// (WT `defaults.json` and `userDefaults.json`, 2026). `ctrl+plus` / `alt+shift+plus` are
/// the `=` / `+` key: Ctrl+=, Alt+Shift+= (reported as Alt+Plus), and the keypad's Plus.
pub const WT_BLOCKED: &[&str] = &[
    // Window, settings, palette, find.
    "Alt+F4",
    "Alt+Enter",
    "F11",
    "Alt+Space",
    "Ctrl+,",
    "Ctrl+Shift+,",
    "Ctrl+Alt+,",
    "Ctrl+Shift+P",
    "Ctrl+Shift+F",
    "Ctrl+Shift+.",
    // Tabs and windows.
    "Ctrl+Shift+Space",
    "Ctrl+Shift+T",
    "Ctrl+Shift+N",
    "Ctrl+Shift+D",
    "Ctrl+Shift+W",
    "Ctrl+Tab",
    "Ctrl+Shift+Tab",
    "Ctrl+Shift+1",
    "Ctrl+Shift+2",
    "Ctrl+Shift+3",
    "Ctrl+Shift+4",
    "Ctrl+Shift+5",
    "Ctrl+Shift+6",
    "Ctrl+Shift+7",
    "Ctrl+Shift+8",
    "Ctrl+Shift+9",
    "Ctrl+Alt+1",
    "Ctrl+Alt+2",
    "Ctrl+Alt+3",
    "Ctrl+Alt+4",
    "Ctrl+Alt+5",
    "Ctrl+Alt+6",
    "Ctrl+Alt+7",
    "Ctrl+Alt+8",
    "Ctrl+Alt+9",
    // Split panes.
    "Alt+Shift+D",
    "Alt+Shift+-",
    "Alt+Plus",
    "Alt+Shift+=",
    // Selection, marks, buffer.
    "Ctrl+Shift+A",
    "Ctrl+Shift+M",
    "Ctrl+Shift+K",
    "Ctrl+Shift+Up",
    "Ctrl+Shift+Down",
    "Ctrl+Shift+PageUp",
    "Ctrl+Shift+PageDown",
    "Ctrl+Shift+Home",
    "Ctrl+Shift+End",
    // Font size.
    "Ctrl+=",
    "Ctrl+Plus",
    "Ctrl+-",
    "Ctrl+0",
];

/// Windows Terminal's paste chords (Ctrl+V comes from `userDefaults.json`, so a
/// `settings.json` from before it may lack it): consumed, then sent as a paste.
pub const WT_PASTE: &[&str] = &["Ctrl+V", "Ctrl+Shift+V", "Shift+Ins"];

/// Chords Windows Terminal only takes in some states (they reach Soyuz otherwise): copy
/// with a selection, moving focus between / resizing split panes.
pub const WT_CONDITIONAL: &[&str] = &[
    "Ctrl+C",
    "Ctrl+Shift+C",
    "Ctrl+Ins",
    "Enter",
    "Alt+Left",
    "Alt+Right",
    "Alt+Up",
    "Alt+Down",
    "Ctrl+Alt+Left",
    "Alt+Shift+Left",
    "Alt+Shift+Right",
    "Alt+Shift+Up",
    "Alt+Shift+Down",
];

/// Stand-ins only Windows Terminal needs, tried after [`ALTERNATIVES`]:
/// `(chord WT takes, the stand-in)`. Alt+Shift+D is WT's split pane, so Ctrl+Shift+D
/// gets Ctrl+Alt+D.
pub const WT_ALTERNATIVES: &[(&str, &str)] = &[
    // Explorer's Properties; Alt+Enter is WT's full screen.
    ("Alt+Enter", "Alt+Shift+Enter"),
    // Paste, where WT's paste arrives as typed keys (Windows console).
    ("Ctrl+V", "Alt+V"),
    ("Shift+Ins", "Alt+V"),
    ("Ctrl+Shift+A", "Alt+Shift+A"),
    ("Ctrl+Shift+D", "Ctrl+Alt+D"),
    ("Ctrl+Shift+K", "Alt+Shift+K"),
    ("Ctrl+Shift+M", "Alt+Shift+M"),
    ("Ctrl+Shift+W", "Alt+Shift+W"),
];

/// Which overlay the terminal UI installs ([`Overlay::pick`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// [`tui_defaults`]: kitty keyboard protocol or the Windows console.
    Defaults,
    /// [`legacy`]: a legacy unix terminal.
    Legacy,
    /// [`windows_terminal`].
    WindowsTerminal,
    /// [`windows_terminal_legacy`]: WSL in Windows Terminal without the kitty protocol.
    WindowsTerminalLegacy,
}

impl Overlay {
    /// `kitty`: the kitty keyboard protocol is on; `is_windows`: `cfg!(windows)` (the
    /// console reports every key, no legacy aliasing); `in_wt`: [`in_windows_terminal`].
    pub fn pick(kitty: bool, is_windows: bool, in_wt: bool) -> Overlay {
        match (!kitty && !is_windows, in_wt) {
            (false, false) => Overlay::Defaults,
            (true, false) => Overlay::Legacy,
            (false, true) => Overlay::WindowsTerminal,
            (true, true) => Overlay::WindowsTerminalLegacy,
        }
    }

    /// The function for `soyuz_appcore::keys::set_keymap_overlay`.
    pub fn function(self) -> fn(&mut Keymap) {
        match self {
            Overlay::Defaults => tui_defaults,
            Overlay::Legacy => legacy,
            Overlay::WindowsTerminal => windows_terminal,
            Overlay::WindowsTerminalLegacy => windows_terminal_legacy,
        }
    }
}

/// Running inside Windows Terminal: `WT_SESSION` is set and not empty (`env` reads an
/// environment variable; WT also forwards it into WSL through `WSLENV`).
pub fn in_windows_terminal(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    env("WT_SESSION").is_some_and(|v| !v.is_empty())
}

/// Windows Terminal takes `c` before Soyuz sees it. `paste_events`: WT's paste reaches
/// Soyuz as a bracketed paste, which runs Paste in the file list, so the paste chords
/// still work for [`Action::Paste`].
pub fn wt_blocks(c: KeyChord, action: &Action, paste_events: bool) -> bool {
    let is = |list: &[&str]| list.iter().any(|s| chord(s) == c);
    is(WT_BLOCKED) || (is(WT_PASTE) && !(paste_events && *action == Action::Paste))
}

/// The overlay inside Windows Terminal: [`tui_defaults`], then [`wt_stand_ins`].
pub fn windows_terminal(km: &mut Keymap) {
    tui_defaults(km);
    wt_stand_ins(km, wt_paste_events());
}

/// The overlay inside Windows Terminal without the kitty keyboard protocol (WSL):
/// [`legacy`], then [`wt_stand_ins`].
pub fn windows_terminal_legacy(km: &mut Keymap) {
    legacy(km);
    wt_stand_ins(km, wt_paste_events());
}

/// crossterm reports bracketed pastes on unix only; the Windows console reader gets a
/// paste as key presses.
fn wt_paste_events() -> bool {
    !cfg!(windows)
}

/// The Windows Terminal stand-ins and order (see the module docs). `paste_events`: see
/// [`wt_blocks`].
pub fn wt_stand_ins(km: &mut Keymap, paste_events: bool) {
    let blocked = |c: KeyChord, a: &Action| wt_blocks(c, a, paste_events);
    // 1. Stand-ins for bindings left without a usable chord (a sequence such as Vim's
    //    `p p` does not count: the shortcut hints show single chords).
    let stranded: Vec<(KeyContext, Action, Vec<KeyChord>)> = km
        .bindings
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .filter(|((_, a), v)| v.iter().all(|c| blocked(*c, a)))
        .map(|((ctx, a), v)| (*ctx, a.clone(), v.clone()))
        .collect();
    for (ctx, action, chords) in stranded {
        let stand_in = chords.iter().find_map(|c| {
            ALTERNATIVES
                .iter()
                .chain(WT_ALTERNATIVES)
                .filter(|(from, _)| chord(from) == *c)
                .map(|(_, to)| chord(to))
                .find(|to| !blocked(*to, &action) && is_free(km, *to))
        });
        if let Some(to) = stand_in {
            km.bind(ctx, action, to);
        }
    }
    // 2. Usable chords first (stable; nothing is removed).
    for ((_, action), v) in km.bindings.iter_mut() {
        let (mut ok, taken): (Vec<KeyChord>, Vec<KeyChord>) = v.iter().partition(|c| !blocked(**c, action));
        ok.extend(taken);
        *v = ok;
    }
}

fn chord(s: &str) -> KeyChord {
    s.parse().unwrap_or(KeyChord::new(Mods::empty(), KeyName::F24))
}

/// `chord` is not bound in any context (but the editor's, which shadows the others only
/// while a document has the keyboard) and starts no sequence.
fn is_free(km: &Keymap, chord: KeyChord) -> bool {
    !km.bindings.iter().any(|((ctx, _), v)| *ctx != KeyContext::Editor && v.contains(&chord))
        && !km.sequences.values().any(|seqs| seqs.iter().any(|s| s.first() == Some(&chord)))
}

/// The editor commands of [`EDITOR_DEFAULTS`]: Ctrl+H ([`EDITOR_UNSENDABLE`]) leaves the
/// editor's bindings, then a command without a chord gets its defaults that are free in
/// `KeyContext::Editor` (a user binding of the command, or of the chord, wins).
pub fn editor_defaults(km: &mut Keymap) {
    let gone = chord(EDITOR_UNSENDABLE);
    for ((ctx, _), v) in km.bindings.iter_mut() {
        if *ctx == KeyContext::Editor {
            v.retain(|c| *c != gone);
        }
    }
    km.bindings.retain(|_, v| !v.is_empty());
    for (action, chords) in EDITOR_DEFAULTS {
        let bound = !km.chords_for(action).is_empty() || km.sequences.keys().any(|(_, a)| a == action);
        if bound {
            continue;
        }
        for c in chords.iter().map(|s| chord(s)) {
            let taken = km.bindings.iter().any(|((ctx, _), v)| *ctx == KeyContext::Editor && v.contains(&c));
            if !taken {
                km.bind(KeyContext::Editor, action.clone(), c);
            }
        }
    }
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
    editor_defaults(km);
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
            for (action, chord, _) in TUI_DEFAULTS {
                assert!(is_free(&km, ch(chord)), "{p:?} binds {chord}: pick another TUI default");
                assert!(km.chords_for(action).is_empty(), "{p:?}: {action:?}");
            }
            let mut over = km.clone();
            legacy(&mut over);
            assert_eq!(over.lookup(KeyContext::FileList, f12), Some(&Action::FocusSidebar), "{p:?}");
            assert_eq!(over.lookup(KeyContext::FileList, ch("Alt+J")), Some(&Action::FocusActivityPanel), "{p:?}");
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

    fn ch(s: &str) -> KeyChord {
        s.parse().unwrap()
    }

    #[test]
    fn editor_defaults_use_terminal_chords() {
        for p in KeyPreset::ALL {
            let mut c = Config::default();
            c.keys.preset = p;
            let (km, _) = keymap(&c);
            let mut over = km.clone();
            legacy(&mut over);
            let ed = |s: &str| soyuz_appcore::editors::commands::binding(&over, ch(s));
            assert_eq!(ed("Ctrl+R"), Some(Action::EditorReplace), "{p:?}");
            assert_eq!(ed("Ctrl+H"), None, "{p:?}: Ctrl+H deletes a word");
            assert_eq!(ed("Ctrl+F"), Some(Action::EditorFind), "{p:?}");
            assert_eq!(ed("Alt+Shift+S"), Some(Action::EditorSaveAs), "{p:?}");
            assert_eq!(ed("Alt+E"), Some(Action::EditorNoticeAction), "{p:?}");
            assert_eq!(over.lookup(KeyContext::FileList, ch("Alt+E")), Some(&Action::FocusEditor), "{p:?}");
            // Outside the editor, Ctrl+R stays Refresh.
            if p != KeyPreset::Finder {
                assert_eq!(over.lookup(KeyContext::FileList, ch("Ctrl+R")), Some(&Action::Refresh), "{p:?}");
            }
            // Legacy alternatives are not blocked by editor chords (Ctrl+PageDown).
            if p == KeyPreset::Explorer {
                assert_eq!(over.lookup(KeyContext::Global, ch("Ctrl+PageDown")), Some(&Action::NextTab));
            }
            assert_eq!(analyze(&over), analyze(&km), "{p:?}");
        }
        // A user binding of Replace stays; its Ctrl+H goes.
        let mut c = Config::default();
        c.keys.bindings.insert("editor_replace".into(), vec!["Ctrl+H".into(), "Alt+H".into()]);
        let (mut km, _) = keymap(&c);
        tui_defaults(&mut km);
        assert_eq!(km.chords_for(&Action::EditorReplace), [ch("Alt+H")]);
        // A user binding of an editor chord wins.
        let mut c = Config::default();
        c.keys.bindings.insert("editor_toggle_wrap".into(), vec!["Alt+M".into()]);
        let (mut km, _) = keymap(&c);
        tui_defaults(&mut km);
        assert_eq!(soyuz_appcore::editors::commands::binding(&km, ch("Alt+M")), Some(Action::EditorToggleWrap));
        assert!(km.chords_for(&Action::EditorMaximize).is_empty());
    }

    fn wt_lists() -> impl Iterator<Item = &'static str> {
        WT_BLOCKED.iter().chain(WT_PASTE).chain(WT_CONDITIONAL).copied()
    }

    #[test]
    fn wt_tables_parse_and_stand_ins_are_sendable() {
        for s in wt_lists() {
            assert!(s.parse::<KeyChord>().is_ok(), "{s}");
        }
        for (from, to) in WT_ALTERNATIVES {
            let (f, t) = (ch(from), ch(to));
            assert!(WT_BLOCKED.iter().chain(WT_PASTE).any(|b| ch(b) == f), "{from} is not taken by WT");
            assert!(!wt_lists().any(|b| ch(b) == t), "WT takes the stand-in {to}");
        }
        // Every legacy alternative WT can need is one WT lets through.
        for (from, to) in ALTERNATIVES {
            if WT_BLOCKED.iter().any(|b| ch(b) == ch(from)) {
                assert!(!wt_lists().any(|b| ch(b) == ch(to)), "WT takes the stand-in {to}");
            }
        }
    }

    #[test]
    fn picks_the_overlay() {
        // (kitty, windows, in WT)
        assert_eq!(Overlay::pick(true, false, false), Overlay::Defaults);
        assert_eq!(Overlay::pick(false, true, false), Overlay::Defaults);
        assert_eq!(Overlay::pick(false, false, false), Overlay::Legacy);
        assert_eq!(Overlay::pick(false, true, true), Overlay::WindowsTerminal);
        assert_eq!(Overlay::pick(true, false, true), Overlay::WindowsTerminal);
        assert_eq!(Overlay::pick(false, false, true), Overlay::WindowsTerminalLegacy);
        // The functions do what their names say.
        let km = defaults::preset(KeyPreset::Explorer);
        let run = |o: Overlay| {
            let mut k = km.clone();
            o.function()(&mut k);
            k
        };
        let mut want = km.clone();
        legacy(&mut want);
        assert_eq!(run(Overlay::Legacy), want);
        wt_stand_ins(&mut want, !cfg!(windows));
        assert_eq!(run(Overlay::WindowsTerminalLegacy), want);
        let mut want = km.clone();
        tui_defaults(&mut want);
        assert_eq!(run(Overlay::Defaults), want);
        wt_stand_ins(&mut want, !cfg!(windows));
        assert_eq!(run(Overlay::WindowsTerminal), want);
        assert_ne!(run(Overlay::WindowsTerminal), run(Overlay::Defaults));
    }

    #[test]
    fn detects_windows_terminal() {
        use std::ffi::OsString;
        assert!(in_windows_terminal(|k| (k == "WT_SESSION").then(|| OsString::from("5b7f-…"))));
        assert!(!in_windows_terminal(|k| (k == "WT_SESSION").then(OsString::new)));
        assert!(!in_windows_terminal(|_| None));
        assert!(!in_windows_terminal(|k| (k == "TERM_PROGRAM").then(|| OsString::from("WezTerm"))));
    }

    /// Every preset × both WT variants (paste events or not) × with and without the legacy
    /// alternatives first.
    fn wt_overlays() -> Vec<(KeyPreset, bool, bool, Keymap, Keymap)> {
        let mut out = Vec::new();
        for p in KeyPreset::ALL {
            let mut c = Config::default();
            c.keys.preset = p;
            let (km, _) = keymap(&c);
            for paste_events in [false, true] {
                for legacy_first in [false, true] {
                    let mut over = km.clone();
                    if legacy_first {
                        legacy(&mut over);
                    } else {
                        tui_defaults(&mut over);
                    }
                    wt_stand_ins(&mut over, paste_events);
                    out.push((p, paste_events, legacy_first, km.clone(), over));
                }
            }
        }
        out
    }

    #[test]
    fn windows_terminal_adds_no_conflicts_and_removes_nothing() {
        for (p, paste, leg, km, over) in wt_overlays() {
            assert_eq!(analyze(&over), analyze(&km), "{p:?} paste {paste} legacy {leg}");
            for ((ctx, a), v) in &km.bindings {
                let o = &over.bindings[&(*ctx, a.clone())];
                // Only the editor's Ctrl+H goes (`editor_defaults`).
                let kept = |c: &KeyChord| o.contains(c) || (*ctx == KeyContext::Editor && *c == ch(EDITOR_UNSENDABLE));
                assert!(v.iter().all(kept), "{p:?}: {a:?} lost a chord");
            }
            assert_eq!(over.sequences, km.sequences);
            // Idempotent.
            let mut twice = over.clone();
            wt_stand_ins(&mut twice, paste);
            assert_eq!(twice, over, "{p:?}");
        }
    }

    #[test]
    fn windows_terminal_leaves_every_binding_a_usable_first_chord() {
        // Not covered on purpose (see the module docs): UI zoom.
        let skipped = [Action::UiZoomIn, Action::UiZoomOut, Action::UiZoomReset];
        for (p, paste, leg, _, over) in wt_overlays() {
            for ((ctx, a), v) in &over.bindings {
                if skipped.contains(a) {
                    continue;
                }
                assert!(!wt_blocks(v[0], a, paste), "{p:?} paste {paste} legacy {leg}: {ctx:?} {a:?} {v:?}");
            }
        }
    }

    #[test]
    fn windows_terminal_stand_ins_in_the_explorer_preset() {
        let mut km = defaults::preset(KeyPreset::Explorer);
        wt_stand_ins(&mut km, false);
        let first = |a: &Action| soyuz_appcore::keys::chord_label(&km, a);
        assert_eq!(first(&Action::CommandPalette).as_deref(), Some("Ctrl+K"));
        assert_eq!(first(&Action::Search).as_deref(), Some("F3"));
        assert_eq!(first(&Action::NextTab).as_deref(), Some("Ctrl+PageDown"));
        assert_eq!(first(&Action::PrevTab).as_deref(), Some("Ctrl+PageUp"));
        assert_eq!(first(&Action::ReopenClosedTab).as_deref(), Some("Alt+Shift+T"));
        assert_eq!(first(&Action::Settings).as_deref(), Some("Alt+,"));
        assert_eq!(first(&Action::ViewDetails).as_deref(), Some("Ctrl+F1"));
        assert_eq!(first(&Action::NewFolder).as_deref(), Some("Alt+Shift+N"));
        assert_eq!(first(&Action::PasteAsLink).as_deref(), Some("Alt+Shift+V"));
        assert_eq!(first(&Action::Properties).as_deref(), Some("Alt+Shift+Enter"));
        // The Windows console: WT's paste arrives as typed keys, so Paste gets Alt+V.
        assert_eq!(first(&Action::Paste).as_deref(), Some("Alt+V"));
        // The user's chords stay, after the usable ones.
        assert_eq!(km.chords_for(&Action::CommandPalette), [ch("Ctrl+K"), ch("Ctrl+Shift+P")]);
        assert_eq!(km.chords_for(&Action::Paste), [ch("Alt+V"), ch("Ctrl+V")]);
        // Conditional chords are not touched: Copy keeps Ctrl+C, Back keeps Alt+Left.
        assert_eq!(first(&Action::Copy).as_deref(), Some("Ctrl+C"));
        assert_eq!(first(&Action::Back).as_deref(), Some("Alt+Left"));
        // Paste events (unix, WSL): Ctrl+V pastes through the bracketed paste.
        let mut km = defaults::preset(KeyPreset::Explorer);
        wt_stand_ins(&mut km, true);
        assert_eq!(km.chords_for(&Action::Paste), [ch("Ctrl+V")]);
        // Finder: Search has only Ctrl+Shift+F.
        let mut f = defaults::preset(KeyPreset::Finder);
        wt_stand_ins(&mut f, true);
        assert_eq!(soyuz_appcore::keys::chord_label(&f, &Action::Search).as_deref(), Some("Alt+Shift+F"));
        assert_eq!(soyuz_appcore::keys::chord_label(&f, &Action::ToggleHidden).as_deref(), Some("Ctrl+H"));
    }

    #[test]
    fn windows_terminal_respects_user_bindings() {
        // The user's own stand-in counts; the WT stand-in is not added.
        let mut km = defaults::preset(KeyPreset::Explorer);
        km.bindings.retain(|(_, a), _| *a != Action::CommandPalette);
        km.bind(KeyContext::Global, Action::CommandPalette, ch("Ctrl+Shift+P"));
        km.bind(KeyContext::Global, Action::CommandPalette, ch("F1"));
        wt_stand_ins(&mut km, false);
        assert_eq!(km.chords_for(&Action::CommandPalette), [ch("F1"), ch("Ctrl+Shift+P")]);
        // A taken stand-in is skipped.
        let mut km = defaults::preset(KeyPreset::Explorer);
        km.bind(KeyContext::FileList, Action::Refresh, ch("Alt+Shift+Enter"));
        wt_stand_ins(&mut km, false);
        assert_eq!(km.chords_for(&Action::Properties), [ch("Alt+Enter")]);
        assert_eq!(km.lookup(KeyContext::FileList, ch("Alt+Shift+Enter")), Some(&Action::Refresh));
    }

    #[test]
    fn tui_hints_name_no_chord_windows_terminal_takes() {
        use soyuz_appcore::tr;
        let hints = [
            tr!("tui-hint"),
            tr!("tui-hint-filter"),
            tr!("tui-hint-close"),
            tr!("tui-hint-delete"),
            tr!("tui-hint-quit"),
            tr!("tui-hint-prompt"),
            tr!("tui-hint-reject"),
            tr!("tui-hint-cancel"),
        ];
        // The palette footer: "↑↓ move · Enter run · Tab complete".
        let palette = ["Up", "Down", "Enter", "Tab"];
        let words = hints.iter().flat_map(|h| h.split([' ', '·', '/']).map(str::to_string).collect::<Vec<_>>());
        let mut checked = 0;
        for w in words.chain(palette.iter().map(|s| s.to_string())) {
            let Ok(c) = w.parse::<KeyChord>() else { continue };
            if !c.mods.is_empty() || w.len() > 1 {
                checked += 1;
            }
            for paste in [false, true] {
                assert!(!wt_blocks(c, &Action::Refresh, paste), "a hint names {w}");
            }
        }
        assert!(checked >= 4, "the hints were parsed ({checked})");
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

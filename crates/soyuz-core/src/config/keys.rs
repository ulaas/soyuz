//! Key chords, contexts and the keymap.
//!
//! Core has its own [`KeyName`] enum (no egui dependency); the app maps `egui::Key` ↔
//! `KeyName` in `soyuz-appcore/src/keys/bindings.rs`.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use bitflags::bitflags;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::actions::Action;

macro_rules! key_names {
    ($( $v:ident => $canon:literal $(| $alias:literal)* ),* $(,)?) => {
        /// A physical/logical key, independent of any GUI toolkit.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord)]
        #[serde(rename_all = "snake_case")]
        pub enum KeyName { $($v,)* }

        impl KeyName {
            pub const ALL: &'static [KeyName] = &[$(KeyName::$v,)*];

            /// Canonical chord spelling (`"A"`, `"F5"`, `"Esc"`, `"`"`).
            pub fn as_str(self) -> &'static str {
                match self { $(KeyName::$v => $canon,)* }
            }

            /// Parse a key name, case-insensitively, accepting aliases.
            pub fn parse(s: &str) -> Option<KeyName> {
                $(
                    if s.eq_ignore_ascii_case($canon) $(|| s.eq_ignore_ascii_case($alias))* {
                        return Some(KeyName::$v);
                    }
                )*
                None
            }
        }
    };
}

key_names! {
    A => "A", B => "B", C => "C", D => "D", E => "E", F => "F", G => "G", H => "H", I => "I",
    J => "J", K => "K", L => "L", M => "M", N => "N", O => "O", P => "P", Q => "Q", R => "R",
    S => "S", T => "T", U => "U", V => "V", W => "W", X => "X", Y => "Y", Z => "Z",
    Num0 => "0" | "Num0", Num1 => "1" | "Num1", Num2 => "2" | "Num2", Num3 => "3" | "Num3",
    Num4 => "4" | "Num4", Num5 => "5" | "Num5", Num6 => "6" | "Num6", Num7 => "7" | "Num7",
    Num8 => "8" | "Num8", Num9 => "9" | "Num9",
    F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6", F7 => "F7",
    F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12", F13 => "F13",
    F14 => "F14", F15 => "F15", F16 => "F16", F17 => "F17", F18 => "F18", F19 => "F19",
    F20 => "F20", F21 => "F21", F22 => "F22", F23 => "F23", F24 => "F24",
    Enter => "Enter" | "Return", Escape => "Esc" | "Escape", Tab => "Tab", Space => "Space",
    Backspace => "Backspace", Delete => "Del" | "Delete", Insert => "Ins" | "Insert",
    Home => "Home", End => "End", PageUp => "PageUp" | "PgUp", PageDown => "PageDown" | "PgDn",
    ArrowUp => "Up" | "ArrowUp", ArrowDown => "Down" | "ArrowDown",
    ArrowLeft => "Left" | "ArrowLeft", ArrowRight => "Right" | "ArrowRight",
    Backtick => "`" | "Backtick" | "Grave", Minus => "-" | "Minus", Plus => "Plus",
    Equals => "=" | "Equals", Comma => "," | "Comma", Period => "." | "Period",
    Slash => "/" | "Slash", Backslash => "\\" | "Backslash", Semicolon => ";" | "Semicolon",
    Quote => "'" | "Quote", OpenBracket => "[" | "OpenBracket", CloseBracket => "]" | "CloseBracket",
}

bitflags! {
    /// Modifier keys. `CMD` is the macOS command key (treated as `CTRL` elsewhere when
    /// `[keys] cmd_is_ctrl = true`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord, Serialize, Deserialize)]
    pub struct Mods: u8 {
        const CTRL = 1;
        const SHIFT = 2;
        const ALT = 4;
        const CMD = 8;
    }
}

/// A key plus modifiers, written `"Ctrl+Shift+P"`, `"F5"`, `"Cmd+C"`, `"Ctrl++"`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct KeyChord {
    pub key: KeyName,
    pub mods: Mods,
}

impl KeyChord {
    pub const fn new(mods: Mods, key: KeyName) -> Self {
        KeyChord { key, mods }
    }

    /// Replace `CMD` by `CTRL` when `cmd_is_ctrl` (non-mac platforms).
    pub fn normalized(self, cmd_is_ctrl: bool) -> Self {
        if cmd_is_ctrl && self.mods.contains(Mods::CMD) {
            KeyChord { key: self.key, mods: (self.mods - Mods::CMD) | Mods::CTRL }
        } else {
            self
        }
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (m, name) in [(Mods::CTRL, "Ctrl"), (Mods::ALT, "Alt"), (Mods::SHIFT, "Shift"), (Mods::CMD, "Cmd")] {
            if self.mods.contains(m) {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(self.key.as_str())
    }
}

/// Error parsing a key chord.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid key chord \"{0}\"")]
pub struct ParseChordError(pub String);

impl FromStr for KeyChord {
    type Err = ParseChordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseChordError(s.to_string());
        let t = s.trim();
        if t.is_empty() {
            return Err(err());
        }
        // A trailing "+" that is the key itself ("Ctrl++" or "+").
        let (mods_part, key_part) = if t == "+" {
            ("", "Plus")
        } else if let Some(stripped) = t.strip_suffix("++") {
            (stripped, "Plus")
        } else {
            match t.rsplit_once('+') {
                Some((m, k)) => (m, k),
                None => ("", t),
            }
        };
        let key = KeyName::parse(key_part.trim()).ok_or_else(err)?;
        let mut mods = Mods::empty();
        if !mods_part.is_empty() {
            for m in mods_part.split('+') {
                let bit = match m.trim().to_ascii_lowercase().as_str() {
                    "ctrl" | "control" | "ctl" => Mods::CTRL,
                    "shift" => Mods::SHIFT,
                    "alt" | "option" | "opt" => Mods::ALT,
                    "cmd" | "command" | "super" | "meta" | "win" => Mods::CMD,
                    _ => return Err(err()),
                };
                mods |= bit;
            }
        }
        Ok(KeyChord { key, mods })
    }
}

impl Serialize for KeyChord {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for KeyChord {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Where a binding applies; the most specific context wins over `Global`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum KeyContext {
    Global,
    FileList,
    Terminal,
    Dialog,
    Omnibar,
    Rename,
    /// The sidebar has keyboard focus.
    Sidebar,
}

impl KeyContext {
    /// Every context, in display order.
    pub const ALL: [KeyContext; 7] = [
        KeyContext::Global,
        KeyContext::FileList,
        KeyContext::Sidebar,
        KeyContext::Terminal,
        KeyContext::Dialog,
        KeyContext::Omnibar,
        KeyContext::Rename,
    ];

    /// Human label ("File list").
    pub fn label(self) -> &'static str {
        match self {
            KeyContext::Global => "Global",
            KeyContext::FileList => "File list",
            KeyContext::Terminal => "Terminal",
            KeyContext::Dialog => "Dialogs",
            KeyContext::Omnibar => "Text fields",
            KeyContext::Rename => "Rename",
            KeyContext::Sidebar => "Sidebar",
        }
    }
}

/// Built-in binding presets.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeyPreset {
    #[default]
    Explorer,
    Commander,
    /// macOS Finder-style (Enter renames, Cmd+Up/Down, Cmd+1..4 views).
    Finder,
    /// Vim / ranger-style letter keys and two-key sequences (`g g`, `y y`, `d d`).
    Vim,
}

impl KeyPreset {
    pub const ALL: [KeyPreset; 4] = [KeyPreset::Explorer, KeyPreset::Commander, KeyPreset::Finder, KeyPreset::Vim];

    pub fn label(self) -> &'static str {
        match self {
            KeyPreset::Explorer => "Explorer",
            KeyPreset::Commander => "Commander (F-keys)",
            KeyPreset::Finder => "Finder",
            KeyPreset::Vim => "Vim-ish",
        }
    }
}

/// A multi-key sequence (`"g g"`, `"Ctrl+K Ctrl+S"`): chords separated by whitespace.
pub type KeySequence = Vec<KeyChord>;

/// Parse a chord or a whitespace-separated chord sequence. A lone `"+"`-free key such as
/// `"g"` is a one-chord sequence.
pub fn parse_sequence(s: &str) -> Result<KeySequence, ParseChordError> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return Err(ParseChordError(s.to_string()));
    }
    parts.iter().map(|p| p.parse::<KeyChord>()).collect()
}

/// `"g g"` / `"Ctrl+K Ctrl+S"`.
pub fn format_sequence(seq: &[KeyChord]) -> String {
    seq.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
}

/// Result of matching typed chords against the sequence bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeqMatch {
    None,
    /// The typed chords start at least one longer sequence: wait for more keys.
    Prefix,
    Exact(Action),
}

/// The resolved key map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keymap {
    pub bindings: BTreeMap<(KeyContext, Action), Vec<KeyChord>>,
    /// Multi-chord sequences (two or more chords each).
    pub sequences: BTreeMap<(KeyContext, Action), Vec<KeySequence>>,
}

impl Keymap {
    /// Bind a sequence; a one-chord sequence is a plain [`bind`](Self::bind).
    pub fn bind_sequence(&mut self, ctx: KeyContext, action: Action, seq: KeySequence) {
        match seq.len() {
            0 => {}
            1 => self.bind(ctx, action, seq[0]),
            _ => {
                let v = self.sequences.entry((ctx, action)).or_default();
                if !v.contains(&seq) {
                    v.push(seq);
                }
            }
        }
    }

    /// Bind `chord` in `ctx`, removing it from every other action of that context first.
    pub fn bind_stealing(&mut self, ctx: KeyContext, action: Action, chord: KeyChord) {
        for ((bctx, _), v) in self.bindings.iter_mut() {
            if *bctx == ctx {
                v.retain(|x| *x != chord);
            }
        }
        self.bindings.retain(|_, v| !v.is_empty());
        self.bind(ctx, action, chord);
    }

    /// Match `typed` (≥ 1 chord) against the sequences of `ctx`, then `Global`.
    /// Single chords are only reported as [`SeqMatch::Prefix`] (use [`lookup`](Self::lookup)).
    pub fn sequence_match(&self, ctx: KeyContext, typed: &[KeyChord]) -> SeqMatch {
        if typed.is_empty() {
            return SeqMatch::None;
        }
        let in_ctx = |c: KeyContext| {
            let mut prefix = false;
            for ((bc, a), seqs) in &self.sequences {
                if *bc != c {
                    continue;
                }
                for s in seqs {
                    if s.as_slice() == typed {
                        return SeqMatch::Exact(a.clone());
                    }
                    if s.len() > typed.len() && s.starts_with(typed) {
                        prefix = true;
                    }
                }
            }
            if prefix { SeqMatch::Prefix } else { SeqMatch::None }
        };
        match in_ctx(ctx) {
            SeqMatch::None if ctx != KeyContext::Global => in_ctx(KeyContext::Global),
            m => m,
        }
    }

    /// All sequences bound to `action` in any context.
    pub fn sequences_for(&self, action: &Action) -> Vec<KeySequence> {
        self.sequences.iter().filter(|((_, a), _)| a == action).flat_map(|(_, s)| s.iter().cloned()).collect()
    }

    /// Sequences that can never fire, within one context: a single chord bound to an action
    /// that also starts a sequence, or a sequence that is a prefix of another one. Returns
    /// `(context, shorter binding, longer sequence, actions)`.
    pub fn prefix_conflicts(&self) -> Vec<(KeyContext, KeySequence, KeySequence, Vec<Action>)> {
        let mut out = Vec::new();
        for ((ctx, a), seqs) in &self.sequences {
            for s in seqs {
                for ((bctx, b), chords) in &self.bindings {
                    if bctx == ctx && chords.contains(&s[0]) {
                        out.push((*ctx, vec![s[0]], s.clone(), vec![b.clone(), a.clone()]));
                    }
                }
                for ((cctx, b), others) in &self.sequences {
                    if cctx != ctx {
                        continue;
                    }
                    for o in others {
                        if o.len() < s.len() && s.starts_with(o) {
                            out.push((*ctx, o.clone(), s.clone(), vec![b.clone(), a.clone()]));
                        }
                    }
                }
            }
        }
        out
    }

    /// Add a binding (no-op if already present).
    pub fn bind(&mut self, ctx: KeyContext, action: Action, chord: KeyChord) {
        let v = self.bindings.entry((ctx, action)).or_default();
        if !v.contains(&chord) {
            v.push(chord);
        }
    }

    /// Action bound to `chord` in `ctx`, falling back to `Global`.
    pub fn lookup(&self, ctx: KeyContext, chord: KeyChord) -> Option<&Action> {
        let find = |c: KeyContext| {
            self.bindings.iter().find(|((bc, _), chords)| *bc == c && chords.contains(&chord)).map(|((_, a), _)| a)
        };
        find(ctx).or_else(|| if ctx == KeyContext::Global { None } else { find(KeyContext::Global) })
    }

    /// Chords bound to more than one action within the same context.
    pub fn conflicts(&self) -> Vec<(KeyChord, Vec<Action>)> {
        let mut map: BTreeMap<(KeyContext, KeyChord), Vec<Action>> = BTreeMap::new();
        for ((ctx, action), chords) in &self.bindings {
            for c in chords {
                map.entry((*ctx, *c)).or_default().push(action.clone());
            }
        }
        map.into_iter().filter(|(_, v)| v.len() > 1).map(|((_, c), v)| (c, v)).collect()
    }

    /// All chords bound to `action` in any context (for shortcut hints).
    pub fn chords_for(&self, action: &Action) -> Vec<KeyChord> {
        self.bindings.iter().filter(|((_, a), _)| a == action).flat_map(|(_, c)| c.iter().copied()).collect()
    }

    /// Apply `[keys.bindings]` overrides (`action string → chord strings`). An overridden
    /// action loses all preset bindings and is re-bound in the context the preset used
    /// for it (else `Global`); an empty list unbinds it. Returns warnings for invalid
    /// entries.
    pub fn apply_overrides(&mut self, overrides: &BTreeMap<String, Vec<String>>, cmd_is_ctrl: bool) -> Vec<String> {
        let mut warnings = Vec::new();
        for (action_str, chords) in overrides {
            let action: Action = match action_str.parse() {
                Ok(a) => a,
                Err(e) => {
                    warnings.push(format!("keys.bindings: {e}"));
                    continue;
                }
            };
            let ctx = self
                .bindings
                .keys()
                .chain(self.sequences.keys())
                .find(|(_, a)| *a == action)
                .map(|(c, _)| *c)
                .unwrap_or(KeyContext::Global);
            self.bindings.retain(|(_, a), _| *a != action);
            self.sequences.retain(|(_, a), _| *a != action);
            for c in chords {
                if c.split_whitespace().count() > 1 {
                    match parse_sequence(c) {
                        Ok(seq) => {
                            let seq: KeySequence = seq.into_iter().map(|ch| ch.normalized(cmd_is_ctrl)).collect();
                            for ((bctx, _), v) in self.sequences.iter_mut() {
                                if *bctx == ctx {
                                    v.retain(|x| *x != seq);
                                }
                            }
                            self.sequences.retain(|_, v| !v.is_empty());
                            self.bind_sequence(ctx, action.clone(), seq);
                        }
                        Err(e) => warnings.push(format!("keys.bindings.{action_str}: {e}")),
                    }
                    continue;
                }
                match c.parse::<KeyChord>() {
                    Ok(ch) => {
                        let ch = ch.normalized(cmd_is_ctrl);
                        // The override steals the chord from other actions in the same context.
                        for ((bctx, _), v) in self.bindings.iter_mut() {
                            if *bctx == ctx {
                                v.retain(|x| *x != ch);
                            }
                        }
                        self.bindings.retain(|_, v| !v.is_empty());
                        self.bind(ctx, action.clone(), ch);
                    }
                    Err(e) => warnings.push(format!("keys.bindings.{action_str}: {e}")),
                }
            }
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_parse_format() {
        let c: KeyChord = "Ctrl+Shift+P".parse().unwrap();
        assert_eq!(c, KeyChord::new(Mods::CTRL | Mods::SHIFT, KeyName::P));
        assert_eq!(c.to_string(), "Ctrl+Shift+P");
        assert_eq!("shift+ctrl+p".parse::<KeyChord>().unwrap(), c);
        assert_eq!("F5".parse::<KeyChord>().unwrap(), KeyChord::new(Mods::empty(), KeyName::F5));
        assert_eq!("Ctrl++".parse::<KeyChord>().unwrap(), KeyChord::new(Mods::CTRL, KeyName::Plus));
        assert_eq!("Ctrl+`".parse::<KeyChord>().unwrap(), KeyChord::new(Mods::CTRL, KeyName::Backtick));
        assert_eq!("Alt+Up".parse::<KeyChord>().unwrap(), KeyChord::new(Mods::ALT, KeyName::ArrowUp));
        assert_eq!("Shift+Del".parse::<KeyChord>().unwrap().to_string(), "Shift+Del");
        let cmd: KeyChord = "Cmd+C".parse().unwrap();
        assert_eq!(cmd.normalized(true), KeyChord::new(Mods::CTRL, KeyName::C));
        assert_eq!(cmd.normalized(false), cmd);
        assert!("Ctrl+".parse::<KeyChord>().is_err());
        assert!("Hyper+X".parse::<KeyChord>().is_err());
        assert!("".parse::<KeyChord>().is_err());
    }

    #[test]
    fn all_keys_round_trip() {
        for &k in KeyName::ALL {
            for mods in [Mods::empty(), Mods::CTRL | Mods::ALT | Mods::SHIFT | Mods::CMD] {
                let c = KeyChord::new(mods, k);
                assert_eq!(c.to_string().parse::<KeyChord>().unwrap(), c, "{c}");
            }
        }
    }

    #[test]
    fn keymap_lookup_and_conflicts() {
        let mut km = Keymap::default();
        let f5 = KeyChord::new(Mods::empty(), KeyName::F5);
        km.bind(KeyContext::Global, Action::Refresh, f5);
        km.bind(KeyContext::FileList, Action::CopyToOtherPane, f5);
        assert_eq!(km.lookup(KeyContext::FileList, f5), Some(&Action::CopyToOtherPane));
        assert_eq!(km.lookup(KeyContext::Terminal, f5), Some(&Action::Refresh));
        assert!(km.conflicts().is_empty());
        km.bind(KeyContext::Global, Action::Search, f5);
        let c = km.conflicts();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].0, f5);
        assert_eq!(c[0].1.len(), 2);
    }

    #[test]
    fn overrides() {
        let mut km = Keymap::default();
        km.bind(KeyContext::FileList, Action::Copy, "Ctrl+C".parse().unwrap());
        let mut o = BTreeMap::new();
        o.insert("copy".to_string(), vec!["Cmd+Insert".to_string()]);
        o.insert("bogus".to_string(), vec!["F1".to_string()]);
        let w = km.apply_overrides(&o, true);
        assert_eq!(w.len(), 1);
        assert_eq!(km.chords_for(&Action::Copy), vec![KeyChord::new(Mods::CTRL, KeyName::Insert)]);
        assert_eq!(km.lookup(KeyContext::FileList, KeyChord::new(Mods::CTRL, KeyName::Insert)), Some(&Action::Copy));
    }

    #[test]
    fn sequences_parse_and_match() {
        let seq = parse_sequence("g g").unwrap();
        assert_eq!(seq.len(), 2);
        assert_eq!(format_sequence(&seq), "G G");
        assert_eq!(parse_sequence("Ctrl+K  Ctrl+S").unwrap().len(), 2);
        assert!(parse_sequence("  ").is_err());
        assert!(parse_sequence("g Hyper+x").is_err());

        let mut km = Keymap::default();
        km.bind_sequence(KeyContext::FileList, Action::CursorHome, seq.clone());
        km.bind_sequence(KeyContext::Global, Action::Settings, parse_sequence("Ctrl+K Ctrl+S").unwrap());
        let g = seq[0];
        assert_eq!(km.sequence_match(KeyContext::FileList, &[g]), SeqMatch::Prefix);
        assert_eq!(km.sequence_match(KeyContext::FileList, &seq), SeqMatch::Exact(Action::CursorHome));
        assert_eq!(km.sequence_match(KeyContext::Terminal, &seq), SeqMatch::None);
        let ks = parse_sequence("Ctrl+K Ctrl+S").unwrap();
        assert_eq!(km.sequence_match(KeyContext::FileList, &ks), SeqMatch::Exact(Action::Settings), "global fallback");
        assert_eq!(km.sequences_for(&Action::CursorHome), vec![seq.clone()]);
        assert!(km.prefix_conflicts().is_empty());
        km.bind(KeyContext::FileList, Action::Refresh, g);
        assert_eq!(km.prefix_conflicts().len(), 1);
    }

    #[test]
    fn sequence_overrides() {
        let mut km = Keymap::default();
        km.bind(KeyContext::FileList, Action::CursorHome, "Home".parse().unwrap());
        let mut o = BTreeMap::new();
        o.insert("cursor_home".to_string(), vec!["Home".to_string(), "g g".to_string()]);
        assert!(km.apply_overrides(&o, true).is_empty());
        assert_eq!(km.chords_for(&Action::CursorHome).len(), 1);
        assert_eq!(km.sequences_for(&Action::CursorHome), vec![parse_sequence("g g").unwrap()]);
        // Sequences live in the action's original context.
        assert!(km.sequences.contains_key(&(KeyContext::FileList, Action::CursorHome)));
        let mut o = BTreeMap::new();
        o.insert("cursor_home".to_string(), vec![]);
        km.apply_overrides(&o, true);
        assert!(km.sequences_for(&Action::CursorHome).is_empty());
        assert!(km.chords_for(&Action::CursorHome).is_empty());
    }

    #[test]
    fn stealing_bind() {
        let mut km = Keymap::default();
        let c: KeyChord = "Ctrl+1".parse().unwrap();
        km.bind(KeyContext::Global, Action::SelectTab(1), c);
        km.bind_stealing(KeyContext::Global, Action::OpenBookmark(1), c);
        assert_eq!(km.lookup(KeyContext::Global, c), Some(&Action::OpenBookmark(1)));
        assert!(km.chords_for(&Action::SelectTab(1)).is_empty());
        assert!(km.conflicts().is_empty());
    }
}

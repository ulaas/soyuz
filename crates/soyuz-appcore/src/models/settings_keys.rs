//! The keyboard settings page without a toolkit: the `[keys.bindings]` override helpers,
//! the effective key map with its conflicts (cached until key-related config changes),
//! the filtered action rows and the shortcut recorder's rules.

use soyuz_core::config::{
    self, Action, Bookmark, Config, CustomCommand, KeyChord, KeyContext, KeyName, Keymap, Keys, format_sequence,
};

use crate::keys::conflicts::{self, Conflict, ConflictKind, for_action};
use crate::models::settings::matches;
use crate::palette::catalog;

/// Effective bindings of `a` as strings (chords first, then sequences).
pub fn current_bindings(km: &Keymap, a: &Action) -> Vec<String> {
    let mut v: Vec<String> = km.chords_for(a).iter().map(|c| c.to_string()).collect();
    v.extend(km.sequences_for(a).iter().map(|s| format_sequence(s)));
    v.dedup();
    v
}

/// Replace the `[keys.bindings]` entry of `a` (removing differently spelled keys).
pub fn set_override(c: &mut Config, a: &Action, list: Vec<String>) {
    c.keys.bindings.retain(|k, _| k.parse::<Action>().ok().as_ref() != Some(a));
    c.keys.bindings.insert(a.to_string(), list);
}

/// Drop the override of `a` (back to the preset).
pub fn reset_override(c: &mut Config, a: &Action) {
    c.keys.bindings.retain(|k, _| k.parse::<Action>().ok().as_ref() != Some(a));
}

pub fn is_overridden(c: &Config, a: &Action) -> bool {
    c.keys.bindings.keys().any(|k| k.parse::<Action>().ok().as_ref() == Some(a))
}

/// The key maps of a config, rebuilt only when key-related config changes.
#[derive(Clone, Debug)]
pub struct KeyCache {
    keys: Keys,
    bookmarks: Vec<Bookmark>,
    commands: Vec<CustomCommand>,
    /// `soyuz_core::config::keymap`: the config's own bindings (what `config.toml` says).
    pub base: Keymap,
    /// [`crate::keys::build_keymap`]: plus the frontend's overlay (the terminal UI's
    /// alternatives for chords legacy terminals cannot send).
    pub keymap: Keymap,
    pub warnings: Vec<String>,
    /// Conflicts of the effective key map.
    pub conflicts: Vec<Conflict>,
}

impl KeyCache {
    pub fn build(c: &Config) -> KeyCache {
        let (base, _) = config::keymap(c);
        let (keymap, warnings) = crate::keys::build_keymap(c);
        Self::with_keymaps(c, base, keymap, warnings)
    }

    /// A cache from given key maps (tests use a hand-made overlay).
    pub fn with_keymaps(c: &Config, base: Keymap, keymap: Keymap, warnings: Vec<String>) -> KeyCache {
        let conflicts = conflicts::analyze(&keymap);
        KeyCache {
            keys: c.keys.clone(),
            bookmarks: c.bookmarks.clone(),
            commands: c.custom_commands.clone(),
            base,
            keymap,
            warnings,
            conflicts,
        }
    }

    /// Whether `c` changed anything the key maps depend on.
    pub fn is_stale(&self, c: &Config) -> bool {
        self.keys != c.keys || self.bookmarks != c.bookmarks || self.commands != c.custom_commands
    }

    /// The cache in `slot`, rebuilt first when missing or stale.
    pub fn refresh<'a>(slot: &'a mut Option<KeyCache>, c: &Config) -> &'a KeyCache {
        if slot.as_ref().is_none_or(|k| k.is_stale(c)) {
            *slot = Some(KeyCache::build(c));
        }
        slot.as_ref().expect("just built")
    }
}

/// One action of the keyboard page.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyRow {
    pub action: Action,
    pub entry: catalog::Entry,
    /// Effective bindings (chords, then sequences), overlay included.
    pub bindings: Vec<String>,
    /// `[keys.bindings]` overrides this action.
    pub overridden: bool,
    /// Bindings that only the frontend overlay adds (not in `config.toml`; tag them).
    pub overlay_only: Vec<String>,
    /// Contexts that bind the action.
    pub contexts: Vec<KeyContext>,
    /// Conflicts that involve the action.
    pub conflicts: Vec<Conflict>,
}

/// The keyboard page's rows for `filter` (every word must match the title, English
/// title, keywords, action id, bindings or category), as the GUI filters them.
pub fn rows(c: &Config, filter: &str, conflicts_only: bool, modified_only: bool) -> Vec<KeyRow> {
    rows_with(&KeyCache::build(c), c, filter, conflicts_only, modified_only)
}

/// [`rows`] with a prepared cache.
pub fn rows_with(cache: &KeyCache, c: &Config, filter: &str, conflicts_only: bool, modified_only: bool) -> Vec<KeyRow> {
    let mut actions: Vec<Action> = Action::all();
    let n_bm = c.bookmarks.len();
    actions.retain(|a| {
        !matches!(a, Action::OpenBookmark(n) if *n as usize > n_bm.max(3)) && !crate::actions::unavailable(a)
    });
    actions.extend(
        c.custom_commands.iter().filter(|c| !c.name.trim().is_empty()).map(|c| Action::CustomCommand(c.name.clone())),
    );
    actions
        .into_iter()
        .filter_map(|a| {
            let entry = catalog::entry(&a);
            let bindings = current_bindings(&cache.keymap, &a);
            let text_ok = matches(
                filter,
                &[
                    &entry.title,
                    &entry.title_en,
                    entry.keywords,
                    &a.to_string(),
                    &bindings.join(" "),
                    crate::i18n::labels::action_category(entry.category),
                ],
            );
            let mine: Vec<Conflict> = for_action(&cache.conflicts, &a).into_iter().cloned().collect();
            let conflict_ok = !conflicts_only || mine.iter().any(|c| c.kind != ConflictKind::Shadowed);
            let overridden = is_overridden(c, &a);
            if !(text_ok && conflict_ok && (!modified_only || overridden)) {
                return None;
            }
            let base = current_bindings(&cache.base, &a);
            let overlay_only = bindings.iter().filter(|b| !base.contains(b)).cloned().collect();
            let mut contexts: Vec<KeyContext> = cache
                .keymap
                .bindings
                .keys()
                .chain(cache.keymap.sequences.keys())
                .filter(|(_, b)| *b == a)
                .map(|(ctx, _)| *ctx)
                .collect();
            contexts.sort();
            contexts.dedup();
            Some(KeyRow { action: a, entry, bindings, overridden, overlay_only, contexts, conflicts: mine })
        })
        .collect()
}

/// Most chords in one recorded binding (a sequence).
pub const MAX_CHORDS: usize = 3;

/// What a key press does in the shortcut recorder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordOutcome {
    /// Recorded (or ignored past [`MAX_CHORDS`]); keep listening.
    Pending,
    /// Enter after at least one chord: save this binding string.
    Save(String),
    /// Esc: close without changes.
    Cancel,
}

/// Feed one chord to the recorder (`chords` so far): Esc cancels, Enter saves when
/// something was recorded, anything else is recorded (up to [`MAX_CHORDS`]). The GUI
/// recorder's rules.
pub fn record_outcome(chords: &mut Vec<KeyChord>, next: KeyChord) -> RecordOutcome {
    if next.mods.is_empty() && next.key == KeyName::Escape {
        RecordOutcome::Cancel
    } else if next.mods.is_empty() && next.key == KeyName::Enter && !chords.is_empty() {
        RecordOutcome::Save(format_sequence(chords))
    } else {
        if chords.len() < MAX_CHORDS {
            chords.push(next);
        }
        RecordOutcome::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::{Mods, keymap};

    #[test]
    fn override_helpers() {
        let mut c = Config::default();
        let (km, _) = keymap(&c);
        let before = current_bindings(&km, &Action::CommandPalette);
        assert!(before.contains(&"Ctrl+Shift+P".to_string()), "{before:?}");
        c.keys.bindings.insert("Command-Palette".into(), vec!["F1".into()]);
        assert!(is_overridden(&c, &Action::CommandPalette));
        set_override(&mut c, &Action::CommandPalette, vec!["F1".into(), "Ctrl+K Ctrl+P".into()]);
        assert_eq!(c.keys.bindings.len(), 1, "differently spelled key replaced");
        let (km, w) = keymap(&c);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(current_bindings(&km, &Action::CommandPalette), vec!["F1", "Ctrl+K Ctrl+P"]);
        reset_override(&mut c, &Action::CommandPalette);
        assert!(!is_overridden(&c, &Action::CommandPalette));
        // Removing every chord keeps an empty override (= unbound).
        set_override(&mut c, &Action::Refresh, vec![]);
        let (km, _) = keymap(&c);
        assert!(current_bindings(&km, &Action::Refresh).is_empty());
    }

    #[test]
    fn rows_filter_like_the_gui() {
        let mut c = Config::default();
        let all = rows(&c, "", false, false);
        assert!(all.len() > 100, "{}", all.len());
        assert!(!all.iter().any(|r| crate::actions::unavailable(&r.action)));
        assert!(!all.iter().any(|r| matches!(r.action, Action::OpenBookmark(n) if n > 3)));
        let refresh = rows(&c, "refresh", false, false);
        assert!(refresh.iter().any(|r| r.action == Action::Refresh), "{refresh:?}");
        assert!(refresh.len() < 20);
        // Words AND together and match bindings too.
        let by_chord = rows(&c, "ctrl+shift+p", false, false);
        assert!(by_chord.iter().any(|r| r.action == Action::CommandPalette));
        assert!(rows(&c, "", false, true).is_empty(), "nothing overridden yet");
        set_override(&mut c, &Action::Refresh, vec!["F9".into()]);
        let modified = rows(&c, "", false, true);
        assert_eq!(modified.iter().map(|r| r.action.clone()).collect::<Vec<_>>(), vec![Action::Refresh]);
        assert!(modified[0].overridden && modified[0].bindings == ["F9"]);
        // A duplicate shows under "conflicts only".
        // (Overrides steal their chord, so make the duplicate by hand.)
        let (base, w) = keymap(&c);
        let mut km = base.clone();
        km.bind(KeyContext::Global, Action::NewTab, KeyChord::new(Mods::empty(), KeyName::F9));
        km.bind(KeyContext::Global, Action::Refresh, KeyChord::new(Mods::empty(), KeyName::F9));
        let cache = KeyCache::with_keymaps(&c, base, km, w);
        let conflicting = rows_with(&cache, &c, "", true, false);
        assert!(conflicting.iter().any(|r| r.action == Action::Refresh), "{conflicting:?}");
        assert!(conflicting.iter().all(|r| r.conflicts.iter().any(|x| x.kind != ConflictKind::Shadowed)));
        // Custom commands with a name are listed.
        c.custom_commands.push(CustomCommand { name: "Lint".into(), ..Default::default() });
        assert!(rows(&c, "lint", false, false).iter().any(|r| r.action == Action::CustomCommand("Lint".into())));
    }

    #[test]
    fn overlay_only_chords_are_tagged() {
        let c = Config::default();
        let (base, w) = keymap(&c);
        let mut effective = base.clone();
        let f12 = KeyChord::new(Mods::empty(), KeyName::F12);
        effective.bind(KeyContext::Global, Action::Refresh, f12);
        let cache = KeyCache::with_keymaps(&c, base, effective, w);
        let rows = rows_with(&cache, &c, "", false, false);
        let r = rows.iter().find(|r| r.action == Action::Refresh).unwrap();
        assert_eq!(r.overlay_only, vec!["F12".to_string()]);
        assert!(r.bindings.contains(&"F12".to_string()));
        assert!(r.contexts.contains(&KeyContext::Global));
        let other = rows.iter().find(|r| r.action == Action::CommandPalette).unwrap();
        assert!(other.overlay_only.is_empty());
        // The cache rebuilds only when key-related config changes.
        let mut slot = Some(cache);
        let mut c2 = c.clone();
        c2.general.show_hidden = !c2.general.show_hidden;
        assert!(!slot.as_ref().unwrap().is_stale(&c2));
        c2.keys.marks = !c2.keys.marks;
        assert!(slot.as_ref().unwrap().is_stale(&c2));
        let fresh = KeyCache::refresh(&mut slot, &c2);
        assert!(!fresh.is_stale(&c2));
    }

    #[test]
    fn recorder_rules() {
        let ch = |m: Mods, k: KeyName| KeyChord::new(m, k);
        let mut chords = Vec::new();
        // Enter before anything is recorded is recorded itself.
        assert_eq!(record_outcome(&mut chords, ch(Mods::empty(), KeyName::Enter)), RecordOutcome::Pending);
        assert_eq!(chords.len(), 1);
        chords.clear();
        assert_eq!(record_outcome(&mut chords, ch(Mods::CTRL, KeyName::K)), RecordOutcome::Pending);
        assert_eq!(record_outcome(&mut chords, ch(Mods::CTRL, KeyName::S)), RecordOutcome::Pending);
        assert_eq!(record_outcome(&mut chords, ch(Mods::empty(), KeyName::G)), RecordOutcome::Pending);
        assert_eq!(record_outcome(&mut chords, ch(Mods::empty(), KeyName::H)), RecordOutcome::Pending);
        assert_eq!(chords.len(), MAX_CHORDS, "at most three chords");
        assert_eq!(
            record_outcome(&mut chords, ch(Mods::empty(), KeyName::Enter)),
            RecordOutcome::Save("Ctrl+K Ctrl+S G".into())
        );
        assert_eq!(record_outcome(&mut chords, ch(Mods::empty(), KeyName::Escape)), RecordOutcome::Cancel);
        // Shift+Esc is a chord, not a cancel.
        let mut one = Vec::new();
        assert_eq!(record_outcome(&mut one, ch(Mods::SHIFT, KeyName::Escape)), RecordOutcome::Pending);
    }
}

//! The key bindings page of the settings editor (the Keyboard section, Phase T6): the
//! `[keys]` switches (preset, Cmd is Ctrl, marks) as ordinary keys, the problems of the
//! effective key map (conflicts other than shadowing, invalid entries), two filter switches
//! and every action with its bindings as chips, grouped by category.
//!
//! The rows come from `soyuz_appcore::models::settings_keys` with the terminal UI's own
//! key map (`keys::build_keymap`: the overlay's alternatives for chords legacy terminals
//! cannot send are tagged); the search field filters them like the GUI.
//!
//! | Key (on an action) | Does |
//! |---|---|
//! | Enter | record another shortcut (the recorder dialog, `dialogs::keybind`) |
//! | Left / Right | choose a chip |
//! | Delete | remove the chosen binding |
//! | Backspace | back to the preset's bindings |
//! | Alt+C / Alt+M | only conflicts / only changed actions (also Enter / Space on the switches) |
//!
//! Ctrl+R (the editor's section reset) resets every key setting, overrides included.

use soyuz_appcore::keys::conflicts::ConflictKind;
use soyuz_appcore::models::settings_keys::{self as model, KeyCache, KeyRow};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_core::config::{Action, Config, KeyChord, KeyName, Mods};

use super::settings::{Commit, Line, Used, post};
use crate::app::App;
use crate::input::dialogs;

/// A filter switch of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyFilter {
    /// Only actions in a conflict (shadowing aside).
    Conflicts,
    /// Only actions with their own bindings (`[keys.bindings]`).
    Modified,
}

/// What the page keeps between frames (`settings::State::keys`).
#[derive(Debug, Default)]
pub struct State {
    /// The chosen chip of the action under the cursor.
    pub chip: usize,
    pub conflicts_only: bool,
    pub modified_only: bool,
    /// The key maps of the live config.
    pub cache: Option<KeyCache>,
    /// The listed actions, by category.
    pub rows: Vec<KeyRow>,
    /// Conflicts (shadowing aside) and key map warnings, as messages.
    pub problems: Vec<String>,
    /// What `rows` were built for: (filter, conflicts only, modified only).
    built: Option<(String, bool, bool)>,
}

/// Most problem lines listed (then "and N more").
pub const MAX_PROBLEMS: usize = 3;

/// Keep the rows current for `config` and the filter `query`.
pub fn refresh(st: &mut State, config: &Config, query: &str) {
    let stale = st.cache.as_ref().is_none_or(|k| k.is_stale(config));
    let key = (query.to_string(), st.conflicts_only, st.modified_only);
    if !stale && st.built.as_ref() == Some(&key) {
        return;
    }
    let cache = KeyCache::refresh(&mut st.cache, config);
    let mut rows = model::rows_with(cache, config, query, st.conflicts_only, st.modified_only);
    rows.sort_by_key(|r| r.entry.category);
    st.rows = rows;
    st.problems = cache
        .conflicts
        .iter()
        .filter(|c| c.kind != ConflictKind::Shadowed)
        .map(|c| c.message())
        .chain(cache.warnings.iter().cloned())
        .collect();
    st.built = Some(key);
}

/// Append the page's lines (after the `[keys]` switches): problems, filter switches,
/// then the actions under their category headings.
pub fn lines(st: &State, out: &mut Vec<Line>) {
    if !st.problems.is_empty() {
        out.push(Line::Note(tr!("tui-settings-keys-problems", count = st.problems.len())));
        for p in st.problems.iter().take(MAX_PROBLEMS) {
            out.push(Line::Note(format!("  {p}")));
        }
        if st.problems.len() > MAX_PROBLEMS {
            out.push(Line::Note(format!(
                "  {}",
                tr!("tui-settings-keys-more", count = st.problems.len() - MAX_PROBLEMS)
            )));
        }
    }
    out.push(Line::Toggle(KeyFilter::Conflicts));
    out.push(Line::Toggle(KeyFilter::Modified));
    let mut last = None;
    for (i, r) in st.rows.iter().enumerate() {
        if last != Some(r.entry.category) {
            last = Some(r.entry.category);
            out.push(Line::Header(soyuz_appcore::i18n::labels::action_category(r.entry.category).to_string()));
        }
        out.push(Line::Action(i));
    }
    if st.rows.is_empty() {
        out.push(Line::Header(tr!("tui-settings-no-results").to_string()));
    }
}

/// Flip a filter switch.
pub fn toggle_filter(st: &mut State, f: KeyFilter) {
    match f {
        KeyFilter::Conflicts => st.conflicts_only = !st.conflicts_only,
        KeyFilter::Modified => st.modified_only = !st.modified_only,
    }
}

/// Whether a filter switch is on.
pub fn filter_on(st: &State, f: KeyFilter) -> bool {
    match f {
        KeyFilter::Conflicts => st.conflicts_only,
        KeyFilter::Modified => st.modified_only,
    }
}

/// Keys of the whole page (Alt+C, Alt+M).
pub fn page_key(app: &mut App, c: &KeyChord) -> Used {
    if c.mods != Mods::ALT {
        return Used::No;
    }
    let f = match c.key {
        KeyName::C => KeyFilter::Conflicts,
        KeyName::M => KeyFilter::Modified,
        _ => return Used::No,
    };
    toggle_filter(&mut app.input.settings.keys, f);
    super::settings::refresh(&app.core, &mut app.input.settings);
    Used::Yes
}

/// A key on the line of action `i`.
pub fn action_key(app: &mut App, i: usize, c: &KeyChord) -> Used {
    if !c.mods.is_empty() {
        return page_key(app, c);
    }
    let st = &mut app.input.settings.keys;
    let Some(row) = st.rows.get(i) else { return Used::No };
    let n = row.bindings.len();
    match c.key {
        KeyName::Enter => record(app, i, None),
        KeyName::ArrowLeft => st.chip = st.chip.saturating_sub(1),
        KeyName::ArrowRight => st.chip = (st.chip + 1).min(n.saturating_sub(1)),
        KeyName::Delete => remove_chip(app, i),
        KeyName::Backspace => {
            let action = row.action.clone();
            reset_action(app, &action);
        }
        _ => return Used::No,
    }
    Used::Yes
}

/// A click on chip `chip` of the action on line `line`: choose it; a second click
/// records a replacement for it.
pub fn chip_click(app: &mut App, line: usize, chip: usize) {
    let st = &mut app.input.settings;
    let Some(super::settings::Line::Action(i)) = st.lines.get(line).cloned() else { return };
    let again = st.list.cursor == line && st.keys.chip == chip;
    st.list.cursor = line;
    st.part = super::settings::Part::Rows;
    st.keys.chip = chip;
    if again {
        let replace = st.keys.rows.get(i).and_then(|r| r.bindings.get(chip)).cloned();
        record(app, i, replace);
    }
}

/// Open the recorder for action `i` (replacing binding `replace`, else adding one).
pub fn record(app: &mut App, i: usize, replace: Option<String>) {
    let Some(row) = app.input.settings.keys.rows.get(i) else { return };
    let action = row.action.clone();
    let inbox = app.input.settings.inbox.clone();
    dialogs::keybind::open_action(app, action, replace, inbox);
}

/// The config's own bindings of `a` (without the terminal UI's additions).
pub fn base_bindings(config: &Config, a: &Action) -> Vec<String> {
    let (base, _) = soyuz_core::config::keymap(config);
    model::current_bindings(&base, a)
}

/// Remove the chosen binding of action `i` (the terminal UI's own additions cannot be
/// removed here: they follow the preset).
fn remove_chip(app: &mut App, i: usize) {
    let st = &app.input.settings.keys;
    let Some(row) = st.rows.get(i) else { return };
    let Some(chip) = row.bindings.get(st.chip).cloned() else { return };
    if row.overlay_only.contains(&chip) {
        app.core.ui.toasts.push(Toast::info(tr!("tui-settings-keys-overlay-fixed", keys = chip.as_str())));
        return;
    }
    let action = row.action.clone();
    let list: Vec<String> = base_bindings(&app.core.config, &action).into_iter().filter(|b| *b != chip).collect();
    let inbox = app.input.settings.inbox.clone();
    post(&inbox, &app.core, Commit::Bindings { action, list });
    let chip = &mut app.input.settings.keys.chip;
    *chip = chip.saturating_sub(1);
    super::settings::sync(app);
}

/// Drop the override of `a`: back to the preset's bindings.
fn reset_action(app: &mut App, a: &Action) {
    if !model::is_overridden(&app.core.config, a) {
        return;
    }
    let mut c = app.core.config.clone();
    model::reset_override(&mut c, a);
    let Ok(value) = toml::Value::try_from(&c.keys.bindings) else { return };
    let inbox = app.input.settings.inbox.clone();
    post(&inbox, &app.core, Commit::Set { path: "keys.bindings".into(), value });
    app.input.settings.keys.chip = 0;
    super::settings::sync(app);
}

/// The role of chip `chip` of `row`: a conflict, a shadowed chord, one the terminal UI
/// adds, or an ordinary one.
pub fn chip_kind(row: &KeyRow, chip: &str) -> ChipKind {
    let names = |keys: &str| keys.split(" / ").any(|k| k == chip);
    if row.conflicts.iter().any(|c| c.kind != ConflictKind::Shadowed && names(&c.keys)) {
        ChipKind::Conflict
    } else if row.conflicts.iter().any(|c| c.kind == ConflictKind::Shadowed && names(&c.keys)) {
        ChipKind::Shadowed
    } else if row.overlay_only.iter().any(|o| o == chip) {
        ChipKind::Overlay
    } else {
        ChipKind::Plain
    }
}

/// What a chip is (its colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipKind {
    Plain,
    Conflict,
    Shadowed,
    Overlay,
}

#[cfg(test)]
mod tests {
    use soyuz_core::config::Config;

    use super::*;

    #[test]
    fn rows_group_by_category_and_follow_the_filters() {
        let c = Config::default();
        let mut st = State::default();
        refresh(&mut st, &c, "");
        assert!(st.rows.len() > 100);
        assert!(st.rows.windows(2).all(|w| w[0].entry.category <= w[1].entry.category));
        let mut out = Vec::new();
        lines(&st, &mut out);
        assert!(out.contains(&Line::Toggle(KeyFilter::Conflicts)));
        assert_eq!(out.iter().filter(|l| matches!(l, Line::Action(_))).count(), st.rows.len());
        refresh(&mut st, &c, "refresh");
        assert!(st.rows.iter().any(|r| r.action == Action::Refresh));
        assert!(st.rows.len() < 20);
        toggle_filter(&mut st, KeyFilter::Modified);
        refresh(&mut st, &c, "");
        assert!(st.rows.is_empty(), "nothing is overridden");
        let mut out = Vec::new();
        lines(&st, &mut out);
        assert!(matches!(out.last(), Some(Line::Header(_))));
    }

    #[test]
    fn a_sequence_after_a_bound_chord_is_a_problem() {
        let mut c = Config::default();
        let mut st = State::default();
        refresh(&mut st, &c, "");
        let before = st.problems.len();
        let first = base_bindings(&c, &Action::NewTab);
        let chord = first.first().expect("New tab is bound").clone();
        model::set_override(&mut c, &Action::Refresh, vec![format!("{chord} G")]);
        refresh(&mut st, &c, "refresh");
        assert!(st.problems.len() > before, "{:?}", st.problems);
        let row = st.rows.iter().find(|r| r.action == Action::Refresh).unwrap();
        assert_eq!(chip_kind(row, &format!("{chord} G")), ChipKind::Conflict, "{row:?}");
    }
}

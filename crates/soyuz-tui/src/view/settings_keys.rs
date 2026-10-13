//! The view of the key bindings page of the settings editor (the Keyboard section): the
//! line of an action (title, then its bindings as chips and a `+` to add one), the filter
//! switches, and the description of an action (contexts, conflicts, the terminal UI's own
//! additions). Drawn inside `view::settings`.
//!
//! Chips: a conflict in [`Role::Danger`], a shadowed chord in [`Role::Warning`], a chord
//! the terminal UI adds for legacy terminals muted, the chosen chip of the cursor line as
//! a focused button.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use soyuz_appcore::models::settings_keys::KeyRow;
use soyuz_appcore::tr;

use super::settings::{LineKind, LineVm};
use super::text;
use crate::input::settings_keys::{ChipKind, KeyFilter, State, chip_kind, filter_on};
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};

/// One binding of an action as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChipVm {
    pub text: String,
    pub kind: ChipKind,
    /// The chosen chip of the cursor line (Delete removes it).
    pub chosen: bool,
}

/// The line of `row` (`chip`: the chosen chip when it is the cursor line).
pub fn action_line(index: usize, row: &KeyRow, chip: Option<usize>) -> LineVm {
    LineVm {
        index,
        kind: LineKind::Action,
        label: crate::dialog::safe(&row.entry.title),
        modified: row.overridden,
        cursor: chip.is_some(),
        chips: row
            .bindings
            .iter()
            .enumerate()
            .map(|(i, b)| ChipVm { text: b.clone(), kind: chip_kind(row, b), chosen: chip == Some(i) })
            .collect(),
        ..LineVm::default()
    }
}

/// The line of a filter switch.
pub fn toggle_line(index: usize, st: &State, f: KeyFilter, cursor: bool) -> LineVm {
    let (label, key) = match f {
        KeyFilter::Conflicts => (tr!("tui-settings-keys-conflicts-only"), "Alt+C"),
        KeyFilter::Modified => (tr!("tui-settings-keys-modified-only"), "Alt+M"),
    };
    let mark = if filter_on(st, f) { "[x]" } else { "[ ]" };
    LineVm {
        index,
        kind: LineKind::Toggle,
        label: format!("{mark} {label}"),
        value: key.into(),
        value_role: Some(Role::KeyHint),
        cursor,
        ..LineVm::default()
    }
}

/// What the description lines say about an action.
pub fn description(row: &KeyRow) -> Vec<String> {
    let mut out = Vec::new();
    if !row.entry.description.is_empty() {
        out.push(row.entry.description.clone());
    }
    if !row.contexts.is_empty() {
        let names: Vec<&str> = row.contexts.iter().map(|c| soyuz_appcore::i18n::labels::key_context(*c)).collect();
        out.push(tr!("tui-settings-keys-contexts", contexts = names.join(", ")));
    }
    out.extend(row.conflicts.iter().map(|c| c.message()));
    if !row.overlay_only.is_empty() {
        out.push(tr!("tui-settings-keys-overlay", keys = row.overlay_only.join(", ")));
    }
    out
}

fn chip_style(theme: &TuiTheme, c: &ChipVm) -> Style {
    if c.chosen {
        return theme.style(Role::ButtonFocused);
    }
    theme.style(match c.kind {
        ChipKind::Plain => Role::Button,
        ChipKind::Conflict => Role::Danger,
        ChipKind::Shadowed => Role::Warning,
        ChipKind::Overlay => Role::Muted,
    })
}

/// Draw the chips (and the `+`) of action line `m` into `area` (one row), left to right
/// while they fit; recording `SettingsKeyChip` / `SettingsKeyAdd` targets.
pub fn draw_chips(buf: &mut Buffer, area: Rect, m: &LineVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    let mut x = area.x;
    let right = area.right();
    for (i, c) in m.chips.iter().enumerate() {
        let label = format!("[{}]", crate::dialog::safe(&c.text));
        let w = text::width(&label) as u16;
        // Keep room for the `+` (2 cells).
        if x + w + 2 > right {
            if x < right {
                buf.set_stringn(x, area.y, "…", 1, theme.style(Role::Muted));
                x += 2;
            }
            break;
        }
        buf.set_stringn(x, area.y, &label, usize::from(w), chip_style(theme, c));
        hits.push(Rect::new(x, area.y, w, 1), HitTarget::SettingsKeyChip { row: m.index, chip: i });
        x += w + 1;
    }
    if x < right {
        buf.set_stringn(x, area.y, "+", 1, theme.style(Role::Accent));
        hits.push(Rect::new(x, area.y, 1, 1), HitTarget::SettingsKeyAdd(m.index));
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use soyuz_core::config::Config;

    use super::*;
    use crate::input::settings_keys::refresh;

    #[test]
    fn chips_fit_and_are_targets() {
        let mut st = State::default();
        refresh(&mut st, &Config::default(), "command palette");
        let row = st.rows.iter().find(|r| r.action == soyuz_core::config::Action::CommandPalette).unwrap();
        let line = action_line(7, row, Some(0));
        assert!(line.chips[0].chosen);
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 1));
        let mut hits = Hits::default();
        draw_chips(&mut buf, Rect::new(10, 0, 50, 1), &line, &TuiTheme::plain(), &mut hits);
        let text: String = (0..60).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(text.contains(&format!("[{}]", line.chips[0].text)), "{text}");
        assert!(hits.iter().any(|(_, t)| *t == HitTarget::SettingsKeyChip { row: 7, chip: 0 }));
        assert!(hits.iter().any(|(_, t)| *t == HitTarget::SettingsKeyAdd(7)));
        // Too narrow: never panics, never writes outside.
        for w in 0..12 {
            let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
            draw_chips(&mut buf, Rect::new(0, 0, w, 1), &line, &TuiTheme::plain(), &mut Hits::default());
        }
        assert!(!description(row).is_empty());
    }
}

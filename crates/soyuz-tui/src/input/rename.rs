//! Keys of the inline rename field (`TuiFocus::Rename`, the core's `fileops.rename`) —
//! Phase T3.
//!
//! F2 (the core's `Action::Rename` on one item), and the new folder / new file flows
//! (the core's `AfterJob::SelectAndRename`), put an `InlineRename` in
//! `core.fileops.rename`; while one is open on a pane of the current tab the field owns
//! the keyboard ([`active`]). Like the GUI (`ui::views::rename`):
//!
//! | Key | Does |
//! |---|---|
//! | typing, paste | replaces the selected part, else inserts at the cursor |
//! | F2 | selects the stem, then the extension, then the whole name (`next_sel`) |
//! | Ctrl+A | selects the whole name |
//! | Enter | commits (an undoable `Rename` job through the jobs engine) |
//! | Tab / Shift+Tab | commits and renames the next / previous item |
//! | Esc | cancels |
//! | a click elsewhere | commits (a click away with an invalid name drops the edit) |
//!
//! The field starts with the part `[behaviour] rename_selects` asks for selected (the
//! stem by default). An invalid name (empty, reserved, bad characters, an existing name)
//! shows its reason on the row and blocks Enter. Validation and the commit are the
//! core's (`soyuz_appcore::ops::fileops::rename::{revalidate, finish}`); this module
//! keeps the cursor and the selection, which the core does not.

use crossterm::event::{Event, MouseEvent, MouseEventKind};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::ops::fileops::rename::{InlineRename, Outcome, finish, next_sel, revalidate, sel_range};
use soyuz_appcore::state::pane::PaneId;
use soyuz_core::config::{KeyName, Mods};
use unicode_segmentation::UnicodeSegmentation;

use super::{InputState, TuiFocus, core_input, key_context};
use crate::app::App;
use crate::layout::HitTarget;
use crate::view::text;
use crate::widgets::LineEdit;
use crate::widgets::line_edit::EditOutcome;

/// What the rename field keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// Cursor and scroll of the field.
    pub edit: LineEdit,
    /// The selected bytes of the name (typing replaces them).
    pub sel: Option<(usize, usize)>,
    /// The rename the field was set up for: pane and original name.
    pub of: Option<(PaneId, String)>,
}

/// Whether an inline rename owns the keyboard: one is open on a pane of the current tab.
pub fn active(core: &AppCore, _st: &InputState) -> bool {
    core.fileops.rename.as_ref().is_some_and(|r| core.ws.tab().panes.iter().any(|p| p.id == r.pane))
}

/// Byte offset of character `c` in `s`.
fn byte_of(s: &str, c: usize) -> usize {
    s.char_indices().nth(c).map_or(s.len(), |(i, _)| i)
}

/// The selection `sel` asks for in `r.text`, with the cursor at its end.
fn selected(r: &InlineRename) -> (LineEdit, Option<(usize, usize)>) {
    let (a, b) = sel_range(&r.text, r.is_dir, r.sel);
    let (a, b) = (byte_of(&r.text, a), byte_of(&r.text, b));
    let mut e = LineEdit::new();
    e.set_cursor(&r.text, b);
    (e, (a < b).then_some((a, b)))
}

/// The field's cursor and selection for `r`: the kept ones, or fresh ones when the
/// rename is new or asks for its selection again (`apply_sel`).
pub fn effective(r: &InlineRename, st: &State) -> (LineEdit, Option<(usize, usize)>) {
    let ours = st.of.as_ref().is_some_and(|(p, k)| *p == r.pane && *k == r.key);
    if r.apply_sel || !ours {
        return selected(r);
    }
    let sel =
        st.sel.filter(|&(a, b)| a < b && b <= r.text.len() && r.text.is_char_boundary(a) && r.text.is_char_boundary(b));
    (st.edit.clone(), sel)
}

/// Take over a new rename (or one that asks for its selection again).
fn sync(st: &mut State, r: &mut InlineRename) {
    let (edit, sel) = effective(r, st);
    st.edit = edit;
    st.sel = sel;
    st.of = Some((r.pane, r.key.clone()));
    r.apply_sel = false;
}

/// What one event did to the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Ignored,
    Consumed,
    Done(Outcome),
}

fn plain(m: Mods) -> bool {
    !m.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT)
}

/// Apply one event to the field (`r.text`, `st`'s cursor and selection).
fn apply(st: &mut State, r: &mut InlineRename, ev: &InputEvent) -> Step {
    if let InputEvent::Chord(c) = ev {
        let ctrl = c.mods.intersects(Mods::CTRL | Mods::CMD) && !c.mods.contains(Mods::ALT);
        match c.key {
            KeyName::F2 if plain(c.mods) => {
                r.sel = next_sel(&r.text, r.is_dir, r.sel);
                (st.edit, st.sel) = selected(r);
                return Step::Consumed;
            }
            KeyName::A if ctrl && !c.mods.contains(Mods::SHIFT) => {
                st.sel = (!r.text.is_empty()).then_some((0, r.text.len()));
                st.edit.set_cursor(&r.text, r.text.len());
                return Step::Consumed;
            }
            _ => {}
        }
        if let Some((a, b)) = st.sel
            && !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD)
        {
            match c.key {
                KeyName::Backspace | KeyName::Delete => {
                    r.text.replace_range(a..b, "");
                    st.edit.set_cursor(&r.text, a);
                    st.sel = None;
                    return Step::Consumed;
                }
                KeyName::ArrowLeft | KeyName::ArrowRight if !c.mods.contains(Mods::SHIFT) => {
                    st.edit.set_cursor(&r.text, if c.key == KeyName::ArrowLeft { a } else { b });
                    st.sel = None;
                    return Step::Consumed;
                }
                _ => {}
            }
        }
    }
    if let (InputEvent::Text(_), Some((a, b))) = (ev, st.sel) {
        r.text.replace_range(a..b, "");
        st.edit.set_cursor(&r.text, a);
        st.sel = None;
    }
    let (before, at) = (r.text.clone(), st.edit.cursor());
    let out = st.edit.apply(&mut r.text, ev);
    if matches!(ev, InputEvent::Chord(_)) && (r.text != before || st.edit.cursor() != at) {
        // Moving or editing drops the selection; a typing key (its text follows) keeps it.
        st.sel = None;
    }
    match out {
        EditOutcome::Consumed => Step::Consumed,
        EditOutcome::Submit => Step::Done(Outcome::Commit),
        EditOutcome::Cancel => Step::Done(Outcome::Cancel),
        EditOutcome::Tab => Step::Done(Outcome::CommitAndMove(1)),
        EditOutcome::ShiftTab => Step::Done(Outcome::CommitAndMove(-1)),
        // A one-line field: Up / Down / PageUp / PageDown do nothing (like the GUI's).
        EditOutcome::Up | EditOutcome::Down | EditOutcome::PageUp | EditOutcome::PageDown => Step::Consumed,
        EditOutcome::Ignored => Step::Ignored,
    }
}

/// A mouse press this frame that is not on the field.
fn clicked_away(app: &App, events: &[Event]) -> bool {
    events.iter().any(|e| match e {
        Event::Mouse(m) if matches!(m.kind, MouseEventKind::Down(_)) => {
            crate::input::mouse::hit(&app.hits, m.column, m.row).map(|(_, t)| t) != Some(HitTarget::RenameField)
        }
        _ => false,
    })
}

/// Keys while the field is active.
pub fn route(app: &mut App, events: &[Event]) {
    if let Some(r) = app.core.fileops.rename.as_mut() {
        sync(&mut app.input.rename, r);
    }
    let (kctx, text_focused) = key_context(TuiFocus::Rename);
    let mut frame = core_input(app, events, kctx, text_focused);
    let away = app.core.config.tui.mouse && clicked_away(app, events);
    let core = &mut app.core;
    let st = &mut app.input.rename;
    // A binding may have ended or replaced the rename.
    let Some(r) = core.fileops.rename.as_mut() else { return };
    if st.of.as_ref().is_none_or(|(p, k)| *p != r.pane || *k != r.key) || r.apply_sel {
        sync(st, r);
    }
    let mut outcome = Outcome::Editing;
    for i in 0..frame.events.len() {
        if frame.consumed[i] {
            continue;
        }
        match apply(st, r, &frame.events[i]) {
            Step::Ignored => {}
            Step::Consumed => frame.consumed[i] = true,
            Step::Done(o) => {
                frame.consumed[i] = true;
                outcome = o;
                break;
            }
        }
    }
    let clicked = outcome == Outcome::Editing && away;
    if clicked {
        outcome = Outcome::Commit;
    }
    let pane_id = r.pane;
    let Some(pane) = core.ws.find_pane(pane_id) else { return };
    let text = revalidate(r, pane, core.config.general.strict_windows_names, &core.ui.volumes);
    let selects = core.config.behaviour.rename_selects;
    let Some(pane) = core.ws.find_pane_mut(pane_id) else { return };
    finish(&mut core.fileops.rename, pane, outcome, text, clicked, selects, &mut core.cmds);
}

/// A mouse press on the field (`RenameField`): the cursor goes under the pointer.
pub fn click(app: &mut App, m: &MouseEvent) {
    if !matches!(m.kind, MouseEventKind::Down(_)) {
        return;
    }
    let Some(field) = app.hits.iter().filter(|(_, t)| *t == HitTarget::RenameField).map(|(r, _)| *r).next() else {
        return;
    };
    let Some(r) = app.core.fileops.rename.as_mut() else { return };
    let st = &mut app.input.rename;
    sync(st, r);
    let col = usize::from(m.column.saturating_sub(field.x));
    // The field scrolls only when the name does not fit; then the click keeps the cursor.
    if text::width(&r.text) < usize::from(field.width) {
        let mut x = 0;
        let mut at = r.text.len();
        for (i, g) in r.text.grapheme_indices(true) {
            let w = text::width(g);
            if col < x + w.max(1) {
                at = i;
                break;
            }
            x += w;
        }
        st.edit.set_cursor(&r.text, at);
    }
    st.sel = None;
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use soyuz_appcore::ops::fileops::rename::RenameSel;

    use super::*;

    fn rename(name: &str) -> InlineRename {
        InlineRename {
            pane: PaneId(1),
            key: name.into(),
            path: PathBuf::from(name),
            is_dir: false,
            text: name.into(),
            sel: RenameSel::Stem,
            apply_sel: true,
            error: None,
        }
    }

    fn ch(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn txt(s: &str) -> InputEvent {
        InputEvent::Text(s.into())
    }

    #[test]
    fn the_stem_is_selected_and_typing_replaces_it() {
        let mut st = State::default();
        let mut r = rename("report.txt");
        sync(&mut st, &mut r);
        assert_eq!(st.sel, Some((0, 6)));
        assert!(!r.apply_sel);
        // The typing chord keeps the selection; its text replaces it.
        assert_eq!(apply(&mut st, &mut r, &ch("N")), Step::Consumed);
        assert_eq!(st.sel, Some((0, 6)));
        assert_eq!(apply(&mut st, &mut r, &txt("n")), Step::Consumed);
        assert_eq!(r.text, "n.txt");
        assert_eq!(st.edit.cursor(), 1);
        assert_eq!(st.sel, None);
        assert_eq!(apply(&mut st, &mut r, &ch("Enter")), Step::Done(Outcome::Commit));
    }

    #[test]
    fn f2_cycles_and_keys_end_the_edit() {
        let mut st = State::default();
        let mut r = rename("çağrı.tar");
        sync(&mut st, &mut r);
        assert_eq!(st.sel, Some((0, "çağrı".len())));
        apply(&mut st, &mut r, &ch("F2"));
        assert_eq!(st.sel, Some(("çağrı.".len(), r.text.len())), "the extension");
        apply(&mut st, &mut r, &ch("F2"));
        assert_eq!(st.sel, Some((0, r.text.len())), "the whole name");
        apply(&mut st, &mut r, &ch("Backspace"));
        assert_eq!(r.text, "");
        apply(&mut st, &mut r, &txt("x"));
        apply(&mut st, &mut r, &ch("Ctrl+A"));
        assert_eq!(st.sel, Some((0, 1)));
        apply(&mut st, &mut r, &ch("Left"));
        assert_eq!((st.sel, st.edit.cursor()), (None, 0));
        assert_eq!(apply(&mut st, &mut r, &ch("Esc")), Step::Done(Outcome::Cancel));
        assert_eq!(apply(&mut st, &mut r, &ch("Tab")), Step::Done(Outcome::CommitAndMove(1)));
        assert_eq!(apply(&mut st, &mut r, &ch("Shift+Tab")), Step::Done(Outcome::CommitAndMove(-1)));
        assert_eq!(apply(&mut st, &mut r, &ch("Ctrl+Q")), Step::Ignored);
    }

    #[test]
    fn a_new_rename_starts_fresh() {
        let mut st = State::default();
        let mut r = rename("a.txt");
        sync(&mut st, &mut r);
        apply(&mut st, &mut r, &ch("End"));
        assert_eq!(st.sel, None);
        assert_eq!(effective(&r, &st).1, None, "kept");
        let other = rename("b.txt");
        assert_eq!(effective(&other, &st).1, Some((0, 1)), "another item starts with its stem selected");
    }
}

//! The drop shelf (`TuiDialog::Shelf`, Phase T7, agent A; owner decision 6): the paths on
//! the shelf (`models::tools::with_shelf`, stat'ed on workers; missing ones marked) with
//! copy / move here (into the focused pane's folder, as jobs: the GUI's footer), go to
//! (the pane shows the item's folder with the item selected), remove and clear, like the
//! GUI's shelf panel. The shelf only holds references.
//!
//! [`sync`] opens it when the core's shelf flag is on (`Action::ToggleShelf`, "Add to
//! shelf") and none is open; closing the dialog any way turns the flag off ([`State`]'s
//! `Drop`), and a flag turned off elsewhere closes it ([`stale`]).
//!
//! Keys: Up / Down / PageUp / PageDown / Home / End move the cursor; Space marks the row
//! (copy, move and remove then apply to the marked rows, else copy / move take every
//! present item and remove the cursor row); Enter goes to the item; Del removes; the
//! buttons' letters work too; Esc closes. Mouse: a click puts the cursor on a row,
//! Ctrl+click marks it, a second click goes there; the wheel scrolls.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::KeyModifiers;
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::tools::{shelf_open, with_shelf};
use soyuz_appcore::ops::fileops::FileOpCmd;
use soyuz_appcore::ops::fileops::drop::DropEffect;
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::KeyName;

use super::confirm::{ListClick, list_mouse};
use super::prompt::{LIST_PAGE, buttons, click, is_key, list_key};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Form, FormOutcome, SelectList};

/// Buttons, in order.
pub const COPY: usize = 0;
pub const MOVE: usize = 1;
pub const REMOVE: usize = 2;
pub const CLEAR: usize = 3;
pub const CLOSE: usize = 4;

/// How often the items are stat'ed again while the shelf shows (moved / deleted ones).
pub const RECHECK: Duration = Duration::from_secs(2);

/// What the dialog keeps while it is open.
#[derive(Debug)]
pub struct State {
    pub list: SelectList,
    /// The marked rows.
    pub marked: BTreeSet<usize>,
    /// Buttons: copy here, move here, remove, clear, close.
    pub form: Form,
    last_check: Instant,
}

impl Default for State {
    fn default() -> Self {
        let labels = [
            tr!("tools-shelf-copy-here"),
            tr!("tools-shelf-move-here"),
            tr!("tui-shelf-remove"),
            tr!("tui-shelf-clear"),
            tr!("common-close"),
        ];
        let mut form = Form::new(Vec::new(), buttons(&labels)).default_button(CLOSE);
        form.focus_button(CLOSE);
        State { list: SelectList::new(), marked: BTreeSet::new(), form, last_check: Instant::now() }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // However it closes (Esc, a stale close, a quit), the shelf is off again.
        with_shelf(|d| d.open = false);
    }
}

/// Open the shelf dialog (and turn the shelf on).
pub fn open(app: &mut App) {
    with_shelf(|d| d.open = true);
    app.push_dialog(TuiDialog::Shelf(State::default()));
}

/// Every `crate::tools::drain` (before the tool queue): follow the core's shelf flag
/// (`models::tools::shelf_open`, flipped by `Action::ToggleShelf` and turned on by "Add to
/// shelf"): the dialog opens when the flag is on and none is open. The flag turning off
/// closes the dialog through [`stale`].
pub fn sync(app: &mut App) {
    let shown = app.input.dialogs.tui.iter().any(|d| matches!(d, TuiDialog::Shelf(_)));
    if shelf_open() && !shown {
        open(app);
    }
}

/// The focused pane's folder (copy / move here), if it is one.
pub fn destination(core: &AppCore) -> Option<PathBuf> {
    match &core.ws.pane().location {
        Location::Dir(d) => Some(d.clone()),
        _ => None,
    }
}

/// The rows copy / move / remove apply to: the marked ones, else (`all`) every present
/// item, or the cursor row.
fn targets(s: &State, all: bool) -> Vec<PathBuf> {
    with_shelf(|d| {
        if !s.marked.is_empty() || all {
            d.targets(&s.marked)
        } else {
            d.items.get(s.list.cursor).map(|i| vec![i.path.clone()]).unwrap_or_default()
        }
    })
}

fn transfer(core: &mut AppCore, s: &mut State, effect: DropEffect) {
    let Some(dest) = destination(core) else { return };
    let paths = targets(s, true);
    if paths.is_empty() {
        return;
    }
    core.cmds.push(Cmd::FileOp(FileOpCmd::Transfer { paths: paths.clone(), dest, effect }));
    if effect == DropEffect::Move {
        // Moved items are delivered: take them off the shelf.
        with_shelf(|d| d.items.retain(|i| !paths.contains(&i.path)));
        s.marked.clear();
    }
}

fn remove(s: &mut State) {
    let rows = if s.marked.is_empty() { BTreeSet::from([s.list.cursor]) } else { std::mem::take(&mut s.marked) };
    with_shelf(|d| d.remove(&rows));
}

fn go(s: &State) -> Option<Outcome> {
    let path = with_shelf(|d| d.items.get(s.list.cursor).map(|i| i.path.clone()))?;
    Some(Outcome::Close(Some(Cmd::RevealPath(path))))
}

fn outcome(core: &mut AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(COPY) => transfer(core, s, DropEffect::Copy),
        FormOutcome::Pressed(MOVE) => transfer(core, s, DropEffect::Move),
        FormOutcome::Pressed(REMOVE) => remove(s),
        FormOutcome::Pressed(CLEAR) => {
            with_shelf(|d| d.items.clear());
            s.marked.clear();
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => return Some(Outcome::Close(None)),
        _ => {}
    }
    None
}

/// Keep the cursor and the marks inside the list and the buttons that need items (or a
/// folder to copy into) enabled only then.
fn sync_form(core: &AppCore, s: &mut State) {
    let n = with_shelf(|d| d.items.len());
    s.list.clamp(n, LIST_PAGE);
    s.marked.retain(|&i| i < n);
    let here = destination(core).is_some();
    for (i, b) in s.form.buttons.iter_mut().enumerate() {
        b.enabled = match i {
            COPY | MOVE => n > 0 && here,
            REMOVE | CLEAR => n > 0,
            _ => true,
        };
    }
    if s.form.focused_button().is_none_or(|b| !s.form.buttons[b].enabled) {
        s.form.focus_button(CLOSE);
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        sync_form(core, s);
        let n = with_shelf(|d| d.items.len());
        if list_key(&mut s.list, ev, n) {
            continue;
        }
        if is_key(ev, KeyName::Space) && n > 0 {
            let c = s.list.cursor;
            if !s.marked.remove(&c) {
                s.marked.insert(c);
            }
            continue;
        }
        if is_key(ev, KeyName::Delete) && n > 0 {
            remove(s);
            continue;
        }
        // Enter goes to the item, unless a button was picked with Tab / arrows.
        if is_key(ev, KeyName::Enter) && n > 0 && s.form.focused_button() == Some(CLOSE) {
            return go(s).unwrap_or(Outcome::Keep);
        }
        let o = s.form.apply(ev);
        if let Some(out) = outcome(core, s, o) {
            return out;
        }
    }
    sync_form(core, s);
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    sync_form(core, s);
    let n = with_shelf(|d| d.items.len());
    let ctrl = m.event.modifiers.contains(KeyModifiers::CONTROL);
    match list_mouse(&mut s.list, n, m) {
        Some(ListClick::Row { i, .. }) if ctrl => {
            if !s.marked.remove(&i) {
                s.marked.insert(i);
            }
            return Outcome::Keep;
        }
        Some(ListClick::Row { again: true, .. }) => return go(s).unwrap_or(Outcome::Keep),
        Some(_) => return Outcome::Keep,
        None => {}
    }
    let o = click(&mut s.form, m);
    let out = outcome(core, s, o).unwrap_or(Outcome::Keep);
    sync_form(core, s);
    out
}

/// The dialog went away by itself: the shelf was turned off elsewhere.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    !shelf_open()
}

/// Every frame while it is open (`input::dialogs::tick`, from `view::prepare`): finished
/// stats, a fresh stat of every item every [`RECHECK`], the cursor and buttons kept in
/// step.
pub fn tick(core: &mut AppCore, s: &mut State) {
    with_shelf(|d| {
        d.poll();
        if !d.items.is_empty() && s.last_check.elapsed() >= RECHECK {
            s.last_check = Instant::now();
            d.stat(d.items.iter().map(|i| i.path.clone()).collect());
        }
    });
    sync_form(core, s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_turns_the_shelf_off_and_remove_takes_the_marked_rows() {
        with_shelf(|d| {
            d.items.clear();
            d.add(vec![PathBuf::from("/x/a"), PathBuf::from("/x/b"), PathBuf::from("/x/c")]);
            d.open = true;
        });
        let mut s = State::default();
        s.marked.insert(0);
        s.marked.insert(2);
        assert_eq!(targets(&s, false), [PathBuf::from("/x/a"), PathBuf::from("/x/c")]);
        remove(&mut s);
        assert_eq!(soyuz_appcore::models::tools::shelf_paths(), [PathBuf::from("/x/b")]);
        // Without marks remove takes the cursor row.
        remove(&mut s);
        assert!(soyuz_appcore::models::tools::shelf_paths().is_empty());
        assert!(shelf_open());
        drop(s);
        assert!(!shelf_open(), "a closed dialog turns the shelf off");
    }
}

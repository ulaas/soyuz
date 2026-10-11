//! Quick Look (frontend-agnostic): whether the overlay is open and how it moves through
//! the focused pane's list. The previewed item is the pane's cursor item; the frontend
//! (`ui::quicklook`) renders it and maps keys to [`step`] / [`open_item`] / [`close`].

use crate::app::{Focus, SoyuzApp};
use crate::cmd::Cmd;

#[derive(Clone, Debug, Default)]
pub struct QuickLook {
    pub open: bool,
}

impl QuickLook {
    /// Close the overlay (the frontend drops what it rendered once it sees it closed).
    pub fn close(&mut self) {
        self.open = false;
    }
}

/// A move through the list while Quick Look is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Prev,
    Next,
    First,
    Last,
}

/// Space: open on the cursor item (or close when open).
pub fn toggle(app: &mut SoyuzApp) {
    if app.ui.quicklook.open {
        app.ui.quicklook.close();
        return;
    }
    let pane = app.ws.pane_mut();
    if pane.cursor.is_none() && !pane.visible.is_empty() {
        let first = pane.selected_entries().first().map(|e| e.name.clone());
        match first.and_then(|n| pane.visible_pos(&n)) {
            Some(pos) => pane.move_cursor(pos, false, true),
            None => pane.move_cursor(0, false, false),
        }
    }
    if pane.cursor.is_some() {
        app.ui.quicklook.open = true;
    }
}

/// Close Quick Look.
pub fn close(app: &mut SoyuzApp) {
    app.ui.quicklook.close();
}

/// Move the cursor (and so the previewed item) linearly through the list, regardless of
/// the view layout.
pub fn step(app: &mut SoyuzApp, d: Step) {
    let pane = app.ws.pane_mut();
    let n = pane.visible.len();
    if n > 0 {
        let i = pane.cursor_pos().unwrap_or(0);
        let t = match d {
            Step::Prev => i.saturating_sub(1),
            Step::Next => (i + 1).min(n - 1),
            Step::First => 0,
            Step::Last => n - 1,
        };
        pane.move_cursor(t, false, false);
    }
    app.ui.focus = Focus::List;
}

/// Enter: close and open the item.
pub fn open_item(app: &mut SoyuzApp) {
    app.ui.quicklook.close();
    app.cmds.push(Cmd::Action(soyuz_core::config::Action::Open));
}

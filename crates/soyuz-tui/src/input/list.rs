//! Keys for the file list: the core's bindings, sequences, Tab / Left / Right and
//! type-ahead (`soyuz_appcore::keys::input::handle`).
//!
//! A paste from the terminal (`Event::Paste`: Ctrl+Shift+V, Ctrl+V in Windows Terminal,
//! Shift+Insert, a right / middle click) runs Soyuz's Paste instead of typing the text:
//! the terminal ate the paste chord itself, and pasted text in type-ahead is never useful
//! (it would jump to a name that starts with a path or a whole paragraph). Paste reads the
//! system clipboard, so files copied in another file manager or in Soyuz land in the
//! current folder like Ctrl+V in the GUI. Text fields (address, filter, palette, dialogs)
//! still get the text. This is done in every terminal, not only Windows Terminal: any
//! terminal paste into the list means "paste here".

use crossterm::event::Event;

use super::{TuiFocus, core_input, key_context};
use crate::app::App;

/// The frame's events in order: runs of terminal events for the core's key handling,
/// split at each paste.
#[derive(Debug, PartialEq)]
pub enum Part<'a> {
    Keys(&'a [Event]),
    Paste,
}

/// Split `events` at the pastes (empty runs are dropped).
pub fn split_pastes(events: &[Event]) -> Vec<Part<'_>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, e) in events.iter().enumerate() {
        if matches!(e, Event::Paste(_)) {
            if i > start {
                out.push(Part::Keys(&events[start..i]));
            }
            out.push(Part::Paste);
            start = i + 1;
        }
    }
    if start < events.len() {
        out.push(Part::Keys(&events[start..]));
    }
    out
}

pub fn route(app: &mut App, events: &[Event]) {
    let (kctx, text) = key_context(TuiFocus::List);
    for part in split_pastes(events) {
        match part {
            Part::Keys(keys) => {
                core_input(app, keys, kctx, text);
            }
            Part::Paste => (app.list_paste)(app),
        }
    }
}

/// What a terminal paste into the file list does ([`App::list_paste`]): the core's Paste.
pub fn paste(app: &mut App) {
    soyuz_appcore::actions::execute(&mut app.core, soyuz_core::config::Action::Paste, &app.host);
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    fn key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    #[test]
    fn pastes_split_the_frame() {
        assert_eq!(split_pastes(&[]), vec![]);
        let keys = [key('a'), key('b')];
        assert_eq!(split_pastes(&keys), vec![Part::Keys(&keys)]);
        let p = Event::Paste("C:\\x.txt\r\n".into());
        let empty = Event::Paste(String::new());
        let evs = [key('a'), p.clone(), key('b'), key('c'), empty, p];
        assert_eq!(
            split_pastes(&evs),
            vec![Part::Keys(&evs[..1]), Part::Paste, Part::Keys(&evs[2..4]), Part::Paste, Part::Paste]
        );
    }
}

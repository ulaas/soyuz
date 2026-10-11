//! One frame of keyboard / pointer input, without a toolkit type.
//!
//! The frontend lists this frame's events in order (one [`InputEvent`] per raw event, so
//! indices line up); the core marks what it consumed, and the frontend removes exactly
//! those events before its widgets run.

use soyuz_core::ViewMode;
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, Mods};

use crate::app::SoyuzApp;
use crate::host::Host;
use crate::state::layout::NavDir;
use crate::{actions, ops};

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// A key press (repeats included) or a clipboard shortcut, as a chord.
    Chord(KeyChord),
    /// Typed text.
    Text(String),
    /// Anything else (pointer, key release, IME, …): never consumed by the core.
    Other,
}

/// This frame's input for one window.
#[derive(Clone, Debug, Default)]
pub struct InputFrame {
    pub events: Vec<InputEvent>,
    /// Parallel to `events`: the core took this event.
    pub consumed: Vec<bool>,
    /// Ctrl, Alt or Cmd is held (typed text is then not type-ahead).
    pub command_held: bool,
    /// The mouse "back" / "forward" buttons were pressed.
    pub back: bool,
    pub forward: bool,
}

impl InputFrame {
    pub fn new(events: Vec<InputEvent>) -> Self {
        let consumed = vec![false; events.len()];
        InputFrame { events, consumed, ..Default::default() }
    }

    /// Indices of events not consumed yet.
    pub fn live(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.events.len()).filter(|&i| !self.consumed[i])
    }

    /// Consume every live press of `key` without Ctrl / Cmd (Shift and Alt are ignored,
    /// like a plain shortcut); returns whether there was one.
    pub fn consume_plain(&mut self, key: KeyName) -> bool {
        let mut hit = false;
        for i in 0..self.events.len() {
            if self.consumed[i] {
                continue;
            }
            if let InputEvent::Chord(c) = &self.events[i]
                && c.key == key
                && !c.mods.intersects(Mods::CTRL | Mods::CMD)
            {
                self.consumed[i] = true;
                hit = true;
            }
        }
        hit
    }

    /// Like [`consume_plain`](Self::consume_plain), but only presses with Shift held.
    pub fn consume_shifted(&mut self, key: KeyName) -> bool {
        let mut hit = false;
        for i in 0..self.events.len() {
            if self.consumed[i] {
                continue;
            }
            if let InputEvent::Chord(c) = &self.events[i]
                && c.key == key
                && c.mods.contains(Mods::SHIFT)
                && !c.mods.intersects(Mods::CTRL | Mods::CMD)
            {
                self.consumed[i] = true;
                hit = true;
            }
        }
        hit
    }

    /// Printable text typed this frame that nothing consumed (for type-ahead).
    pub fn typed_text(&self) -> String {
        if self.command_held {
            return String::new();
        }
        self.live()
            .filter_map(|i| match &self.events[i] {
                InputEvent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// The main window's keyboard handling once no modal, panel or editor owns the keyboard:
/// key bindings, sequences and marks, mouse back / forward, and in the file list Tab
/// (switch panes), Left / Right (grid, list and Miller views) and type-ahead.
pub fn handle(app: &mut SoyuzApp, host: &dyn Host, frame: &mut InputFrame, kctx: KeyContext, text_focused: bool) {
    super::sequence::dispatch_all(app, host, frame, kctx, text_focused);
    if frame.back {
        actions::execute(app, Action::Back, host);
    }
    if frame.forward {
        actions::execute(app, Action::Forward, host);
    }
    if kctx != KeyContext::FileList || text_focused {
        return;
    }
    // Unbound Tab switches panes in dual mode instead of moving widget focus.
    if frame.consume_plain(KeyName::Tab) && app.ws.tab().dual {
        actions::execute(app, Action::FocusOtherPane, host);
    }
    // Left/Right: horizontal movement in the icon/list views, Miller in/out.
    for (key, dir) in [(KeyName::ArrowLeft, NavDir::Left), (KeyName::ArrowRight, NavDir::Right)] {
        let plain = frame.consume_plain(key);
        let shift = frame.consume_shifted(key);
        if !plain && !shift {
            continue;
        }
        match app.ws.pane().view.mode {
            ViewMode::Grid | ViewMode::Gallery | ViewMode::List => actions::cursor(app, dir, shift, false),
            ViewMode::Miller if dir == NavDir::Left => actions::execute(app, Action::Up, host),
            ViewMode::Miller => {
                let pane = app.ws.pane();
                let target = pane.cursor.as_deref().and_then(|c| pane.entry(c)).filter(|e| e.is_dir_like());
                if let Some(e) = target {
                    let key = e.name.clone();
                    let id = pane.id;
                    ops::open_entry(app, id, &key);
                }
            }
            ViewMode::Details => {}
        }
    }
    let typed = frame.typed_text();
    if !typed.is_empty() {
        ops::type_ahead(app, &typed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    #[test]
    fn a_bound_letter_never_leaks_into_type_ahead() {
        use crate::keys::sequence::{Feed, Sequencer, feed_frame};
        use soyuz_core::config::{KeyPreset, defaults};
        let km = defaults::preset(KeyPreset::Vim);
        let mut seq = Sequencer::default();
        // `j` is bound (cursor down), `q` is not: only `q` reaches type-ahead.
        let mut f = InputFrame::new(vec![ch("J"), InputEvent::Text("j".into()), ch("Q"), InputEvent::Text("q".into())]);
        let now = std::time::Instant::now();
        let out = feed_frame(&mut f, &km, &mut seq, KeyContext::FileList, false, false, now);
        assert_eq!(out, vec![Feed::Action(Action::CursorDown)]);
        assert_eq!(f.consumed, vec![true, true, false, false]);
        assert_eq!(f.typed_text(), "q");
        // A focused text field keeps editing keys and text.
        let mut f = InputFrame::new(vec![ch("J"), InputEvent::Text("j".into())]);
        feed_frame(&mut f, &km, &mut seq, KeyContext::Omnibar, true, false, now);
        assert_eq!(f.consumed, vec![false, false]);
    }

    #[test]
    fn plain_consumes_shifted_presses_too() {
        let mut f = InputFrame::new(vec![ch("Shift+Left"), ch("Ctrl+Left"), ch("Left")]);
        assert!(f.consume_plain(KeyName::ArrowLeft));
        assert_eq!(f.consumed, vec![true, false, true]);
        assert!(!f.consume_shifted(KeyName::ArrowLeft));
    }

    #[test]
    fn typed_text_skips_consumed_and_command_frames() {
        let mut f =
            InputFrame::new(vec![InputEvent::Text("a".into()), InputEvent::Other, InputEvent::Text("b".into())]);
        f.consumed[0] = true;
        assert_eq!(f.typed_text(), "b");
        f.command_held = true;
        assert_eq!(f.typed_text(), "");
    }
}

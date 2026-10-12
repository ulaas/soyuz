//! A one-line text field for the address bar, the filter and the palette.
//!
//! [`LineEdit`] keeps only the cursor and the horizontal scroll; the text is a borrowed
//! `&mut String` owned by the core (`ui.address.text`, `pane.filter`, `palette.input`),
//! so there is one source of truth. It reads the frame's [`InputFrame`] events by index,
//! in order, and marks what it used as consumed.
//!
//! Keys: typing and bracketed paste insert (newlines and control characters are dropped),
//! Backspace / Delete remove a grapheme (with Ctrl: a word), Left / Right move by a
//! grapheme (with Ctrl: by a word), Home / End (with or without Ctrl). Enter, Esc, Tab,
//! Shift+Tab, Up, Down, PageUp and PageDown end the run with an [`EditOutcome`] for the
//! caller. Everything else with Ctrl / Alt / Cmd is left alone (not consumed), and so are
//! F-keys and Insert. There is no selection (yet); Ctrl+V is left to the caller because a
//! terminal paste arrives as text anyway.
//!
//! Key order (follow it in the steps that use the widget):
//! - **Address bar and filter** (T2.5, T2.8): the core's input runs first, with
//!   `KeyContext::Omnibar` and `text_focused = true` like the GUI (bindings that are not
//!   text editing, e.g. Ctrl+T, still work); the `LineEdit` then takes only the events the
//!   core left unconsumed.
//! - **Palette** (T2.7): the palette reads its own `soyuz_appcore::palette::KeyInput`
//!   first (Up / Down / Ctrl+J / Ctrl+K / Ctrl+N / Ctrl+P / Enter / Esc / Tab / toggles),
//!   like the GUI; the `LineEdit` sees the rest. That is why it must never bind Ctrl+K,
//!   Ctrl+J, Ctrl+N or Ctrl+P.

use soyuz_appcore::keys::input::{InputEvent, InputFrame};
use soyuz_core::config::{KeyChord, KeyName, Mods};
use unicode_segmentation::UnicodeSegmentation;

use crate::view::text;

/// What an event did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOutcome {
    /// Edited the text or moved the cursor (or a typing key whose text follows).
    Consumed,
    /// Enter.
    Submit,
    /// Esc.
    Cancel,
    Tab,
    ShiftTab,
    Up,
    Down,
    PageUp,
    PageDown,
    /// Not for the field: left unconsumed.
    Ignored,
}

/// Cursor and scroll of a one-line field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEdit {
    /// Byte offset in the text (always on a grapheme boundary).
    cursor: usize,
    /// First display column shown.
    scroll: usize,
}

/// Keys that type a character (their `Text` event follows the chord).
fn types(key: KeyName) -> bool {
    use KeyName as N;
    matches!(
        key,
        N::A | N::B
            | N::C
            | N::D
            | N::E
            | N::F
            | N::G
            | N::H
            | N::I
            | N::J
            | N::K
            | N::L
            | N::M
            | N::N
            | N::O
            | N::P
            | N::Q
            | N::R
            | N::S
            | N::T
            | N::U
            | N::V
            | N::W
            | N::X
            | N::Y
            | N::Z
            | N::Num0
            | N::Num1
            | N::Num2
            | N::Num3
            | N::Num4
            | N::Num5
            | N::Num6
            | N::Num7
            | N::Num8
            | N::Num9
            | N::Space
            | N::Backtick
            | N::Minus
            | N::Plus
            | N::Equals
            | N::Comma
            | N::Period
            | N::Slash
            | N::Backslash
            | N::Semicolon
            | N::Quote
            | N::OpenBracket
            | N::CloseBracket
    )
}

/// Word characters for Ctrl+Left / Right / Backspace / Delete.
fn is_word(g: &str) -> bool {
    g.chars().next().is_some_and(char::is_alphanumeric)
}

impl LineEdit {
    pub fn new() -> LineEdit {
        LineEdit::default()
    }

    /// A field with the cursor at the end of `text`.
    pub fn at_end(text: &str) -> LineEdit {
        LineEdit { cursor: text.len(), scroll: 0 }
    }

    /// The cursor as a byte offset in the text.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Put the cursor at byte `pos` (clamped to the text and moved back to a grapheme
    /// boundary).
    pub fn set_cursor(&mut self, text: &str, pos: usize) {
        let pos = pos.min(text.len());
        self.cursor = text.grapheme_indices(true).map(|(i, _)| i).take_while(|&i| i <= pos).last().unwrap_or(0);
        if pos == text.len() {
            self.cursor = pos;
        }
    }

    /// Keep the cursor valid after the text changed elsewhere.
    fn clamp(&mut self, text: &str) {
        if self.cursor > text.len() || !text.is_char_boundary(self.cursor) {
            self.set_cursor(text, self.cursor);
        }
    }

    fn prev(&self, text: &str) -> usize {
        text[..self.cursor].grapheme_indices(true).next_back().map_or(0, |(i, _)| i)
    }

    fn next(&self, text: &str) -> usize {
        text[self.cursor..].graphemes(true).next().map_or(self.cursor, |g| self.cursor + g.len())
    }

    /// Start of the word before the cursor (skipping non-word graphemes first).
    fn prev_word(&self, text: &str) -> usize {
        let mut gs = text[..self.cursor].grapheme_indices(true).rev().peekable();
        while gs.next_if(|(_, g)| !is_word(g)).is_some() {}
        let mut at = gs.peek().map_or(0, |(i, _)| *i);
        while let Some((i, _)) = gs.next_if(|(_, g)| is_word(g)) {
            at = i;
        }
        at
    }

    /// End of the word after the cursor (skipping non-word graphemes first).
    fn next_word(&self, text: &str) -> usize {
        let mut gs = text[self.cursor..].grapheme_indices(true).peekable();
        while gs.next_if(|(_, g)| !is_word(g)).is_some() {}
        while gs.next_if(|(_, g)| is_word(g)).is_some() {}
        gs.peek().map_or(text.len(), |(i, _)| self.cursor + i)
    }

    fn insert(&mut self, text: &mut String, s: &str) {
        let clean: String = s.chars().filter(|c| !c.is_control()).collect();
        text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }

    fn chord(&mut self, text: &mut String, c: KeyChord) -> EditOutcome {
        use EditOutcome as O;
        let ctrl = c.mods.intersects(Mods::CTRL | Mods::CMD);
        let alt = c.mods.contains(Mods::ALT);
        let shift = c.mods.contains(Mods::SHIFT);
        if alt {
            return O::Ignored;
        }
        match c.key {
            KeyName::Backspace => {
                let from = if ctrl { self.prev_word(text) } else { self.prev(text) };
                text.replace_range(from..self.cursor, "");
                self.cursor = from;
            }
            KeyName::Delete => {
                let to = if ctrl { self.next_word(text) } else { self.next(text) };
                text.replace_range(self.cursor..to, "");
            }
            KeyName::ArrowLeft => self.cursor = if ctrl { self.prev_word(text) } else { self.prev(text) },
            KeyName::ArrowRight => self.cursor = if ctrl { self.next_word(text) } else { self.next(text) },
            KeyName::Home => self.cursor = 0,
            KeyName::End => self.cursor = text.len(),
            _ if ctrl => return O::Ignored,
            KeyName::Enter => return O::Submit,
            KeyName::Escape => return O::Cancel,
            KeyName::Tab if shift => return O::ShiftTab,
            KeyName::Tab => return O::Tab,
            KeyName::ArrowUp => return O::Up,
            KeyName::ArrowDown => return O::Down,
            KeyName::PageUp => return O::PageUp,
            KeyName::PageDown => return O::PageDown,
            k if types(k) => {}
            _ => return O::Ignored,
        }
        O::Consumed
    }

    /// Apply one event to `text`.
    pub fn apply(&mut self, text: &mut String, ev: &InputEvent) -> EditOutcome {
        self.clamp(text);
        match ev {
            InputEvent::Chord(c) => self.chord(text, *c),
            InputEvent::Text(s) => {
                self.insert(text, s);
                EditOutcome::Consumed
            }
            InputEvent::Other => EditOutcome::Ignored,
        }
    }

    /// Apply the frame's live events in order, marking the ones used as consumed. Stops
    /// at the first event that ends the edit (Enter, Esc, Tab, …: consumed, returned);
    /// later events stay live. Otherwise returns `Consumed` when anything was used.
    pub fn feed(&mut self, text: &mut String, frame: &mut InputFrame) -> EditOutcome {
        let mut used = false;
        for i in 0..frame.events.len() {
            if frame.consumed[i] {
                continue;
            }
            match self.apply(text, &frame.events[i]) {
                EditOutcome::Ignored => {}
                EditOutcome::Consumed => {
                    frame.consumed[i] = true;
                    used = true;
                }
                other => {
                    frame.consumed[i] = true;
                    return other;
                }
            }
        }
        if used { EditOutcome::Consumed } else { EditOutcome::Ignored }
    }

    /// What a field `width` cells wide shows of `text`, and the cursor's column in it.
    /// Scrolls so the cursor stays visible (one cell is kept for it at the end).
    pub fn view(&mut self, text: &str, width: u16) -> (String, u16) {
        self.clamp(text);
        let width = usize::from(width);
        if width == 0 {
            return (String::new(), 0);
        }
        let col = text::width(&text[..self.cursor]);
        if col < self.scroll {
            self.scroll = col;
        } else if col >= self.scroll + width {
            self.scroll = col + 1 - width;
        }
        // Never scroll further than needed to show the end.
        self.scroll = self.scroll.min((text::width(text) + 1).saturating_sub(width)).min(col);
        let mut out = String::new();
        let mut x = 0;
        for g in text.graphemes(true) {
            let w = text::width(g);
            if x >= self.scroll && x + w <= self.scroll + width {
                out.push_str(g);
            } else if x + w > self.scroll + width {
                break;
            }
            x += w;
        }
        (out, (col - self.scroll) as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn txt(s: &str) -> InputEvent {
        InputEvent::Text(s.into())
    }

    fn run(text: &str, cursor: usize, events: Vec<InputEvent>) -> (String, LineEdit, EditOutcome, InputFrame) {
        let mut t = text.to_string();
        let mut e = LineEdit::new();
        e.set_cursor(&t, cursor);
        let mut f = InputFrame::new(events);
        let out = e.feed(&mut t, &mut f);
        (t, e, out, f)
    }

    #[test]
    fn typing_inserts_and_consumes_the_chord_too() {
        let (t, e, out, f) = run("", 0, vec![ch("A"), txt("a"), ch("Shift+B"), txt("B"), ch("Space"), txt(" ")]);
        assert_eq!((t.as_str(), e.cursor(), out), ("aB ", 3, EditOutcome::Consumed));
        assert!(f.consumed.iter().all(|c| *c));
        let (t, _, _, _) = run("ac", 1, vec![txt("b")]);
        assert_eq!(t, "abc");
    }

    #[test]
    fn paste_drops_newlines() {
        let (t, e, _, _) = run("x", 1, vec![txt("a\r\nb\nc\td")]);
        assert_eq!((t.as_str(), e.cursor()), ("xabcd", 5));
    }

    #[test]
    fn deleting_and_moving_by_grapheme() {
        let s = "ae\u{301}日"; // a, é (combining), 日
        let (t, e, _, _) = run(s, s.len(), vec![ch("Backspace")]);
        assert_eq!((t.as_str(), e.cursor()), ("ae\u{301}", 4));
        let (t, _, _, _) = run(s, 1, vec![ch("Delete")]);
        assert_eq!(t, "a日");
        let (_, e, _, _) = run(s, 0, vec![ch("Right"), ch("Right")]);
        assert_eq!(e.cursor(), 4);
        let (_, e, _, _) = run(s, s.len(), vec![ch("Left")]);
        assert_eq!(e.cursor(), 4);
        let (_, e, _, _) = run(s, 2, vec![ch("Home")]);
        assert_eq!(e.cursor(), 0);
        let (_, e, _, _) = run(s, 0, vec![ch("End")]);
        assert_eq!(e.cursor(), s.len());
        // A cursor inside a cluster snaps back to its start.
        let mut e = LineEdit::new();
        e.set_cursor(s, 3);
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn word_moves() {
        let s = "foo bar/baz qux";
        let (_, e, _, _) = run(s, s.len(), vec![ch("Ctrl+Left")]);
        assert_eq!(e.cursor(), 12);
        let (_, e, _, _) = run(s, 12, vec![ch("Ctrl+Left"), ch("Ctrl+Left")]);
        assert_eq!(e.cursor(), 4);
        let (_, e, _, _) = run(s, 0, vec![ch("Ctrl+Right")]);
        assert_eq!(e.cursor(), 3);
        let (_, e, _, _) = run(s, 3, vec![ch("Ctrl+Right")]);
        assert_eq!(e.cursor(), 7);
        let (_, e, _, _) = run(s, 0, vec![ch("Ctrl+Left")]);
        assert_eq!(e.cursor(), 0);
        let (t, e, _, _) = run(s, 11, vec![ch("Ctrl+Backspace")]);
        assert_eq!((t.as_str(), e.cursor()), ("foo bar/ qux", 8));
        let (t, _, _, _) = run(s, 3, vec![ch("Ctrl+Delete")]);
        assert_eq!(t, "foo/baz qux");
        let (_, e, _, _) = run("  foo", 5, vec![ch("Ctrl+Left")]);
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn outcomes_end_the_run() {
        for (key, want) in [
            ("Enter", EditOutcome::Submit),
            ("Esc", EditOutcome::Cancel),
            ("Tab", EditOutcome::Tab),
            ("Shift+Tab", EditOutcome::ShiftTab),
            ("Up", EditOutcome::Up),
            ("Down", EditOutcome::Down),
            ("PageUp", EditOutcome::PageUp),
            ("PageDown", EditOutcome::PageDown),
        ] {
            let (t, _, out, f) = run("", 0, vec![txt("a"), ch(key), txt("b")]);
            assert_eq!(out, want, "{key}");
            assert_eq!(t, "a", "{key}: stops before the next event");
            assert_eq!(f.consumed, vec![true, true, false], "{key}");
        }
    }

    #[test]
    fn command_chords_are_left_alone() {
        for key in ["Ctrl+K", "Ctrl+J", "Ctrl+N", "Ctrl+P", "Ctrl+T", "Alt+Left", "Ctrl+V", "F5", "Ins", "Ctrl+Enter"] {
            let (t, _, out, f) = run("x", 1, vec![ch(key)]);
            assert_eq!((t.as_str(), out), ("x", EditOutcome::Ignored), "{key}");
            assert_eq!(f.consumed, vec![false], "{key}");
        }
        // Events the core consumed are skipped.
        let mut t = String::new();
        let mut f = InputFrame::new(vec![txt("a"), txt("b")]);
        f.consumed[0] = true;
        assert_eq!(LineEdit::new().feed(&mut t, &mut f), EditOutcome::Consumed);
        assert_eq!(t, "b");
        let mut f = InputFrame::new(vec![InputEvent::Other]);
        assert_eq!(LineEdit::new().feed(&mut t, &mut f), EditOutcome::Ignored);
    }

    #[test]
    fn view_scrolls_to_the_cursor() {
        let s = "0123456789";
        let mut e = LineEdit::at_end(s);
        assert_eq!(e.view(s, 5), ("6789".to_string(), 4));
        e.set_cursor(s, 2);
        assert_eq!(e.view(s, 5), ("23456".to_string(), 0));
        e.set_cursor(s, 0);
        assert_eq!(e.view(s, 5), ("01234".to_string(), 0));
        assert_eq!(e.view(s, 20), (s.to_string(), 0));
        assert_eq!(e.view(s, 0), (String::new(), 0));
        // Wide characters are shown whole.
        let s = "日本語テキスト";
        let mut e = LineEdit::at_end(s);
        let (shown, col) = e.view(s, 5);
        assert_eq!((shown.as_str(), col), ("スト", 4));
        assert!(text::width(&shown) <= 5);
        // Text that shrank under the field.
        let mut e = LineEdit::at_end("long text");
        assert_eq!(e.view("ab", 5), ("ab".to_string(), 2));
    }
}

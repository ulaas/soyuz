//! A scrolling list with a cursor: dialog lists (open with, recovery, crash reports), the
//! jobs panel, the archive listing of the preview, menus that do not fit.
//!
//! [`SelectList`] keeps only the cursor and the first row shown; the items belong to the
//! caller (pass their count). Keys ([`SelectList::apply`]): Up / Down move by one (no
//! wrap), PageUp / PageDown by a page, Home / End to the ends. The wheel
//! ([`SelectList::wheel`]) scrolls without moving the cursor; [`SelectList::show_cursor`]
//! brings the cursor back into view (call it after a key moved it).
//!
//! [`draw`] renders rows of text with the cursor row in [`Role::Cursor`] (focused) or
//! [`Role::CursorInactive`], recording a click target per row.

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_core::config::{KeyName, Mods};

use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// Cursor and scroll of a list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectList {
    /// The selected row (`0` in an empty list).
    pub cursor: usize,
    /// The first row shown.
    pub top: usize,
}

/// A cursor movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMove {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

impl SelectList {
    pub fn new() -> SelectList {
        SelectList::default()
    }

    /// Keep the cursor and the scroll inside a list of `len` rows shown `height` at a time.
    pub fn clamp(&mut self, len: usize, height: usize) {
        self.cursor = self.cursor.min(len.saturating_sub(1));
        self.top = self.top.min(len.saturating_sub(height.max(1)));
    }

    /// Move the cursor (`page`: rows per page).
    pub fn go(&mut self, m: ListMove, len: usize, page: usize) {
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let page = page.max(1);
        self.cursor = match m {
            ListMove::Up => self.cursor.saturating_sub(1),
            ListMove::Down => self.cursor + 1,
            ListMove::PageUp => self.cursor.saturating_sub(page),
            ListMove::PageDown => self.cursor + page,
            ListMove::Home => 0,
            ListMove::End => len - 1,
        }
        .min(len - 1);
    }

    /// Put the cursor on row `i` (clamped).
    pub fn select(&mut self, i: usize, len: usize) {
        self.cursor = i.min(len.saturating_sub(1));
    }

    /// Scroll so the cursor is among the `height` rows shown.
    pub fn show_cursor(&mut self, height: usize) {
        let height = height.max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + height {
            self.top = self.cursor + 1 - height;
        }
    }

    /// Scroll by `rows` (negative: up) without moving the cursor.
    pub fn wheel(&mut self, rows: i32, len: usize, height: usize) {
        let max = len.saturating_sub(height.max(1));
        let top = i64::try_from(self.top).unwrap_or(i64::MAX).saturating_add(i64::from(rows));
        self.top = usize::try_from(top.max(0)).unwrap_or(0).min(max);
    }

    /// The rows shown in `height` rows.
    pub fn visible(&self, len: usize, height: usize) -> Range<usize> {
        let start = self.top.min(len);
        start..(start + height).min(len)
    }

    /// Apply a navigation key to a list of `len` rows shown `height` at a time. Returns
    /// whether the key was one (Up, Down, PageUp, PageDown, Home, End without
    /// modifiers); the cursor is then in view.
    pub fn apply(&mut self, ev: &InputEvent, len: usize, height: usize) -> bool {
        let InputEvent::Chord(c) = ev else { return false };
        if c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT) {
            return false;
        }
        let m = match c.key {
            KeyName::ArrowUp => ListMove::Up,
            KeyName::ArrowDown => ListMove::Down,
            KeyName::PageUp => ListMove::PageUp,
            KeyName::PageDown => ListMove::PageDown,
            KeyName::Home => ListMove::Home,
            KeyName::End => ListMove::End,
            _ => return false,
        };
        self.go(m, len, height.saturating_sub(1));
        self.show_cursor(height);
        true
    }
}

/// One row of a drawn list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListRow {
    pub text: String,
    /// Right-aligned text (a size, a key).
    pub right: String,
    pub role: Option<Role>,
}

impl ListRow {
    pub fn new(text: impl Into<String>) -> ListRow {
        ListRow { text: text.into(), ..ListRow::default() }
    }
}

/// Draw `rows` (all of them; only those in view are drawn) into `area`, starting at
/// `list.top`; `cursor`: highlight the cursor row (`focused`: the active style). Each
/// drawn row gets the click target `target(index)`.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    buf: &mut Buffer,
    area: Rect,
    rows: &[ListRow],
    list: &SelectList,
    cursor: bool,
    focused: bool,
    theme: &TuiTheme,
    hits: &mut Hits,
    target: impl Fn(usize) -> HitTarget,
) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    let w = usize::from(area.width);
    for (y, i) in (area.y..area.bottom()).zip(list.visible(rows.len(), usize::from(area.height))) {
        let row = &rows[i];
        let mut style = theme.style(Role::Text);
        if let Some(r) = row.role {
            style = style.patch(theme.style(r));
        }
        if cursor && i == list.cursor {
            style = style.patch(theme.style(if focused { Role::Cursor } else { Role::CursorInactive }));
        }
        let rect = Rect::new(area.x, y, area.width, 1);
        buf.set_style(rect, style);
        let right_w = text::width(&row.right);
        let left_w = if right_w > 0 && right_w + 2 < w { w - right_w - 1 } else { w };
        buf.set_stringn(area.x, y, text::truncate_end(&row.text, left_w), left_w, style);
        if left_w < w {
            let x = area.x + (w - right_w) as u16;
            buf.set_stringn(x, y, &row.right, right_w, style);
        }
        hits.push(rect, target(i));
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    #[test]
    fn keys_move_and_scroll() {
        let mut l = SelectList::new();
        assert!(l.apply(&chord("Down"), 10, 3));
        assert!(l.apply(&chord("Down"), 10, 3));
        assert!(l.apply(&chord("Down"), 10, 3));
        assert_eq!((l.cursor, l.top), (3, 1));
        assert!(l.apply(&chord("End"), 10, 3));
        assert_eq!((l.cursor, l.top), (9, 7));
        assert!(l.apply(&chord("PageUp"), 10, 3));
        assert_eq!((l.cursor, l.top), (7, 7));
        assert!(l.apply(&chord("Home"), 10, 3));
        assert_eq!((l.cursor, l.top), (0, 0));
        assert!(l.apply(&chord("Up"), 10, 3), "no wrap");
        assert_eq!(l.cursor, 0);
        assert!(!l.apply(&chord("Ctrl+Down"), 10, 3));
        assert!(!l.apply(&chord("Enter"), 10, 3));
        assert!(!l.apply(&InputEvent::Text("j".into()), 10, 3));
        // Empty lists stay at 0.
        let mut e = SelectList::new();
        assert!(e.apply(&chord("End"), 0, 3));
        assert_eq!((e.cursor, e.top), (0, 0));
    }

    #[test]
    fn wheel_and_clamp() {
        let mut l = SelectList { cursor: 2, top: 0 };
        l.wheel(5, 10, 4);
        assert_eq!((l.cursor, l.top), (2, 5));
        l.wheel(50, 10, 4);
        assert_eq!(l.top, 6);
        l.wheel(-100, 10, 4);
        assert_eq!(l.top, 0);
        let mut l = SelectList { cursor: 9, top: 8 };
        l.clamp(3, 2);
        assert_eq!((l.cursor, l.top), (2, 1));
        assert_eq!(l.visible(3, 2), 1..3);
        assert_eq!(SelectList::default().visible(0, 5), 0..0);
    }

    #[test]
    fn draws_rows_with_cursor_and_targets() {
        let rows: Vec<ListRow> = (0..5)
            .map(|i| ListRow {
                text: format!("item {i}"),
                right: if i == 1 { "4 KiB".into() } else { String::new() },
                role: None,
            })
            .collect();
        let list = SelectList { cursor: 2, top: 1 };
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
        let mut hits = Hits::default();
        draw(
            &mut buf,
            Rect::new(0, 0, 12, 3),
            &rows,
            &list,
            true,
            true,
            &TuiTheme::plain(),
            &mut hits,
            HitTarget::JobRow,
        );
        let line = |y: u16| (0..12).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert_eq!(line(0), "item 1 4 KiB");
        assert_eq!(line(1), "item 2      ");
        assert!(buf[(0, 1)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(0, 0)].modifier.contains(Modifier::REVERSED));
        assert_eq!(hits.at(3, 2), Some(HitTarget::JobRow(3)));
        // Tiny areas never panic.
        for w in 0..4 {
            for h in 0..3 {
                let mut b = Buffer::empty(Rect::new(0, 0, w, h));
                draw(
                    &mut b,
                    Rect::new(0, 0, w + 2, h + 2),
                    &rows,
                    &list,
                    true,
                    false,
                    &TuiTheme::plain(),
                    &mut hits,
                    HitTarget::JobRow,
                );
            }
        }
    }
}

//! The inline rename field on a file row (F2) — Phase T3.
//!
//! The rename itself is the core's `fileops.rename` (`InlineRename`); the TUI keeps the
//! field's cursor and selection in `crate::input::rename::State`. [`decorate`] puts an
//! [`EditVm`] on the row being renamed (`Row::edit`) after the panes' models are built,
//! and `draw_pane` calls [`draw_edit`] for such a row: the name in an input field after
//! the row's mark and icon, the selected part highlighted, the cursor as a reversed cell,
//! and the reason an invalid name cannot be used on the right in the danger colour.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;
use unicode_segmentation::UnicodeSegmentation;

use super::pane::{PaneLine, PaneVm};
use super::text;
use crate::input::InputState;
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::widgets::LineEdit;

/// A row's inline edit field as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditVm {
    pub text: String,
    /// Cursor (and scroll) of the field.
    pub edit: LineEdit,
    /// The selected bytes of `text`.
    pub sel: Option<(usize, usize)>,
    /// Why the name cannot be used, if it cannot.
    pub error: Option<String>,
}

/// Put the rename field on its row (`ViewModel::from_core`).
pub fn decorate(core: &AppCore, input: &InputState, panes: &mut [PaneVm]) {
    let Some(r) = &core.fileops.rename else { return };
    let Some(pos) = core.ws.find_pane(r.pane).and_then(|p| p.visible_pos(&r.key)) else { return };
    let (mut edit, mut sel) = crate::input::rename::effective(r, &input.rename);
    let shown = crate::dialog::safe(&r.text);
    if shown != r.text {
        // Control characters were replaced: offsets in the name no longer apply.
        edit = LineEdit::at_end(&shown);
        sel = None;
    }
    let error = r.error.as_deref().map(crate::dialog::safe);
    for p in panes.iter_mut().filter(|p| p.id == r.pane) {
        for line in &mut p.lines {
            if let PaneLine::Item(row) = line
                && row.index == pos
            {
                row.edit = Some(EditVm { text: shown.clone(), edit: edit.clone(), sel, error: error.clone() });
            }
        }
    }
}

/// Draw the field of a row into `row` (the whole row; `prefix` cells of mark and icon
/// come first), recording a `RenameField` target.
pub fn draw_edit(buf: &mut Buffer, row: Rect, prefix: u16, e: &EditVm, theme: &TuiTheme, hits: &mut Hits) {
    let row = row.intersection(buf.area);
    if row.is_empty() {
        return;
    }
    let x0 = row.x + prefix.min(row.width.saturating_sub(1));
    let avail = row.right().saturating_sub(x0);
    if avail == 0 {
        return;
    }
    let y = row.y;
    // The error takes the room the name does not need, at least half of the row (when
    // the field keeps 8 cells).
    let err = e.error.as_ref().map(|m| format!(" ! {m}"));
    let err_w = match &err {
        Some(m) if avail >= 16 => {
            let w = text::width(m) as u16;
            let need = (text::width(&e.text) as u16).saturating_add(2).max(8);
            w.min(avail.saturating_sub(need)).max(w.min(avail / 2))
        }
        _ => 0,
    };
    let fw = avail - err_w;
    let input = theme.style(Role::Input);
    let field = Rect::new(x0, y, fw, 1);
    buf.set_stringn(x0, y, " ".repeat(usize::from(fw)), usize::from(fw), input);
    buf.set_style(field, input);
    let mut ed = e.edit.clone();
    let (_, col) = ed.view(&e.text, fw);
    let cursor = ed.cursor();
    let scroll = text::width(&e.text[..cursor.min(e.text.len())]).saturating_sub(usize::from(col));
    let mark = input.patch(theme.style(Role::Cursor));
    let mut x = 0usize;
    for (i, g) in e.text.grapheme_indices(true) {
        let w = text::width(g);
        if x >= scroll && x + w <= scroll + usize::from(fw) {
            let selected = e.sel.is_some_and(|(a, b)| i >= a && i < b);
            let at_cursor = e.sel.is_none() && i == cursor;
            let style = if selected || at_cursor { mark } else { input };
            buf.set_stringn(x0 + (x - scroll) as u16, y, g, w, style);
        } else if x >= scroll + usize::from(fw) {
            break;
        }
        x += w;
    }
    if e.sel.is_none() && cursor >= e.text.len() && col < fw {
        buf.set_stringn(x0 + col, y, " ", 1, mark);
    }
    if let Some(m) = err
        && err_w > 0
    {
        let style = theme.style(Role::Danger);
        let shown = text::truncate_end(&m, usize::from(err_w));
        buf.set_stringn(x0 + fw, y, " ".repeat(usize::from(err_w)), usize::from(err_w), style);
        buf.set_stringn(x0 + fw, y, shown, usize::from(err_w), style);
    }
    hits.push(Rect::new(x0, y, avail, 1), HitTarget::RenameField);
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use super::*;

    fn draw(e: &EditVm, w: u16) -> (String, Buffer, Hits) {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, 1));
        let mut hits = Hits::default();
        draw_edit(&mut buf, Rect::new(0, 0, w, 1), 2, e, &TuiTheme::plain(), &mut hits);
        let line = (0..w).map(|x| buf[(x, 0)].symbol().to_string()).collect::<String>();
        (line, buf, hits)
    }

    #[test]
    fn draws_the_name_with_its_selection() {
        let mut edit = LineEdit::new();
        edit.set_cursor("report.txt", 6);
        let e = EditVm { text: "report.txt".into(), edit, sel: Some((0, 6)), error: None };
        let (line, buf, hits) = draw(&e, 30);
        assert!(line.starts_with("  report.txt"), "{line:?}");
        assert!(buf[(2, 0)].modifier.contains(Modifier::REVERSED), "selected");
        assert!(!buf[(9, 0)].modifier.contains(Modifier::REVERSED), "the extension is not");
        assert_eq!(hits.at(5, 0), Some(HitTarget::RenameField));
    }

    #[test]
    fn errors_show_on_the_right() {
        let e = EditVm {
            text: "a?b".into(),
            edit: LineEdit::at_end("a?b"),
            sel: None,
            error: Some("“?” is not allowed".into()),
        };
        let (line, buf, _) = draw(&e, 40);
        assert!(line.contains("a?b") && line.contains("! “?” is not allowed"), "{line:?}");
        assert!(buf[(5, 0)].modifier.contains(Modifier::REVERSED), "the cursor after the text");
    }

    #[test]
    fn long_names_scroll_and_tiny_rows_never_panic() {
        let text = "a_very_long_file_name_that_does_not_fit.txt".to_string();
        let e = EditVm { edit: LineEdit::at_end(&text), text, sel: None, error: Some("bad".into()) };
        let (line, _, _) = draw(&e, 20);
        assert!(line.contains(".txt"), "the cursor end is in view: {line:?}");
        for w in 0..=20 {
            for h in 0..=6u16 {
                let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
                let mut hits = Hits::default();
                draw_edit(&mut buf, Rect::new(0, 0, w + 3, h.max(1)), 4, &e, &TuiTheme::plain(), &mut hits);
            }
        }
    }
}

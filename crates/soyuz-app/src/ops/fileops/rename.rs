//! Inline rename in the views: F2 starts it (stem selected), F2 again cycles the
//! selection stem → extension → whole name, Enter / click-away commits, Tab commits and
//! moves to the next item (Shift+Tab: previous), Esc cancels. Validation errors are shown
//! under the editor and block the commit.

use std::path::PathBuf;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Key, Margin, Modifiers, Order, Rect, Stroke, TextEdit, Ui, vec2,
};
use soyuz_core::EntryInfo;
use soyuz_core::config::RenameSelects;

use super::FileOpCmd;
use crate::cmd::Cmd;
use crate::state::pane::{Pane, PaneId};
use crate::theme::palette;
use crate::theme::tokens::radius;
use crate::ui::views::ViewCtx;

/// Which part of the name is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameSel {
    Stem,
    Ext,
    All,
}

/// Character range (start, end) of `sel` in `name`. Folders and names without an
/// extension (incl. dot-files) have no separate stem / extension.
pub fn sel_range(name: &str, is_dir: bool, sel: RenameSel) -> (usize, usize) {
    let total = name.chars().count();
    let dot = if is_dir {
        None
    } else {
        name.char_indices()
            .filter(|(_, c)| *c == '.')
            .map(|(i, _)| i)
            .next_back()
            .filter(|&i| i > 0 && i + 1 < name.len())
    };
    let Some(dot_byte) = dot else { return (0, total) };
    let stem_chars = name[..dot_byte].chars().count();
    match sel {
        RenameSel::Stem => (0, stem_chars),
        RenameSel::Ext => (stem_chars + 1, total),
        RenameSel::All => (0, total),
    }
}

/// The next selection for F2, skipping states that select the same range.
pub fn next_sel(name: &str, is_dir: bool, cur: RenameSel) -> RenameSel {
    let order = [RenameSel::Stem, RenameSel::Ext, RenameSel::All];
    let start = order.iter().position(|s| *s == cur).unwrap_or(0);
    let here = sel_range(name, is_dir, cur);
    for k in 1..=3 {
        let cand = order[(start + k) % 3];
        if sel_range(name, is_dir, cand) != here {
            return cand;
        }
    }
    cur
}

/// Validate a new name for an item called `original` among `siblings`.
pub fn validate<'a>(
    new: &str,
    original: &str,
    siblings: impl IntoIterator<Item = &'a str>,
    windows_rules: bool,
    case_insensitive: bool,
) -> Result<(), String> {
    use soyuz_core::NameError as E;
    soyuz_core::validate_name_with(new, windows_rules).map_err(|e| match e {
        E::Empty => tr!("ops-rename-required").to_string(),
        E::Reserved(n) => tr!("ops-rename-reserved", name = n),
        E::InvalidChar(c) => tr!("ops-rename-invalid-char", char = c.to_string()),
        E::TrailingDotOrSpace => tr!("ops-rename-trailing-dot").to_string(),
        E::TooLong(n) => tr!("ops-rename-too-long", count = n),
    })?;
    let same = |a: &str, b: &str| if case_insensitive { a.to_lowercase() == b.to_lowercase() } else { a == b };
    if same(new, original) {
        return Ok(()); // unchanged or a case-only change of the item itself
    }
    if siblings.into_iter().any(|s| s != original && same(s, new)) {
        return Err(tr!("ops-rename-exists", name = new));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct InlineRename {
    pub pane: PaneId,
    /// Entry key (current name) of the item being renamed.
    pub key: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub text: String,
    pub sel: RenameSel,
    /// Apply `sel` to the editor on the next frame.
    pub apply_sel: bool,
    pub error: Option<String>,
}

impl InlineRename {
    pub fn new(pane: PaneId, e: &EntryInfo, selects: RenameSelects) -> Self {
        InlineRename {
            pane,
            key: e.name.clone(),
            path: e.path.clone(),
            is_dir: e.is_dir_like(),
            text: e.name.clone(),
            sel: match selects {
                RenameSelects::Stem => RenameSel::Stem,
                RenameSelects::All => RenameSel::All,
            },
            apply_sel: true,
            error: None,
        }
    }

    fn id(&self) -> Id {
        Id::new(("soyuz.rename", self.pane.0, self.key.as_str()))
    }
}

enum Outcome {
    Editing,
    Cancel,
    Commit,
    CommitAndMove(i32),
}

/// Draw the editor over `rect` (the item's name area). Called by the views for the
/// item being renamed.
pub fn editor(ui: &mut Ui, rect: Rect, font: FontId, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    let Some(r) = v.fileops.rename.as_mut() else { return };
    if r.pane != pane.id {
        return;
    }
    let p = palette(ui.ctx());
    let id = r.id();
    let first = r.apply_sel;
    let has_focus = ui.memory(|m| m.has_focus(id));
    let (esc, enter, tab, back_tab, f2) = if has_focus || first {
        ui.input_mut(|i| {
            (
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::SHIFT, Key::Tab),
                i.consume_key(Modifiers::NONE, Key::F2),
            )
        })
    } else {
        (false, false, false, false, false)
    };
    if f2 {
        r.sel = next_sel(&r.text, r.is_dir, r.sel);
        r.apply_sel = true;
    }
    if r.apply_sel {
        let (a, b) = sel_range(&r.text, r.is_dir, r.sel);
        let mut st = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        st.store(ui.ctx(), id);
        // Keep applying until the editor owns the focus (gaining focus resets the cursor).
        if has_focus {
            r.apply_sel = false;
        }
    }
    // Opaque backing so the painted name underneath does not show through.
    ui.painter().rect_filled(rect.expand(1.0), CornerRadius::same(radius::CONTROL), p.bg_surface1);
    let te = TextEdit::singleline(&mut r.text)
        .id(id)
        .font(font)
        .margin(Margin::symmetric(4, 2))
        .lock_focus(true)
        .desired_width(rect.width() - 8.0);
    let out = ui.put(rect, te);
    if first && !has_focus {
        out.request_focus();
    }
    // Validation (siblings from the listing; no I/O).
    let windows_rules = cfg!(windows) || v.config.general.strict_windows_names;
    let ci = super::names::case_insensitive_fs();
    let text = r.text.trim().to_string();
    r.error = validate(&text, &r.key, pane.entries.iter().map(|e| e.name.as_str()), windows_rules, ci).err();
    let stroke = if r.error.is_some() { p.danger } else { p.accent };
    ui.painter().rect_stroke(
        rect.expand(1.0),
        CornerRadius::same(radius::CONTROL),
        Stroke::new(1.5, stroke),
        egui::StrokeKind::Outside,
    );
    if let Some(err) = &r.error {
        error_bubble(ui, rect, err, &p);
    }

    let outcome = if esc {
        Outcome::Cancel
    } else if tab || back_tab {
        Outcome::CommitAndMove(if back_tab { -1 } else { 1 })
    } else if (enter && has_focus) || out.lost_focus() {
        Outcome::Commit
    } else {
        Outcome::Editing
    };
    match outcome {
        Outcome::Editing => {}
        Outcome::Cancel => v.fileops.rename = None,
        Outcome::Commit | Outcome::CommitAndMove(_) => {
            if r.error.is_some() {
                if out.lost_focus() && !enter && !tab && !back_tab {
                    // Clicked away with an invalid name: drop the edit.
                    v.fileops.rename = None;
                } else {
                    // Keep editing until the name is valid.
                    out.request_focus();
                }
                return;
            }
            let (key, path) = (r.key.clone(), r.path.clone());
            let changed = text != key;
            if changed {
                v.cmds.push(Cmd::FileOp(FileOpCmd::Rename { pane: pane.id, from: path, to: text.clone() }));
            }
            v.fileops.rename = None;
            if let Outcome::CommitAndMove(d) = outcome {
                let pos = pane.visible_pos(&key).map(|p| p as i64 + d as i64).filter(|p| *p >= 0).map(|p| p as usize);
                if let Some(pos) = pos
                    && let (Some(next), Some(k)) = (pane.entry_at(pos).cloned(), pane.key_at(pos).cloned())
                {
                    pane.selection.select_only(k.clone());
                    pane.cursor = Some(k);
                    pane.scroll_to_cursor = true;
                    v.fileops.rename = Some(InlineRename::new(pane.id, &next, v.config.behaviour.rename_selects));
                }
            } else if changed {
                pane.pending_cursor = Some(text);
            }
        }
    }
}

fn error_bubble(ui: &Ui, rect: Rect, msg: &str, p: &crate::theme::Palette) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(Order::Foreground, Id::new("soyuz.rename.err")));
    let galley = painter.layout(msg.to_string(), FontId::proportional(11.5), Color32::WHITE, 280.0);
    let size = galley.size() + vec2(16.0, 10.0);
    let r = Rect::from_min_size(rect.left_bottom() + vec2(0.0, 5.0), size);
    painter.rect_filled(r, CornerRadius::same(radius::ROW), p.danger);
    painter.galley(r.min + vec2(8.0, 5.0), galley, Color32::WHITE);
    painter.text(
        r.left_top() + vec2(12.0, 0.0),
        Align2::CENTER_BOTTOM,
        "\u{25b2}",
        FontId::proportional(7.0),
        p.danger,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_ranges() {
        assert_eq!(sel_range("report.final.pdf", false, RenameSel::Stem), (0, 12));
        assert_eq!(sel_range("report.final.pdf", false, RenameSel::Ext), (13, 16));
        assert_eq!(sel_range("report.final.pdf", false, RenameSel::All), (0, 16));
        assert_eq!(sel_range(".bashrc", false, RenameSel::Stem), (0, 7), "dot-files have no extension");
        assert_eq!(sel_range("my.dir", true, RenameSel::Stem), (0, 6), "folders select the whole name");
        assert_eq!(sel_range("trailing.", false, RenameSel::Stem), (0, 9));
        // Character (not byte) offsets.
        assert_eq!(sel_range("çağrı.txt", false, RenameSel::Stem), (0, 5));
        assert_eq!(sel_range("çağrı.txt", false, RenameSel::Ext), (6, 9));
    }

    #[test]
    fn f2_cycles() {
        assert_eq!(next_sel("a.txt", false, RenameSel::Stem), RenameSel::Ext);
        assert_eq!(next_sel("a.txt", false, RenameSel::Ext), RenameSel::All);
        assert_eq!(next_sel("a.txt", false, RenameSel::All), RenameSel::Stem);
        // No extension: stem == all, so the cycle stays on the whole name.
        assert_eq!(next_sel("folder", true, RenameSel::Stem), RenameSel::Stem);
        assert_eq!(next_sel("Makefile", false, RenameSel::All), RenameSel::All);
    }

    #[test]
    fn validation() {
        let sib = ["a.txt", "B.txt", "c"];
        assert!(validate("new.txt", "a.txt", sib, true, true).is_ok());
        assert!(validate("A.TXT", "a.txt", sib, true, true).is_ok(), "case-only rename of itself");
        assert!(validate("b.TXT", "a.txt", sib, true, true).unwrap_err().contains("already exists"));
        assert!(validate("b.txt", "a.txt", sib, false, false).is_ok(), "case-sensitive fs");
        assert!(validate("", "a.txt", sib, true, true).unwrap_err().contains("required"));
        assert!(validate("x?y", "a.txt", sib, true, true).unwrap_err().contains('?'));
        assert!(validate("x?y", "a.txt", sib, false, false).is_ok(), "unix allows ?");
        assert!(validate("CON", "a.txt", sib, true, true).is_err());
        assert!(validate("end.", "a.txt", sib, true, true).is_err());
    }
}

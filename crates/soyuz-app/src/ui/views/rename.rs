//! The inline rename editor drawn over an item's name in the views (F2). State,
//! validation and the commit are [`crate::ops::fileops::rename`].

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Key, Margin, Modifiers, Order, Rect, Stroke, TextEdit, Ui, vec2,
};

use crate::ops::fileops::rename::{InlineRename, Outcome, finish, next_sel, revalidate, sel_range};
use crate::state::pane::Pane;
use crate::theme::palette;
use crate::theme::tokens::radius;
use crate::ui::views::ViewCtx;

/// The text field's id for `r`.
fn editor_id(r: &InlineRename) -> Id {
    Id::unique(("soyuz.rename", r.pane.0, r.key.as_str()))
}

/// Draw the editor over `rect` (the item's name area). Called by the views for the
/// item being renamed.
pub fn editor(ui: &mut Ui, rect: Rect, font: FontId, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    let Some(r) = v.fileops.rename.as_mut() else { return };
    if r.pane != pane.id {
        return;
    }
    let p = palette(ui.ctx());
    let id = editor_id(r);
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
    let text = revalidate(r, pane, v.config.general.strict_windows_names, v.volumes);
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
    let clicked_away = out.lost_focus() && !enter && !tab && !back_tab;
    let selects = v.config.behaviour.rename_selects;
    if finish(&mut v.fileops.rename, pane, outcome, text, clicked_away, selects, v.cmds) {
        out.request_focus();
    }
}

fn error_bubble(ui: &Ui, rect: Rect, msg: &str, p: &crate::theme::Palette) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(Order::Foreground, Id::unique("soyuz.rename.err")));
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

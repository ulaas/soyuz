//! Dialog views: one [`DialogVm`] for the top-most dialog of either stack (see
//! `crate::input::dialogs`), drawn by [`draw_dialog`] as a box in the middle of the
//! screen: title, body text, an optional list, form fields, the validation message, the
//! buttons and the key hints. Each kind's file builds its model (`vm`); [`from_core`]
//! picks the kind.
//!
//! Sizes: the box is as wide as [`DialogVm::width`] (default [`DEFAULT_WIDTH`]) but at
//! most the screen minus a margin, and as tall as its content. When the screen is too
//! short the body is cut first, then the list; the buttons and hints stay.

pub mod approval_mcp;
pub mod approval_script;
pub mod attributes;
pub mod batch_rename;
pub mod columns;
pub mod confirm;
pub mod crash_reports;
pub mod delete;
pub mod link;
pub mod open_with;
pub mod pattern;
pub mod prompt;
pub mod properties;
pub mod quit;
pub mod recovery;
pub mod transfer;
pub mod unsaved;

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::widgets::Clear;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::dialogs::DialogModel;

use super::text;
use crate::borders;
use crate::input::InputState;
use crate::input::dialogs::{ModelUi, Top, TuiDialog};
use crate::layout::Hits;
use crate::theme::{Role, TuiTheme};
use crate::widgets::form::{self, ButtonVm, FieldVm};
use crate::widgets::{Form, ListRow, SelectList};

/// Widest box by default (cells, borders included).
pub const DEFAULT_WIDTH: u16 = 72;
/// List rows shown at most.
pub const MAX_LIST_ROWS: u16 = 10;

/// A dialog as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DialogVm {
    pub title: String,
    /// Paragraphs (wrapped to the box).
    pub body: Vec<String>,
    /// A list (open with, recovery, conflict details, …) with its cursor.
    pub list: Vec<ListRow>,
    pub list_state: SelectList,
    pub fields: Vec<FieldVm>,
    pub buttons: Vec<ButtonVm>,
    /// The form's focus stop (fields, then buttons; `widgets::Form::focus`).
    pub focus: usize,
    /// A validation message.
    pub error: Option<String>,
    /// Key hints at the bottom.
    pub hints: String,
    /// Preferred width (0: [`DEFAULT_WIDTH`]).
    pub width: u16,
    /// A destructive or blocking confirmation (drawn with a warning border).
    pub alert: bool,
}

impl DialogVm {
    /// A dialog of `title` showing `form` (fields, buttons, error).
    pub fn from_form(title: &str, form: &Form) -> DialogVm {
        DialogVm {
            title: crate::dialog::safe(title),
            fields: form.field_vms(),
            buttons: form.button_vms(),
            focus: form.focus,
            error: form.error.clone(),
            ..DialogVm::default()
        }
    }
}

/// The model of the top-most dialog, if any.
pub fn from_core(core: &AppCore, input: &InputState) -> Option<DialogVm> {
    let st = &input.dialogs;
    match crate::input::dialogs::top(core, st)? {
        Top::Model => {
            let m = core.dialogs.top()?;
            let blank = ModelUi::default();
            let ui = st.model.as_ref().filter(|u| Some(u.seq) == core.dialogs.top_seq()).unwrap_or(&blank);
            Some(match m {
                DialogModel::Prompt(_) => prompt::vm(m, ui),
                DialogModel::DeleteConfirm(_) => delete::vm(m, ui),
                DialogModel::Quit(_) => quit::vm(m, ui),
                DialogModel::Transfer(_) => transfer::vm(m, ui),
                DialogModel::NewLink(_) => link::model_vm(m, ui),
                DialogModel::McpApproval(_) => approval_mcp::vm(m, ui),
                DialogModel::ScriptApproval(_) => approval_script::vm(m, ui),
            })
        }
        Top::Tui => Some(match st.tui.top()? {
            TuiDialog::Confirm(s) => confirm::vm(core, s),
            TuiDialog::Pattern(s) => pattern::vm(core, s),
            TuiDialog::Columns(s) => columns::vm(core, s),
            TuiDialog::OpenWith(s) => open_with::vm(core, s),
            TuiDialog::Link(s) => link::vm(core, s),
            TuiDialog::BatchRename(s) => batch_rename::vm(core, s),
            TuiDialog::Attributes(s) => attributes::vm(core, s),
            TuiDialog::Properties(s) => properties::vm(core, s),
            TuiDialog::Recovery(s) => recovery::vm(core, s),
            TuiDialog::CrashReports(s) => crash_reports::vm(core, s),
            TuiDialog::Unsaved(s) => unsaved::vm(core, s),
        }),
    }
}

/// Draw `d` centred in `area`, recording `DialogField`, `DialogButton` and `DialogRow`
/// targets; the terminal cursor goes into the focused text field.
pub fn draw_dialog(f: &mut Frame<'_>, area: Rect, d: &DialogVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() {
        return;
    }
    let want = if d.width == 0 { DEFAULT_WIDTH } else { d.width };
    let width = area.width.saturating_sub(4).clamp(1, want).min(area.width);
    let inner_w = usize::from(width.saturating_sub(2)).max(1);
    let mut body: Vec<String> = d.body.iter().flat_map(|l| text::wrap(l, inner_w)).collect();
    let hints: Vec<String> = if d.hints.is_empty() { Vec::new() } else { text::wrap(&d.hints, inner_w) };
    let mut list_h = (d.list.len() as u16).min(MAX_LIST_ROWS);
    let fields_h = d.fields.len() as u16 + u16::from(d.error.is_some());
    let buttons_h = u16::from(!d.buttons.is_empty());
    let hints_h = hints.len() as u16;
    // Groups top to bottom: body, list, fields (+ error), buttons, hints; a blank line
    // between two shown groups unless the screen is short (`tight`).
    let total = |body: u16, list: u16, tight: bool| {
        let parts = [body, list, fields_h, buttons_h, hints_h];
        let gaps = parts.iter().filter(|&&p| p > 0).count().saturating_sub(1) as u16;
        parts.iter().sum::<u16>() + if tight { 0 } else { gaps }
    };
    let max_inner = area.height.saturating_sub(2);
    let mut tight = false;
    loop {
        if total(body.len() as u16, list_h, tight) <= max_inner {
            break;
        }
        if !tight {
            tight = true;
        } else if list_h > 0 {
            list_h -= 1;
        } else if !body.is_empty() {
            body.pop();
        } else {
            break;
        }
    }
    let height = (total(body.len() as u16, list_h, tight) + 2).min(area.height);
    let r = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height };
    f.render_widget(Clear, r);
    let title = text::truncate_end(&d.title, inner_w.saturating_sub(2));
    let border = theme.style(if d.alert { Role::Warning } else { Role::Border });
    let block = borders::overlay_block(theme.borders(), border).title(format!(" {title} "));
    let inner = block.inner(r);
    f.render_widget(block, r);
    // The blank line after a group, when a later group is shown.
    let sep = |later: u16| u16::from(!tight && later > 0);
    let buf = f.buffer_mut();
    let mut y = inner.y;
    let row = |y: u16, h: u16| Rect::new(inner.x, y, inner.width, h.min(inner.bottom().saturating_sub(y)));
    for l in &body {
        if y >= inner.bottom() {
            break;
        }
        buf.set_stringn(inner.x, y, l, inner_w, theme.style(Role::Text));
        y += 1;
    }
    if !body.is_empty() {
        y += sep(list_h + fields_h + buttons_h + hints_h);
    }
    if list_h > 0 && y < inner.bottom() {
        let mut ls = d.list_state.clone();
        ls.show_cursor(usize::from(list_h));
        let rect = row(y, list_h);
        crate::widgets::select_list::draw(
            buf,
            rect,
            &d.list,
            &ls,
            true,
            true,
            theme,
            hits,
            crate::layout::HitTarget::DialogRow,
        );
        y += list_h + sep(fields_h + buttons_h + hints_h);
    }
    let mut cursor = None;
    if !d.fields.is_empty() && y < inner.bottom() {
        let n = d.fields.len() as u16;
        cursor = form::draw_fields(buf, row(y, n), &d.fields, theme, hits);
        y += n;
    }
    if let Some(e) = &d.error
        && y < inner.bottom()
    {
        buf.set_stringn(inner.x, y, text::truncate_end(e, inner_w), inner_w, theme.style(Role::Danger));
        y += 1;
    }
    if fields_h > 0 {
        y += sep(buttons_h + hints_h);
    }
    if buttons_h > 0 && y < inner.bottom() {
        form::draw_buttons(buf, row(y, 1), &d.buttons, theme, hits);
        y += 1 + sep(hints_h);
    }
    for l in &hints {
        if y >= inner.bottom() {
            break;
        }
        buf.set_stringn(inner.x, y, l, inner_w, theme.style(Role::KeyHint));
        y += 1;
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(Position { x, y });
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::layout::HitTarget;
    use crate::widgets::{Button, Field};

    fn screen(d: &DialogVm, w: u16, h: u16) -> (Vec<String>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_dialog(f, f.area(), d, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer();
        let lines =
            (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string());
        (lines.collect(), hits)
    }

    fn full() -> DialogVm {
        let mut form = Form::new(
            vec![Field::text("Name", "a.txt"), Field::check("Apply to all", false)],
            vec![Button::new("Replace").accel('r'), Button::new("Skip").accel('s'), Button::new("Cancel")],
        );
        form.error = Some("A file with this name exists".into());
        DialogVm {
            body: vec!["“a.txt” already exists in the destination.".into()],
            list: vec![ListRow::new("source: 1 KiB"), ListRow::new("target: 2 KiB")],
            hints: "Tab next · Esc cancel".into(),
            ..DialogVm::from_form("Replace “a.txt”?", &form)
        }
    }

    #[test]
    fn draws_every_part_with_targets() {
        let (lines, hits) = screen(&full(), 60, 20);
        let all = lines.join("\n");
        for s in ["Replace “a.txt”?", "already exists", "source: 1 KiB", "Name a.txt", "[ ] Apply to all"] {
            assert!(all.contains(s), "{s}: {all}");
        }
        for s in ["A file with this name exists", "[ Replace ] [ Skip ] [ Cancel ]", "Tab next · Esc cancel"] {
            assert!(all.contains(s), "{s}: {all}");
        }
        let kinds: Vec<HitTarget> = hits.iter().map(|(_, t)| *t).collect();
        for t in [HitTarget::DialogRow(1), HitTarget::DialogField(1), HitTarget::DialogButton(2)] {
            assert!(kinds.contains(&t), "{t:?} in {kinds:?}");
        }
    }

    #[test]
    fn short_screens_keep_buttons_and_hints() {
        let (lines, _) = screen(&full(), 60, 9);
        let all = lines.join("\n");
        assert!(all.contains("[ Replace ]") && all.contains("Esc cancel"), "{all}");
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let d = full();
        for w in 0..=20 {
            for h in 0..=6 {
                let _ = screen(&d, w, h);
            }
        }
    }
}

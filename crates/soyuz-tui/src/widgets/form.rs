//! Dialog forms: a focus ring over one-line text fields ([`LineEdit`]), checkboxes, radio
//! groups and a row of buttons, plus a validation message.
//!
//! [`Form`] is the state a dialog keeps between frames (`input/dialogs/*`); [`Form::vm`]
//! turns it into plain data ([`FieldVm`], [`ButtonVm`]) for the dialog view, which draws
//! it with [`draw_fields`] and [`draw_buttons`].
//!
//! Focus stops are the enabled fields, then the enabled buttons. Keys ([`Form::apply`]):
//! - Tab / Shift+Tab: next / previous stop (wrapping); Up / Down too, except where a
//!   field uses them;
//! - text field: typing and editing go to the [`LineEdit`]; Enter presses the default
//!   button;
//! - checkbox: Space toggles; radio group: Left / Right (Space: next) pick the option;
//!   Enter presses the default button;
//! - buttons: Left / Right move between them, Enter / Space press the focused one;
//! - a button's letter accelerator presses it: the plain letter when the focus is not in
//!   a text field, Alt+letter anywhere;
//! - Esc: [`FormOutcome::Cancel`].
//!
//! The dialog validates in its own code (on [`FormOutcome::Changed`] or before acting on
//! [`FormOutcome::Pressed`]) and puts the message in [`Form::error`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_core::config::{KeyName, Mods};

use super::line_edit::{EditOutcome, LineEdit};
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// What a field holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// A one-line text field (`secret`: drawn as bullets, for passwords).
    Text {
        text: String,
        edit: LineEdit,
        secret: bool,
    },
    Check(bool),
    Radio {
        options: Vec<String>,
        selected: usize,
    },
}

/// One field of a form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub kind: FieldKind,
    pub enabled: bool,
}

impl Field {
    /// A text field holding `text`, the cursor at its end.
    pub fn text(label: impl Into<String>, text: impl Into<String>) -> Field {
        let text = text.into();
        Field {
            label: label.into(),
            kind: FieldKind::Text { edit: LineEdit::at_end(&text), text, secret: false },
            enabled: true,
        }
    }

    /// A password field.
    pub fn secret(label: impl Into<String>) -> Field {
        Field {
            label: label.into(),
            kind: FieldKind::Text { text: String::new(), edit: LineEdit::new(), secret: true },
            enabled: true,
        }
    }

    pub fn check(label: impl Into<String>, on: bool) -> Field {
        Field { label: label.into(), kind: FieldKind::Check(on), enabled: true }
    }

    pub fn radio(label: impl Into<String>, options: Vec<String>, selected: usize) -> Field {
        Field { label: label.into(), kind: FieldKind::Radio { options, selected }, enabled: true }
    }

    /// The text of a text field.
    pub fn value(&self) -> Option<&str> {
        match &self.kind {
            FieldKind::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    /// Show a password field's text in clear (`false`: bullets again). Phase T7 (unlock).
    pub fn set_secret(&mut self, on: bool) {
        if let FieldKind::Text { secret, .. } = &mut self.kind {
            *secret = on;
        }
    }

    /// Move a text field's text out (no copy), leaving it empty. Phase T7 (unlock: the
    /// passphrase goes into a `Zeroizing` buffer).
    pub fn take_text(&mut self) -> String {
        match &mut self.kind {
            FieldKind::Text { text, edit, .. } => {
                *edit = LineEdit::new();
                std::mem::take(text)
            }
            _ => String::new(),
        }
    }

    /// Whether a checkbox is on.
    pub fn on(&self) -> Option<bool> {
        match self.kind {
            FieldKind::Check(on) => Some(on),
            _ => None,
        }
    }

    /// The option picked in a radio group.
    pub fn selected(&self) -> Option<usize> {
        match self.kind {
            FieldKind::Radio { selected, .. } => Some(selected),
            _ => None,
        }
    }
}

/// A button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub label: String,
    /// Letter accelerator (lower case).
    pub accel: Option<char>,
    pub enabled: bool,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Button {
        Button { label: label.into(), accel: None, enabled: true }
    }

    pub fn accel(mut self, c: char) -> Button {
        self.accel = Some(c.to_ascii_lowercase());
        self
    }
}

/// What a key did to the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormOutcome {
    /// Used (moved the focus, the text cursor, …), nothing for the dialog to do.
    Consumed,
    /// Not a form key.
    Ignored,
    /// Field `i` changed (text edited, box toggled, option picked): validate.
    Changed(usize),
    /// Button `i` was pressed.
    Pressed(usize),
    /// Esc.
    Cancel,
}

/// A form's state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    pub fields: Vec<Field>,
    pub buttons: Vec<Button>,
    /// The focus stop: `0..fields.len()` a field, then the buttons.
    pub focus: usize,
    /// The button Enter presses in a field.
    pub default_button: Option<usize>,
    /// A validation message shown under the fields.
    pub error: Option<String>,
}

impl Form {
    /// A form with the focus on the first enabled stop and the first button as default.
    pub fn new(fields: Vec<Field>, buttons: Vec<Button>) -> Form {
        let mut f = Form { default_button: (!buttons.is_empty()).then_some(0), fields, buttons, ..Form::default() };
        let first = f.stops().next().unwrap_or(0);
        f.focus = first;
        f
    }

    /// With `i` as the default button.
    pub fn default_button(mut self, i: usize) -> Form {
        self.default_button = Some(i);
        self
    }

    /// Focus on button `i` (when it is enabled).
    pub fn focus_button(&mut self, i: usize) {
        if self.buttons.get(i).is_some_and(|b| b.enabled) {
            self.focus = self.fields.len() + i;
        }
    }

    fn stop_enabled(&self, s: usize) -> bool {
        match s.checked_sub(self.fields.len()) {
            None => self.fields[s].enabled,
            Some(b) => self.buttons.get(b).is_some_and(|b| b.enabled),
        }
    }

    fn stops(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.fields.len() + self.buttons.len()).filter(|&s| self.stop_enabled(s))
    }

    /// The focused field, if the focus is on one.
    pub fn focused_field(&self) -> Option<usize> {
        (self.focus < self.fields.len()).then_some(self.focus)
    }

    /// The focused button, if the focus is on one.
    pub fn focused_button(&self) -> Option<usize> {
        self.focus.checked_sub(self.fields.len()).filter(|&b| b < self.buttons.len())
    }

    /// Move the focus to the next (or previous) enabled stop, wrapping.
    pub fn move_focus(&mut self, forward: bool) {
        let stops: Vec<usize> = self.stops().collect();
        if stops.is_empty() {
            return;
        }
        let pos = stops.iter().position(|&s| s == self.focus);
        let n = stops.len();
        self.focus = match (pos, forward) {
            (Some(p), true) => stops[(p + 1) % n],
            (Some(p), false) => stops[(p + n - 1) % n],
            (None, _) => stops[0],
        };
    }

    fn move_in_buttons(&mut self, forward: bool) {
        let Some(b) = self.focused_button() else { return };
        let n = self.buttons.len();
        for k in 1..n {
            let i = if forward { (b + k) % n } else { (b + n - k) % n };
            if self.buttons[i].enabled {
                self.focus = self.fields.len() + i;
                return;
            }
        }
    }

    fn press_default(&self) -> FormOutcome {
        match self.default_button {
            Some(d) if self.buttons.get(d).is_some_and(|b| b.enabled) => FormOutcome::Pressed(d),
            _ => FormOutcome::Consumed,
        }
    }

    fn accel(&self, c: char) -> Option<usize> {
        let c = c.to_ascii_lowercase();
        self.buttons.iter().position(|b| b.enabled && b.accel == Some(c))
    }

    /// Apply one input event (see the module docs).
    pub fn apply(&mut self, ev: &InputEvent) -> FormOutcome {
        let text_focus = self.focused_field().is_some_and(|i| matches!(self.fields[i].kind, FieldKind::Text { .. }));
        if let InputEvent::Chord(c) = ev {
            let letter = letter_of(c.key);
            if let Some(l) = letter {
                let alt = c.mods.contains(Mods::ALT) && !c.mods.intersects(Mods::CTRL | Mods::CMD);
                let plain = !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD);
                if (alt || (plain && !text_focus))
                    && let Some(b) = self.accel(l)
                {
                    return FormOutcome::Pressed(b);
                }
            }
            if !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) {
                match c.key {
                    KeyName::Escape => return FormOutcome::Cancel,
                    KeyName::Tab => {
                        self.move_focus(!c.mods.contains(Mods::SHIFT));
                        return FormOutcome::Consumed;
                    }
                    _ => {}
                }
            }
        }
        if let Some(i) = self.focused_field() {
            return self.field_key(i, ev);
        }
        if self.focused_button().is_some() {
            return self.button_key(ev);
        }
        FormOutcome::Ignored
    }

    fn field_key(&mut self, i: usize, ev: &InputEvent) -> FormOutcome {
        let chord = match ev {
            InputEvent::Chord(c) if !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) => Some(c.key),
            _ => None,
        };
        let enter = chord == Some(KeyName::Enter);
        match &mut self.fields[i].kind {
            FieldKind::Text { text, edit, .. } => {
                let before = text.clone();
                match edit.apply(text, ev) {
                    EditOutcome::Consumed if *text != before => FormOutcome::Changed(i),
                    EditOutcome::Consumed => FormOutcome::Consumed,
                    EditOutcome::Submit => self.press_default(),
                    EditOutcome::Cancel => FormOutcome::Cancel,
                    EditOutcome::Tab | EditOutcome::Down => {
                        self.move_focus(true);
                        FormOutcome::Consumed
                    }
                    EditOutcome::ShiftTab | EditOutcome::Up => {
                        self.move_focus(false);
                        FormOutcome::Consumed
                    }
                    EditOutcome::PageUp | EditOutcome::PageDown | EditOutcome::Ignored => FormOutcome::Ignored,
                }
            }
            FieldKind::Check(on) => match chord {
                Some(KeyName::Space) => {
                    *on = !*on;
                    FormOutcome::Changed(i)
                }
                _ if enter => self.press_default(),
                Some(KeyName::ArrowDown) => {
                    self.move_focus(true);
                    FormOutcome::Consumed
                }
                Some(KeyName::ArrowUp) => {
                    self.move_focus(false);
                    FormOutcome::Consumed
                }
                _ => FormOutcome::Ignored,
            },
            FieldKind::Radio { options, selected } => {
                let n = options.len().max(1);
                match chord {
                    Some(KeyName::ArrowRight | KeyName::Space) => {
                        *selected = (*selected + 1) % n;
                        FormOutcome::Changed(i)
                    }
                    Some(KeyName::ArrowLeft) => {
                        *selected = (*selected + n - 1) % n;
                        FormOutcome::Changed(i)
                    }
                    _ if enter => self.press_default(),
                    Some(KeyName::ArrowDown) => {
                        self.move_focus(true);
                        FormOutcome::Consumed
                    }
                    Some(KeyName::ArrowUp) => {
                        self.move_focus(false);
                        FormOutcome::Consumed
                    }
                    _ => FormOutcome::Ignored,
                }
            }
        }
    }

    fn button_key(&mut self, ev: &InputEvent) -> FormOutcome {
        let InputEvent::Chord(c) = ev else { return FormOutcome::Ignored };
        if c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) {
            return FormOutcome::Ignored;
        }
        match c.key {
            KeyName::Enter | KeyName::Space => self.focused_button().map_or(FormOutcome::Ignored, FormOutcome::Pressed),
            KeyName::ArrowRight => {
                self.move_in_buttons(true);
                FormOutcome::Consumed
            }
            KeyName::ArrowLeft => {
                self.move_in_buttons(false);
                FormOutcome::Consumed
            }
            KeyName::ArrowUp => {
                self.move_focus(false);
                FormOutcome::Consumed
            }
            KeyName::ArrowDown => {
                self.move_focus(true);
                FormOutcome::Consumed
            }
            _ => FormOutcome::Ignored,
        }
    }

    /// A click on field `i` (`DialogField`): focus it; a checkbox toggles. A click in a
    /// text field's box ([`set_field_click`]) puts the cursor under the pointer.
    pub fn click_field(&mut self, i: usize) -> FormOutcome {
        let at = FIELD_CLICK.take();
        let Some(f) = self.fields.get_mut(i).filter(|f| f.enabled) else { return FormOutcome::Ignored };
        self.focus = i;
        if let (FieldKind::Text { text, edit, secret }, Some((col, width))) = (&mut f.kind, at) {
            place_cursor(text, edit, *secret, col, width);
        }
        if let FieldKind::Check(on) = &mut f.kind {
            *on = !*on;
            return FormOutcome::Changed(i);
        }
        FormOutcome::Consumed
    }

    /// A click on button `i` (`DialogButton`).
    pub fn click_button(&mut self, i: usize) -> FormOutcome {
        if self.buttons.get(i).is_some_and(|b| b.enabled) {
            self.focus = self.fields.len() + i;
            FormOutcome::Pressed(i)
        } else {
            FormOutcome::Ignored
        }
    }

    /// The fields as drawn.
    pub fn field_vms(&self) -> Vec<FieldVm> {
        self.fields
            .iter()
            .enumerate()
            .map(|(i, f)| FieldVm {
                label: f.label.clone(),
                kind: match &f.kind {
                    FieldKind::Text { text, edit, secret } => {
                        let shown = if *secret { "•".repeat(text.chars().count()) } else { text.clone() };
                        // The cursor's byte offset maps to the bullets by characters.
                        let cursor =
                            if *secret { "•".len() * text[..edit.cursor()].chars().count() } else { edit.cursor() };
                        let mut e = LineEdit::new();
                        e.set_cursor(&shown, cursor);
                        FieldVmKind::Text { text: shown, edit: e }
                    }
                    FieldKind::Check(on) => FieldVmKind::Check(*on),
                    FieldKind::Radio { options, selected } => {
                        FieldVmKind::Radio { options: options.clone(), selected: *selected }
                    }
                },
                enabled: f.enabled,
                focused: self.focus == i,
            })
            .collect()
    }

    /// The buttons as drawn.
    pub fn button_vms(&self) -> Vec<ButtonVm> {
        self.buttons
            .iter()
            .enumerate()
            .map(|(i, b)| ButtonVm {
                label: b.label.clone(),
                accel: b.accel,
                enabled: b.enabled,
                focused: self.focused_button() == Some(i),
                default: self.default_button == Some(i),
            })
            .collect()
    }
}

thread_local! {
    /// The click being handed to the top-most dialog, when it landed in a text field's
    /// box: its column in the box and the box's width ([`set_field_click`]).
    static FIELD_CLICK: std::cell::Cell<Option<(u16, u16)>> = const { std::cell::Cell::new(None) };
}

/// Before a dialog gets a mouse press (`input::dialogs::mouse`): it landed `col` cells
/// into the text box of the field it hit, `width` cells wide (`Hits::field_text_at`), so
/// [`Form::click_field`] puts the cursor there; `None` after the dispatch. Kinds that do
/// not call `click_field` ignore it.
pub fn set_field_click(click: Option<(u16, u16)>) {
    FIELD_CLICK.set(click);
}

/// Put `edit`'s cursor under a click `col` cells into the box (`width` cells) showing
/// `text` as [`Form::field_vms`] / [`draw_fields`] draw it (bullets for a secret).
fn place_cursor(text: &str, edit: &mut LineEdit, secret: bool, col: u16, width: u16) {
    if !secret {
        // The drawn editor starts unscrolled with the same cursor (`field_vms`).
        let mut e = LineEdit::new();
        e.set_cursor(text, edit.cursor());
        e.click(text, col, width);
        edit.set_cursor(text, e.cursor());
        return;
    }
    let bullets = "•".repeat(text.chars().count());
    let mut e = LineEdit::new();
    e.set_cursor(&bullets, "•".len() * text[..edit.cursor()].chars().count());
    e.click(&bullets, col, width);
    let n = e.cursor() / "•".len();
    let pos = text.char_indices().nth(n).map_or(text.len(), |(b, _)| b);
    edit.set_cursor(text, pos);
}

/// The letter of a key (`a`–`z`).
fn letter_of(k: KeyName) -> Option<char> {
    let name = k.as_str();
    let mut cs = name.chars();
    match (cs.next(), cs.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

/// A field as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldVm {
    pub label: String,
    pub kind: FieldVmKind,
    pub enabled: bool,
    pub focused: bool,
}

/// What a drawn field shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldVmKind {
    /// The text (bullets for a secret) with a cursor-only editor for scrolling.
    Text {
        text: String,
        edit: LineEdit,
    },
    Check(bool),
    Radio {
        options: Vec<String>,
        selected: usize,
    },
}

/// A revealed password lives in the drawn text too: wipe every drawn text when it goes
/// (Phase T7, unlock dialog; best effort, the model is rebuilt every frame).
impl Drop for FieldVm {
    fn drop(&mut self) {
        if let FieldVmKind::Text { text, .. } = &mut self.kind {
            zeroize::Zeroize::zeroize(text);
        }
    }
}

/// A button as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonVm {
    pub label: String,
    pub accel: Option<char>,
    pub enabled: bool,
    pub focused: bool,
    /// Enter in a field presses it.
    pub default: bool,
}

/// Draw `fields` one per row from the top of `area` (labels in a column on the left),
/// recording `DialogField(i)` targets. Returns the cell of the text cursor, if a text
/// field has focus.
pub fn draw_fields(
    buf: &mut Buffer,
    area: Rect,
    fields: &[FieldVm],
    theme: &TuiTheme,
    hits: &mut Hits,
) -> Option<(u16, u16)> {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return None;
    }
    let label_w = fields
        .iter()
        .filter(|f| !matches!(f.kind, FieldVmKind::Check(_)))
        .map(|f| text::width(&f.label))
        .max()
        .unwrap_or(0)
        .min(usize::from(area.width) / 3);
    let mut cursor = None;
    for ((i, f), y) in fields.iter().enumerate().zip(area.y..area.bottom()) {
        let row = Rect::new(area.x, y, area.width, 1);
        let base = theme.style(if f.enabled { Role::Text } else { Role::Disabled });
        let w = usize::from(area.width);
        match &f.kind {
            FieldVmKind::Check(on) => {
                let mark = if *on { "[x] " } else { "[ ] " };
                let s = text::truncate_end(&format!("{mark}{}", f.label), w);
                let style = if f.focused { base.patch(theme.style(Role::PaletteSelected)) } else { base };
                buf.set_stringn(area.x, y, s, w, style);
            }
            FieldVmKind::Text { text: value, edit } => {
                let gap = if label_w > 0 { 1 } else { 0 };
                buf.set_stringn(area.x, y, text::truncate_end(&f.label, label_w), label_w, base);
                let fx = area.x + (label_w + gap) as u16;
                let fw = area.right().saturating_sub(fx);
                if fw > 0 {
                    let field = Rect::new(fx, y, fw, 1);
                    hits.push_field_text(field, i);
                    buf.set_style(field, theme.style(Role::Input));
                    let mut e = edit.clone();
                    let (shown, col) = e.view(value, fw);
                    buf.set_stringn(fx, y, shown, usize::from(fw), theme.style(Role::Input));
                    if f.focused {
                        cursor = Some((fx + col.min(fw - 1), y));
                    }
                }
            }
            FieldVmKind::Radio { options, selected } => {
                let gap = if label_w > 0 { 1 } else { 0 };
                buf.set_stringn(area.x, y, text::truncate_end(&f.label, label_w), label_w, base);
                let mut x = area.x + (label_w + gap) as u16;
                for (k, o) in options.iter().enumerate() {
                    let item = format!("({}) {o}", if k == *selected { "•" } else { " " });
                    let iw = text::width(&item);
                    let room = usize::from(area.right().saturating_sub(x));
                    if room == 0 {
                        break;
                    }
                    let style =
                        if f.focused && k == *selected { base.patch(theme.style(Role::PaletteSelected)) } else { base };
                    buf.set_stringn(x, y, text::truncate_end(&item, room), room, style);
                    x = x.saturating_add((iw + 2) as u16).min(area.right());
                }
            }
        }
        hits.push(row, HitTarget::DialogField(i));
    }
    cursor
}

/// The label of a button as drawn: `[ OK ]`.
pub fn button_label(b: &ButtonVm) -> String {
    format!("[ {} ]", b.label)
}

/// Draw `buttons` right-aligned on the first row of `area` (one space apart; the first
/// ones are dropped when they do not fit), recording `DialogButton(i)` targets.
pub fn draw_buttons(buf: &mut Buffer, area: Rect, buttons: &[ButtonVm], theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(buf.area);
    if area.is_empty() || buttons.is_empty() {
        return;
    }
    let labels: Vec<String> = buttons.iter().map(button_label).collect();
    let mut x = area.right();
    for (i, (b, l)) in buttons.iter().zip(&labels).enumerate().rev() {
        let w = text::width(l) as u16;
        if x < area.x + w {
            break;
        }
        x -= w;
        let role = if !b.enabled {
            Role::Disabled
        } else if b.focused {
            Role::ButtonFocused
        } else {
            Role::Button
        };
        let mut style = theme.style(role);
        if b.default && !b.focused {
            style = style.patch(theme.style(Role::Title));
        }
        let rect = Rect::new(x, area.y, w, 1);
        buf.set_stringn(x, area.y, l, usize::from(w), style);
        if let Some(a) = b.accel
            && let Some(off) = b.label.chars().position(|c| c.to_ascii_lowercase() == a)
            && let Some(cell) = buf.cell_mut((x + 2 + off as u16, area.y))
        {
            cell.modifier.insert(ratatui::style::Modifier::UNDERLINED);
        }
        hits.push(rect, HitTarget::DialogButton(i));
        x = x.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn txt(s: &str) -> InputEvent {
        InputEvent::Text(s.into())
    }

    fn sample() -> Form {
        Form::new(
            vec![
                Field::text("Name", "a.txt"),
                Field::check("Overwrite", false),
                Field::radio("Kind", vec!["Symlink".into(), "Hardlink".into()], 0),
            ],
            vec![Button::new("OK").accel('o'), Button::new("Cancel").accel('c')],
        )
    }

    #[test]
    fn a_click_in_a_text_box_places_the_cursor() {
        let mut f = sample();
        f.focus = 1;
        set_field_click(Some((2, 20)));
        assert_eq!(f.click_field(0), FormOutcome::Consumed);
        assert_eq!(f.focus, 0);
        let FieldKind::Text { edit, .. } = &f.fields[0].kind else { unreachable!() };
        assert_eq!(edit.cursor(), 2);
        // Taken once: a click without a box position keeps the cursor.
        f.click_field(0);
        let FieldKind::Text { edit, .. } = &f.fields[0].kind else { unreachable!() };
        assert_eq!(edit.cursor(), 2);
        // A secret maps the bullets back to characters.
        let mut f = Form::new(vec![Field::secret("Password")], vec![Button::new("OK")]);
        f.fields[0].kind =
            FieldKind::Text { text: "pässwörd".into(), edit: LineEdit::at_end("pässwörd"), secret: true };
        set_field_click(Some((2, 20)));
        f.click_field(0);
        let FieldKind::Text { edit, text, .. } = &f.fields[0].kind else { unreachable!() };
        assert_eq!(&text[edit.cursor()..], "sswörd");
        set_field_click(None);
    }

    #[test]
    fn text_boxes_are_recorded_for_clicks() {
        let m = sample();
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 4)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| {
            draw_fields(f.buffer_mut(), Rect::new(0, 0, 30, 3), &m.field_vms(), &TuiTheme::plain(), &mut hits);
        })
        .unwrap();
        let (r, i) = hits.field_text_at(10, 0).expect("the name box");
        assert_eq!((r.x, r.width, i), (5, 25, 0));
        assert_eq!(hits.field_text_at(1, 0), None, "the label");
        assert_eq!(hits.field_text_at(10, 1), None, "a checkbox has no box");
    }

    #[test]
    fn tab_ring_and_arrows() {
        let mut f = sample();
        assert_eq!(f.focused_field(), Some(0));
        assert_eq!(f.apply(&chord("Tab")), FormOutcome::Consumed);
        assert_eq!(f.focused_field(), Some(1));
        assert_eq!(f.apply(&chord("Down")), FormOutcome::Consumed);
        assert_eq!(f.focused_field(), Some(2));
        f.apply(&chord("Tab"));
        assert_eq!(f.focused_button(), Some(0));
        assert_eq!(f.apply(&chord("Right")), FormOutcome::Consumed);
        assert_eq!(f.focused_button(), Some(1));
        f.apply(&chord("Tab"));
        assert_eq!(f.focused_field(), Some(0), "wraps");
        f.apply(&chord("Shift+Tab"));
        assert_eq!(f.focused_button(), Some(1));
        // Disabled stops are skipped.
        f.buttons[0].enabled = false;
        f.apply(&chord("Shift+Tab"));
        assert_eq!(f.focused_field(), Some(2));
    }

    #[test]
    fn editing_toggling_and_pressing() {
        let mut f = sample();
        assert_eq!(f.apply(&chord("Backspace")), FormOutcome::Changed(0));
        assert_eq!(f.apply(&chord("End")), FormOutcome::Consumed);
        assert_eq!(f.apply(&txt("x")), FormOutcome::Changed(0));
        assert_eq!(f.fields[0].value(), Some("a.txx"));
        // In a text field a plain letter types; Alt+letter presses the button.
        assert_eq!(f.apply(&chord("O")), FormOutcome::Consumed);
        assert_eq!(f.apply(&chord("Alt+C")), FormOutcome::Pressed(1));
        assert_eq!(f.apply(&chord("Enter")), FormOutcome::Pressed(0), "the default button");
        f.apply(&chord("Tab"));
        assert_eq!(f.apply(&chord("Space")), FormOutcome::Changed(1));
        assert_eq!(f.fields[1].on(), Some(true));
        assert_eq!(f.apply(&chord("C")), FormOutcome::Pressed(1), "outside text fields plain letters press");
        f.apply(&chord("Tab"));
        assert_eq!(f.apply(&chord("Right")), FormOutcome::Changed(2));
        assert_eq!(f.fields[2].selected(), Some(1));
        assert_eq!(f.apply(&chord("Right")), FormOutcome::Changed(2));
        assert_eq!(f.fields[2].selected(), Some(0));
        f.apply(&chord("Tab"));
        f.apply(&chord("Right"));
        assert_eq!(f.apply(&chord("Enter")), FormOutcome::Pressed(1));
        assert_eq!(f.apply(&chord("Esc")), FormOutcome::Cancel);
        assert_eq!(f.click_field(1), FormOutcome::Changed(1));
        assert_eq!(f.click_button(0), FormOutcome::Pressed(0));
        assert_eq!(f.apply(&chord("Ctrl+Q")), FormOutcome::Ignored);
    }

    #[test]
    fn secret_fields_hide_their_text() {
        let mut f = Form::new(vec![Field::secret("Password")], vec![Button::new("OK")]);
        f.apply(&txt("hunter2"));
        assert_eq!(f.fields[0].value(), Some("hunter2"));
        let vm = f.field_vms();
        match &vm[0].kind {
            FieldVmKind::Text { text, edit } => {
                assert_eq!(text, "•••••••");
                assert_eq!(edit.cursor(), text.len());
            }
            k => panic!("{k:?}"),
        }
    }

    #[test]
    fn secrets_can_be_revealed_and_taken() {
        let mut f = Form::new(vec![Field::secret("Password")], vec![Button::new("OK")]);
        f.apply(&txt("hunter2"));
        f.fields[0].set_secret(false);
        assert!(matches!(&f.field_vms()[0].kind, FieldVmKind::Text { text, .. } if text == "hunter2"));
        f.fields[0].set_secret(true);
        assert!(matches!(&f.field_vms()[0].kind, FieldVmKind::Text { text, .. } if text == "•••••••"));
        assert_eq!(f.fields[0].take_text(), "hunter2");
        assert_eq!(f.fields[0].value(), Some(""));
        // The cursor went with the text: typing starts again at 0.
        f.apply(&txt("x"));
        assert_eq!(f.fields[0].value(), Some("x"));
        assert_eq!(Field::check("c", true).take_text(), "");
    }

    #[test]
    fn draws_fields_and_buttons() {
        let mut f = sample();
        f.apply(&chord("Tab"));
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 4));
        let mut hits = Hits::default();
        let cur = draw_fields(&mut buf, Rect::new(0, 0, 30, 3), &f.field_vms(), &TuiTheme::plain(), &mut hits);
        assert_eq!(cur, None, "no text field has focus");
        draw_buttons(&mut buf, Rect::new(0, 3, 30, 1), &f.button_vms(), &TuiTheme::plain(), &mut hits);
        let line = |y: u16| (0..30).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert!(line(0).starts_with("Name a.txt"), "{:?}", line(0));
        assert!(line(1).starts_with("[ ] Overwrite"), "{:?}", line(1));
        assert!(line(2).starts_with("Kind (•) Symlink  ( ) Hardlink"), "{:?}", line(2));
        assert!(line(3).ends_with("[ OK ] [ Cancel ]"), "{:?}", line(3));
        assert!(buf[(15, 3)].modifier.contains(Modifier::UNDERLINED), "the O of OK");
        assert_eq!(hits.at(1, 1), Some(HitTarget::DialogField(1)));
        assert_eq!(hits.at(29, 3), Some(HitTarget::DialogButton(1)));
        assert_eq!(hits.at(14, 3), Some(HitTarget::DialogButton(0)));
        // The text cursor when the text field has focus.
        f.focus = 0;
        let cur = draw_fields(&mut buf, Rect::new(0, 0, 30, 3), &f.field_vms(), &TuiTheme::plain(), &mut hits);
        assert_eq!(cur, Some((10, 0)));
        for w in 0..12 {
            for h in 0..3 {
                let mut b = Buffer::empty(Rect::new(0, 0, w, h));
                draw_fields(&mut b, Rect::new(0, 0, w + 3, h), &f.field_vms(), &TuiTheme::plain(), &mut hits);
                draw_buttons(&mut b, Rect::new(0, 0, w + 3, h), &f.button_vms(), &TuiTheme::plain(), &mut hits);
            }
        }
    }
}

//! Modal dialog host — the extension point for conflict/error/properties/batch-rename
//! dialogs of later milestones. A dialog implements [`Dialog`]; push it with
//! [`Dialogs::push`]. The host draws the top-most dialog in a styled modal (fade +
//! scale-in, backdrop, Esc / click-outside to cancel) and routes results as [`Cmd`]s.

pub mod attributes;
pub mod batch_rename;
pub mod conflict;
pub mod delete_confirm;
pub mod error;
pub mod keybind_record;
pub mod links;
pub mod new_item;
pub mod properties;
pub mod properties_attrs;
pub mod properties_checksum;
pub mod properties_details;
pub mod properties_security;
pub mod properties_widgets;

use egui::{Align, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, RichText, Stroke, TextEdit, Ui};
use soyuz_core::config::{ActionCategory, Keymap};
use soyuz_core::{Column, NamePatternSpec, PatternKind};

use crate::cmd::Cmd;
use crate::state::pane::PaneId;
use crate::state::workspace::Workspace;
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{kbd, pill_button};

/// What dialogs may touch while drawing.
pub struct DialogCtx<'a> {
    pub cmds: &'a mut Vec<Cmd>,
    pub ws: &'a mut Workspace,
}

pub trait Dialog {
    fn title(&self) -> &str;
    /// Optional glyph shown next to the title.
    fn icon(&self) -> Option<&'static str> {
        None
    }
    fn width(&self) -> f32 {
        420.0
    }
    /// Draw the body. Return `false` to close.
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool;
}

#[derive(Default)]
pub struct Dialogs {
    stack: Vec<Box<dyn Dialog>>,
}

impl Dialogs {
    pub fn push(&mut self, d: impl Dialog + 'static) {
        self.stack.push(Box::new(d));
    }

    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    /// Draw the top-most dialog.
    pub fn show(&mut self, ctx: &egui::Context, dctx: &mut DialogCtx<'_>) {
        let depth = self.stack.len();
        let Some(top) = self.stack.last_mut() else { return };
        // Keyed by the dialog's identity too, so a smaller dialog opened after a larger
        // one doesn't inherit the remembered size of the previous modal.
        let id = Id::new("soyuz.dialog").with(depth).with((top.icon(), top.width() as i32));
        let p = palette(ctx);
        let t = ctx.animate_bool_with_time(id.with("anim"), true, crate::theme::tokens::motion::MODAL);
        let frame = Frame::new()
            .fill(p.bg_surface3)
            .stroke(Stroke::new(1.0, p.border_strong))
            .corner_radius(CornerRadius::same(radius::MODAL))
            .inner_margin(Margin::same(20))
            .shadow(egui::epaint::Shadow {
                offset: [0, 12],
                blur: 40,
                spread: 0,
                color: Color32::from_black_alpha((p.shadow_alpha * 255.0) as u8),
            });
        let width = top.width();
        let mut keep = true;
        let resp = egui::Modal::new(id)
            .frame(frame)
            .backdrop_color(Color32::from_black_alpha((if p.dark { 110.0 } else { 60.0 } * t) as u8))
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.set_opacity(t);
                ui.horizontal(|ui| {
                    if let Some(g) = top.icon() {
                        ui.label(icons::icon(g, 18.0).color(p.accent));
                        ui.add_space(2.0);
                    }
                    ui.label(RichText::new(top.title()).font(semibold(16.0)).color(p.text_primary));
                });
                ui.add_space(12.0);
                keep = top.show(ui, dctx);
            });
        if resp.should_close() || !keep {
            self.stack.pop();
        }
    }
}

/// Footer row with right-aligned buttons; returns `(primary clicked, cancel clicked)`.
pub fn footer(ui: &mut Ui, primary: &str, cancel: &str) -> (bool, bool) {
    ui.add_space(16.0);
    let mut out = (false, false);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        out.0 = pill_button(ui, None, primary, true).clicked();
        ui.add_space(6.0);
        out.1 = pill_button(ui, None, cancel, false).clicked();
    });
    out
}

/// Footer with a single right-aligned primary button; returns whether it was clicked.
pub fn close_footer(ui: &mut Ui, label: &str) -> bool {
    ui.add_space(16.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| pill_button(ui, None, label, true).clicked()).inner
}

/// Generic confirmation: on confirm the stored [`Cmd`] is queued.
pub struct ConfirmDialog {
    title: String,
    message: String,
    confirm: String,
    on_confirm: Option<Cmd>,
}

impl ConfirmDialog {
    pub fn new(title: &str, message: &str, confirm: &str, on_confirm: Cmd) -> Self {
        ConfirmDialog {
            title: title.into(),
            message: message.into(),
            confirm: confirm.into(),
            on_confirm: Some(on_confirm),
        }
    }
}

impl Dialog for ConfirmDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::QUESTION)
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(&self.message).color(p.text_secondary));
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, &self.confirm, "Cancel");
        if ok || enter {
            if let Some(c) = self.on_confirm.take() {
                ctx.cmds.push(c);
            }
            return false;
        }
        !cancel
    }
}

/// Select by pattern (glob / substring).
pub struct PatternDialog {
    pane: PaneId,
    text: String,
    add: bool,
    focus: bool,
}

impl PatternDialog {
    pub fn new(pane: PaneId) -> Self {
        PatternDialog { pane, text: "*.".into(), add: false, focus: true }
    }
}

impl Dialog for PatternDialog {
    fn title(&self) -> &str {
        "Select by pattern"
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::SELECTION_PLUS)
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new("Glob (*.rs, IMG_??.jpg) or plain text").color(p.text_secondary));
        ui.add_space(8.0);
        let r =
            ui.add(TextEdit::singleline(&mut self.text).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        ui.add_space(8.0);
        ui.checkbox(&mut self.add, "Add to the current selection");
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, "Select", "Cancel");
        if ok || enter {
            let kind = if self.text.contains(['*', '?']) { PatternKind::Glob } else { PatternKind::Substring };
            if let Ok(pat) = NamePatternSpec::new(kind, self.text.trim()).compile()
                && let Some(pane) = ctx.ws.find_pane_mut(self.pane)
            {
                if !self.add {
                    pane.selection.clear();
                }
                let keys: Vec<_> = pane
                    .visible
                    .iter()
                    .filter(|&&i| pat.is_match(&pane.entries[i].name))
                    .map(|&i| pane.keys[i].clone())
                    .collect();
                let n = keys.len();
                for k in keys {
                    pane.selection.insert(k);
                }
                ctx.cmds.push(Cmd::Toast(crate::ui::toasts::Toast::info(format!(
                    "Selected {n} item{}",
                    if n == 1 { "" } else { "s" }
                ))));
            }
            return false;
        }
        !cancel
    }
}

/// Column chooser as a dialog (Action::ChooseColumns).
pub struct ColumnsDialog {
    pane: PaneId,
}

impl ColumnsDialog {
    pub fn new(pane: PaneId) -> Self {
        ColumnsDialog { pane }
    }
}

impl Dialog for ColumnsDialog {
    fn title(&self) -> &str {
        "Columns"
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::COLUMNS)
    }

    fn width(&self) -> f32 {
        300.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let Some(pane) = ctx.ws.find_pane_mut(self.pane) else { return false };
        let visible: Vec<Column> = pane.view.visible_columns().iter().map(|c| c.column).collect();
        let mut changed = false;
        for col in Column::ALL {
            if matches!(
                col,
                Column::Tags
                    | Column::GitStatus
                    | Column::Dimensions
                    | Column::Duration
                    | Column::Checksum
                    | Column::ItemCount
                    | Column::LinkTarget
            ) {
                continue;
            }
            let mut on = visible.contains(&col);
            let r = ui.add_enabled(col != Column::Name, egui::Checkbox::new(&mut on, col.label()));
            if r.changed() {
                pane.view.set_column_visible(col, on);
                pane.layout_sig = None;
                changed = true;
            }
        }
        if changed {
            ctx.cmds.push(Cmd::ViewChanged(self.pane));
        }
        !close_footer(ui, "Done") && !ui.input(|i| i.key_pressed(Key::Enter))
    }
}

/// About Soyuz.
pub struct AboutDialog {
    renderer: &'static str,
}

impl AboutDialog {
    pub fn new(renderer: &'static str) -> Self {
        AboutDialog { renderer }
    }
}

impl Dialog for AboutDialog {
    fn title(&self) -> &str {
        "About Soyuz"
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::PLANET)
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(p.text_primary));
        ui.label(
            RichText::new(format!("A keyboard-first, non-blocking file explorer · renderer: {}", self.renderer))
                .color(p.text_secondary),
        );
        !close_footer(ui, "Close")
    }
}

/// Read-only list of the active key bindings, grouped by category.
pub struct KeybindingsDialog {
    rows: Vec<(ActionCategory, String, String)>,
    filter: String,
}

impl KeybindingsDialog {
    pub fn new(km: &Keymap) -> Self {
        let mut rows: Vec<(ActionCategory, String, String)> = km
            .bindings
            .iter()
            .filter(|(_, chords)| !chords.is_empty())
            .map(|((_, a), chords)| {
                (a.category(), a.title(), chords.iter().map(|c| c.to_string()).collect::<Vec<_>>().join("  "))
            })
            .collect();
        rows.sort();
        rows.dedup();
        KeybindingsDialog { rows, filter: String::new() }
    }
}

impl Dialog for KeybindingsDialog {
    fn title(&self) -> &str {
        "Keyboard shortcuts"
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::KEYBOARD)
    }

    fn width(&self) -> f32 {
        520.0
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.add(
            TextEdit::singleline(&mut self.filter)
                .hint_text("Filter shortcuts")
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        ui.add_space(8.0);
        let f = self.filter.to_lowercase();
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            let mut last: Option<ActionCategory> = None;
            for (cat, title, chords) in &self.rows {
                if !f.is_empty() && !title.to_lowercase().contains(&f) && !chords.to_lowercase().contains(&f) {
                    continue;
                }
                if last != Some(*cat) {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(cat.label().to_uppercase())
                            .font(FontId::proportional(10.5))
                            .color(p.text_tertiary),
                    );
                    last = Some(*cat);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).color(p.text_primary));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        kbd(ui, chords);
                    });
                });
            }
        });
        !close_footer(ui, "Close")
    }
}

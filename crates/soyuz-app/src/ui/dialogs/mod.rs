//! Modal dialog host. Two kinds of dialogs share it:
//!
//! * core prompts ([`DialogModel`], pushed onto `AppCore::dialogs` by core logic: job
//!   prompts, delete confirmations, new link, Copy / Move to, quit, MCP and script
//!   approvals), drawn here by free functions per model;
//! * GUI-only dialogs implementing [`Dialog`] (About, columns, properties, …), pushed
//!   with [`Dialogs::push`].
//!
//! The host draws the top-most dialog of either kind (the most recently opened) in a
//! styled modal (fade + scale-in, backdrop, Esc / click-outside to cancel) and routes
//! results as [`Cmd`]s.

pub mod attributes;
pub mod batch_rename;
pub mod conflict;
pub mod delete_confirm;
pub mod error;
pub mod keybind_record;
pub mod links;
pub mod mcp_approval;
pub mod media_tools;
pub mod new_item;
pub mod open_with;
pub mod properties;
pub mod properties_attrs;
pub mod properties_checksum;
pub mod properties_details;
pub mod properties_inode;
pub mod properties_security;
pub mod properties_widgets;
pub mod quit;
pub mod recovery;
pub mod recovery_crash;
pub mod unlock_volume;

use crate::models::requests::{columns, pattern};
use egui::{Align, Color32, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, RichText, Stroke, TextEdit, Ui};
use soyuz_core::config::{ActionCategory, Keymap};

use crate::cmd::Cmd;
use crate::models::dialogs::{DialogModel, DialogStack, next_seq};
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
    /// Confirmations of destructive or blocking choices are exposed as alert dialogs.
    fn alert(&self) -> bool {
        false
    }
    /// Kind of dialog for the remembered native window position (the type name).
    fn kind_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    /// Draw the body. Return `false` to close.
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool;
}

#[derive(Default)]
pub struct Dialogs {
    /// GUI-only dialogs with their ordering stamp ([`next_seq`]), top-most last.
    stack: Vec<(u64, Box<dyn Dialog>)>,
}

/// Window kind of a core prompt for the remembered native window position (the type
/// names of the dialogs these models replaced, so remembered positions carry over).
fn model_kind(m: &DialogModel) -> &'static str {
    match m {
        DialogModel::Prompt(_) => "soyuz_app::ui::dialogs::conflict::PromptDialog",
        DialogModel::DeleteConfirm(_) => "soyuz_app::ui::dialogs::delete_confirm::DeleteConfirmDialog",
        DialogModel::NewLink(_) => "soyuz_app::ui::dialogs::new_item::LinkDialog",
        DialogModel::Transfer(_) => "soyuz_app::ui::dialogs::new_item::TransferDialog",
        DialogModel::Quit(_) => "soyuz_app::ui::dialogs::quit::QuitDialog",
        DialogModel::McpApproval(_) => "soyuz_app::ui::dialogs::mcp_approval::McpApprovalDialog",
        DialogModel::ScriptApproval(_) => "soyuz_app::ui::automation::approval::ScriptApprovalDialog",
        DialogModel::MediaTools(_) => "soyuz_app::ui::dialogs::media_tools::MediaToolsDialog",
    }
}

fn model_icon(m: &DialogModel) -> Option<&'static str> {
    Some(match m {
        DialogModel::Prompt(m) => conflict::icon(m),
        DialogModel::DeleteConfirm(m) => delete_confirm::icon(m),
        DialogModel::NewLink(_) => ph::LINK,
        DialogModel::Transfer(m) => new_item::transfer_icon(m),
        DialogModel::Quit(_) => ph::WARNING,
        DialogModel::McpApproval(m) => mcp_approval::icon(m),
        DialogModel::ScriptApproval(m) => crate::ui::automation::approval::icon(m),
        DialogModel::MediaTools(m) => media_tools::icon(m),
    })
}

fn model_width(m: &DialogModel) -> f32 {
    match m {
        DialogModel::Prompt(m) => conflict::width(m),
        DialogModel::DeleteConfirm(_) => 440.0,
        DialogModel::NewLink(_) | DialogModel::Transfer(_) => 480.0,
        DialogModel::Quit(_) => 460.0,
        DialogModel::McpApproval(_) => 600.0,
        DialogModel::ScriptApproval(_) => 560.0,
        DialogModel::MediaTools(_) => 560.0,
    }
}

/// Draw a core prompt's body. Returns `false` to close.
fn show_model(ui: &mut Ui, m: &mut DialogModel, ctx: &mut DialogCtx<'_>) -> bool {
    match m {
        DialogModel::Prompt(m) => conflict::show(ui, m, ctx),
        DialogModel::DeleteConfirm(m) => delete_confirm::show(ui, m, ctx),
        DialogModel::NewLink(m) => new_item::show_link(ui, m, ctx),
        DialogModel::Transfer(m) => new_item::show_transfer(ui, m, ctx),
        DialogModel::Quit(m) => quit::show(ui, m, ctx),
        DialogModel::McpApproval(m) => mcp_approval::show(ui, m, ctx),
        DialogModel::ScriptApproval(m) => crate::ui::automation::approval::show(ui, m, ctx),
        DialogModel::MediaTools(m) => media_tools::show(ui, m, ctx),
    }
}

/// The top-most dialog, of either kind.
enum Top<'a> {
    Gui(&'a mut Box<dyn Dialog>),
    Model(&'a mut DialogModel),
}

impl Top<'_> {
    fn title(&self) -> &str {
        match self {
            Top::Gui(d) => d.title(),
            Top::Model(m) => m.title(),
        }
    }

    fn icon(&self) -> Option<&'static str> {
        match self {
            Top::Gui(d) => d.icon(),
            Top::Model(m) => model_icon(m),
        }
    }

    fn width(&self) -> f32 {
        match self {
            Top::Gui(d) => d.width(),
            Top::Model(m) => model_width(m),
        }
    }

    fn alert(&self) -> bool {
        match self {
            Top::Gui(d) => d.alert(),
            Top::Model(m) => m.alert(),
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Top::Gui(d) => d.kind_name(),
            Top::Model(m) => model_kind(m),
        }
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        match self {
            Top::Gui(d) => d.show(ui, ctx),
            Top::Model(m) => show_model(ui, m, ctx),
        }
    }
}

impl Dialogs {
    pub fn push(&mut self, d: impl Dialog + 'static) {
        self.stack.push((next_seq(), Box::new(d)));
    }

    /// A GUI-only dialog is open (`SoyuzApp::dialog_open` also counts core prompts).
    pub fn is_open(&self) -> bool {
        !self.stack.is_empty()
    }

    /// Draw the top-most dialog: the most recently opened of the GUI-only dialogs and
    /// the core prompts in `models`.
    pub fn show(&mut self, ctx: &egui::Context, models: &mut DialogStack, dctx: &mut DialogCtx<'_>) {
        let depth = self.stack.len() + models.len();
        let gui_seq = self.stack.last().map(|(s, _)| *s);
        let model_on_top = match (gui_seq, models.top_seq()) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(g), Some(m)) => m > g,
        };
        let mut top = if model_on_top {
            let Some(m) = models.top_mut() else { return };
            Top::Model(m)
        } else {
            let Some((_, d)) = self.stack.last_mut() else { return };
            Top::Gui(d)
        };
        if Self::draw(ctx, depth, &mut top, dctx) {
            if model_on_top {
                models.close_top();
            } else {
                self.stack.pop();
            }
        }
    }

    /// Draw `top`; returns whether it closes.
    fn draw(ctx: &egui::Context, depth: usize, top: &mut Top<'_>, dctx: &mut DialogCtx<'_>) -> bool {
        if crate::ui::native::active(ctx) {
            let kind = Id::new(top.kind_name());
            let title = top.title().to_string();
            let mut w = crate::ui::native::dialog::DialogWindow::new(
                Id::new("soyuz.dialog.native").with(depth).with(kind),
                kind,
                &title,
            );
            w.icon = top.icon();
            w.alert = top.alert();
            w.width = top.width();
            let shown = w.show(ctx, |ui| top.show(ui, dctx));
            return shown.close || shown.inner == Some(false);
        }
        // Keyed by the dialog's identity too, so a smaller dialog opened after a larger
        // one doesn't inherit the remembered size of the previous modal.
        let id = Id::new("soyuz.dialog").with(depth).with((top.icon(), top.width() as i32));
        let p = palette(ctx);
        let t = ctx.animate_bool_with_time(
            id.with("anim"),
            true,
            crate::theme::anim(ctx, crate::theme::tokens::motion::MODAL),
        );
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
                crate::ui::a11y::dialog(ui.ctx(), ui.unique_id(), top.title(), top.alert(), true);
                ui.horizontal(|ui| {
                    if let Some(g) = top.icon() {
                        crate::ui::widgets::icon_label(ui, icons::icon(g, 18.0).color(p.accent));
                        ui.add_space(2.0);
                    }
                    ui.label(RichText::new(top.title()).font(semibold(16.0)).color(p.text_primary));
                });
                ui.add_space(12.0);
                keep = top.show(ui, dctx);
            });
        resp.should_close() || !keep
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
    m: crate::models::requests::confirm::ConfirmModel,
}

impl ConfirmDialog {
    pub fn new(title: &str, message: &str, confirm: &str, on_confirm: Cmd) -> Self {
        ConfirmDialog { m: crate::models::requests::confirm::ConfirmModel::new(title, message, confirm, on_confirm) }
    }
}

impl Dialog for ConfirmDialog {
    fn title(&self) -> &str {
        &self.m.title
    }

    fn alert(&self) -> bool {
        true
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::QUESTION)
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(&self.m.message).color(p.text_secondary));
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, &self.m.confirm, tr!("common-cancel"));
        if ok || enter {
            if let Some(c) = self.m.confirm() {
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
        PatternDialog { pane, text: pattern::DEFAULT_PATTERN.into(), add: false, focus: true }
    }
}

impl Dialog for PatternDialog {
    fn title(&self) -> &str {
        tr!("dialogs-pattern-title")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::SELECTION_PLUS)
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(tr!("dialogs-pattern-hint")).color(p.text_secondary));
        ui.add_space(8.0);
        let r =
            ui.add(TextEdit::singleline(&mut self.text).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        ui.add_space(8.0);
        ui.checkbox(&mut self.add, tr!("dialogs-pattern-add"));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("dialogs-pattern-select"), tr!("common-cancel"));
        if ok || enter {
            if let Some(toast) = pattern::apply(ctx.ws, self.pane, &self.text, self.add) {
                ctx.cmds.push(toast);
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
        tr!("dialogs-columns-title")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::COLUMNS)
    }

    fn width(&self) -> f32 {
        300.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let Some(pane) = ctx.ws.find_pane_mut(self.pane) else { return false };
        let mut changed = false;
        for col in columns::choosable() {
            let mut on = columns::is_visible(pane, col);
            let label = columns::label(col);
            let r = ui.add_enabled(columns::can_toggle(col), egui::Checkbox::new(&mut on, label));
            if r.changed() {
                changed |= columns::set_visible(pane, col, on);
            }
        }
        if changed {
            ctx.cmds.push(Cmd::ViewChanged(self.pane));
        }
        !close_footer(ui, tr!("dialogs-done")) && !ui.input(|i| i.key_pressed(Key::Enter))
    }
}

/// About Soyuz: icon, version, build facts, license, and a "Copy info" button for bug
/// reports.
pub struct AboutDialog {
    renderer: &'static str,
    icon: Option<egui::TextureHandle>,
}

impl AboutDialog {
    pub fn new(renderer: &'static str) -> Self {
        AboutDialog { renderer, icon: None }
    }

    /// `(label, value)` facts copied by "Copy info" (`models::about::facts`).
    pub fn facts(&self) -> Vec<(&'static str, String)> {
        crate::models::about::facts(self.renderer)
    }
}

/// Upper-case for small captions, with the Turkish dotted/dotless i rules
/// (`str::to_uppercase` maps `i` to `I`).
pub(crate) fn caption_upper(s: &str) -> String {
    if soyuz_core::locale::current() == soyuz_core::Locale::TrTr {
        s.chars().map(|c| if c == 'i' { "\u{130}".to_string() } else { c.to_uppercase().collect() }).collect()
    } else {
        s.to_uppercase()
    }
}

impl Dialog for AboutDialog {
    fn title(&self) -> &str {
        tr!("dialogs-about-title")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::PLANET)
    }

    fn width(&self) -> f32 {
        440.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let tex = self
            .icon
            .get_or_insert_with(|| {
                ui.ctx().load_texture(
                    "soyuz.about.icon",
                    crate::app_icon::color_image(128),
                    egui::TextureOptions::LINEAR,
                )
            })
            .clone();
        ui.horizontal(|ui| {
            ui.add(egui::Image::new((tex.id(), egui::vec2(72.0, 72.0))));
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("Soyuz").font(semibold(22.0)).color(p.text_primary));
                ui.label(
                    RichText::new(tr!("dialogs-about-version-line", version = env!("CARGO_PKG_VERSION")))
                        .color(p.text_secondary),
                );
                ui.label(RichText::new(tr!("dialogs-about-tagline")).color(p.text_tertiary).size(12.0));
            });
        });
        ui.add_space(12.0);
        let facts = self.facts();
        egui::Grid::new("soyuz.about.facts").num_columns(2).striped(false).spacing([16.0, 6.0]).show(ui, |ui| {
            // The version is already in the header.
            for (k, v) in facts.iter().skip(1) {
                ui.label(RichText::new(crate::models::about::fact_label(k)).color(p.text_tertiary));
                ui.add(egui::Label::new(RichText::new(v).color(p.text_primary)).truncate());
                ui.end_row();
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("dialogs-about-license")).color(p.text_tertiary).size(11.5));
        ui.label(RichText::new(crate::models::about::ffmpeg_notice()).color(p.text_tertiary).size(11.5));
        let (close, copy) = footer(ui, tr!("common-close"), tr!("dialogs-about-copy-info"));
        if copy {
            ctx.cmds.push(Cmd::CopyText(crate::models::about::copy_text(&facts)));
        }
        !close && !ui.input(|i| i.key_pressed(Key::Enter))
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
        tr!("dialogs-keys-title")
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
                .hint_text(tr!("dialogs-keys-filter"))
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
                        RichText::new(caption_upper(crate::i18n::labels::action_category(*cat)))
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
        !close_footer(ui, tr!("common-close"))
    }
}

#[cfg(test)]
mod tests {
    use crate::i18n::{FluentArgs, t_args_in};
    use soyuz_core::Locale;

    fn render(key: &str, args: &[(&str, i64)], l: Locale) -> String {
        let mut a = FluentArgs::new();
        for (k, v) in args {
            a.set(*k, *v);
        }
        t_args_in(key, &a, l)
    }

    #[test]
    fn plural_messages_render() {
        assert_eq!(render("dialogs-props-contains", &[("files", 1), ("dirs", 2)], Locale::EnUs), "1 file, 2 folders");
        assert_eq!(render("dialogs-props-contains", &[("files", 1), ("dirs", 2)], Locale::TrTr), "1 dosya, 2 klasör");
        assert_eq!(render("dialogs-delete-trash-many", &[("count", 3)], Locale::EnUs), "Move 3 items to the trash?");
        assert_eq!(
            render("dialogs-delete-trash-many", &[("count", 3)], Locale::TrTr),
            "3 öğe Çöp Kutusu'na taşınsın mı?"
        );
        assert_eq!(render("dialogs-quit-title", &[("count", 1)], Locale::EnUs), "A file operation is still running");
    }
}

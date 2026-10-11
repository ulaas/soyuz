//! Modal dialog host — the extension point for conflict/error/properties/batch-rename
//! dialogs. A dialog implements [`Dialog`]; push it with
//! [`Dialogs::push`]. The host draws the top-most dialog in a styled modal (fade +
//! scale-in, backdrop, Esc / click-outside to cancel) and routes results as [`Cmd`]s.

pub mod attributes;
pub mod batch_rename;
pub mod conflict;
pub mod delete_confirm;
pub mod error;
pub mod keybind_record;
pub mod links;
pub mod mcp_approval;
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
            if shown.close || shown.inner == Some(false) {
                self.stack.pop();
            }
            return;
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

    fn alert(&self) -> bool {
        true
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::QUESTION)
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(&self.message).color(p.text_secondary));
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, &self.confirm, tr!("common-cancel"));
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
                ctx.cmds.push(Cmd::Toast(crate::ui::toasts::Toast::info(tr!("dialogs-pattern-selected", count = n))));
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
        let visible: Vec<Column> = pane.view.visible_columns().iter().map(|c| c.column).collect();
        let mut changed = false;
        for col in Column::ALL {
            if matches!(
                col,
                Column::Dimensions | Column::Duration | Column::Checksum | Column::ItemCount | Column::LinkTarget
            ) {
                continue;
            }
            if col.script_slot().is_some() && crate::automation::scripts::column_label(col).is_none() {
                continue;
            }
            let mut on = visible.contains(&col);
            let label = crate::ui::views::columns::header_label(col);
            let r = ui.add_enabled(col != Column::Name, egui::Checkbox::new(&mut on, label));
            if r.changed() {
                pane.view.set_column_visible(col, on);
                pane.layout_sig = None;
                changed = true;
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

    /// `(label, value)` facts copied by "Copy info" (English, for bug reports).
    pub fn facts(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Version", env!("CARGO_PKG_VERSION").to_string()),
            ("Commit", crate::GIT_SHA.to_string()),
            ("Build", if cfg!(debug_assertions) { "debug".into() } else { "release".into() }),
            ("Platform", format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH)),
            ("Renderer", self.renderer.to_string()),
            ("Config", soyuz_fs::strip_verbatim(&soyuz_core::config::config_dir()).display().to_string()),
            ("Memory", memory_line()),
        ]
    }

    /// Translated label of a [`facts`](Self::facts) entry.
    fn fact_label(key: &str) -> &'static str {
        match key {
            "Commit" => tr!("dialogs-about-commit"),
            "Build" => tr!("dialogs-about-build"),
            "Platform" => tr!("dialogs-about-platform"),
            "Renderer" => tr!("dialogs-about-renderer"),
            "Config" => tr!("dialogs-about-config"),
            "Memory" => tr!("dialogs-about-memory"),
            _ => tr!("dialogs-about-version"),
        }
    }
}

/// Cache memory for the About dialog: thumbnails and previews (decoded bytes).
fn memory_line() -> String {
    let mib = |b: u64| format!("{} MiB", soyuz_core::locale::format_decimal(b as f64 / (1024.0 * 1024.0), 1));
    let (tb, tn) = crate::services::thumbs::memory();
    let (pb, pn) = crate::services::preview_cache::memory();
    tr!("dialogs-about-memory-line", thumbs = mib(tb), thumb_count = tn, previews = mib(pb), preview_count = pn)
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
                ui.label(RichText::new(Self::fact_label(k)).color(p.text_tertiary));
                ui.add(egui::Label::new(RichText::new(v).color(p.text_primary)).truncate());
                ui.end_row();
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("dialogs-about-license")).color(p.text_tertiary).size(11.5));
        let (close, copy) = footer(ui, tr!("common-close"), tr!("dialogs-about-copy-info"));
        if copy {
            let text = facts.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n");
            ctx.cmds.push(Cmd::CopyText(format!("Soyuz\n{text}")));
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

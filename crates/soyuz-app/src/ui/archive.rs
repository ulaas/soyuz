//! Archives (GUI): the archive rows of the items menu and the password / extract /
//! compress / rename dialogs. Opening, extracting, compressing and zip editing live in
//! `models::archive`.

use std::path::PathBuf;

use egui::{Align2, CornerRadius, FontId, Key, Margin, RichText, Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2};
use soyuz_archive::ArchiveFormat;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::models::archive::{
    ArchivePrompt, CreateForm, ExtractForm, format_label, level_label, rename_cmd, unlock, valid_entry_name,
    writable_formats,
};
pub use crate::models::archive::{
    archive_stem, child_location, create_dialog, execute_edit, extract_entries, extract_here, extract_to_dialog,
    extract_to_folder, handles_edit, indices_for, open_entry, pump, submit_job, test_archive, unique_dir,
};
use crate::state::pane::PaneId;
use crate::theme::icons::ph;
use crate::theme::palette;
use crate::ui::dialogs::{Dialog, DialogCtx, footer};

/// Show a dialog the core asked for ([`crate::models::frontend::FrontendCmd::Archive`]).
pub fn prompt(app: &mut SoyuzApp, p: ArchivePrompt) {
    match p {
        ArchivePrompt::Password { pane, archive, wrong } => {
            app.gui.dialogs.push(PasswordDialog { pane, archive, wrong, text: String::new(), focus: true })
        }
        ArchivePrompt::Extract(form) => app.gui.dialogs.push(ExtractDialog { form }),
        ArchivePrompt::Create(form) => app.gui.dialogs.push(CreateDialog { form, focus: true }),
        ArchivePrompt::Rename { archive, from, is_dir } => {
            let text = from.rsplit('/').next().unwrap_or_default().to_string();
            app.gui.dialogs.push(RenameEntryDialog { archive, from, is_dir, text, focus: true })
        }
    }
}

// ---- dialogs ------------------------------------------------------------------------

pub struct PasswordDialog {
    pane: PaneId,
    archive: PathBuf,
    wrong: bool,
    text: String,
    focus: bool,
}

impl Dialog for PasswordDialog {
    fn title(&self) -> &str {
        tr!("views-archive-password-title")
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::LOCK_SIMPLE)
    }
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let name = soyuz_core::entry::display_name(&self.archive);
        ui.label(RichText::new(tr!("views-archive-password-body", name = &name)).color(p.text_secondary));
        if self.wrong {
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("views-archive-password-wrong")).color(p.danger));
        }
        ui.add_space(10.0);
        let r = ui.add(
            TextEdit::singleline(&mut self.text)
                .password(true)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("views-archive-unlock"), tr!("common-cancel"));
        if (ok || enter) && !self.text.is_empty() {
            ctx.cmds.push(unlock(self.pane, &self.archive, std::mem::take(&mut self.text)));
            return false;
        }
        !cancel
    }
}

pub struct ExtractDialog {
    form: ExtractForm,
}

impl Dialog for ExtractDialog {
    fn title(&self) -> &str {
        tr!("views-archive-extract")
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::TRAY_ARROW_DOWN)
    }
    fn width(&self) -> f32 {
        460.0
    }
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(self.form.summary()).color(p.text_secondary));
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("views-archive-destination")).size(11.5).color(p.text_tertiary));
        ui.add_space(2.0);
        let r = ui.add(
            TextEdit::singleline(&mut self.form.dest).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)),
        );
        ui.add_space(6.0);
        ui.checkbox(&mut self.form.subfolder, tr!("views-archive-subfolder"));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("views-archive-extract"), tr!("common-cancel"));
        if ok || enter {
            let Some(cmds) = self.form.submit() else { return true };
            ctx.cmds.extend(cmds);
            return false;
        }
        !cancel
    }
}

pub struct CreateDialog {
    form: CreateForm,
    focus: bool,
}

impl Dialog for CreateDialog {
    fn title(&self) -> &str {
        tr!("views-archive-compress")
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::PACKAGE)
    }
    fn width(&self) -> f32 {
        480.0
    }
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let n = self.form.sources.len();
        ui.label(
            RichText::new(if n == 1 {
                soyuz_core::entry::display_name(&self.form.sources[0])
            } else {
                tr!("common-items", count = n)
            })
            .color(p.text_secondary),
        );
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("views-archive-name")).size(11.5).color(p.text_tertiary));
        ui.add_space(2.0);
        let r = ui.add(
            TextEdit::singleline(&mut self.form.name).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)),
        );
        if self.focus {
            r.request_focus();
            let stem_len = archive_stem(&self.form.name).chars().count();
            if let Some(mut state) = TextEdit::load_state(ui.ctx(), r.id) {
                let range =
                    egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(stem_len));
                state.cursor.set_char_range(Some(range));
                state.store(ui.ctx(), r.id);
            }
            self.focus = false;
        }
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("views-archive-format")).size(11.5).color(p.text_tertiary));
        ui.add_space(2.0);
        // Text segmented control.
        let formats = writable_formats();
        let cell = vec2(68.0, 26.0);
        let (rect, _) = ui.allocate_exact_size(vec2(cell.x * formats.len() as f32 + 4.0, cell.y + 4.0), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(6), p.bg_surface2);
        for (i, f) in formats.iter().enumerate() {
            let r = egui::Rect::from_min_size(pos2(rect.left() + 2.0 + i as f32 * cell.x, rect.top() + 2.0), cell);
            let resp = ui.interact(r, ui.scope_id().with(("fmt", i)), Sense::click());
            let sel = *f == self.form.format;
            if sel {
                ui.painter().rect_filled(r, CornerRadius::same(5), p.bg_surface1);
                ui.painter().rect_stroke(
                    r,
                    CornerRadius::same(5),
                    Stroke::new(1.0, p.border_subtle),
                    StrokeKind::Inside,
                );
            }
            let c = if sel {
                p.text_primary
            } else if resp.hovered() {
                p.text_secondary
            } else {
                p.text_tertiary
            };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, format_label(*f), FontId::proportional(12.0), c);
            if resp.clicked() {
                self.form.set_format(*f);
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("views-archive-compression")).size(11.5).color(p.text_tertiary));
            let mut lvl = self.form.level as f32;
            ui.add(egui::Slider::new(&mut lvl, 0.0..=9.0).step_by(1.0).show_value(false));
            self.form.level = lvl.round() as u8;
            let label = level_label(self.form.level);
            ui.label(RichText::new(format!("{label} ({})", self.form.level)).size(11.5).color(p.text_secondary));
        });
        if self.form.format.supports_password() {
            ui.add_space(8.0);
            ui.label(RichText::new(tr!("views-archive-password-optional")).size(11.5).color(p.text_tertiary));
            ui.add_space(2.0);
            ui.add(
                TextEdit::singleline(&mut self.form.password)
                    .password(true)
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(10, 7)),
            );
            if self.form.format == ArchiveFormat::SevenZ && !self.form.password.is_empty() {
                ui.label(RichText::new(tr!("views-archive-names-encrypted")).size(11.0).color(p.text_tertiary));
            }
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("views-archive-compress"), tr!("common-cancel"));
        if ok || enter {
            match self.form.submit() {
                Ok(cmd) => ctx.cmds.push(cmd),
                Err(toast) => {
                    ctx.cmds.extend(toast.map(Cmd::Toast));
                    return true;
                }
            }
            return false;
        }
        !cancel
    }
}

pub struct RenameEntryDialog {
    archive: PathBuf,
    from: String,
    is_dir: bool,
    text: String,
    focus: bool,
}

impl Dialog for RenameEntryDialog {
    fn title(&self) -> &str {
        tr!("views-archive-rename-title")
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::PENCIL_SIMPLE)
    }
    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let kind = if self.is_dir { "folder" } else { "file" };
        ui.label(
            RichText::new(tr!(
                "views-archive-rename-body",
                kind = kind,
                name = &self.from,
                archive = soyuz_core::entry::display_name(&self.archive)
            ))
            .color(p.text_secondary),
        );
        ui.add_space(10.0);
        let r =
            ui.add(TextEdit::singleline(&mut self.text).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        let name = self.text.trim().to_string();
        let valid = valid_entry_name(&name);
        if !valid && !name.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("views-archive-no-slashes")).color(p.danger));
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("common-rename"), tr!("common-cancel"));
        if (ok || enter) && valid {
            ctx.cmds.extend(rename_cmd(&self.archive, &self.from, &name));
            return false;
        }
        !cancel
    }
}

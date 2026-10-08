//! Small input dialogs for file operations: create a symbolic link / hard link /
//! junction, and "Copy to…" / "Move to…" with a destination path.

use std::path::PathBuf;

use egui::{Align, FontId, Key, Layout, Margin, RichText, TextEdit, Ui};

use super::conflict::{Btn, button};
use super::{Dialog, DialogCtx};
use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;
use crate::ops::fileops::dnd::DropEffect;
use crate::theme::{icons::ph, palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkChoice {
    Symlink,
    Hardlink,
    Junction,
}

impl LinkChoice {
    fn label(self) -> &'static str {
        match self {
            LinkChoice::Symlink => "Symbolic link",
            LinkChoice::Hardlink => "Hard link",
            LinkChoice::Junction => "Junction",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            LinkChoice::Symlink => {
                "Points to a file or folder by path; may cross volumes. On Windows this needs \
                                    Developer Mode or elevation."
            }
            LinkChoice::Hardlink => "Another name for the same file data; files only, same volume.",
            LinkChoice::Junction => "A Windows folder link (no special rights needed); folders only.",
        }
    }
}

fn field(ui: &mut Ui, label: &str, text: &mut String, hint: &str, focus: &mut bool) -> egui::Response {
    let p = palette(ui.ctx());
    ui.label(RichText::new(label).font(FontId::proportional(11.5)).color(p.text_secondary));
    let r = ui
        .add(TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
    if *focus {
        r.request_focus();
        *focus = false;
    }
    ui.add_space(8.0);
    r
}

pub struct LinkDialog {
    dir: PathBuf,
    target: String,
    name: String,
    kind: LinkChoice,
    existing: Vec<String>,
    focus: bool,
    name_touched: bool,
}

impl LinkDialog {
    pub fn new(dir: PathBuf, target: Option<PathBuf>, kind: LinkChoice, existing: Vec<String>) -> Self {
        let target_s = target.as_ref().map(|t| soyuz_fs::strip_verbatim(t).display().to_string()).unwrap_or_default();
        let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
        let name = target.as_ref().map(|t| crate::ops::fileops::names::link_name(t, &refs)).unwrap_or_default();
        LinkDialog { dir, target: target_s, name, kind, existing, focus: true, name_touched: false }
    }
}

impl Dialog for LinkDialog {
    fn title(&self) -> &str {
        "Create link"
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::LINK)
    }

    fn width(&self) -> f32 {
        480.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.horizontal(|ui| {
            for k in [LinkChoice::Symlink, LinkChoice::Hardlink, LinkChoice::Junction] {
                if k == LinkChoice::Junction && !cfg!(windows) {
                    continue;
                }
                ui.selectable_value(&mut self.kind, k, k.label());
            }
        });
        ui.label(RichText::new(self.kind.hint()).font(FontId::proportional(11.5)).color(p.text_tertiary));
        ui.add_space(10.0);
        let mut no_focus = false;
        let t = field(ui, "Target", &mut self.target, "Path of the file or folder to link to", &mut self.focus);
        if t.changed() && !self.name_touched {
            let refs: Vec<&str> = self.existing.iter().map(String::as_str).collect();
            self.name = crate::ops::fileops::names::link_name(std::path::Path::new(self.target.trim()), &refs);
        }
        let n = field(ui, "Link name", &mut self.name, "Name of the new link", &mut no_focus);
        if n.changed() {
            self.name_touched = true;
        }
        let name = self.name.trim().to_string();
        let target = self.target.trim().to_string();
        let err = if target.is_empty() {
            Some("Enter a target path.".to_string())
        } else if let Err(e) = soyuz_core::validate_name(&name) {
            Some(format!("Invalid name: {e}"))
        } else if self.existing.iter().any(|e| e.eq_ignore_ascii_case(&name)) {
            Some(format!("\u{201c}{name}\u{201d} already exists here."))
        } else {
            None
        };
        if let Some(e) = &err
            && !target.is_empty()
        {
            ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.danger));
        }
        ui.label(
            RichText::new(format!("In {}", soyuz_fs::strip_verbatim(&self.dir).display()))
                .font(FontId::proportional(11.5))
                .color(p.text_tertiary),
        );
        ui.add_space(16.0);
        let mut ok = false;
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = button(ui, "Create", Btn::Primary).clicked();
            cancel = button(ui, "Cancel", Btn::Secondary).clicked();
        });
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            ok = true;
        }
        if ok && err.is_none() {
            ctx.cmds.push(Cmd::FileOp(FileOpCmd::CreateLink {
                dir: self.dir.clone(),
                name,
                target: PathBuf::from(target),
                kind: self.kind,
            }));
            return false;
        }
        !cancel
    }
}

/// "Copy to…" / "Move to…" with a destination folder.
pub struct TransferDialog {
    paths: Vec<PathBuf>,
    dest: String,
    mv: bool,
    focus: bool,
}

impl TransferDialog {
    pub fn new(paths: Vec<PathBuf>, start: PathBuf, mv: bool) -> Self {
        TransferDialog { paths, dest: soyuz_fs::strip_verbatim(&start).display().to_string(), mv, focus: true }
    }
}

impl Dialog for TransferDialog {
    fn title(&self) -> &str {
        if self.mv { "Move to folder" } else { "Copy to folder" }
    }

    fn icon(&self) -> Option<&'static str> {
        Some(if self.mv { ph::ARROW_BEND_DOWN_RIGHT } else { ph::COPY })
    }

    fn width(&self) -> f32 {
        480.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let n = self.paths.len();
        let what = if n == 1 {
            format!("\u{201c}{}\u{201d}", soyuz_core::entry::display_name(&self.paths[0]))
        } else {
            format!("{n} items")
        };
        ui.label(
            RichText::new(format!("{} {what} to:", if self.mv { "Move" } else { "Copy" })).color(p.text_secondary),
        );
        ui.add_space(6.0);
        let mut f = self.focus;
        field(ui, "Destination folder", &mut self.dest, "Folder path", &mut f);
        self.focus = f;
        let dest = PathBuf::from(soyuz_fs::expand_env(self.dest.trim()));
        let mut ok = false;
        let mut cancel = false;
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = button(ui, if self.mv { "Move" } else { "Copy" }, Btn::Primary).clicked();
            cancel = button(ui, "Cancel", Btn::Secondary).clicked();
        });
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            ok = true;
        }
        if ok && !self.dest.trim().is_empty() {
            let effect = if self.mv { DropEffect::Move } else { DropEffect::Copy };
            ctx.cmds.push(Cmd::FileOp(FileOpCmd::Transfer { paths: self.paths.clone(), dest, effect }));
            return false;
        }
        !cancel
    }
}

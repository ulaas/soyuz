//! Small input dialogs for file operations: create a symbolic link / hard link /
//! junction, and "Copy to…" / "Move to…" with a destination path.

use std::path::PathBuf;

use egui::{Align, FontId, Key, Layout, Margin, RichText, TextEdit, Ui};

use super::conflict::{Btn, button};
use super::{Dialog, DialogCtx};
use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;
use crate::ops::fileops::drop::DropEffect;
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
            LinkChoice::Symlink => tr!("dialogs-link-symlink"),
            LinkChoice::Hardlink => tr!("dialogs-link-hardlink"),
            LinkChoice::Junction => tr!("dialogs-link-junction"),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            LinkChoice::Symlink if cfg!(windows) => tr!("dialogs-link-symlink-hint"),
            LinkChoice::Symlink => tr!("dialogs-link-symlink-hint-unix"),
            LinkChoice::Hardlink => tr!("dialogs-link-hardlink-hint"),
            LinkChoice::Junction => tr!("dialogs-link-junction-hint"),
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
    /// Validate the name with the Windows rules (see `rename::windows_rules_for`).
    windows_rules: bool,
}

impl LinkDialog {
    pub fn new(dir: PathBuf, target: Option<PathBuf>, kind: LinkChoice, existing: Vec<String>) -> Self {
        let target_s = target.as_ref().map(|t| soyuz_fs::strip_verbatim(t).display().to_string()).unwrap_or_default();
        let refs: Vec<&str> = existing.iter().map(String::as_str).collect();
        let name = target.as_ref().map(|t| crate::ops::fileops::names::link_name(t, &refs)).unwrap_or_default();
        LinkDialog {
            dir,
            target: target_s,
            name,
            kind,
            existing,
            focus: true,
            name_touched: false,
            windows_rules: cfg!(windows),
        }
    }

    /// Validate names with the Windows rules (Windows-family volume or strict names).
    pub fn windows_rules(mut self, on: bool) -> Self {
        self.windows_rules = on;
        self
    }
}

impl Dialog for LinkDialog {
    fn title(&self) -> &str {
        tr!("dialogs-link-title")
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
        let t =
            field(ui, tr!("dialogs-link-target"), &mut self.target, tr!("dialogs-link-target-hint"), &mut self.focus);
        if t.changed() && !self.name_touched {
            let refs: Vec<&str> = self.existing.iter().map(String::as_str).collect();
            self.name = crate::ops::fileops::names::link_name(std::path::Path::new(self.target.trim()), &refs);
        }
        let n = field(ui, tr!("dialogs-link-name"), &mut self.name, tr!("dialogs-link-name-hint"), &mut no_focus);
        if n.changed() {
            self.name_touched = true;
        }
        let name = self.name.trim().to_string();
        let target = self.target.trim().to_string();
        let err = if target.is_empty() {
            Some(tr!("dialogs-link-need-target").to_string())
        } else if let Err(e) = soyuz_core::validate_name_with(&name, self.windows_rules) {
            Some(tr!("dialogs-props-invalid-name", error = e.to_string()))
        } else if self.existing.iter().any(|e| e.eq_ignore_ascii_case(&name)) {
            Some(tr!("dialogs-link-exists", name = &name))
        } else {
            None
        };
        if let Some(e) = &err
            && !target.is_empty()
        {
            ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.danger));
        }
        ui.label(
            RichText::new(tr!("dialogs-link-in", dir = soyuz_fs::strip_verbatim(&self.dir).display().to_string()))
                .font(FontId::proportional(11.5))
                .color(p.text_tertiary),
        );
        ui.add_space(16.0);
        let mut ok = false;
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = button(ui, tr!("dialogs-link-create"), Btn::Primary).clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
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
        if self.mv { tr!("dialogs-transfer-move-title") } else { tr!("dialogs-transfer-copy-title") }
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
        let text = if n == 1 {
            let name = crate::ui::widgets::shown_name(&soyuz_core::entry::display_name(&self.paths[0]));
            if self.mv {
                tr!("dialogs-transfer-move-one", name = name)
            } else {
                tr!("dialogs-transfer-copy-one", name = name)
            }
        } else if self.mv {
            tr!("dialogs-transfer-move-many", count = n)
        } else {
            tr!("dialogs-transfer-copy-many", count = n)
        };
        ui.label(RichText::new(text).color(p.text_secondary));
        ui.add_space(6.0);
        let mut f = self.focus;
        field(ui, tr!("dialogs-transfer-dest"), &mut self.dest, tr!("dialogs-transfer-dest-hint"), &mut f);
        self.focus = f;
        let dest = PathBuf::from(soyuz_fs::expand_env(self.dest.trim()));
        let mut ok = false;
        let mut cancel = false;
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = button(ui, if self.mv { tr!("common-move") } else { tr!("common-copy") }, Btn::Primary).clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
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

//! "Open with…" chooser: the applications registered for the file's type (looked up off
//! the UI thread, shared with the context menu's cache) plus any program by path.

use std::path::{Path, PathBuf};

use egui::{Margin, RichText, ScrollArea, TextEdit, Ui};
use soyuz_fs::AppInfo;

use super::{Dialog, DialogCtx, footer};
use crate::cmd::Cmd;
use crate::theme::{icons::ph, palette};

pub struct OpenWithDialog {
    path: PathBuf,
    title: String,
    /// Index into the registered apps.
    selected: Option<usize>,
    /// "Another program" path typed by the user.
    custom: String,
    error: Option<String>,
}

impl OpenWithDialog {
    pub fn new(path: PathBuf) -> Self {
        let title = format!(
            "Open \u{201c}{}\u{201d} with",
            crate::ui::widgets::shown_name(&soyuz_core::entry::display_name(&path))
        );
        OpenWithDialog { path, title, selected: None, custom: String::new(), error: None }
    }
}

/// An [`AppInfo`] for a program chosen by path (the file is appended as its argument).
pub fn custom_app(exe: &Path) -> AppInfo {
    let name = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| exe.display().to_string());
    AppInfo {
        id: exe.display().to_string(),
        name,
        exec: exe.to_path_buf(),
        args: Vec::new(),
        icon: None,
        launchable: true,
    }
}

/// Parse the "another program" field: trims whitespace and surrounding quotes.
pub fn parse_program(text: &str) -> Option<PathBuf> {
    let t = text.trim().trim_matches('"').trim();
    (!t.is_empty()).then(|| PathBuf::from(t))
}

impl Dialog for OpenWithDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::APP_WINDOW)
    }

    fn width(&self) -> f32 {
        460.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let apps = crate::ui::context_menu::apps_for_cached(ui.ctx(), &self.path);
        let mut open_now = false;
        egui::Frame::new()
            .fill(p.bg_sunken)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(Margin::symmetric(6, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| match &apps {
                    None => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0));
                            ui.label(RichText::new("Looking up applications…").color(p.text_secondary));
                        });
                    }
                    Some(list) if list.is_empty() => {
                        ui.label(RichText::new("No registered applications for this file type").color(p.text_tertiary));
                    }
                    Some(list) => {
                        for (i, app) in list.iter().enumerate() {
                            let label = if app.launchable {
                                app.name.clone()
                            } else {
                                format!("{} (opens with the default handler)", app.name)
                            };
                            let r = ui.add_sized(
                                [ui.available_width(), 26.0],
                                egui::Button::selectable(self.selected == Some(i), label),
                            );
                            if r.clicked() {
                                self.selected = Some(i);
                                self.custom.clear();
                                self.error = None;
                            }
                            if r.double_clicked() {
                                open_now = true;
                            }
                        }
                    }
                });
            });
        ui.add_space(12.0);
        ui.label(RichText::new("Or another program (full path):").color(p.text_secondary));
        ui.add_space(4.0);
        let hint = if cfg!(windows) { r"C:\Program Files\App\app.exe" } else { "/usr/bin/app" };
        let r = ui.add(
            TextEdit::singleline(&mut self.custom)
                .hint_text(hint)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        if r.changed() && !self.custom.trim().is_empty() {
            self.selected = None;
            self.error = None;
        }
        let submit = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if let Some(e) = &self.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(p.danger));
        }
        let (ok, cancel) = footer(ui, "Open", "Cancel");
        if cancel {
            return false;
        }
        if !(ok || open_now || submit) {
            return true;
        }
        let chosen = if let Some(exe) = parse_program(&self.custom) {
            if !exe.is_absolute() || !exe.is_file() {
                self.error = Some(format!("{} is not a program file", exe.display()));
                return true;
            }
            Some(custom_app(&exe))
        } else {
            self.selected.and_then(|i| apps.as_ref().and_then(|l| l.get(i).cloned()))
        };
        match chosen {
            Some(app) => {
                ctx.cmds.push(Cmd::OpenWith { path: self.path.clone(), app });
                false
            }
            None => {
                self.error = Some("Choose an application or type a program path".into());
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_field() {
        assert_eq!(parse_program("  \"C:\\A B\\x.exe\" "), Some(PathBuf::from("C:\\A B\\x.exe")));
        assert_eq!(parse_program("   "), None);
        let a = custom_app(Path::new("/usr/bin/gimp"));
        assert_eq!(a.name, "gimp");
        assert!(a.args.is_empty() && a.launchable);
    }
}

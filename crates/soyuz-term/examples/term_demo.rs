//! Minimal eframe window hosting a `soyuz-term` terminal.
//!
//! `cargo run -p soyuz-term --example term_demo [-- <shell> [args...]]`
//! Ctrl+= / Ctrl+- change the font size; links open with Ctrl+click.

use eframe::egui;
use soyuz_term::{TermEvent, TermSettings, TermTheme, Terminal, TerminalView};

struct Demo {
    term: Option<Terminal>,
    error: Option<String>,
    focused: bool,
    font_size: f32,
    status: String,
    theme: TermTheme,
}

impl Demo {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut args = std::env::args().skip(1);
        let settings = TermSettings {
            shell: args.next(),
            args: args.collect(),
            cwd: std::env::current_dir().unwrap_or_else(|_| ".".into()),
            ..Default::default()
        };
        let (term, error) = match Terminal::spawn(settings, soyuz_term::egui_wake(cc.egui_ctx.clone()), 1) {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(e.to_string())),
        };
        Demo { term, error, focused: true, font_size: 13.0, status: String::new(), theme: TermTheme::dark() }
    }
}

impl eframe::App for Demo {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(term) = &self.term {
            for ev in term.events().try_iter() {
                match ev {
                    TermEvent::Title(t) => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{t} - term_demo")))
                    }
                    TermEvent::CwdChanged(p) => self.status = format!("cwd: {}", p.display()),
                    TermEvent::Exited(code) => self.status = format!("shell exited ({code:?})"),
                    TermEvent::Bell => self.status = "bell".into(),
                    TermEvent::Clipboard(text) => ctx.copy_text(text),
                }
            }
        }
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::CTRL, egui::Key::Equals)
                || i.consume_key(egui::Modifiers::CTRL, egui::Key::Plus)
            {
                self.font_size = (self.font_size + 1.0).min(32.0);
            }
            if i.consume_key(egui::Modifiers::CTRL, egui::Key::Minus) {
                self.font_size = (self.font_size - 1.0).max(7.0);
            }
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                if ui.button("Light/Dark").clicked() {
                    self.theme = if self.theme == TermTheme::dark() { TermTheme::light() } else { TermTheme::dark() };
                }
                if let Some(term) = &self.term
                    && ui.button("cd ~").clicked()
                {
                    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default();
                    if !term.send_cd(std::path::Path::new(&home)) {
                        self.status = "terminal busy".into();
                    }
                }
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| match &self.term {
            Some(term) => {
                let r = TerminalView::new(term)
                    .focused(self.focused)
                    .theme(&self.theme)
                    .font(egui::FontId::monospace(self.font_size))
                    .show(ui);
                if r.wants_focus {
                    self.focused = true;
                }
                if let Some(url) = r.link_clicked {
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
                if r.escape_on_empty_line {
                    self.status = "Esc on empty prompt (app would move focus)".into();
                }
            }
            None => {
                ui.label(format!("failed to start terminal: {}", self.error.as_deref().unwrap_or("?")));
            }
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 600.0]).with_title("term_demo"),
        ..Default::default()
    };
    eframe::run_native("term_demo", options, Box::new(|cc| Ok(Box::new(Demo::new(cc)))))
}

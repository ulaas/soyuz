//! `SoyuzApp`: the eframe application.

use eframe::CreationContext;
use soyuz_core::config::{self, Config};

use crate::theme;
use crate::ui::shell::{self, ShellState};

pub struct SoyuzApp {
    config: Config,
    shell: ShellState,
    renderer: &'static str,
}

impl SoyuzApp {
    pub fn new(cc: &CreationContext<'_>, renderer: &'static str) -> Self {
        let path = config::default_path();
        let config = match config::load_or_default(&path) {
            Ok(loaded) => {
                for w in &loaded.warnings {
                    log::warn!("{}: {w}", path.display());
                }
                loaded.config
            }
            Err(e) => {
                log::error!("{}: {e}; using defaults", path.display());
                Config::default()
            }
        };
        egui_extras::install_image_loaders(&cc.egui_ctx);
        for w in theme::install(&cc.egui_ctx, &config.appearance, &config.theme.colors) {
            log::warn!("{w}");
        }
        let shell = ShellState::new(config.appearance.sidebar_width);
        SoyuzApp { config, shell, renderer }
    }
}

impl eframe::App for SoyuzApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        shell::show(ui, &mut self.shell, self.renderer, self.config.layout.sidebar, self.config.layout.status_bar);
    }
}

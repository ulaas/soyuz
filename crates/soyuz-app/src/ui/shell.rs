//! Root layout: status bar (bottom), sidebar (left), toolbar (top) and the content area.

use egui::{CentralPanel, Frame, Ui};

use crate::theme::{icons::ph, palette};
use crate::ui::empty_state::EmptyState;
use crate::ui::sidebar::{self, SidebarSel, SidebarState};
use crate::ui::statusbar;
use crate::ui::toolbar::{self, ToolbarState};

pub struct ShellState {
    pub sidebar: SidebarState,
    pub toolbar: ToolbarState,
}

impl ShellState {
    pub fn new(sidebar_width: f32) -> Self {
        ShellState { sidebar: SidebarState::new(sidebar_width), toolbar: ToolbarState { filter: String::new() } }
    }
}

pub fn show(ui: &mut Ui, state: &mut ShellState, renderer: &str, show_sidebar: bool, show_status: bool) {
    let selection = state.sidebar.selection();
    let (title, path) = selection.clone().unwrap_or_else(|| ("Soyuz".into(), None));

    if show_status {
        statusbar::show(ui, "0 items", renderer);
    }
    if show_sidebar {
        sidebar::show(ui, &mut state.sidebar);
    }
    toolbar::show(ui, &mut state.toolbar, &title, path.as_deref());

    let p = palette(ui.ctx());
    CentralPanel::default().frame(Frame::new().fill(p.bg_window)).show(ui, |ui| {
        let (glyph, color, subtitle) = match state.sidebar.selected {
            Some(SidebarSel::Trash) => (ph::TRASH, Some(p.text_secondary), "The trash view is not available yet."),
            Some(SidebarSel::Volume(_)) => (ph::HARD_DRIVES, Some(p.info), "Folder listing is not wired up yet."),
            _ => (ph::FOLDER_OPEN, Some(p.glyph_folder), "Folder listing is not wired up yet."),
        };
        EmptyState {
            glyph,
            glyph_color: color,
            title: &title,
            subtitle,
            hints: &[("Ctrl+Shift+P", "Command palette"), ("Ctrl+L", "Go to path"), ("Ctrl+`", "Terminal")],
        }
        .show(ui);
    });
}

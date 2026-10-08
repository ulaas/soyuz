//! Toolbar above the content area: navigation, breadcrumb, filter, view toggles.
//! M0: visual only (navigation lands in M1).

use std::path::{Component, Path};

use egui::{Align, Frame, Layout, Margin, Panel, RichText, Stroke, TextEdit, Ui, vec2};

use crate::theme::tokens::{radius, size};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::{icon_button, vdivider};

pub struct ToolbarState {
    pub filter: String,
}

pub fn show(root: &mut Ui, state: &mut ToolbarState, title: &str, path: Option<&Path>) {
    let p = palette(root.ctx());
    Panel::top("soyuz.toolbar")
        .exact_size(size::TOOLBAR)
        .frame(Frame::new().fill(p.bg_surface1).inner_margin(Margin::symmetric(10, 0)))
        .show(root, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                icon_button(ui, ph::ARROW_LEFT, "Back (Alt+Left)", false);
                icon_button(ui, ph::ARROW_RIGHT, "Forward (Alt+Right)", false);
                icon_button(ui, ph::ARROW_UP, "Up (Alt+Up)", false);
                vdivider(ui, 20.0);
                breadcrumb(ui, title, path);

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    icon_button(ui, ph::BELL, "Activity", true);
                    icon_button(ui, ph::SQUARE_SPLIT_HORIZONTAL, "Dual pane (F9)", true);
                    icon_button(ui, ph::SORT_ASCENDING, "Sort", true);
                    icon_button(ui, ph::SQUARES_FOUR, "Grid view (Ctrl+Shift+2)", true);
                    icon_button(ui, ph::LIST, "Details view (Ctrl+Shift+1)", true);
                    ui.add_space(6.0);
                    Frame::new()
                        .fill(p.bg_surface2)
                        .stroke(Stroke::new(1.0, p.border_subtle))
                        .corner_radius(radius::ROW)
                        .inner_margin(Margin::symmetric(8, 3))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(icons::icon(ph::MAGNIFYING_GLASS, 14.0).color(p.text_tertiary));
                                ui.add(
                                    TextEdit::singleline(&mut state.filter)
                                        .hint_text("Filter")
                                        .frame(Frame::NONE)
                                        .desired_width(180.0),
                                );
                            });
                        });
                });
            });
        });
}

/// Path segments as subtle buttons separated by carets.
fn breadcrumb(ui: &mut Ui, title: &str, path: Option<&Path>) {
    let p = palette(ui.ctx());
    let segments: Vec<String> = match path {
        Some(path) => path
            .components()
            .filter_map(|c| match c {
                Component::Prefix(pre) => Some(pre.as_os_str().to_string_lossy().into_owned()),
                Component::RootDir => None,
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect(),
        None => vec![title.to_string()],
    };
    let segments = if segments.is_empty() { vec!["/".to_string()] } else { segments };
    let last = segments.len() - 1;
    ui.spacing_mut().item_spacing.x = 0.0;
    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == last;
        let color = if is_last { p.text_primary } else { p.text_secondary };
        let text = RichText::new(seg).color(color);
        let text = if is_last { text.strong() } else { text };
        ui.add(egui::Button::new(text).frame_when_inactive(false).min_size(vec2(0.0, 26.0)));
        if !is_last {
            ui.label(icons::icon(ph::CARET_RIGHT, 11.0).color(p.text_tertiary));
        }
    }
}

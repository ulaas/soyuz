//! First-run welcome card: a dismissible tip card in the bottom-right corner with the
//! handful of shortcuts that make Soyuz click. Dismissal is remembered in the session;
//! the card can be brought back from the palette (`Welcome tips`).

use egui::{Align, Align2, Area, CornerRadius, Frame, Id, Layout, Margin, Order, RichText, Stroke, vec2};
use soyuz_core::config::Action;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::theme::icons::{self, ph};
use crate::theme::palette;
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{icon_button, kbd, pill_button};

const TIPS: &[(Action, &str, &str)] = &[
    (Action::CommandPalette, ph::COMMAND, "Every command, searchable"),
    (Action::JumpToFolderDialog, ph::FOLDER_OPEN, "Jump to any folder you visited"),
    (Action::Search, ph::MAGNIFYING_GLASS, "Search names and contents"),
    (Action::ToggleDualPane, ph::SQUARE_SPLIT_HORIZONTAL, "Two folders side by side"),
    (Action::ToggleTerminal, ph::TERMINAL_WINDOW, "A terminal that follows you"),
    (Action::QuickLook, ph::EYE, "Preview the selected file"),
];

/// Draw the card when it is due. `bottom` is the space taken by the status bar.
pub fn show(ctx: &egui::Context, app: &mut SoyuzApp, bottom: f32) {
    if !app.ui.welcome || app.ui.dialogs.is_open() || app.settings.is_open() {
        return;
    }
    let p = palette(ctx);
    let mut dismiss = false;
    let mut settings = false;
    Area::new(Id::new("soyuz.welcome"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -(bottom + 12.0)))
        .show(ctx, |ui| {
            Frame::new()
                .fill(p.bg_surface3)
                .stroke(Stroke::new(1.0, p.border_strong))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(16))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 28,
                    spread: 0,
                    color: egui::Color32::from_black_alpha((p.shadow_alpha * 255.0) as u8),
                })
                .show(ui, |ui| {
                    ui.set_width(340.0);
                    crate::ui::a11y::container(
                        ui.ctx(),
                        ui.unique_id(),
                        egui::accesskit::Role::Dialog,
                        "Welcome to Soyuz",
                    );
                    ui.horizontal(|ui| {
                        ui.label(icons::icon(ph::ROCKET_LAUNCH, 18.0).color(p.accent));
                        ui.label(RichText::new("Welcome to Soyuz").font(semibold(15.0)).color(p.text_primary));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if icon_button(ui, ph::X, "Dismiss", true).clicked() {
                                dismiss = true;
                            }
                        });
                    });
                    ui.label(
                        RichText::new("Keyboard-first and never blocking. A few shortcuts to start with:")
                            .color(p.text_secondary),
                    );
                    ui.add_space(8.0);
                    for (action, glyph, what) in TIPS {
                        let Some(keys) = chord_label(&app.keymap, action) else { continue };
                        ui.horizontal(|ui| {
                            ui.set_min_height(24.0);
                            ui.label(icons::icon(glyph, 15.0).color(p.text_tertiary));
                            ui.label(RichText::new(*what).color(p.text_primary));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                kbd(ui, &keys);
                            });
                        });
                    }
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill_button(ui, None, "Got it", true).clicked() {
                            dismiss = true;
                        }
                        ui.add_space(6.0);
                        if pill_button(ui, Some(ph::GEAR_SIX), "Settings", false).clicked() {
                            settings = true;
                        }
                    });
                });
        });
    if settings {
        app.cmds.push(Cmd::Action(Action::Settings));
        dismiss = true;
    }
    if dismiss {
        app.ui.welcome = false;
        app.ui.welcome_seen = true;
        app.save_state();
    }
}

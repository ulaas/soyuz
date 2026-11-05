//! First-run welcome card: a dismissible tip card in the bottom-right corner with the
//! handful of shortcuts that make Soyuz click. Esc or its buttons close it; that is
//! remembered in the session, and the card can be brought back from the palette
//! (`Welcome tips`). `[appearance] welcome_tips` off: new profiles start without it.

use egui::{Align, Align2, Area, CornerRadius, Frame, Id, Layout, Margin, Order, RichText, Stroke, vec2};
use soyuz_core::config::Action;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::keys::chord_label;
use crate::theme::icons::{self, ph};
use crate::theme::palette;
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{Btn, icon_button, kbd, pill_button};

const TIPS: &[(Action, &str)] = &[
    (Action::CommandPalette, ph::COMMAND),
    (Action::JumpToFolderDialog, ph::FOLDER_OPEN),
    (Action::Search, ph::MAGNIFYING_GLASS),
    (Action::ToggleDualPane, ph::SQUARE_SPLIT_HORIZONTAL),
    (Action::ToggleTerminal, ph::TERMINAL_WINDOW),
    (Action::QuickLook, ph::EYE),
];

fn tip_text(action: &Action) -> &'static str {
    match action {
        Action::CommandPalette => tr!("app-welcome-tip-palette"),
        Action::JumpToFolderDialog => tr!("app-welcome-tip-jump"),
        Action::Search => tr!("app-welcome-tip-search"),
        Action::ToggleDualPane => tr!("app-welcome-tip-dual"),
        Action::ToggleTerminal => tr!("app-welcome-tip-terminal"),
        _ => tr!("app-welcome-tip-preview"),
    }
}

/// Close the card and remember that it was seen (its buttons, Esc).
pub fn dismiss(app: &mut SoyuzApp) {
    app.core.ui.welcome = false;
    app.core.ui.welcome_seen = true;
    app.core.save_state();
}

/// Draw the card when it is due. `bottom` is the space taken by the status bar.
pub fn show(ctx: &egui::Context, app: &mut SoyuzApp, bottom: f32) {
    if !app.core.ui.welcome || app.dialog_open() || app.gui.settings.is_open() {
        return;
    }
    let p = palette(ctx);
    let mut close = false;
    let mut settings = false;
    Area::new(Id::unique("soyuz.welcome"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -(bottom + 12.0)))
        .show(ctx, |ui| {
            Frame::new()
                .fill(p.bg_surface3)
                .stroke(Stroke::new(1.0, p.border_strong))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(16))
                .shadow(p.shadow(crate::theme::tokens::shadow::WINDOW))
                .show(ui, |ui| {
                    ui.set_width(340.0);
                    crate::ui::a11y::container(
                        ui.ctx(),
                        ui.unique_id(),
                        egui::accesskit::Role::Dialog,
                        tr!("app-welcome-title"),
                    );
                    ui.horizontal(|ui| {
                        ui.label(icons::icon(ph::ROCKET_LAUNCH, 18.0).color(p.accent));
                        ui.label(RichText::new(tr!("app-welcome-title")).font(semibold(15.0)).color(p.text_primary));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if icon_button(ui, ph::X, tr!("app-welcome-dismiss"), true).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.label(RichText::new(tr!("app-welcome-intro")).color(p.text_secondary));
                    ui.add_space(8.0);
                    for (action, glyph) in TIPS {
                        let what = tip_text(action);
                        let Some(keys) = chord_label(&app.core.keymap, action) else { continue };
                        ui.horizontal(|ui| {
                            ui.set_min_height(24.0);
                            ui.label(icons::icon(glyph, 15.0).color(p.text_tertiary));
                            ui.label(RichText::new(what).color(p.text_primary));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                kbd(ui, &keys);
                            });
                        });
                    }
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pill_button(ui, None, tr!("app-welcome-got-it"), Btn::Primary).clicked() {
                            close = true;
                        }
                        ui.add_space(6.0);
                        if pill_button(ui, Some(ph::GEAR_SIX), tr!("app-welcome-settings"), Btn::Secondary).clicked() {
                            settings = true;
                        }
                    });
                });
        });
    if settings {
        app.core.cmds.push(Cmd::Action(Action::Settings));
        close = true;
    }
    if close {
        dismiss(app);
    }
}

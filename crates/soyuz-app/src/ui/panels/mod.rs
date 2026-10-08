//! Bottom panel host — the extension point for the integrated terminal (M8) and the
//! activity / jobs panel (M2). The host owns visibility, the active tab and the
//! resizable height; integrations replace the placeholder bodies in [`show`] with their
//! own widgets (e.g. `soyuz_term::TerminalView`, the job cards).

pub mod activity;
pub mod activity_history;
pub mod terminal;

use egui::{Align2, CornerRadius, FontId, Frame, Margin, Panel, Sense, Stroke, Ui, pos2, vec2};

use crate::theme::tokens::{radius, size};
use crate::theme::{icons, icons::ph, palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomTab {
    Terminal,
    Activity,
}

impl BottomTab {
    fn label(self) -> &'static str {
        match self {
            BottomTab::Terminal => "Terminal",
            BottomTab::Activity => "Activity",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            BottomTab::Terminal => ph::TERMINAL_WINDOW,
            BottomTab::Activity => ph::PULSE,
        }
    }
}

pub struct BottomPanel {
    pub open: bool,
    pub tab: BottomTab,
    pub height: f32,
}

impl Default for BottomPanel {
    fn default() -> Self {
        BottomPanel { open: false, tab: BottomTab::Terminal, height: size::TERMINAL_DEFAULT }
    }
}

impl BottomPanel {
    /// Toggle `tab`: opens the panel on it, switches to it, or closes when already shown.
    pub fn toggle(&mut self, tab: BottomTab) {
        if self.open && self.tab == tab {
            self.open = false;
        } else {
            self.open = true;
            self.tab = tab;
        }
    }
}

/// Draw the panel (when open) above the status bar.
pub fn show(root: &mut Ui, app: &mut crate::app::SoyuzApp) {
    if !app.ui.bottom.open {
        return;
    }
    let p = palette(root.ctx());
    let resp = Panel::bottom("soyuz.bottom")
        .resizable(true)
        .default_size(app.ui.bottom.height)
        .size_range(120.0..=720.0)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 8, right: 8, top: 0, bottom: 6 }))
        .show(root, |ui| {
            let card = ui.available_rect_before_wrap();
            ui.painter().rect_filled(card, CornerRadius::same(radius::MODAL), p.bg_surface1);
            ui.painter().rect_stroke(
                card,
                CornerRadius::same(radius::MODAL),
                Stroke::new(1.0, p.border_subtle),
                egui::StrokeKind::Inside,
            );
            // Tab header.
            let mut x = card.left() + 8.0;
            for tab in [BottomTab::Terminal, BottomTab::Activity] {
                let r = egui::Rect::from_min_size(pos2(x, card.top() + 6.0), vec2(108.0, 26.0));
                let resp = ui.interact(r, ui.id().with(("bottom.tab", tab.label())), Sense::click());
                let active = app.ui.bottom.tab == tab;
                if active {
                    ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface2);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface2.gamma_multiply(0.6));
                }
                let c = if active { p.text_primary } else { p.text_secondary };
                ui.painter().text(
                    pos2(r.left() + 16.0, r.center().y),
                    Align2::CENTER_CENTER,
                    tab.glyph(),
                    icons::font(14.0),
                    c,
                );
                ui.painter().text(
                    pos2(r.left() + 30.0, r.center().y),
                    Align2::LEFT_CENTER,
                    tab.label(),
                    FontId::proportional(12.5),
                    c,
                );
                if resp.clicked() {
                    app.ui.bottom.tab = tab;
                }
                x += 112.0;
            }
            let close = egui::Rect::from_center_size(pos2(card.right() - 20.0, card.top() + 19.0), vec2(24.0, 24.0));
            let cresp = ui.interact(close, ui.id().with("bottom.close"), Sense::click());
            if cresp.hovered() {
                ui.painter().rect_filled(close, CornerRadius::same(radius::CONTROL), p.bg_surface2);
            }
            ui.painter().text(close.center(), Align2::CENTER_CENTER, ph::X, icons::font(13.0), p.text_tertiary);
            if cresp.clicked() {
                app.ui.bottom.open = false;
            }
            let body = egui::Rect::from_min_max(pos2(card.left(), card.top() + 38.0), card.max);
            ui.allocate_rect(card, Sense::hover());
            match app.ui.bottom.tab {
                BottomTab::Terminal => terminal::body(ui, body, app),
                BottomTab::Activity => activity::show(ui, body, app),
            }
        });
    app.ui.bottom.height = resp.response.rect.height();
}

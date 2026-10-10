//! Bottom panel host — the extension point for the integrated terminal and the
//! activity / jobs panel. The host owns visibility, the active tab and the
//! resizable height; integrations replace the placeholder bodies in [`show`] with their
//! own widgets (e.g. `soyuz_term::TerminalView`, the job cards).

pub mod activity;
pub mod activity_history;
pub mod terminal;

use egui::{Align2, CornerRadius, FontId, Frame, Margin, Panel, Sense, Stroke, Ui, pos2, vec2};

use soyuz_core::config::ActivityPlacement;

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
            BottomTab::Terminal => tr!("panels-tab-terminal"),
            BottomTab::Activity => tr!("panels-tab-activity"),
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
    /// Apply `height` on the next frame even if the user resized the panel before
    /// (`[layout] terminal_height` changed).
    pub reset_height: bool,
    /// `[layout] activity_panel`: a bottom-panel tab, a right side panel or a window.
    pub activity_placement: ActivityPlacement,
    /// The activity panel is shown (right / floating placements).
    pub activity_open: bool,
}

impl Default for BottomPanel {
    fn default() -> Self {
        BottomPanel {
            open: false,
            tab: BottomTab::Terminal,
            height: size::TERMINAL_DEFAULT,
            reset_height: false,
            activity_placement: ActivityPlacement::Bottom,
            activity_open: false,
        }
    }
}

impl BottomPanel {
    /// Toggle `tab`: opens the panel on it, switches to it, or closes when already shown.
    pub fn toggle(&mut self, tab: BottomTab) {
        if tab == BottomTab::Activity && self.activity_placement != ActivityPlacement::Bottom {
            self.activity_open = !self.activity_open;
            return;
        }
        if self.open && self.tab == tab {
            self.open = false;
        } else {
            self.open = true;
            self.tab = tab;
        }
    }

    /// Show the activity panel wherever it is placed.
    pub fn open_activity(&mut self) {
        if self.activity_placement == ActivityPlacement::Bottom {
            self.open = true;
            self.tab = BottomTab::Activity;
        } else {
            self.activity_open = true;
        }
    }

    pub fn activity_shown(&self) -> bool {
        match self.activity_placement {
            ActivityPlacement::Bottom => self.open && self.tab == BottomTab::Activity,
            _ => self.activity_open,
        }
    }

    /// Move the activity panel (keeps it visible if it was).
    pub fn set_activity_placement(&mut self, placement: ActivityPlacement) {
        if placement == self.activity_placement {
            return;
        }
        let shown = self.activity_shown();
        if self.activity_placement == ActivityPlacement::Bottom && shown {
            self.open = false;
            self.tab = BottomTab::Terminal;
        }
        self.activity_placement = placement;
        self.activity_open = false;
        if shown {
            self.open_activity();
        }
    }

    fn tabs(&self) -> &'static [BottomTab] {
        if self.activity_placement == ActivityPlacement::Bottom {
            &[BottomTab::Terminal, BottomTab::Activity]
        } else {
            &[BottomTab::Terminal]
        }
    }
}

/// The activity panel for the right / floating placements (the bottom placement is a
/// tab of the bottom panel).
pub fn show_activity(root: &mut Ui, app: &mut crate::app::SoyuzApp) {
    let placement = app.ui.bottom.activity_placement;
    if placement == ActivityPlacement::Bottom || !app.ui.bottom.activity_open {
        return;
    }
    let p = palette(root.ctx());
    if placement == ActivityPlacement::Floating {
        let mut open = true;
        egui::Window::new(tr!("panels-tab-activity"))
            .id(egui::Id::new("soyuz.activity.window"))
            .open(&mut open)
            .default_size([440.0, 340.0])
            .min_size([320.0, 200.0])
            .resizable(true)
            .collapsible(false)
            .frame(
                Frame::new()
                    .inner_margin(Margin::same(8))
                    .fill(p.bg_surface1)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(CornerRadius::same(radius::MODAL)),
            )
            .show(root.ctx(), |ui| {
                let body = ui.available_rect_before_wrap();
                ui.allocate_rect(body, Sense::hover());
                activity::show(ui, body, app);
            });
        if !open {
            app.ui.bottom.activity_open = false;
        }
        return;
    }
    let mut close = false;
    Panel::right("soyuz.activity")
        .resizable(true)
        .default_size(360.0)
        .size_range(280.0..=720.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 0, right: 8, top: 2, bottom: 6 }))
        .show(root, |ui| {
            let card = ui.available_rect_before_wrap();
            ui.painter().rect_filled(card, CornerRadius::same(radius::MODAL), p.bg_surface1);
            ui.painter().rect_stroke(
                card,
                CornerRadius::same(radius::MODAL),
                Stroke::new(1.0, p.border_subtle),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                pos2(card.left() + 22.0, card.top() + 19.0),
                Align2::CENTER_CENTER,
                ph::PULSE,
                icons::font(14.0),
                p.text_primary,
            );
            ui.painter().text(
                pos2(card.left() + 36.0, card.top() + 19.0),
                Align2::LEFT_CENTER,
                tr!("panels-tab-activity"),
                FontId::proportional(12.5),
                p.text_primary,
            );
            let x = egui::Rect::from_center_size(pos2(card.right() - 20.0, card.top() + 19.0), vec2(24.0, 24.0));
            let xr = ui.interact(x, ui.id().with("activity.close"), Sense::click());
            crate::ui::a11y::button(&xr, tr!("panels-close-activity"));
            if xr.hovered() {
                ui.painter().rect_filled(x, CornerRadius::same(radius::CONTROL), p.bg_surface2);
            }
            ui.painter().text(x.center(), Align2::CENTER_CENTER, ph::X, icons::font(13.0), p.text_tertiary);
            close = xr.clicked();
            let body = egui::Rect::from_min_max(pos2(card.left(), card.top() + 38.0), card.max);
            ui.allocate_rect(card, Sense::hover());
            activity::show(ui, body, app);
        });
    if close {
        app.ui.bottom.activity_open = false;
    }
}

/// Draw the panel (when open) above the status bar.
pub fn show(root: &mut Ui, app: &mut crate::app::SoyuzApp) {
    if !app.ui.bottom.open {
        return;
    }
    if !app.ui.bottom.tabs().contains(&app.ui.bottom.tab) {
        app.ui.bottom.tab = BottomTab::Terminal;
    }
    let p = palette(root.ctx());
    let mut panel = Panel::bottom("soyuz.bottom").resizable(true).default_size(app.ui.bottom.height);
    if std::mem::take(&mut app.ui.bottom.reset_height) {
        panel = panel.exact_size(app.ui.bottom.height);
    }
    let resp = panel
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
            for &tab in app.ui.bottom.tabs() {
                let label_w = ui
                    .painter()
                    .layout_no_wrap(tab.label().to_string(), FontId::proportional(12.5), p.text_primary)
                    .size()
                    .x;
                let tab_w = (label_w + 44.0).max(108.0);
                let r = egui::Rect::from_min_size(pos2(x, card.top() + 6.0), vec2(tab_w, 26.0));
                let resp = ui.interact(r, ui.id().with(("bottom.tab", tab as u8)), Sense::click());
                let active = app.ui.bottom.tab == tab;
                crate::ui::a11y::tab(&resp, tab.label(), active);
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
                x += tab_w + 4.0;
            }
            let close = egui::Rect::from_center_size(pos2(card.right() - 20.0, card.top() + 19.0), vec2(24.0, 24.0));
            let cresp = ui.interact(close, ui.id().with("bottom.close"), Sense::click());
            crate::ui::a11y::button(&cresp, tr!("panels-close-panel"));
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

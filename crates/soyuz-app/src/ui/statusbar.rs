//! Bottom status bar: item / selection counts, free space, view-mode switcher, zoom
//! slider and theme switcher. Describes the focused pane.

use egui::{
    Align, Align2, CornerRadius, FontId, Frame, Layout, Margin, Panel, Rect, RichText, Sense, ThemePreference, Ui, vec2,
};
use soyuz_core::config::Action;
use soyuz_core::{SizeStyle, ViewMode, format_size};

use crate::cmd::Cmd;
use crate::state::pane::PaneId;
use crate::theme::tokens::size;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::{mini_slider, segmented, spinner, usage_bar};

pub struct StatusInfo {
    pub pane: PaneId,
    pub visible: usize,
    pub total: usize,
    pub selected: usize,
    pub selected_bytes: u64,
    pub loading: bool,
    pub filtered: bool,
    /// Entries hidden by the hidden-files setting.
    pub hidden: usize,
    pub free: Option<(u64, u64)>,
    pub mode: ViewMode,
    pub zoom: u16,
    pub theme: ThemePreference,
    pub size_style: SizeStyle,
    /// Jobs indicator (ring + count) shown while operations run.
    pub activity: crate::ui::panels::activity::Indicator,
    /// Folder shown by the pane (git branch chip).
    pub git_dir: Option<std::path::PathBuf>,
}

/// `12345` → `12,345` (`12.345` in Turkish: the current locale's digit grouping).
pub fn group_thousands(n: usize) -> String {
    soyuz_core::locale::group_int(n as u64)
}

const MODES: [(ViewMode, &str, Action); 4] = [
    (ViewMode::Details, ph::LIST, Action::ViewDetails),
    (ViewMode::List, ph::LIST_DASHES, Action::ViewList),
    (ViewMode::Grid, ph::SQUARES_FOUR, Action::ViewGrid),
    (ViewMode::Miller, ph::COLUMNS, Action::ViewMiller),
];

pub fn show(root: &mut Ui, info: &StatusInfo, cmds: &mut Vec<Cmd>, zoom_out: &mut Option<u16>) {
    let p = palette(root.ctx());
    Panel::bottom("soyuz.statusbar")
        .exact_size(size::STATUS_BAR + 4.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 14, right: 10, top: 0, bottom: 2 }))
        .show(root, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let small = |s: &str| RichText::new(s).font(FontId::proportional(11.5)).color(p.text_secondary);
                if info.loading {
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    spinner(ui, r.center(), 5.0, p.accent);
                }
                let items = if info.filtered {
                    tr!("app-status-items-filtered", visible = info.visible, count = info.total)
                } else {
                    tr!("app-status-items", count = info.visible)
                };
                ui.label(small(&items));
                if info.hidden > 0 {
                    ui.label(
                        RichText::new(tr!("app-status-hidden", count = info.hidden))
                            .font(FontId::proportional(11.5))
                            .color(p.text_tertiary),
                    );
                }
                if info.selected > 0 {
                    ui.label(RichText::new("·").color(p.text_tertiary));
                    let s = if info.selected_bytes > 0 {
                        tr!(
                            "app-status-selected-size",
                            count = info.selected,
                            size = format_size(info.selected_bytes, info.size_style)
                        )
                    } else {
                        tr!("app-status-selected", count = info.selected)
                    };
                    ui.label(RichText::new(s).font(FontId::proportional(11.5)).color(p.accent));
                }
                crate::ui::tools::git_chip(ui, info.git_dir.as_deref(), info.pane, cmds);

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    theme_switch(ui, info.theme, cmds);
                    if let Some(r) = crate::ui::panels::activity::status_indicator(ui, &info.activity)
                        && r.clicked()
                    {
                        cmds.push(Cmd::Action(Action::ToggleActivityPanel));
                    }
                    crate::ui::tools::shelf_toggle(ui, cmds);
                    crate::ui::smart::status_chip(ui, cmds);
                    let current = MODES.iter().position(|m| m.0 == info.mode).unwrap_or(match info.mode {
                        ViewMode::Gallery => 2,
                        _ => 0,
                    });
                    let items: Vec<(&str, &str)> =
                        MODES.iter().map(|m| (m.1, crate::ui::context_menu::mode_label(m.0))).collect();
                    if let Some(i) = segmented(ui, &items, current, vec2(26.0, 18.0)) {
                        cmds.push(Cmd::PaneAction(info.pane, MODES[i].2.clone()));
                    }
                    if matches!(info.mode, ViewMode::Grid | ViewMode::Gallery) {
                        let mut z = info.zoom as f32;
                        let r = mini_slider(ui, &mut z, 16.0..=256.0, 96.0)
                            .on_hover_text(tr!("app-status-icon-size", size = info.zoom.to_string()));
                        if r.changed() || r.dragged() || r.clicked() {
                            *zoom_out = Some(z.round() as u16);
                        }
                        ui.label(icons::icon(ph::MAGNIFYING_GLASS_PLUS, 12.0).color(p.text_tertiary));
                    }
                    if let Some((total, free)) = info.free
                        && total > 0
                    {
                        let used = 1.0 - free as f32 / total as f32;
                        let (r, _) = ui.allocate_exact_size(vec2(56.0, 14.0), Sense::hover());
                        let bar = Rect::from_center_size(r.center(), vec2(56.0, 4.0));
                        usage_bar(ui, &p, bar, used);
                        ui.label(small(&tr!("app-status-free", free = format_size(free, SizeStyle::Decimal))))
                            .on_hover_text(tr!(
                                "app-status-free-of",
                                free = format_size(free, SizeStyle::Decimal),
                                total = format_size(total, SizeStyle::Decimal)
                            ));
                    }
                });
            });
        });
}

/// System / light / dark switch.
fn theme_switch(ui: &mut Ui, current: ThemePreference, cmds: &mut Vec<Cmd>) {
    let p = palette(ui.ctx());
    let items = [
        (ThemePreference::System, ph::CIRCLE_HALF, tr!("app-status-theme-system")),
        (ThemePreference::Light, ph::SUN, tr!("app-status-theme-light")),
        (ThemePreference::Dark, ph::MOON, tr!("app-status-theme-dark")),
    ];
    // Right-to-left layout: allocate in reverse so the visual order is system, light, dark.
    for (pref, glyph, tip) in items.iter().rev() {
        let (r, resp) = ui.allocate_exact_size(vec2(20.0, 18.0), Sense::click());
        let active = current == *pref;
        crate::ui::a11y::toggle(&resp, tip, active);
        if active {
            ui.painter().rect_filled(r, CornerRadius::same(4), p.bg_surface2);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(4), p.bg_surface2.gamma_multiply(0.6));
        }
        let color = if active { p.accent } else { p.text_tertiary };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *glyph, icons::font(12.0), color);
        if resp.on_hover_text(*tip).clicked() {
            cmds.push(Cmd::SetTheme(*pref));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(1234567), "1,234,567");
    }
}

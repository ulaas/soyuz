//! The system monitor (`monitor:`): a rail of pages on the left, a top bar with the
//! sampling controls, and the page itself. State and commands live in
//! `models::monitor` (shared with the terminal UI); this module only draws and asks.
//! Everything is painted from the newest frame the hub handed over: nothing here reads
//! the OS, and the window repaints when the hub wakes it or on input, never on a timer.

pub mod charts;
pub mod dialogs;
pub mod keys;
pub mod table;

mod cpu;
mod details;
mod disks;
mod gpu;
mod history;
mod lists;
mod memory;
mod network;
mod overview;
mod processes;
mod sensors;
mod snapshots;

use std::sync::Arc;

use egui::{Align2, Color32, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use soyuz_core::SizeStyle;
use soyuz_core::config::Config;
use soyuz_core::config::monitor::{Monitor, MonitorPage};
use soyuz_fs::VolumeInfo;
use soyuz_sysmon::{Frame, Metric, Pt, SystemInfo};

use super::ViewCtx;
use crate::cmd::Cmd;
use crate::models::monitor::{MonitorCmd, MonitorState, MonitorView, labels};
use crate::services::thumbs::Thumbs;
use crate::state::pane::{Pane, PaneId};
use crate::theme::font::semibold;
use crate::theme::tokens::radius;
use crate::theme::{Palette, font, icons as ticons, icons::ph, palette};
use crate::ui::empty_state::EmptyState;
use crate::ui::widgets::dropdown::{Dropdown, Item};

use charts::{Fmt, Unit};

/// Everything a page needs.
pub struct PageCtx<'a> {
    pub pane: PaneId,
    pub p: Palette,
    pub config: &'a Config,
    pub frame: Option<Arc<Frame>>,
    pub generation: u64,
    pub info: Arc<SystemInfo>,
    pub monitor: &'a mut MonitorState,
    pub cmds: &'a mut Vec<Cmd>,
    pub focused: bool,
    pub list_focus: bool,
    pub volumes: &'a [VolumeInfo],
    pub thumbs: &'a Thumbs,
}

impl PageCtx<'_> {
    pub fn cfg(&self) -> &Monitor {
        &self.config.monitor
    }

    pub fn view(&mut self) -> &mut MonitorView {
        self.monitor.view(self.pane, &self.config.monitor)
    }

    pub fn cmd(&mut self, c: MonitorCmd) {
        self.cmds.push(Cmd::Monitor(self.pane, c));
    }

    pub fn style(&self) -> SizeStyle {
        self.config.general.size_style
    }

    pub fn fmt(&self) -> Fmt {
        Fmt { style: self.config.general.size_style }
    }

    /// The live series of `metric` over the configured graph span.
    pub fn series(&mut self, metric: &Metric) -> Arc<Vec<Pt>> {
        let secs = self.config.monitor.graph_span();
        self.monitor.series(metric, secs, 0)
    }

    /// The time span of the live charts, ending at the newest frame.
    pub fn span(&self) -> (i64, i64) {
        let to = self.frame.as_ref().map_or(0, |f| f.at_ms);
        let secs = self.config.monitor.graph_span();
        (to - i64::from(secs) * 1000, to)
    }

    /// A value the OS does not offer.
    pub fn dash() -> &'static str {
        "–"
    }

    pub fn unit_temp(&self) -> Unit {
        Unit::Temp(self.config.monitor.temperature_unit)
    }
}

fn rail_wide() -> f32 {
    font::grow(168.0)
}
fn rail_narrow() -> f32 {
    font::grow(46.0)
}
fn topbar_h() -> f32 {
    font::grow(48.0)
}

/// Phosphor glyph of a page.
pub fn page_glyph(p: MonitorPage) -> &'static str {
    match p {
        MonitorPage::Overview => ph::GAUGE,
        MonitorPage::Processes => ph::LIST_BULLETS,
        MonitorPage::Cpu => ph::CPU,
        MonitorPage::Memory => ph::MEMORY,
        MonitorPage::Disks => ph::HARD_DRIVES,
        MonitorPage::Network => ph::NETWORK,
        MonitorPage::Gpu => ph::GRAPHICS_CARD,
        MonitorPage::Sensors => ph::THERMOMETER,
        MonitorPage::Connections => ph::PLUGS_CONNECTED,
        MonitorPage::Services => ph::GEAR_SIX,
        MonitorPage::Startup => ph::ROCKET_LAUNCH,
        MonitorPage::History => ph::CLOCK_COUNTER_CLOCKWISE,
        MonitorPage::Snapshots => ph::CAMERA,
    }
}

/// The rail groups: caption key and pages.
fn groups() -> [(&'static str, &'static [MonitorPage]); 4] {
    [
        ("monitor-gui-group-live", &[MonitorPage::Overview, MonitorPage::Processes]),
        (
            "monitor-gui-group-resources",
            &[
                MonitorPage::Cpu,
                MonitorPage::Memory,
                MonitorPage::Disks,
                MonitorPage::Network,
                MonitorPage::Gpu,
                MonitorPage::Sensors,
            ],
        ),
        ("monitor-gui-group-system", &[MonitorPage::Connections, MonitorPage::Services, MonitorPage::Startup]),
        ("monitor-gui-group-records", &[MonitorPage::History, MonitorPage::Snapshots]),
    ]
}

fn group_title(key: &str) -> &'static str {
    // i18n-dynamic: monitor-gui-group-
    crate::i18n::t_dyn(key)
}

pub fn show(ui: &mut Ui, rect: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    let p = palette(ui.ctx());
    let bg = ui.interact(rect, Id::unique(("soyuz.monitor.bg", pane.id.0)), Sense::click());
    if bg.clicked() || bg.secondary_clicked() {
        v.cmds.push(Cmd::FocusPane(pane.id));
    }
    let mut c = PageCtx {
        pane: pane.id,
        p: p.clone(),
        config: v.config,
        frame: v.monitor.frame.clone(),
        generation: v.monitor.generation,
        info: v.monitor.info.clone(),
        monitor: v.monitor,
        cmds: v.cmds,
        focused: v.focused,
        list_focus: v.list_focus,
        volumes: v.volumes,
        thumbs: &v.services.thumbs,
    };
    let page = c.view().page;
    let wide = rect.width() >= font::grow(820.0);
    let rail_w = if wide { rail_wide() } else { rail_narrow() };
    let rail = Rect::from_min_max(rect.min, pos2(rect.left() + rail_w, rect.bottom()));
    let top = Rect::from_min_max(pos2(rail.right(), rect.top()), pos2(rect.right(), rect.top() + topbar_h()));
    let body = Rect::from_min_max(pos2(rail.right(), top.bottom()), rect.max);
    draw_rail(ui, rail, &mut c, page, wide);
    draw_topbar(ui, top, &mut c, page);

    // Page transition: a short fade and slide when the page changes.
    let trans_id = Id::unique(("soyuz.monitor.page_anim", pane.id.0));
    let now = ui.input(|i| i.time);
    let (last_page, since): (u8, f64) = ui.ctx().data(|d| d.get_temp(trans_id)).unwrap_or((page as u8, now - 1.0));
    let since = if last_page != page as u8 { now } else { since };
    ui.ctx().data_mut(|d| d.insert_temp(trans_id, (page as u8, since)));
    let secs = crate::theme::anim(ui.ctx(), crate::theme::tokens::motion::PANEL);
    let t = if secs <= 0.0 { 1.0 } else { crate::ui::motion::ease_out(((now - since) / f64::from(secs)) as f32) };
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    let shifted = body.translate(vec2(0.0, (1.0 - t) * 10.0));
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(shifted).id_salt(("soyuz.monitor.page", pane.id.0, page as u8)));
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    if t < 1.0 {
        child.multiply_opacity(t.max(0.05));
    }
    let inner = shifted.shrink2(vec2(font::grow(14.0), 0.0));
    match page {
        MonitorPage::Overview => overview::show(&mut child, inner, &mut c),
        MonitorPage::Processes => processes::show(&mut child, shifted, &mut c),
        MonitorPage::Cpu => cpu::show(&mut child, inner, &mut c),
        MonitorPage::Memory => memory::show(&mut child, inner, &mut c),
        MonitorPage::Disks => disks::show(&mut child, inner, &mut c),
        MonitorPage::Network => network::show(&mut child, inner, &mut c),
        MonitorPage::Gpu => gpu::show(&mut child, inner, &mut c),
        MonitorPage::Sensors => sensors::show(&mut child, inner, &mut c),
        MonitorPage::Connections | MonitorPage::Services | MonitorPage::Startup => {
            lists::show(&mut child, shifted, &mut c, page)
        }
        MonitorPage::History => history::show(&mut child, inner, &mut c),
        MonitorPage::Snapshots => snapshots::show(&mut child, shifted, &mut c),
    }
    // The rail's right edge.
    ui.painter().vline(rail.right() - 0.5, rail.y_range(), Stroke::new(1.0, p.border_subtle));
}

fn draw_rail(ui: &mut Ui, rail: Rect, c: &mut PageCtx<'_>, page: MonitorPage, wide: bool) {
    let p = c.p.clone();
    let painter = ui.painter_at(rail);
    painter.rect_filled(rail, egui::CornerRadius::ZERO, p.bg_surface1);
    let row_h = font::grow(30.0);
    let mut y = rail.top() + 10.0;
    let a11y = crate::ui::a11y::active(ui.ctx());
    for (gi, (caption, pages)) in groups().into_iter().enumerate() {
        if wide {
            y += if gi == 0 { 2.0 } else { 12.0 };
            painter.text(
                pos2(rail.left() + 14.0, y + 7.0),
                Align2::LEFT_CENTER,
                crate::ui::widgets::caption_upper(group_title(caption)),
                font::ui(10.0),
                p.text_tertiary,
            );
            y += 20.0;
        } else if gi > 0 {
            y += 6.0;
            painter.hline(
                (rail.left() + 12.0)..=(rail.right() - 12.0),
                y.round() + 0.5,
                Stroke::new(1.0, p.border_subtle),
            );
            y += 7.0;
        }
        for pg in pages.iter().copied() {
            let r = if wide {
                Rect::from_min_size(pos2(rail.left() + 8.0, y), vec2(rail.width() - 16.0, row_h))
            } else {
                Rect::from_center_size(pos2(rail.center().x, y + row_h / 2.0), vec2(row_h + 2.0, row_h))
            };
            let resp = ui.interact(r, Id::unique(("soyuz.monitor.rail", c.pane.0, pg as u8)), Sense::click());
            let active = pg == page;
            let title = labels::page_title(pg);
            if a11y {
                crate::ui::a11y::tab(&resp, title, active);
            }
            let hover =
                ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), crate::theme::anim(ui.ctx(), 0.10));
            let cr = crate::theme::tokens::radius::same(radius::ROW);
            if active {
                painter.rect_filled(r, cr, p.accent_subtle);
            } else if hover > 0.0 {
                painter.rect_filled(r, cr, p.bg_surface2.gamma_multiply(hover));
            }
            if resp.has_focus() {
                crate::ui::widgets::focus_ring(ui, r);
            }
            let icon_font = if active { ticons::font_fill(15.0) } else { ticons::font(15.0) };
            let icon_color = if active { p.accent } else { p.text_secondary };
            if wide {
                painter.text(
                    pos2(r.left() + 10.0, r.center().y),
                    Align2::LEFT_CENTER,
                    page_glyph(pg),
                    icon_font,
                    icon_color,
                );
                let color = if active { p.text_primary } else { p.text_primary.gamma_multiply(0.88) };
                let g = crate::ui::widgets::trunc_galley(&painter, title, font::ui(12.5), color, r.width() - 40.0);
                painter.galley(pos2(r.left() + 34.0, r.center().y - g.size().y / 2.0), g, color);
            } else {
                painter.text(r.center(), Align2::CENTER_CENTER, page_glyph(pg), icon_font, icon_color);
            }
            let resp = if wide { resp } else { resp.on_hover_text(title) };
            if resp.clicked() {
                c.cmds.push(Cmd::FocusPane(c.pane));
                c.cmd(MonitorCmd::SetPage(pg));
            }
            y += row_h + 2.0;
        }
    }
}

fn draw_topbar(ui: &mut Ui, top: Rect, c: &mut PageCtx<'_>, page: MonitorPage) {
    let p = c.p.clone();
    let painter = ui.painter_at(top);
    let x = top.left() + font::grow(14.0);
    painter.text(
        pos2(x, top.top() + 17.0),
        Align2::LEFT_CENTER,
        labels::page_title(page),
        semibold(15.0),
        p.text_primary,
    );
    let paused = c.monitor.paused();
    let sub = match (&c.frame, paused) {
        (_, true) => tr!("monitor-gui-paused").to_string(),
        (None, _) => tr!("monitor-no-data").to_string(),
        (Some(f), _) => {
            let interval = interval_label(c.cfg().interval_ms);
            tr!(
                "monitor-gui-status",
                interval = interval.as_str(),
                processes = labels::count(u64::from(f.counts.processes)).as_str(),
                threads = labels::count(f.counts.threads).as_str()
            )
        }
    };
    painter.text(
        pos2(x, top.top() + 34.0),
        Align2::LEFT_CENTER,
        sub,
        font::ui(11.0),
        if paused { p.warning } else { p.text_tertiary },
    );
    let controls = Rect::from_min_max(
        pos2(top.left() + font::grow(230.0), top.top() + 9.0),
        pos2(top.right() - 12.0, top.bottom() - 9.0),
    );
    if controls.width() < 100.0 {
        return;
    }
    let mut bar =
        ui.new_child(UiBuilder::new().max_rect(controls).layout(egui::Layout::right_to_left(egui::Align::Center)));
    bar.spacing_mut().item_spacing.x = 6.0;
    let snap = crate::ui::widgets::rail_button(
        &mut bar,
        ph::CAMERA,
        tr!("monitor-gui-take-snapshot"),
        c.frame.is_some(),
        false,
    );
    if snap.clicked() {
        c.cmd(MonitorCmd::TakeSnapshot(String::new()));
    }
    let glyph = if paused { ph::PLAY } else { ph::PAUSE };
    let tip = if paused { tr!("monitor-gui-resume") } else { tr!("monitor-gui-pause") };
    if crate::ui::widgets::rail_button(&mut bar, glyph, tip, true, paused).clicked() {
        c.cmd(MonitorCmd::TogglePause);
    }
    if controls.width() > 260.0 {
        let mut ms = c.cfg().interval_ms;
        let items: Vec<Item<u32>> =
            [250u32, 500, 1000, 2000, 5000, 10_000].into_iter().map(|v| Item::new(v, interval_label(v))).collect();
        let out = Dropdown::new(format!("monitor.interval.{}", c.pane.0))
            .width(font::grow(88.0))
            .name(tr!("monitor-gui-interval"))
            .show(&mut bar, &mut ms, &items);
        if out.changed {
            c.cmd(MonitorCmd::SetInterval(ms));
        }
        bar.label(egui::RichText::new(tr!("monitor-gui-every")).font(font::ui(11.5)).color(p.text_tertiary));
    }
    let _ = painter;
}

/// `1 s`, `250 ms`.
pub fn interval_label(ms: u32) -> String {
    if ms >= 1000 && ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else if ms >= 1000 {
        format!("{:.1} s", ms as f32 / 1000.0)
    } else {
        format!("{ms} ms")
    }
}

// ---------------------------------------------------------------------------------------
// Shared drawing helpers for the pages.

/// Paint a card surface; returns its response (click-able when `click`).
pub fn card(ui: &mut Ui, p: &Palette, rect: Rect, id: Id, click: bool) -> Response {
    let resp = ui.interact(rect, id, if click { Sense::click() } else { Sense::hover() });
    let hover = click && resp.hovered();
    let cr = crate::theme::tokens::radius::same(radius::CARD);
    let fill = if hover { p.bg_surface2 } else { p.bg_window };
    ui.painter().rect_filled(rect, cr, fill);
    ui.painter().rect_stroke(
        rect,
        cr,
        Stroke::new(1.0, if hover { p.border_strong } else { p.border_subtle }),
        StrokeKind::Inside,
    );
    if resp.has_focus() {
        crate::ui::widgets::focus_ring(ui, rect);
    }
    resp
}

/// A card's title row: icon in a tinted square, then the title; returns the y below it.
pub fn card_title(painter: &egui::Painter, p: &Palette, rect: Rect, glyph: &str, color: Color32, title: &str) -> f32 {
    let icon_r = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(22.0, 22.0));
    painter.rect_filled(icon_r, crate::theme::tokens::radius::same(6), color.gamma_multiply(0.14));
    painter.text(icon_r.center(), Align2::CENTER_CENTER, glyph, ticons::font_fill(13.0), color);
    let g = crate::ui::widgets::trunc_galley(painter, title, semibold(12.5), p.text_primary, rect.width() - 56.0);
    painter.galley(pos2(icon_r.right() + 8.0, icon_r.center().y - g.size().y / 2.0), g, p.text_primary);
    icon_r.bottom() + 10.0
}

/// A small uppercase caption.
pub fn caption(painter: &egui::Painter, p: &Palette, pos: egui::Pos2, text: &str) {
    painter.text(pos, Align2::LEFT_CENTER, crate::ui::widgets::caption_upper(text), font::ui(10.0), p.text_tertiary);
}

/// A `label  value` line; returns the next y.
pub fn fact(painter: &egui::Painter, p: &Palette, x: f32, y: f32, w: f32, label: &str, value: &str) -> f32 {
    let lf = font::ui(11.5);
    let vf = font::numbers(11.5);
    let lg = crate::ui::widgets::trunc_galley(painter, label, lf, p.text_tertiary, w * 0.45);
    painter.galley(pos2(x, y), lg, p.text_tertiary);
    let vg = crate::ui::widgets::trunc_galley(painter, value, vf, p.text_primary, w * 0.55 - 6.0);
    painter.galley(pos2(x + w - vg.size().x, y), vg, p.text_primary);
    y + font::grow(19.0)
}

/// Columns for cards of at least `min_w` across `width` with `gap`: `(count, card width)`.
pub fn grid_cols(width: f32, min_w: f32, gap: f32) -> (usize, f32) {
    let n = ((width + gap) / (min_w + gap)).floor().max(1.0) as usize;
    (n, ((width - gap * (n as f32 - 1.0)) / n as f32).floor())
}

/// A calm message in the middle of `rect`.
pub fn empty(ui: &mut Ui, rect: Rect, glyph: &str, title: &str, subtitle: &str) {
    let p = palette(ui.ctx());
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    EmptyState { glyph, glyph_color: Some(p.text_secondary), title, subtitle, hints: &[] }.show(&mut child);
}

/// A boolean pill ("Virtual", "SSD").
pub fn badge(painter: &egui::Painter, _p: &Palette, pos: egui::Pos2, text: &str, color: Color32) -> f32 {
    let g = painter.layout_no_wrap(text.to_string(), font::ui(10.0), color);
    let r = Rect::from_min_size(pos, vec2(g.size().x + 12.0, 16.0));
    painter.rect_filled(r, crate::theme::tokens::radius::same(radius::PILL), color.gamma_multiply(0.14));
    painter.galley(pos2(r.left() + 6.0, r.center().y - g.size().y / 2.0), g, color);
    r.width()
}

/// A context menu on `resp` that, while open, owns the keyboard (Esc and the arrows go to it,
/// not to the page or the file actions).
pub fn context_menu<R>(resp: &Response, content: impl FnOnce(&mut Ui) -> R) -> Option<egui::InnerResponse<R>> {
    let out = crate::ui::native::popup::context_menu(resp, content);
    if out.is_some() {
        crate::ui::context_menu::mark_open(&resp.ctx);
    }
    out
}

/// Icon-only buttons in a right-to-left row at `rect`.
pub fn tool_row(ui: &mut Ui, rect: Rect) -> Ui {
    let mut bar =
        ui.new_child(UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
    bar.spacing_mut().item_spacing.x = 6.0;
    bar
}

/// A text filter box drawn in `rect`; returns the new text when it changed.
pub fn filter_box(ui: &mut Ui, rect: Rect, id: Id, text: &str, hint: &str, focus: bool) -> (Option<String>, Response) {
    let p = palette(ui.ctx());
    let cr = crate::theme::tokens::radius::same(radius::ROW);
    let mut edit = text.to_string();
    let focused = ui.memory(|m| m.has_focus(id)) || focus;
    let fill = if focused {
        p.rail(crate::theme::tokens::Rail::FieldFocus)
    } else {
        p.rail(crate::theme::tokens::Rail::Field)
    };
    // The frame goes under the text, so it is painted before the field.
    let painter = ui.painter();
    painter.rect_filled(rect, cr, fill);
    painter.rect_stroke(
        rect,
        cr,
        Stroke::new(1.0, if focused { p.accent.gamma_multiply(0.7) } else { p.border_subtle }),
        StrokeKind::Inside,
    );
    painter.text(
        pos2(rect.left() + 13.0, rect.center().y),
        Align2::CENTER_CENTER,
        ph::MAGNIFYING_GLASS,
        ticons::font(12.0),
        p.text_tertiary,
    );
    let field = Rect::from_min_max(pos2(rect.left() + 26.0, rect.top()), pos2(rect.right() - 6.0, rect.bottom()));
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = child.add(
        egui::TextEdit::singleline(&mut edit)
            .id(id)
            .frame(egui::Frame::NONE)
            .hint_text(hint)
            .desired_width(field.width())
            .font(font::ui(12.5))
            // With no frame egui ignores `margin`: the field spans the box and centers its row.
            .align(Align2::LEFT_CENTER)
            .min_size(vec2(0.0, field.height())),
    );
    crate::ui::a11y::name(&resp, hint);
    if focus {
        resp.request_focus();
    }
    ((edit != text).then_some(edit), resp)
}

/// A pill-shaped segmented control with text labels drawn in `rect`; returns the clicked index.
pub fn segmented_text(ui: &mut Ui, rect: Rect, id: Id, items: &[&str], selected: usize) -> Option<usize> {
    let p = palette(ui.ctx());
    let f = font::ui(11.5);
    let widths: Vec<f32> = items
        .iter()
        .map(|l| ui.painter().layout_no_wrap((*l).to_string(), f.clone(), p.text_primary).size().x + 18.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let r = Rect::from_min_size(rect.min, vec2(total.min(rect.width()), rect.height()));
    let cr = crate::theme::tokens::radius::same(radius::ROW);
    ui.painter().rect_filled(r, cr, p.bg_surface2);
    let mut clicked = None;
    let mut x = r.left() + 2.0;
    let sel_x: f32 = widths[..selected.min(widths.len())].iter().sum();
    let ax = ui.ctx().animate_value_with_time(id.with("x"), sel_x, crate::theme::anim(ui.ctx(), 0.14));
    let aw = ui.ctx().animate_value_with_time(
        id.with("w"),
        widths.get(selected).copied().unwrap_or(20.0),
        crate::theme::anim(ui.ctx(), 0.14),
    );
    let thumb = Rect::from_min_size(pos2(r.left() + 2.0 + ax, r.top() + 2.0), vec2(aw, r.height() - 4.0));
    ui.painter().rect_filled(thumb, crate::theme::tokens::radius::same(radius::CONTROL + 1), p.bg_surface1);
    ui.painter().rect_stroke(
        thumb,
        crate::theme::tokens::radius::same(radius::CONTROL + 1),
        Stroke::new(1.0, p.border_subtle),
        StrokeKind::Inside,
    );
    for (i, (label, w)) in items.iter().zip(&widths).enumerate() {
        let cell = Rect::from_min_size(pos2(x, r.top() + 2.0), vec2(*w, r.height() - 4.0));
        x += w;
        let resp = ui.interact(cell, id.with(i), Sense::click());
        crate::ui::a11y::toggle(&resp, label, i == selected);
        if resp.has_focus() {
            crate::ui::widgets::focus_ring(ui, cell);
        }
        let color = if i == selected {
            p.text_primary
        } else if resp.hovered() {
            p.text_secondary
        } else {
            p.text_tertiary
        };
        ui.painter().text(cell.center(), Align2::CENTER_CENTER, *label, f.clone(), color);
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// Width [`segmented_text`] takes for `items`.
pub fn segmented_width(ui: &Ui, items: &[&str]) -> f32 {
    let f = font::ui(11.5);
    items
        .iter()
        .map(|l| ui.painter().layout_no_wrap((*l).to_string(), f.clone(), Color32::PLACEHOLDER).size().x + 18.0)
        .sum::<f32>()
        + 4.0
}

/// A big figure with a small unit or sub-line under a title: the hero stat of a card.
pub fn big_number(painter: &egui::Painter, p: &Palette, pos: egui::Pos2, value: &str, sub: &str, max_w: f32) {
    let g = crate::ui::widgets::trunc_galley(painter, value, font::numbers(24.0), p.text_primary, max_w);
    painter.galley(pos, g, p.text_primary);
    if !sub.is_empty() {
        let s = crate::ui::widgets::trunc_galley(painter, sub, font::ui(11.5), p.text_secondary, max_w);
        painter.galley(pos2(pos.x, pos.y + font::grow(30.0)), s, p.text_secondary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_labels() {
        assert_eq!(interval_label(250), "250 ms");
        assert_eq!(interval_label(1000), "1 s");
        assert_eq!(interval_label(1500), "1.5 s");
        assert_eq!(interval_label(10_000), "10 s");
    }

    #[test]
    fn grid_columns() {
        assert_eq!(grid_cols(1000.0, 240.0, 12.0).0, 4);
        assert_eq!(grid_cols(300.0, 240.0, 12.0).0, 1);
        let (n, w) = grid_cols(500.0, 240.0, 12.0);
        assert_eq!(n, 2);
        assert_eq!(w, 244.0);
    }

    #[test]
    fn every_page_is_in_a_group_once() {
        let mut all: Vec<MonitorPage> = groups().iter().flat_map(|(_, p)| p.iter().copied()).collect();
        all.sort_by_key(|p| *p as u8);
        let mut expected = MonitorPage::ALL.to_vec();
        expected.sort_by_key(|p| *p as u8);
        assert_eq!(all, expected);
    }
}

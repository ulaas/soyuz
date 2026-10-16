//! The drop shelf: a collapsible right-side panel that collects files from
//! any folder. Drag items onto it (or use "Add to shelf"), keep browsing, then drag them
//! out onto a folder / tab / sidebar row, or copy / move everything into the focused
//! pane's folder. The shelf only holds references and persists across navigation.
//! While a drag is in progress and the shelf is closed, a slim drop strip appears at the
//! right edge.

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{
    Align2, Area, CornerRadius, DragAndDrop, FontId, Frame, Id, Margin, Order, Panel, Rect, ScrollArea, Sense, Stroke,
    StrokeKind, Ui, pos2, vec2,
};
use soyuz_core::config::DragDefault;
use soyuz_core::{EntryInfo, Location, SizeStyle, format_size};

use super::button;
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::dnd::{DragPayload, DropEffect, payload};
use crate::ui::views::icons as file_icons;
use crate::ui::widgets::{icon_button, menu_item, trunc_galley};

pub use crate::models::tools::{ShelfData, ShelfItem};

/// The shelf panel's view state (the items live in [`ShelfData`]).
pub struct Shelf {
    selected: BTreeSet<usize>,
    /// Panel rect of the last frame (OS drops are matched against it).
    last_rect: Option<Rect>,
    /// Items are re-checked periodically while the shelf is open (moved / deleted).
    last_check: std::time::Instant,
    width: f32,
}

impl Default for Shelf {
    fn default() -> Self {
        Shelf { selected: BTreeSet::new(), last_rect: None, width: 250.0, last_check: std::time::Instant::now() }
    }
}

impl Shelf {
    /// OS file drops over the shelf are collected instead of being copied into a pane.
    pub fn external_drops(&mut self, ctx: &egui::Context, d: &mut ShelfData) {
        let Some(rect) = self.last_rect.filter(|_| d.open) else { return };
        let over = ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| rect.contains(p));
        if !over {
            return;
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect()
        });
        if !dropped.is_empty() {
            ctx.input_mut(|i| i.raw.dropped_files.clear());
            d.add(dropped);
        }
    }

    pub fn show(&mut self, root: &mut Ui, app: &mut SoyuzApp, d: &mut ShelfData) {
        d.poll();
        let focused = root.ctx().input(|i| i.focused);
        if d.open && focused && !d.items.is_empty() && self.last_check.elapsed() > std::time::Duration::from_secs(2) {
            self.last_check = std::time::Instant::now();
            d.stat(d.items.iter().map(|i| i.path.clone()).collect());
            root.ctx().request_repaint_after(std::time::Duration::from_secs(2));
        }
        if !d.open {
            self.last_rect = None;
            return;
        }
        let ctx = root.ctx().clone();
        let p = palette(&ctx);
        let dest = match &app.core.ws.pane().location {
            Location::Dir(d) => Some(d.clone()),
            _ => None,
        };
        let drag_default = app.core.config.behaviour.drag_default;
        let resp = Panel::right("soyuz.shelf")
            .resizable(true)
            .default_size(self.width)
            .size_range(200.0..=420.0)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 0, right: 8, top: 2, bottom: 2 }))
            .show(root, |ui| {
                let card = ui.available_rect_before_wrap();
                ui.painter().rect_filled(card, CornerRadius::same(radius::MODAL), p.bg_surface1);
                ui.painter().rect_stroke(
                    card,
                    CornerRadius::same(radius::MODAL),
                    Stroke::new(1.0, p.border_subtle),
                    StrokeKind::Inside,
                );
                ui.allocate_rect(card, Sense::hover());
                self.contents(ui, card, app, d, dest.clone(), drag_default);
                card
            });
        self.width = resp.response.rect.width();
        crate::ui::a11y::panel_splitter(root, "soyuz.shelf", tr!("app-a11y-resize-shelf"));
        self.last_rect = Some(resp.inner);
        let _ = ctx;
    }

    fn contents(
        &mut self,
        ui: &mut Ui,
        card: Rect,
        app: &mut SoyuzApp,
        d: &mut ShelfData,
        dest: Option<PathBuf>,
        drag_default: DragDefault,
    ) {
        let p = palette(ui.ctx());
        // Header.
        let hr = Rect::from_min_size(card.min, vec2(card.width(), 44.0));
        ui.painter().text(
            pos2(hr.left() + 18.0, hr.center().y),
            Align2::CENTER_CENTER,
            ph::TRAY,
            icons::font(16.0),
            p.accent,
        );
        ui.painter().text(
            pos2(hr.left() + 34.0, hr.center().y),
            Align2::LEFT_CENTER,
            tr!("tools-shelf-title"),
            crate::ui::views::flow::semibold(13.5),
            p.text_primary,
        );
        if !d.items.is_empty() {
            ui.painter().text(
                pos2(hr.left() + 80.0, hr.center().y),
                Align2::LEFT_CENTER,
                d.items.len().to_string(),
                FontId::proportional(11.5),
                p.text_tertiary,
            );
        }
        let close = Rect::from_center_size(pos2(hr.right() - 20.0, hr.center().y), vec2(30.0, 30.0));
        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(close));
        if icon_button(&mut cu, ph::X, tr!("tools-shelf-hide"), true).clicked() {
            d.open = false;
        }
        let clear = Rect::from_center_size(pos2(hr.right() - 50.0, hr.center().y), vec2(30.0, 30.0));
        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(clear));
        if icon_button(&mut cu, ph::BROOM, tr!("tools-shelf-clear"), !d.items.is_empty()).clicked() {
            d.items.clear();
            self.selected.clear();
        }
        ui.painter().line_segment(
            [pos2(card.left() + 8.0, hr.bottom()), pos2(card.right() - 8.0, hr.bottom())],
            Stroke::new(1.0, p.border_subtle),
        );

        let footer_h = if d.items.is_empty() { 0.0 } else { 78.0 };
        let list = Rect::from_min_max(
            pos2(card.left() + 6.0, hr.bottom() + 6.0),
            pos2(card.right() - 6.0, card.bottom() - footer_h - 4.0),
        );

        // Drop target over the whole card.
        let ctx = ui.ctx().clone();
        if let Some(pl) = payload(&ctx)
            && pl.source_pane.is_some()
            && let Some(pos) = ctx.input(|i| i.pointer.latest_pos())
            && card.contains(pos)
        {
            let painter = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::unique("soyuz.shelf.hl")));
            painter.rect_filled(card, CornerRadius::same(radius::MODAL), p.accent.gamma_multiply(0.08));
            painter.rect_stroke(
                card,
                CornerRadius::same(radius::MODAL),
                Stroke::new(2.0, p.accent),
                StrokeKind::Inside,
            );
            if ctx.input(|i| i.pointer.any_released())
                && let Some(pl) = DragAndDrop::take_payload::<DragPayload>(&ctx)
            {
                d.add(pl.paths.clone());
            }
        }

        if d.items.is_empty() {
            let c = list.center();
            ui.painter().text(
                c - vec2(0.0, 30.0),
                Align2::CENTER_CENTER,
                ph::TRAY_ARROW_DOWN,
                icons::font(30.0),
                p.text_tertiary,
            );
            ui.painter().text(
                c + vec2(0.0, 4.0),
                Align2::CENTER_CENTER,
                tr!("tools-shelf-empty-title"),
                crate::ui::views::flow::semibold(13.0),
                p.text_primary,
            );
            let g = ui.painter().layout(
                tr!("tools-shelf-empty-body").into(),
                FontId::proportional(11.5),
                p.text_secondary,
                list.width() - 40.0,
            );
            ui.painter().galley(pos2(c.x - g.size().x / 2.0, c.y + 18.0), g, p.text_secondary);
            return;
        }

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
        let row_h = 38.0;
        let mut remove: Option<BTreeSet<usize>> = None;
        let mut reveal = None;
        let sel_snapshot = self.selected.clone();
        ScrollArea::vertical().id_salt("shelf.list").auto_shrink([false, false]).show_rows(
            &mut child,
            row_h,
            d.items.len(),
            |ui, range| {
                for i in range {
                    let item = &d.items[i];
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click_and_drag());
                    let selected = sel_snapshot.contains(&i);
                    let painter = ui.painter();
                    if selected {
                        painter.rect_filled(r, CornerRadius::same(6), p.accent_subtle);
                    } else if resp.hovered() {
                        painter.rect_filled(r, CornerRadius::same(6), p.bg_surface2);
                    }
                    let e = item
                        .entry
                        .clone()
                        .unwrap_or_else(|| EntryInfo::minimal(item.path.clone(), soyuz_core::EntryKind::File));
                    let a = if item.missing { 0.45 } else { 1.0 };
                    painter.text(
                        pos2(r.left() + 16.0, r.center().y),
                        Align2::CENTER_CENTER,
                        file_icons::glyph(&e),
                        if e.is_dir_like() { icons::font_fill(18.0) } else { icons::font(18.0) },
                        file_icons::color(&e, &p).gamma_multiply(a),
                    );
                    let name = soyuz_core::entry::display_name(&item.path);
                    let g = trunc_galley(
                        painter,
                        &name,
                        FontId::proportional(12.5),
                        p.text_primary.gamma_multiply(a),
                        r.width() - 44.0,
                    );
                    painter.galley(pos2(r.left() + 32.0, r.top() + 4.0), g, p.text_primary);
                    let sub = if item.missing {
                        tr!("tools-shelf-missing").to_string()
                    } else {
                        let folder = item.path.parent().map(soyuz_core::entry::display_name).unwrap_or_default();
                        match &item.entry {
                            Some(e) if !e.is_dir_like() => {
                                tr!(
                                    "tools-shelf-item-sub",
                                    size = format_size(e.size, SizeStyle::Binary),
                                    folder = &folder
                                )
                            }
                            _ => folder,
                        }
                    };
                    let g = trunc_galley(
                        painter,
                        &sub,
                        FontId::proportional(11.0),
                        if item.missing { p.danger } else { p.text_tertiary },
                        r.width() - 44.0,
                    );
                    painter.galley(pos2(r.left() + 32.0, r.bottom() - 4.0 - g.size().y), g, p.text_tertiary);
                    if resp.clicked() {
                        let ctrl = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
                        if ctrl {
                            if !self.selected.remove(&i) {
                                self.selected.insert(i);
                            }
                        } else {
                            self.selected = BTreeSet::from([i]);
                        }
                    }
                    if resp.double_clicked() {
                        reveal = Some(item.path.clone());
                    }
                    if resp.drag_started() {
                        let paths: Vec<PathBuf> = if selected && self.selected.len() > 1 {
                            self.selected.iter().filter_map(|&k| d.items.get(k)).map(|it| it.path.clone()).collect()
                        } else {
                            vec![item.path.clone()]
                        };
                        DragAndDrop::set_payload(
                            ui.ctx(),
                            DragPayload { paths, source_pane: None, default: drag_default },
                        );
                    }
                    let path = item.path.clone();
                    crate::ui::native::popup::context_menu(
                        &resp.on_hover_text(tr!("tools-shelf-item-tip", path = path.display().to_string())),
                        |ui| {
                            ui.set_width(200.0);
                            if menu_item(ui, Some(ph::FOLDER_OPEN), tr!("tools-shelf-reveal"), None, true).clicked() {
                                reveal = Some(path.clone());
                                ui.close();
                            }
                            if menu_item(ui, Some(ph::X), tr!("tools-shelf-remove"), None, true).clicked() {
                                let mut s = BTreeSet::new();
                                s.insert(i);
                                remove = Some(s);
                                ui.close();
                            }
                        },
                    );
                }
            },
        );
        if let Some(r) = remove {
            d.remove(&r);
            self.selected.clear();
        }
        if let Some(path) = reveal {
            app.core.cmds.push(Cmd::RevealPath(path));
        }
        // Footer: copy / move into the focused pane's folder.
        let fr = Rect::from_min_max(
            pos2(card.left() + 10.0, card.bottom() - footer_h),
            pos2(card.right() - 10.0, card.bottom() - 8.0),
        );
        ui.painter()
            .line_segment([pos2(fr.left(), fr.top()), pos2(fr.right(), fr.top())], Stroke::new(1.0, p.border_subtle));
        let mut fu = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(fr.shrink2(vec2(0.0, 6.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let n = if self.selected.is_empty() { d.items.len() } else { self.selected.len() };
        let to = dest.as_deref().map(soyuz_core::entry::display_name).unwrap_or_else(|| "—".into());
        fu.label(
            egui::RichText::new(tr!(
                "tools-shelf-footer",
                which = if self.selected.is_empty() { "all" } else { "selected" },
                count = n,
                dest = to
            ))
            .font(FontId::proportional(11.5))
            .color(p.text_tertiary),
        );
        fu.horizontal(|ui| {
            let ok = dest.is_some();
            if button(ui, Some(ph::COPY), tr!("tools-shelf-copy-here"), 1, ok).clicked()
                && let Some(to) = &dest
            {
                let paths = d.targets(&self.selected);
                app.core.cmds.push(Cmd::FileOp(FileOpCmd::Transfer {
                    paths,
                    dest: to.clone(),
                    effect: DropEffect::Copy,
                }));
            }
            if button(ui, Some(ph::ARROW_BEND_DOWN_RIGHT), tr!("tools-shelf-move-here"), 0, ok).clicked()
                && let Some(to) = &dest
            {
                let paths = d.targets(&self.selected);
                app.core.cmds.push(Cmd::FileOp(FileOpCmd::Transfer {
                    paths: paths.clone(),
                    dest: to.clone(),
                    effect: DropEffect::Move,
                }));
                // Moved items are delivered: take them off the shelf.
                d.items.retain(|i| !paths.contains(&i.path));
                self.selected.clear();
            }
        });
    }

    /// A slim drop strip at the right edge while dragging files with the shelf closed.
    pub fn drag_strip(&mut self, ctx: &egui::Context, d: &mut ShelfData) {
        if d.open {
            return;
        }
        let Some(pl) = payload(ctx) else { return };
        if pl.source_pane.is_none() || !ctx.input(|i| i.pointer.primary_down() || i.pointer.any_released()) {
            return;
        }
        let p = palette(ctx);
        let screen = ctx.content_rect();
        let pos = ctx.input(|i| i.pointer.latest_pos());
        let strip = Rect::from_min_max(
            pos2(screen.right() - 46.0, screen.top() + 120.0),
            pos2(screen.right() - 6.0, screen.bottom() - 120.0),
        );
        let hot = pos.is_some_and(|q| strip.expand(8.0).contains(q));
        Area::new(Id::unique("soyuz.shelf.strip"))
            .order(Order::Foreground)
            .fixed_pos(strip.min)
            .interactable(false)
            .show(ctx, |ui| {
                let painter = ui.painter();
                painter.rect_filled(
                    strip,
                    CornerRadius::same(10),
                    if hot { p.accent.gamma_multiply(0.25) } else { p.bg_surface3.gamma_multiply(0.92) },
                );
                painter.rect_stroke(
                    strip,
                    CornerRadius::same(10),
                    Stroke::new(if hot { 2.0 } else { 1.0 }, if hot { p.accent } else { p.border_strong }),
                    StrokeKind::Inside,
                );
                painter.text(
                    strip.center() - vec2(0.0, 14.0),
                    Align2::CENTER_CENTER,
                    ph::TRAY_ARROW_DOWN,
                    icons::font(18.0),
                    if hot { p.accent } else { p.text_secondary },
                );
                painter.text(
                    strip.center() + vec2(0.0, 10.0),
                    Align2::CENTER_CENTER,
                    tr!("tools-shelf-title"),
                    FontId::proportional(10.5),
                    p.text_secondary,
                );
            });
        if hot
            && ctx.input(|i| i.pointer.any_released())
            && let Some(pl) = DragAndDrop::take_payload::<DragPayload>(ctx)
        {
            d.open = true;
            d.add(pl.paths.clone());
        }
    }
}

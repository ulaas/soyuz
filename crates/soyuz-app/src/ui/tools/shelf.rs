//! The drop shelf (PLAN §8 #16): a collapsible right-side panel that collects files from
//! any folder. Drag items onto it (or use "Add to shelf"), keep browsing, then drag them
//! out onto a folder / tab / sidebar row, or copy / move everything into the focused
//! pane's folder. The shelf only holds references and persists across navigation.
//! While a drag is in progress and the shelf is closed, a slim drop strip appears at the
//! right edge.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crossbeam_channel::{Receiver, Sender, unbounded};
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
use crate::ops::fileops::dnd::{DragPayload, DropEffect, payload};
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};
use crate::ui::views::icons as file_icons;
use crate::ui::widgets::{icon_button, menu_item, trunc_galley};

pub struct ShelfItem {
    pub path: PathBuf,
    pub entry: Option<EntryInfo>,
    pub missing: bool,
}

pub struct Shelf {
    pub open: bool,
    pub items: Vec<ShelfItem>,
    selected: BTreeSet<usize>,
    stat_tx: Sender<(PathBuf, Option<EntryInfo>)>,
    stat_rx: Receiver<(PathBuf, Option<EntryInfo>)>,
    /// Panel rect of the last frame (OS drops are matched against it).
    last_rect: Option<Rect>,
    /// Items are re-checked periodically while the shelf is open (moved / deleted).
    last_check: std::time::Instant,
    width: f32,
}

impl Default for Shelf {
    fn default() -> Self {
        let (stat_tx, stat_rx) = unbounded();
        Shelf {
            open: false,
            items: Vec::new(),
            selected: BTreeSet::new(),
            stat_tx,
            stat_rx,
            last_rect: None,
            width: 250.0,
            last_check: std::time::Instant::now(),
        }
    }
}

impl Shelf {
    /// Add paths (duplicates ignored) and stat them in the background.
    pub fn add(&mut self, paths: Vec<PathBuf>) {
        let fresh: Vec<PathBuf> =
            paths.into_iter().filter(|p| !self.items.iter().any(|i| &i.path == p)).collect::<Vec<_>>();
        if fresh.is_empty() {
            return;
        }
        for p in &fresh {
            self.items.push(ShelfItem { path: p.clone(), entry: None, missing: false });
        }
        self.stat(fresh);
    }

    /// Stat `paths` on a worker; results arrive through `poll`.
    fn stat(&self, fresh: Vec<PathBuf>) {
        let tx = self.stat_tx.clone();
        let wake = super::wake();
        let _ = std::thread::Builder::new().name("soyuz-shelf-stat".into()).spawn(move || {
            for p in fresh {
                let e = soyuz_fs::stat(&p, false).ok();
                if tx.send((p, e)).is_err() {
                    break;
                }
            }
            wake();
        });
    }

    fn poll(&mut self) {
        for (p, e) in self.stat_rx.try_iter() {
            if let Some(item) = self.items.iter_mut().find(|i| i.path == p) {
                item.missing = e.is_none();
                item.entry = e;
            }
        }
    }

    fn remove(&mut self, idx: &BTreeSet<usize>) {
        let mut i = 0;
        self.items.retain(|_| {
            let keep = !idx.contains(&i);
            i += 1;
            keep
        });
        self.selected.clear();
    }

    /// Paths an action applies to: the selection, else everything.
    fn targets(&self) -> Vec<PathBuf> {
        if self.selected.is_empty() {
            self.items.iter().filter(|i| !i.missing).map(|i| i.path.clone()).collect()
        } else {
            self.selected.iter().filter_map(|&i| self.items.get(i)).map(|i| i.path.clone()).collect()
        }
    }

    /// OS file drops over the shelf are collected instead of being copied into a pane.
    pub fn external_drops(&mut self, ctx: &egui::Context) {
        let Some(rect) = self.last_rect.filter(|_| self.open) else { return };
        let over = ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| rect.contains(p));
        if !over {
            return;
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect()
        });
        if !dropped.is_empty() {
            ctx.input_mut(|i| i.raw.dropped_files.clear());
            self.add(dropped);
        }
    }

    pub fn show(&mut self, root: &mut Ui, app: &mut SoyuzApp) {
        self.poll();
        let focused = root.ctx().input(|i| i.focused);
        if self.open
            && focused
            && !self.items.is_empty()
            && self.last_check.elapsed() > std::time::Duration::from_secs(2)
        {
            self.last_check = std::time::Instant::now();
            self.stat(self.items.iter().map(|i| i.path.clone()).collect());
            root.ctx().request_repaint_after(std::time::Duration::from_secs(2));
        }
        if !self.open {
            self.last_rect = None;
            return;
        }
        let ctx = root.ctx().clone();
        let p = palette(&ctx);
        let dest = match &app.ws.pane().location {
            Location::Dir(d) => Some(d.clone()),
            _ => None,
        };
        let drag_default = app.config.behaviour.drag_default;
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
                self.contents(ui, card, app, dest.clone(), drag_default);
                card
            });
        self.width = resp.response.rect.width();
        self.last_rect = Some(resp.inner);
        let _ = ctx;
    }

    fn contents(
        &mut self,
        ui: &mut Ui,
        card: Rect,
        app: &mut SoyuzApp,
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
            "Shelf",
            crate::ui::views::flow::semibold(13.5),
            p.text_primary,
        );
        if !self.items.is_empty() {
            ui.painter().text(
                pos2(hr.left() + 80.0, hr.center().y),
                Align2::LEFT_CENTER,
                self.items.len().to_string(),
                FontId::proportional(11.5),
                p.text_tertiary,
            );
        }
        let close = Rect::from_center_size(pos2(hr.right() - 20.0, hr.center().y), vec2(30.0, 30.0));
        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(close));
        if icon_button(&mut cu, ph::X, "Hide shelf", true).clicked() {
            self.open = false;
        }
        let clear = Rect::from_center_size(pos2(hr.right() - 50.0, hr.center().y), vec2(30.0, 30.0));
        let mut cu = ui.new_child(egui::UiBuilder::new().max_rect(clear));
        if icon_button(&mut cu, ph::BROOM, "Clear shelf", !self.items.is_empty()).clicked() {
            self.items.clear();
            self.selected.clear();
        }
        ui.painter().line_segment(
            [pos2(card.left() + 8.0, hr.bottom()), pos2(card.right() - 8.0, hr.bottom())],
            Stroke::new(1.0, p.border_subtle),
        );

        let footer_h = if self.items.is_empty() { 0.0 } else { 78.0 };
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
            let painter = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("soyuz.shelf.hl")));
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
                self.add(pl.paths.clone());
            }
        }

        if self.items.is_empty() {
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
                "Drop files here",
                crate::ui::views::flow::semibold(13.0),
                p.text_primary,
            );
            let g = ui.painter().layout(
                "Collect items from different folders, then drag them out or copy them all at once.".into(),
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
            self.items.len(),
            |ui, range| {
                for i in range {
                    let item = &self.items[i];
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
                        "Missing".to_string()
                    } else {
                        let folder = item.path.parent().map(soyuz_core::entry::display_name).unwrap_or_default();
                        match &item.entry {
                            Some(e) if !e.is_dir_like() => {
                                format!("{} · {folder}", format_size(e.size, SizeStyle::Binary))
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
                            self.selected.iter().filter_map(|&k| self.items.get(k)).map(|it| it.path.clone()).collect()
                        } else {
                            vec![item.path.clone()]
                        };
                        DragAndDrop::set_payload(
                            ui.ctx(),
                            DragPayload { paths, source_pane: None, default: drag_default },
                        );
                    }
                    let path = item.path.clone();
                    resp.on_hover_text(format!(
                        "{}\nDrag out to copy or move · double-click to reveal",
                        path.display()
                    ))
                    .context_menu(|ui| {
                        ui.set_width(200.0);
                        if menu_item(ui, Some(ph::FOLDER_OPEN), "Reveal in pane", None, true).clicked() {
                            reveal = Some(path.clone());
                            ui.close();
                        }
                        if menu_item(ui, Some(ph::X), "Remove from shelf", None, true).clicked() {
                            let mut s = BTreeSet::new();
                            s.insert(i);
                            remove = Some(s);
                            ui.close();
                        }
                    });
                }
            },
        );
        if let Some(r) = remove {
            self.remove(&r);
        }
        if let Some(path) = reveal {
            app.cmds.push(Cmd::RevealPath(path));
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
        let n = if self.selected.is_empty() { self.items.len() } else { self.selected.len() };
        let to = dest.as_deref().map(soyuz_core::entry::display_name).unwrap_or_else(|| "—".into());
        fu.label(
            egui::RichText::new(format!("{} {} → {to}", if self.selected.is_empty() { "All" } else { "Selected" }, n))
                .font(FontId::proportional(11.5))
                .color(p.text_tertiary),
        );
        fu.horizontal(|ui| {
            let ok = dest.is_some();
            if button(ui, Some(ph::COPY), "Copy here", 1, ok).clicked()
                && let Some(d) = &dest
            {
                let paths = self.targets();
                app.cmds.push(Cmd::FileOp(FileOpCmd::Transfer { paths, dest: d.clone(), effect: DropEffect::Copy }));
            }
            if button(ui, Some(ph::ARROW_BEND_DOWN_RIGHT), "Move here", 0, ok).clicked()
                && let Some(d) = &dest
            {
                let paths = self.targets();
                app.cmds.push(Cmd::FileOp(FileOpCmd::Transfer {
                    paths: paths.clone(),
                    dest: d.clone(),
                    effect: DropEffect::Move,
                }));
                // Moved items are delivered: take them off the shelf.
                self.items.retain(|i| !paths.contains(&i.path));
                self.selected.clear();
            }
        });
    }

    /// A slim drop strip at the right edge while dragging files with the shelf closed.
    pub fn drag_strip(&mut self, ctx: &egui::Context) {
        if self.open {
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
        Area::new(Id::new("soyuz.shelf.strip")).order(Order::Foreground).fixed_pos(strip.min).interactable(false).show(
            ctx,
            |ui| {
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
                    "Shelf",
                    FontId::proportional(10.5),
                    p.text_secondary,
                );
            },
        );
        if hot
            && ctx.input(|i| i.pointer.any_released())
            && let Some(pl) = DragAndDrop::take_payload::<DragPayload>(ctx)
        {
            self.open = true;
            self.add(pl.paths.clone());
        }
    }
}

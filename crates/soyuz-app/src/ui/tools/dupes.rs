//! Duplicate finder: pick roots and a minimum size, then
//! `soyuz_search::find_duplicates` streams groups of identical files (size → head hash →
//! full BLAKE3) while it runs. Each group offers keep-one helpers (newest, oldest,
//! shortest path); marked files go to the trash through the jobs engine. A group can
//! never have all of its copies marked. Files leave the list only once the trash job has
//! finished and they are really gone (failed / skipped items stay, still marked).
//!
//! The search, the groups, the marks and the trash job live in the core
//! (`models::dupes::DupesModel`, Phase T7); this window draws them.

use std::path::PathBuf;

use egui::{Align, Align2, CornerRadius, FontId, Layout, Rect, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2};
use soyuz_core::{format_size, format_time};

use super::{button, progress_bar, text_field, tool_window};
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::models::dupes::{DupesModel, Empty, Keep, SIZES, other_pane_dir, short_folder, size_label};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::views::icons as file_icons;
use crate::ui::widgets::trunc_galley;

pub struct DupesWindow {
    pub open: bool,
    model: DupesModel,
    new_root: String,
}

impl DupesWindow {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        DupesWindow { open: true, model: DupesModel::new(roots, super::wake()), new_root: String::new() }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        self.model.poll();
        self.model.poll_trash(&app.core.fileops.jobs);
        let sub = self.model.subtitle();
        let keep = tool_window(ctx, "dupes", tr!("tools-dupes-title"), &sub, ph::FILES, vec2(900.0, 640.0), |ui| {
            self.body(ui, app)
        });
        if !keep {
            self.model.stop();
            self.open = false;
        }
    }

    fn body(&mut self, ui: &mut Ui, app: &mut SoyuzApp) {
        let p = palette(ui.ctx());
        // Roots.
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(tr!("tools-dupes-search-in")).font(FontId::proportional(12.0)).color(p.text_tertiary),
            );
            let mut remove = None;
            for (i, r) in self.model.roots.iter().enumerate() {
                let label = format!("{}  ×", soyuz_core::entry::display_name(r));
                if super::chip(ui, &label, p.accent)
                    .on_hover_text(tr!("tools-dupes-root-tip", path = r.display().to_string()))
                    .clicked()
                {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                self.model.remove_root(i);
            }
            let r = text_field(ui, &mut self.new_root, tr!("tools-dupes-add-path-hint"), 220.0);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (enter || button(ui, Some(ph::PLUS), tr!("tools-add"), 0, !self.new_root.trim().is_empty()).clicked())
                && !self.new_root.trim().is_empty()
            {
                self.model.add_root(&self.new_root);
                self.new_root.clear();
            }
            if let Some(other) = other_pane_dir(&app.core)
                && !self.model.roots.contains(&other)
                && button(ui, Some(ph::SQUARE_SPLIT_HORIZONTAL), tr!("tools-dupes-add-other-pane"), 0, true).clicked()
            {
                self.model.add_path(other);
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr!("tools-dupes-min-size")).font(FontId::proportional(12.0)).color(p.text_tertiary),
            );
            for i in 0..SIZES.len() {
                let mut on = self.model.min_idx == i;
                if super::toggle_chip(ui, size_label(i), &mut on, p.accent).clicked() {
                    self.model.set_min(i);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.model.is_running() {
                    if button(ui, Some(ph::STOP), tr!("tools-stop"), 0, true).clicked() {
                        self.model.stop_search();
                    }
                } else if button(
                    ui,
                    Some(ph::MAGNIFYING_GLASS),
                    tr!("tools-menu-find-duplicates"),
                    1,
                    !self.model.roots.is_empty(),
                )
                .clicked()
                {
                    self.model.start();
                }
            });
        });
        ui.add_space(4.0);
        // Progress / summary line.
        if let Some((hashed, total, text)) = self.model.progress() {
            let frac = (total > 0).then(|| hashed as f32 / total as f32);
            progress_bar(ui, ui.available_width(), frac);
            ui.label(RichText::new(text).font(FontId::proportional(12.0)).color(p.text_secondary));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        } else if let Some(summary) = self.model.summary() {
            ui.label(RichText::new(summary).font(FontId::proportional(12.5)).color(p.text_primary));
        }
        if let Some(empty) = self.model.empty_state() {
            let g = match empty {
                Empty::Running => ph::SPINNER,
                Empty::Handled => ph::SPARKLE,
                Empty::None => ph::CHECK_CIRCLE,
                Empty::Start => ph::FILES,
            };
            super::compare::empty_note(ui, g, empty.title(), empty.body());
            return;
        }
        // Bulk helpers.
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr!("tools-dupes-select-every-group"))
                    .font(FontId::proportional(12.0))
                    .color(p.text_tertiary),
            );
            for k in Keep::ALL {
                if button(ui, None, k.label(), 0, true).clicked() {
                    self.model.keep_all(k);
                }
            }
            if button(ui, None, tr!("tools-clear"), 0, true).clicked() {
                self.model.clear_marks();
            }
        });
        ui.add_space(4.0);
        let footer = 44.0;
        let h = (ui.available_height() - footer).max(80.0);
        self.list(ui, app, h);
        // Footer.
        let (n, _, _) = self.model.marked();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let text = self.model.marked_text();
            ui.label(RichText::new(text).font(FontId::proportional(12.5)).color(if n > 0 {
                p.text_primary
            } else {
                p.text_tertiary
            }));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.model.confirm {
                    let busy = self.model.busy();
                    if button(ui, Some(ph::TRASH), &tr!("tools-dupes-move-n-to-trash", count = n), 2, n > 0 && !busy)
                        .clicked()
                    {
                        self.model.trash_marked(&mut app.core);
                    }
                    if button(ui, None, tr!("common-cancel"), 0, true).clicked() {
                        self.model.confirm = false;
                    }
                } else if button(ui, Some(ph::TRASH), tr!("tools-dupes-delete-marked"), 2, n > 0).clicked() {
                    self.model.confirm = true;
                }
            });
        });
    }

    fn list(&mut self, ui: &mut Ui, app: &mut SoyuzApp, height: f32) {
        let p = palette(ui.ctx());
        let header_h = 34.0;
        let row_h = 26.0;
        let gap = 8.0;
        let heights: Vec<f32> = self
            .model
            .groups
            .iter()
            .map(|g| header_h + if g.collapsed { 0.0 } else { g.files.len() as f32 * row_h } + gap)
            .collect();
        let total: f32 = heights.iter().sum();
        let cfg = app.core.config.general.clone();
        let drag_default = app.core.config.behaviour.drag_default;
        let roots = self.model.roots.clone();
        let mut reveal = None;
        ScrollArea::vertical().id_salt("dupes.list").max_height(height).auto_shrink([false, false]).show_viewport(
            ui,
            |ui, viewport| {
                let width = ui.available_width();
                let (area, _) = ui.allocate_exact_size(vec2(width, total), Sense::hover());
                let mut y = 0.0;
                let model = &mut self.model;
                let mut toggles: Vec<(usize, usize)> = Vec::new();
                for (gi, g) in model.groups.iter_mut().enumerate() {
                    let gh = heights[gi];
                    if y + gh < viewport.top() || y > viewport.bottom() {
                        y += gh;
                        continue;
                    }
                    let top = area.top() + y;
                    let card = Rect::from_min_size(pos2(area.left(), top), vec2(width, gh - gap));
                    ui.painter().rect_filled(card, CornerRadius::same(8), p.bg_surface2.gamma_multiply(0.55));
                    ui.painter().rect_stroke(
                        card,
                        CornerRadius::same(8),
                        Stroke::new(1.0, p.border_subtle),
                        egui::StrokeKind::Inside,
                    );
                    // Header.
                    let hr = Rect::from_min_size(card.min, vec2(width, header_h));
                    let hresp = ui.interact(hr, ui.scope_id().with(("dupes.h", gi)), Sense::click());
                    if hresp.clicked() {
                        g.collapsed = !g.collapsed;
                    }
                    let caret = if g.collapsed { ph::CARET_RIGHT } else { ph::CARET_DOWN };
                    let painter = ui.painter();
                    painter.text(
                        pos2(hr.left() + 14.0, hr.center().y),
                        Align2::CENTER_CENTER,
                        caret,
                        icons::font(11.0),
                        p.text_tertiary,
                    );
                    let e0 = &g.files[0];
                    painter.text(
                        pos2(hr.left() + 32.0, hr.center().y),
                        Align2::CENTER_CENTER,
                        file_icons::glyph(e0),
                        icons::font(15.0),
                        file_icons::color(e0, &p),
                    );
                    let title = tr!(
                        "tools-dupes-group-title",
                        count = g.files.len(),
                        size = format_size(g.size(), cfg.size_style)
                    );
                    let title_r = painter.text(
                        pos2(hr.left() + 46.0, hr.center().y),
                        Align2::LEFT_CENTER,
                        title,
                        crate::ui::views::flow::semibold(12.5),
                        p.text_primary,
                    );
                    painter.text(
                        pos2((hr.left() + 230.0).max(title_r.right() + 16.0), hr.center().y),
                        Align2::LEFT_CENTER,
                        tr!("tools-dupes-reclaimable", size = format_size(g.wasted(), cfg.size_style)),
                        FontId::proportional(12.0),
                        p.success,
                    );
                    // Keep helpers (right side of the header).
                    let mut x = hr.right() - 8.0;
                    for k in [Keep::ShortestPath, Keep::Oldest, Keep::Newest] {
                        let label = k.short_label();
                        let gal = ui.painter().layout_no_wrap(
                            label.to_string(),
                            FontId::proportional(11.5),
                            p.text_secondary,
                        );
                        let w = gal.size().x + 16.0;
                        let r = Rect::from_min_size(pos2(x - w, hr.center().y - 11.0), vec2(w, 22.0));
                        x -= w + 4.0;
                        let resp = ui.interact(r, ui.scope_id().with(("dupes.k", gi, k as u8)), Sense::click());
                        ui.painter().rect_filled(
                            r,
                            CornerRadius::same(11),
                            if resp.hovered() { p.bg_surface3 } else { p.bg_surface1 },
                        );
                        ui.painter().galley(
                            pos2(r.left() + 8.0, r.center().y - gal.size().y / 2.0),
                            gal,
                            p.text_secondary,
                        );
                        if resp.on_hover_text(k.label()).clicked() {
                            g.apply(k);
                        }
                    }
                    ui.painter().text(
                        pos2(x - 4.0, hr.center().y),
                        Align2::RIGHT_CENTER,
                        tr!("tools-dupes-keep"),
                        FontId::proportional(11.0),
                        p.text_tertiary,
                    );
                    if g.collapsed {
                        y += gh;
                        continue;
                    }
                    for fi in 0..g.files.len() {
                        let rr =
                            Rect::from_min_size(pos2(card.left(), hr.bottom() + fi as f32 * row_h), vec2(width, row_h));
                        let resp = ui.interact(rr, ui.scope_id().with(("dupes.r", gi, fi)), Sense::click_and_drag());
                        if resp.hovered() {
                            ui.painter().rect_filled(
                                rr.shrink2(vec2(4.0, 1.0)),
                                CornerRadius::same(5),
                                p.bg_surface3.gamma_multiply(0.7),
                            );
                        }
                        let e = g.files[fi].clone();
                        let marked = g.marked[fi];
                        // Checkbox.
                        let cb = Rect::from_center_size(pos2(rr.left() + 22.0, rr.center().y), vec2(14.0, 14.0));
                        if marked {
                            ui.painter().rect_filled(cb, CornerRadius::same(4), p.danger);
                            ui.painter().text(
                                cb.center(),
                                Align2::CENTER_CENTER,
                                ph::CHECK,
                                icons::font(10.0),
                                egui::Color32::WHITE,
                            );
                        } else {
                            ui.painter().rect_stroke(
                                cb,
                                CornerRadius::same(4),
                                Stroke::new(1.0, p.border_strong),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let refused = model.refused.is_some_and(|(a, b, t)| {
                            a == gi && b == fi && t.elapsed() < crate::models::dupes::REFUSED_FLASH
                        });
                        let name = e.name.clone();
                        let folder = e.path.parent().map(|f| short_folder(&roots, f)).unwrap_or_default();
                        let nc = if marked { p.text_tertiary } else { p.text_primary };
                        let g1 = trunc_galley(ui.painter(), &name, FontId::proportional(12.5), nc, width * 0.3);
                        let nw = g1.size().x;
                        ui.painter().galley(pos2(rr.left() + 40.0, rr.center().y - g1.size().y / 2.0), g1, nc);
                        if marked {
                            ui.painter().line_segment(
                                [pos2(rr.left() + 40.0, rr.center().y), pos2(rr.left() + 40.0 + nw, rr.center().y)],
                                Stroke::new(1.0, p.text_tertiary),
                            );
                        }
                        let date = e.modified.map(|t| format_time(t, &cfg.date_format)).unwrap_or_default();
                        let dg = ui.painter().layout_no_wrap(date, FontId::proportional(11.5), p.text_tertiary);
                        let dw = dg.size().x;
                        ui.painter().galley(
                            pos2(rr.right() - 12.0 - dw, rr.center().y - dg.size().y / 2.0),
                            dg,
                            p.text_tertiary,
                        );
                        let fx = rr.left() + 52.0 + nw;
                        let fg = trunc_galley(
                            ui.painter(),
                            &folder,
                            FontId::proportional(11.5),
                            p.text_secondary,
                            rr.right() - 24.0 - dw - fx,
                        );
                        ui.painter().galley(pos2(fx, rr.center().y - fg.size().y / 2.0), fg, p.text_secondary);
                        if refused {
                            ui.painter().text(
                                pos2(rr.left() + 40.0 + nw + 8.0, rr.top() - 2.0),
                                Align2::LEFT_BOTTOM,
                                tr!("tools-dupes-keep-one"),
                                FontId::proportional(11.0),
                                p.warning,
                            );
                        }
                        if resp.clicked() {
                            toggles.push((gi, fi));
                        }
                        if resp.double_clicked() {
                            reveal = Some(e.path.clone());
                        }
                        super::drag_source(&resp, drag_default, || vec![e.path.clone()]);
                        let path = e.path.clone();
                        crate::ui::native::popup::context_menu(
                            &resp.on_hover_text(tr!("tools-dupes-row-tip", path = path.display().to_string())),
                            |ui| {
                                if crate::ui::widgets::menu_item(
                                    ui,
                                    Some(ph::FOLDER_OPEN),
                                    tr!("tools-shelf-reveal"),
                                    None,
                                    true,
                                )
                                .clicked()
                                {
                                    reveal = Some(path.clone());
                                    ui.close();
                                }
                                if crate::ui::widgets::menu_item(
                                    ui,
                                    Some(ph::CLIPBOARD_TEXT),
                                    tr!("tools-copy-path"),
                                    None,
                                    true,
                                )
                                .clicked()
                                {
                                    ui.ctx().copy_text(super::tags_ui::show_path(&path));
                                    ui.close();
                                }
                            },
                        );
                    }
                    y += gh;
                }
                for (gi, fi) in toggles {
                    model.toggle(gi, fi);
                }
            },
        );
        if let Some(p) = reveal {
            app.core.cmds.push(Cmd::RevealPath(p));
        }
    }
}

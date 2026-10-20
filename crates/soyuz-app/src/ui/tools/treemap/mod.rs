//! Disk usage analyzer: a squarified treemap of a folder with cushion-shaded,
//! kind-colored tiles, nested folder frames, hover tooltips (path, size, share of the
//! parent), click to drill in, a breadcrumb to go back up, a side list of the largest
//! items, and a context menu (open, reveal, move to trash via the jobs engine). The scan
//! runs on a worker and the map fills in live.
//!
//! The scan, the tree, the layout and what the toolbar, side list and menu do live in the
//! core (`models::treemap::TreemapModel`, Phase T7); this window draws them.
//!
//! Compare as treemap: the same window with a compare running beside the scan; tiles,
//! folder headers and the legend show what differs (`models::treemap::diff::Mark`), and
//! a switch goes back to the kind colours.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_core::{SizeStyle, format_size};

use super::{button, progress_bar, tool_window};
use crate::app::SoyuzApp;
use crate::models::treemap::diff::Mark;
use crate::models::treemap::layout::{Tile, squarify};
use crate::models::treemap::scan::Tree;
use crate::models::treemap::{Bucket, SideTab, Status, TreemapItem, TreemapModel, percent, share};
use crate::theme::{Palette, icons, icons::ph, palette};
use crate::ui::views::icons::FileClass;
use crate::ui::widgets::{icon_button, menu_item, menu_separator, trunc_galley};

/// Folder frames nest this many levels below the current node; deeper content is drawn
/// as one cushion per folder.
const MAX_NEST: usize = 3;
const HEADER: f32 = 17.0;
const SIDE_W: f32 = 280.0;

/// Tile color for a file class (theme glyph tokens where they exist).
pub fn class_color(class: FileClass, p: &Palette) -> Color32 {
    bucket_color(Bucket::of(class), p)
}

/// Tile color for a legend bucket.
pub fn bucket_color(b: Bucket, p: &Palette) -> Color32 {
    match b {
        Bucket::Folder => p.glyph_folder,
        Bucket::Image => p.glyph_image,
        Bucket::Video => p.glyph_video,
        Bucket::Audio => p.glyph_audio,
        Bucket::Archive => Color32::from_rgb(0xE8, 0x87, 0x5B),
        Bucket::Code => p.glyph_code,
        Bucket::Docs => Color32::from_rgb(0x3C, 0xC6, 0xB4),
        Bucket::Exe => p.glyph_exe,
        Bucket::Other => Color32::from_rgb(0x8A, 0x91, 0xA0),
    }
}

/// Tile color for a diff mark.
pub fn mark_color(m: Mark, p: &Palette) -> Color32 {
    match m {
        Mark::OnlyHere => p.success,
        Mark::NewerHere => p.info,
        Mark::NewerThere => p.warning,
        Mark::Differ => p.danger,
        Mark::Changed => p.accent,
        Mark::Same => {
            if p.dark {
                Color32::from_rgb(0x5C, 0x63, 0x70)
            } else {
                Color32::from_rgb(0xB4, 0xBA, 0xC4)
            }
        }
    }
}

fn scale(c: Color32, f: f32) -> Color32 {
    let s = |x: u8| ((x as f32) * f).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(s(c.r()), s(c.g()), s(c.b()))
}

fn luminance(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

/// Append a cushion-shaded rectangle (3×3 vertex grid: bright center, dark corners).
fn cushion(mesh: &mut Mesh, r: Rect, base: Color32, dark_theme: bool) {
    let base = if dark_theme { scale(base, 0.88) } else { base };
    let xs = [r.left(), r.center().x, r.right()];
    let ys = [r.top(), r.center().y, r.bottom()];
    let start = mesh.vertices.len() as u32;
    for (j, y) in ys.iter().enumerate() {
        for (i, x) in xs.iter().enumerate() {
            let f = match (i, j) {
                (1, 1) => 1.16,
                (1, _) | (_, 1) => 0.94,
                (0, 0) => 0.84,
                (2, 2) => 0.58,
                _ => 0.7,
            };
            mesh.vertices.push(Vertex { pos: pos2(*x, *y), uv: WHITE_UV, color: scale(base, f) });
        }
    }
    for j in 0..2u32 {
        for i in 0..2u32 {
            let a = start + j * 3 + i;
            mesh.add_triangle(a, a + 1, a + 4);
            mesh.add_triangle(a, a + 4, a + 3);
        }
    }
}

pub struct TreemapWindow {
    pub open: bool,
    model: TreemapModel,
    hover: Option<usize>,
    side_hover: Option<usize>,
    menu_target: Option<usize>,
}

impl TreemapWindow {
    pub fn new(root: PathBuf) -> Self {
        Self::with_model(TreemapModel::new(root, super::wake()))
    }

    /// Compare as treemap: `root` coloured by its differences from `other`.
    pub fn new_diff(root: PathBuf, other: PathBuf, by_content: bool) -> Self {
        Self::with_model(TreemapModel::new(root, super::wake()).with_diff(other, by_content))
    }

    fn with_model(model: TreemapModel) -> Self {
        TreemapWindow { open: true, model, hover: None, side_hover: None, menu_target: None }
    }

    pub fn root_path(&self) -> &Path {
        self.model.root_path()
    }

    /// The folder compared with (Compare as treemap).
    pub fn diff_other(&self) -> Option<&Path> {
        self.model.diff_other()
    }

    /// A tile's colour: its diff mark, else its kind.
    fn tile_color(&self, id: usize, p: &Palette) -> Color32 {
        match self.model.mark(id) {
            Some(m) => mark_color(m, p),
            None => class_color(self.model.tree.nodes[id].class, p),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        self.model.poll();
        let sub = super::tags_ui::show_path(self.model.root_path());
        let keep = tool_window(
            ctx,
            "treemap",
            tr!("tools-treemap-title"),
            &sub,
            ph::CHART_PIE_SLICE,
            vec2(1100.0, 720.0),
            |ui| {
                self.body(ui, app);
            },
        );
        if !keep {
            self.model.cancel();
            self.open = false;
        }
    }

    fn body(&mut self, ui: &mut Ui, app: &mut SoyuzApp) {
        let p = palette(ui.ctx());
        let style = app.core.config.general.size_style;
        // Toolbar: up, breadcrumb, totals, rescan / cancel.
        ui.horizontal(|ui| {
            let up = self.model.tree.nodes[self.model.current].parent;
            if icon_button(ui, ph::ARROW_UP, tr!("tools-treemap-up"), up.is_some()).clicked()
                && let Some(u) = up
            {
                self.model.go(u);
            }
            let chain = self.model.crumbs();
            for (k, id) in chain.iter().enumerate() {
                if k > 0 {
                    ui.label(icons::icon(ph::CARET_RIGHT, 11.0).color(p.text_tertiary));
                }
                let last = k + 1 == chain.len();
                let name = self.model.tree.nodes[*id].label().to_string();
                let text = egui::RichText::new(name).font(if last {
                    crate::ui::views::flow::semibold(13.0)
                } else {
                    FontId::proportional(13.0)
                });
                let text = if last { text.color(p.text_primary) } else { text.color(p.text_secondary) };
                if ui
                    .add(egui::Label::new(text).sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.model.go(*id);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.model.is_scanning() {
                    if button(ui, Some(ph::STOP), tr!("tools-stop"), 0, true).clicked() {
                        self.model.cancel();
                    }
                } else if button(ui, Some(ph::ARROW_CLOCKWISE), tr!("tools-rescan"), 0, true).clicked() {
                    self.model.rescan();
                }
                ui.label(
                    egui::RichText::new(self.model.totals(style))
                        .font(FontId::proportional(12.5))
                        .color(p.text_secondary),
                );
            });
        });
        let status = self.model.status(style);
        match &status {
            Status::Scanning { text, .. } => {
                progress_bar(ui, ui.available_width(), status.fraction());
                ui.label(egui::RichText::new(text).font(FontId::proportional(11.5)).color(p.text_tertiary));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
            }
            Status::Failed(e) => {
                ui.label(egui::RichText::new(e).color(p.danger));
            }
            Status::Done(s) => {
                ui.label(egui::RichText::new(s).font(FontId::proportional(11.5)).color(p.text_tertiary));
            }
            Status::Idle => {}
        }
        if let Some(d) = &self.model.diff {
            let (text, comparing, show) = (d.status(), d.is_comparing(), d.show);
            let full = super::tags_ui::show_path(&d.other);
            ui.horizontal(|ui| {
                if comparing {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (on, label) in
                        [(false, tr!("tools-treemap-diff-by-kind")), (true, tr!("tools-treemap-diff-by-changes"))]
                    {
                        let mut sel = show == on;
                        if super::toggle_chip(ui, label, &mut sel, p.accent).clicked() && show != on {
                            self.model.toggle_diff();
                        }
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let text = egui::RichText::new(text).font(FontId::proportional(11.5)).color(p.text_secondary);
                        ui.add(egui::Label::new(text).truncate()).on_hover_text(full);
                    });
                });
            });
        }
        ui.add_space(4.0);

        let area = ui.available_rect_before_wrap();
        let legend_h = 24.0;
        let confirm_h = if self.model.confirm_delete.is_some() { 40.0 } else { 0.0 };
        let map =
            Rect::from_min_max(area.min, pos2(area.right() - SIDE_W - 10.0, area.bottom() - legend_h - confirm_h));
        let side = Rect::from_min_max(pos2(map.right() + 10.0, area.top()), pos2(area.right(), map.bottom()));
        ui.allocate_rect(area, Sense::hover());
        self.treemap(ui, map, app);
        self.side(ui, side, style);
        // Legend: the kinds, or the diff marks with their bytes in this level.
        let lr = Rect::from_min_max(pos2(area.left(), map.bottom() + 6.0), pos2(area.right(), map.bottom() + legend_h));
        let mut x = lr.left() + 2.0;
        let marks = self.model.diff_totals();
        let entries: Vec<(String, Color32)> = if marks.is_empty() {
            Bucket::LEGEND.iter().map(|b| (b.label().to_string(), bucket_color(*b, &p))).collect()
        } else {
            marks
                .iter()
                .map(|(m, b)| (format!("{}  {}", m.label(), format_size(*b, style)), mark_color(*m, &p)))
                .collect()
        };
        for (label, c) in entries {
            ui.painter().rect_filled(
                Rect::from_center_size(pos2(x + 5.0, lr.center().y), vec2(10.0, 10.0)),
                CornerRadius::same(3),
                c,
            );
            let g = ui.painter().layout_no_wrap(label, FontId::proportional(11.5), p.text_secondary);
            let w = g.size().x;
            ui.painter().galley(pos2(x + 14.0, lr.center().y - g.size().y / 2.0), g, p.text_secondary);
            x += w + 30.0;
        }
        // Delete confirmation bar.
        if let Some(msg) = self.model.confirm_text(style) {
            let cr = Rect::from_min_max(pos2(area.left(), area.bottom() - confirm_h + 4.0), area.max);
            ui.painter().rect_filled(cr, CornerRadius::same(8), p.danger.gamma_multiply(0.12));
            ui.painter().text(
                pos2(cr.left() + 14.0, cr.center().y),
                Align2::LEFT_CENTER,
                msg,
                FontId::proportional(12.5),
                p.text_primary,
            );
            let br = Rect::from_min_max(pos2(cr.right() - 320.0, cr.top() + 4.0), cr.max - vec2(8.0, 4.0));
            let mut bu = ui.new_child(
                egui::UiBuilder::new().max_rect(br).layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            if button(&mut bu, Some(ph::TRASH), tr!("tools-move-to-trash"), 2, true).clicked() {
                self.model.confirm_trash(&mut app.core);
            }
            if button(&mut bu, None, tr!("common-cancel"), 0, true).clicked() {
                self.model.confirm_delete = None;
            }
        }
    }

    fn treemap(&mut self, ui: &mut Ui, rect: Rect, app: &mut SoyuzApp) {
        let p = palette(ui.ctx());
        ui.painter().rect_filled(rect, CornerRadius::same(8), p.bg_sunken);
        let resp = ui.interact(rect, ui.scope_id().with("treemap.area"), Sense::click());
        let inner = rect.shrink(4.0);
        let mut hits: Vec<(Rect, usize)> = Vec::new();
        let mut mesh = Mesh::default();
        let mut labels: Vec<Shape> = Vec::new();
        let mut bgs: Vec<Shape> = Vec::new();
        let mut borders: Vec<Shape> = Vec::new();
        let mut out = Out {
            hits: &mut hits,
            mesh: &mut mesh,
            bgs: &mut bgs,
            borders: &mut borders,
            labels: &mut labels,
            style: app.core.config.general.size_style,
        };
        self.layout_level(ui, &p, inner, self.model.current, 0, &mut out);
        let painter = ui.painter_at(rect);
        // Frame backgrounds, then the cushions, then frame borders and labels.
        painter.extend(bgs);
        painter.add(Shape::mesh(mesh));
        painter.extend(borders);
        painter.extend(labels);
        // Screen readers: the map is a list of its tiles (name and size).
        if crate::ui::a11y::active(ui.ctx()) {
            crate::ui::a11y::container(ui.ctx(), resp.id, egui::accesskit::Role::ListBox, tr!("tools-treemap-title"));
            let style = app.core.config.general.size_style;
            let total = hits.len();
            for (pos, (r, id)) in hits.iter().enumerate() {
                let n = &self.model.tree.nodes[*id];
                let label = tr!("tools-treemap-tile-a11y", name = n.label(), size = format_size(n.bytes, style));
                let aid = ui.scope_id().with(("treemap.tile", *id));
                crate::ui::a11y::list_item(ui, aid, *r, &label, self.model.selected == Some(*id), pos, total);
            }
        }
        // Live highlights of what the assistant / MCP clients read or change. A folder
        // tile whose contents are not drawn shows the strongest mark inside it.
        if crate::ai::touch::any() {
            let shown: std::collections::HashSet<usize> = hits.iter().map(|(_, id)| *id).collect();
            let reduced = app.core.ui.reduced_motion;
            for (r, id) in &hits {
                let n = &self.model.tree.nodes[*id];
                let opened = n.children.iter().any(|c| shown.contains(c));
                let t = if n.is_dir && !opened {
                    crate::ai::touch::get_within(&n.path)
                } else {
                    crate::ai::touch::get(&n.path)
                };
                if let Some(t) = t {
                    crate::ui::ai::touch::paint_touch(&painter, &p, *r, t, reduced);
                }
            }
        }

        let n = &self.model.tree.nodes[self.model.current];
        if n.children.is_empty() {
            let (glyph, text) = if n.loading || self.model.is_scanning() {
                (ph::SPINNER, tr!("tools-treemap-measuring"))
            } else if n.is_dir {
                (ph::FOLDER_DASHED, tr!("tools-treemap-empty-folder"))
            } else {
                (ph::FILE, tr!("tools-treemap-single-file"))
            };
            painter.text(
                rect.center() - vec2(0.0, 14.0),
                Align2::CENTER_CENTER,
                glyph,
                icons::font(28.0),
                p.text_tertiary,
            );
            painter.text(
                rect.center() + vec2(0.0, 14.0),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(13.0),
                p.text_secondary,
            );
        }

        // Hover: deepest hit under the pointer.
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p));
        self.hover = pointer.and_then(|pos| hits.iter().rev().find(|(r, _)| r.contains(pos)).map(|(_, id)| *id));
        let highlight = |id: Option<usize>, color: Color32, width: f32| {
            if let Some(id) = id
                && let Some((r, _)) = hits.iter().find(|(_, h)| *h == id)
            {
                painter.rect_stroke(*r, CornerRadius::same(2), Stroke::new(width, color), StrokeKind::Inside);
            }
        };
        highlight(self.side_hover, p.text_primary.gamma_multiply(0.85), 2.0);
        highlight(self.model.selected, p.accent, 2.0);
        highlight(self.hover, Color32::WHITE.gamma_multiply(0.9), 1.5);
        if let Some(id) = self.hover {
            let parent_bytes = self.model.tree.nodes[id].parent.map(|pp| self.model.tree.nodes[pp].bytes).unwrap_or(0);
            let cur_bytes = self.model.tree.nodes[self.model.current].bytes;
            let tree = &self.model.tree;
            let style = app.core.config.general.size_style;
            let (mark, color) = (self.model.mark(id), self.tile_color(id, &p));
            resp.clone().on_hover_ui_at_pointer(|ui| {
                tooltip(ui, tree, id, parent_bytes, cur_bytes, style, color);
                if let Some(m) = mark {
                    ui.label(
                        egui::RichText::new(tr!("tools-treemap-diff-tile", mark = m.label()))
                            .font(crate::ui::views::flow::semibold(11.5))
                            .color(mark_color(m, &p)),
                    );
                }
            });
        }
        if resp.clicked()
            && let Some(id) = self.hover
        {
            // Clicking a tile drills into the top-level child of the current view that contains it.
            self.model.click(id);
        }
        if resp.double_clicked()
            && let Some(id) = self.hover
            && self.model.launchable(id)
        {
            let path = self.model.tree.nodes[id].path.clone();
            crate::ops::launch(&mut app.core, &path);
        }
        if resp.secondary_clicked() {
            self.menu_target = self.hover;
        }
        if let Some(target) = self.menu_target {
            crate::ui::native::popup::context_menu(&resp, |ui| self.menu(ui, app, target));
        }
    }

    fn layout_level(&self, ui: &Ui, p: &Palette, rect: Rect, node: usize, depth: usize, out: &mut Out<'_>) {
        let style = out.style;
        let kids: Vec<usize> = self.model.tree.nodes[node]
            .children
            .iter()
            .copied()
            .filter(|&k| self.model.tree.nodes[k].bytes > 0)
            .collect();
        if kids.is_empty() || rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        let weights: Vec<f64> = kids.iter().map(|&k| self.model.tree.nodes[k].bytes as f64).collect();
        let tiles = squarify(
            &weights,
            Tile::new(rect.left() as f64, rect.top() as f64, rect.width() as f64, rect.height() as f64),
        );
        for (k, t) in kids.iter().zip(tiles) {
            let r = Rect::from_min_size(pos2(t.x as f32, t.y as f32), vec2(t.w as f32, t.h as f32))
                .shrink(if depth == 0 { 1.0 } else { 0.75 });
            if r.width() < 1.5 || r.height() < 1.5 {
                continue;
            }
            let n = &self.model.tree.nodes[*k];
            let nested =
                n.is_dir && !n.children.is_empty() && depth < MAX_NEST && r.width() > 44.0 && r.height() > 34.0;
            out.hits.push((r, *k));
            if nested {
                let fc = self.model.mark(*k).map_or(p.glyph_folder, |m| mark_color(m, p));
                let bg = if p.dark {
                    scale(p.bg_surface2, 1.0 + 0.08 * depth as f32)
                } else {
                    scale(p.bg_surface2, 1.0 - 0.03 * depth as f32)
                };
                out.bgs.push(Shape::rect_filled(r, CornerRadius::same(3), bg));
                let hr = Rect::from_min_size(r.min, vec2(r.width(), HEADER));
                out.bgs.push(Shape::rect_filled(
                    hr,
                    CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 },
                    fc.gamma_multiply(if p.dark { 0.20 } else { 0.28 }),
                ));
                out.borders.push(Shape::rect_stroke(
                    r,
                    CornerRadius::same(3),
                    Stroke::new(1.0, p.border_strong),
                    StrokeKind::Inside,
                ));
                let label = format!("{}  {}", n.label(), format_size(n.bytes, style));
                let g = trunc_galley_shape(ui, &label, 11.0, p.text_primary, r.width() - 10.0);
                out.labels.push(Shape::galley(
                    pos2(r.left() + 5.0, hr.center().y - g.size().y / 2.0),
                    g,
                    p.text_primary,
                ));
                let inner = Rect::from_min_max(
                    pos2(r.left() + 2.0, r.top() + HEADER + 1.0),
                    pos2(r.right() - 2.0, r.bottom() - 2.0),
                );
                self.layout_level(ui, p, inner, *k, depth + 1, out);
            } else {
                let base = self.tile_color(*k, p);
                cushion(out.mesh, r, base, p.dark);
                if r.width() > 54.0 && r.height() > 30.0 {
                    let tc = if luminance(base) > 0.55 {
                        Color32::from_black_alpha(220)
                    } else {
                        Color32::from_white_alpha(235)
                    };
                    let g = trunc_galley_shape(ui, n.label(), 11.5, tc, r.width() - 10.0);
                    let y = r.center().y - if r.height() > 44.0 { 8.0 } else { 0.0 };
                    out.labels.push(Shape::galley(pos2(r.center().x - g.size().x / 2.0, y - g.size().y / 2.0), g, tc));
                    if r.height() > 44.0 {
                        let g = trunc_galley_shape(
                            ui,
                            &format_size(n.bytes, style),
                            10.5,
                            tc.gamma_multiply(0.8),
                            r.width() - 10.0,
                        );
                        out.labels.push(Shape::galley(pos2(r.center().x - g.size().x / 2.0, y + 8.0), g, tc));
                    }
                }
                if n.loading {
                    out.labels.push(Shape::rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(60)));
                }
            }
        }
    }

    fn side(&mut self, ui: &mut Ui, rect: Rect, style: SizeStyle) {
        let p = palette(ui.ctx());
        ui.painter().rect_filled(rect, CornerRadius::same(8), p.bg_surface2.gamma_multiply(0.5));
        let mut child = ui.new_child(
            egui::UiBuilder::new().max_rect(rect.shrink(8.0)).layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut child;
        ui.horizontal(|ui| {
            for (t, label) in [
                (SideTab::Contents, tr!("tools-treemap-tab-contents")),
                (SideTab::Largest, tr!("tools-treemap-tab-largest")),
            ] {
                let mut on = self.model.side == t;
                let r = super::toggle_chip(ui, label, &mut on, p.accent);
                if r.clicked() {
                    self.model.side = t;
                }
            }
        });
        ui.add_space(4.0);
        let items: Vec<usize> = self.model.side_items();
        if items.is_empty() {
            let msg = self.model.side_empty();
            ui.add_space(12.0);
            ui.label(egui::RichText::new(msg).font(FontId::proportional(12.0)).color(p.text_tertiary));
        }
        let total = self.model.tree.nodes[self.model.current].bytes.max(1);
        let mut hover = None;
        let mut click = None;
        egui::ScrollArea::vertical().id_salt("treemap.side").auto_shrink([false, false]).show_rows(
            ui,
            40.0,
            items.len(),
            |ui, range| {
                for &id in &items[range] {
                    let n = &self.model.tree.nodes[id];
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    let painter = ui.painter();
                    let sel = self.model.selected == Some(id);
                    if sel {
                        painter.rect_filled(r, CornerRadius::same(6), p.accent_subtle);
                    } else if resp.hovered() {
                        painter.rect_filled(r, CornerRadius::same(6), p.bg_surface3);
                    }
                    if resp.hovered() {
                        hover = Some(id);
                    }
                    let c = self.tile_color(id, &p);
                    let glyph = if n.is_dir {
                        ph::FOLDER_SIMPLE
                    } else if n.other {
                        ph::DOTS_THREE
                    } else {
                        ph::FILE
                    };
                    painter.text(
                        pos2(r.left() + 14.0, r.top() + 13.0),
                        Align2::CENTER_CENTER,
                        glyph,
                        icons::font(14.0),
                        c,
                    );
                    let size = format_size(n.bytes, style);
                    let sg = painter.layout_no_wrap(size, FontId::proportional(11.5), p.text_secondary);
                    let sw = sg.size().x;
                    painter.galley(
                        pos2(r.right() - 8.0 - sw, r.top() + 13.0 - sg.size().y / 2.0),
                        sg,
                        p.text_secondary,
                    );
                    let g = trunc_galley(
                        painter,
                        n.label(),
                        FontId::proportional(12.5),
                        p.text_primary,
                        r.width() - 40.0 - sw - 8.0,
                    );
                    painter.galley(pos2(r.left() + 28.0, r.top() + 13.0 - g.size().y / 2.0), g, p.text_primary);
                    // Share bar.
                    let frac = n.bytes as f32 / total as f32;
                    let bar =
                        Rect::from_min_size(pos2(r.left() + 28.0, r.bottom() - 11.0), vec2(r.width() - 80.0, 4.0));
                    painter.rect_filled(bar, CornerRadius::same(2), p.border_subtle);
                    let mut fill = bar;
                    fill.set_width((bar.width() * frac).max(2.0));
                    painter.rect_filled(fill, CornerRadius::same(2), c);
                    painter.text(
                        pos2(r.right() - 8.0, bar.center().y),
                        Align2::RIGHT_CENTER,
                        percent(f64::from(frac) * 100.0),
                        FontId::proportional(10.5),
                        p.text_tertiary,
                    );
                    if resp.clicked() {
                        click = Some(id);
                    }
                    if self.model.side == SideTab::Largest {
                        let rel = n
                            .path
                            .strip_prefix(&self.model.tree.nodes[self.model.current].path)
                            .unwrap_or(&n.path)
                            .to_string_lossy()
                            .into_owned();
                        resp.on_hover_text(rel);
                    }
                }
            },
        );
        self.side_hover = hover;
        if let Some(id) = click {
            self.model.side_click(id);
        }
    }

    fn menu(&mut self, ui: &mut Ui, app: &mut SoyuzApp, id: usize) {
        ui.set_width(230.0);
        for item in self.model.menu_items(id) {
            if item == TreemapItem::Trash {
                menu_separator(ui);
            }
            let enabled = self.model.item_enabled(id, item);
            if menu_item(ui, Some(item_icon(item)), item.label(), None, enabled).clicked() {
                self.model.apply_item(&mut app.core, id, item);
                ui.close();
            }
        }
    }
}

/// The GUI's icon of a tile menu entry.
fn item_icon(item: TreemapItem) -> &'static str {
    match item {
        TreemapItem::ManySmall => ph::INFO,
        TreemapItem::ZoomIn => ph::MAGNIFYING_GLASS_PLUS,
        TreemapItem::OpenInPane => ph::FOLDER_OPEN,
        TreemapItem::Open => ph::ARROW_SQUARE_OUT,
        TreemapItem::Reveal => ph::ARROW_SQUARE_RIGHT,
        TreemapItem::RevealInSystem => ph::ARROW_SQUARE_UP_RIGHT,
        TreemapItem::CopyPath => ph::CLIPBOARD_TEXT,
        TreemapItem::AddToShelf => ph::TRAY_ARROW_DOWN,
        TreemapItem::Trash => ph::TRASH,
    }
}

/// Shapes collected while laying out one frame of the treemap.
struct Out<'a> {
    hits: &'a mut Vec<(Rect, usize)>,
    mesh: &'a mut Mesh,
    bgs: &'a mut Vec<Shape>,
    borders: &'a mut Vec<Shape>,
    labels: &'a mut Vec<Shape>,
    style: SizeStyle,
}

fn trunc_galley_shape(ui: &Ui, text: &str, size: f32, color: Color32, max_w: f32) -> Arc<egui::Galley> {
    trunc_galley(ui.painter(), text, FontId::proportional(size), color, max_w)
}

fn tooltip(ui: &mut Ui, tree: &Tree, id: usize, parent_bytes: u64, cur_bytes: u64, style: SizeStyle, color: Color32) {
    let p = palette(ui.ctx());
    let n = &tree.nodes[id];
    ui.set_max_width(360.0);
    ui.horizontal(|ui| {
        ui.label(icons::icon(if n.is_dir { ph::FOLDER_SIMPLE } else { ph::FILE }, 14.0).color(color));
        ui.label(egui::RichText::new(n.label()).font(crate::ui::views::flow::semibold(13.0)).color(p.text_primary));
    });
    if !n.other {
        ui.label(
            egui::RichText::new(super::tags_ui::show_path(&n.path))
                .font(FontId::proportional(11.5))
                .color(p.text_tertiary),
        );
    }
    ui.label(
        egui::RichText::new(format_size(n.bytes, style))
            .font(crate::ui::views::flow::semibold(14.0))
            .color(p.text_primary),
    );
    ui.label(
        egui::RichText::new(tr!(
            "tools-treemap-share",
            parent = percent(share(n.bytes, parent_bytes)),
            view = percent(share(n.bytes, cur_bytes))
        ))
        .font(FontId::proportional(11.5))
        .color(p.text_secondary),
    );
    if n.is_dir || n.other {
        ui.label(
            egui::RichText::new(tr!("tools-files", count = n.files))
                .font(FontId::proportional(11.5))
                .color(p.text_secondary),
        );
    }
    if n.is_dir && !n.complete {
        ui.label(
            egui::RichText::new(tr!("tools-treemap-measure-deeper"))
                .font(FontId::proportional(11.0))
                .color(p.text_tertiary),
        );
    }
}

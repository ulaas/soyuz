//! Disk usage analyzer (PLAN M12): a squarified treemap of a folder with cushion-shaded,
//! kind-colored tiles, nested folder frames, hover tooltips (path, size, share of the
//! parent), click to drill in, a breadcrumb to go back up, a side list of the largest
//! items, and a context menu (open, reveal, move to trash via the jobs engine). The scan
//! runs on a worker and the map fills in live.

pub mod layout;
pub mod scan;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_core::{Location, OpControl, SizeStyle, format_size};
use soyuz_jobs::DeleteMode;

use super::{button, progress_bar, tool_window};
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;
use crate::theme::{Palette, icons, icons::ph, palette};
use crate::ui::views::icons::FileClass;
use crate::ui::widgets::{icon_button, menu_item, menu_separator, trunc_galley};
use layout::{Tile, squarify};
use scan::{ScanMsg, Tree};

/// Folder frames nest this many levels below the current node; deeper content is drawn
/// as one cushion per folder.
const MAX_NEST: usize = 3;
const HEADER: f32 = 17.0;
const SIDE_W: f32 = 280.0;

/// Tile color for a file class (theme glyph tokens where they exist).
pub fn class_color(class: FileClass, p: &Palette) -> Color32 {
    match class {
        FileClass::Folder => p.glyph_folder,
        FileClass::Image => p.glyph_image,
        FileClass::Video => p.glyph_video,
        FileClass::Audio => p.glyph_audio,
        FileClass::Archive => Color32::from_rgb(0xE8, 0x87, 0x5B),
        FileClass::Code => p.glyph_code,
        FileClass::Pdf | FileClass::Doc | FileClass::Sheet | FileClass::Slides | FileClass::Text => {
            Color32::from_rgb(0x3C, 0xC6, 0xB4)
        }
        FileClass::Exe => p.glyph_exe,
        FileClass::Font | FileClass::Link | FileClass::Other => Color32::from_rgb(0x8A, 0x91, 0xA0),
    }
}

const LEGEND: [FileClass; 9] = [
    FileClass::Folder,
    FileClass::Image,
    FileClass::Video,
    FileClass::Audio,
    FileClass::Archive,
    FileClass::Code,
    FileClass::Doc,
    FileClass::Exe,
    FileClass::Other,
];

fn legend_label(class: FileClass) -> &'static str {
    match class {
        FileClass::Folder => tr!("tools-treemap-legend-folders"),
        FileClass::Image => tr!("tools-treemap-legend-images"),
        FileClass::Video => tr!("tools-treemap-legend-video"),
        FileClass::Audio => tr!("tools-treemap-legend-audio"),
        FileClass::Archive => tr!("tools-treemap-legend-archives"),
        FileClass::Code => tr!("tools-treemap-legend-code"),
        FileClass::Doc => tr!("tools-treemap-legend-documents"),
        FileClass::Exe => tr!("tools-treemap-legend-programs"),
        _ => tr!("tools-treemap-legend-other"),
    }
}

/// `12.5%` / `%12,5`.
fn percent(v: f64) -> String {
    tr!("tools-percent", pct = soyuz_core::locale::format_decimal(v, 1))
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum SideTab {
    Contents,
    Largest,
}

struct Scanning {
    ctl: Arc<OpControl>,
    started: Instant,
    total: usize,
    done: usize,
    current: PathBuf,
}

pub struct TreemapWindow {
    pub open: bool,
    root: PathBuf,
    tree: Tree,
    current: usize,
    tx: Sender<ScanMsg>,
    rx: Receiver<ScanMsg>,
    scanning: Option<Scanning>,
    sub_ctl: Arc<OpControl>,
    errors: u32,
    failed: Option<String>,
    hover: Option<usize>,
    side_hover: Option<usize>,
    selected: Option<usize>,
    menu_target: Option<usize>,
    confirm_delete: Option<usize>,
    tab: SideTab,
    finished_in: Option<f32>,
}

impl TreemapWindow {
    pub fn new(root: PathBuf) -> Self {
        let (tx, rx) = unbounded();
        let mut w = TreemapWindow {
            open: true,
            tree: Tree::new(&root),
            root,
            current: 0,
            tx,
            rx,
            scanning: None,
            sub_ctl: OpControl::new(),
            errors: 0,
            failed: None,
            hover: None,
            side_hover: None,
            selected: None,
            menu_target: None,
            confirm_delete: None,
            tab: SideTab::Contents,
            finished_in: None,
        };
        w.rescan();
        w
    }

    pub fn root_path(&self) -> &Path {
        &self.root
    }

    fn rescan(&mut self) {
        self.cancel();
        let (tx, rx) = unbounded();
        self.tx = tx;
        self.rx = rx;
        self.tree = Tree::new(&self.root);
        self.current = 0;
        self.selected = None;
        self.errors = 0;
        self.failed = None;
        self.finished_in = None;
        self.sub_ctl = OpControl::new();
        let ctl = OpControl::new();
        scan::start_scan(self.root.clone(), self.tx.clone(), ctl.clone(), super::wake());
        self.scanning = Some(Scanning { ctl, started: Instant::now(), total: 0, done: 0, current: PathBuf::new() });
    }

    fn cancel(&mut self) {
        if let Some(s) = self.scanning.take() {
            s.ctl.cancel();
            self.tree.nodes[0].loading = false;
        }
        self.sub_ctl.cancel();
    }

    fn poll(&mut self) {
        for msg in self.rx.try_iter() {
            match msg {
                ScanMsg::Begin { files, dirs } => {
                    for f in files {
                        self.tree.add_child(0, f);
                    }
                    if let Some(s) = &mut self.scanning {
                        s.total = dirs;
                    }
                }
                ScanMsg::Child(c) => {
                    self.tree.add_child(0, c);
                    if let Some(s) = &mut self.scanning {
                        s.done += 1;
                    }
                }
                ScanMsg::Progress { done, current } => {
                    if let Some(s) = &mut self.scanning {
                        s.done = done;
                        s.current = current;
                    }
                }
                ScanMsg::Done { errors } => {
                    self.errors = errors;
                    let n = &mut self.tree.nodes[0];
                    n.loading = false;
                    n.complete = true;
                    self.finished_in = self.scanning.take().map(|s| s.started.elapsed().as_secs_f32());
                }
                ScanMsg::Failed(e) => {
                    self.failed = Some(e);
                    self.scanning = None;
                    self.tree.nodes[0].loading = false;
                }
                ScanMsg::Subtree { path, result } => {
                    if let Some(id) = self.tree.find(&path) {
                        match result {
                            Ok(s) => self.tree.replace(id, s),
                            Err(_) => {
                                let n = &mut self.tree.nodes[id];
                                n.loading = false;
                                n.complete = true;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Drill into `id` (folders only), loading deeper levels when needed.
    fn enter(&mut self, id: usize) {
        let n = &self.tree.nodes[id];
        if !n.is_dir || n.other {
            self.selected = Some(id);
            return;
        }
        // Measure deeper when this level (or a folder right below it) was cut off by the
        // scan depth, so the view always shows real contents.
        let shallow =
            !n.complete || n.children.iter().any(|&c| self.tree.nodes[c].is_dir && !self.tree.nodes[c].complete);
        if shallow && !n.loading {
            let path = n.path.clone();
            self.tree.nodes[id].loading = true;
            scan::scan_subtree(path, self.tx.clone(), self.sub_ctl.clone(), super::wake());
        }
        self.current = id;
        self.selected = None;
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        self.poll();
        let sub = super::tags_ui::show_path(&self.root);
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
            self.cancel();
            self.open = false;
        }
    }

    fn body(&mut self, ui: &mut Ui, app: &mut SoyuzApp) {
        let p = palette(ui.ctx());
        let style = app.config.general.size_style;
        // Toolbar: up, breadcrumb, totals, rescan / cancel.
        ui.horizontal(|ui| {
            let up = self.tree.nodes[self.current].parent;
            if icon_button(ui, ph::ARROW_UP, tr!("tools-treemap-up"), up.is_some()).clicked()
                && let Some(u) = up
            {
                self.current = u;
                self.selected = None;
            }
            let chain = self.tree.chain(self.current);
            for (k, id) in chain.iter().enumerate() {
                if k > 0 {
                    ui.label(icons::icon(ph::CARET_RIGHT, 11.0).color(p.text_tertiary));
                }
                let last = k + 1 == chain.len();
                let name = self.tree.nodes[*id].label().to_string();
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
                    self.current = *id;
                    self.selected = None;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.scanning.is_some() {
                    if button(ui, Some(ph::STOP), tr!("tools-stop"), 0, true).clicked() {
                        self.cancel();
                    }
                } else if button(ui, Some(ph::ARROW_CLOCKWISE), tr!("tools-rescan"), 0, true).clicked() {
                    self.rescan();
                }
                let n = &self.tree.nodes[self.current];
                ui.label(
                    egui::RichText::new(tr!(
                        "tools-treemap-totals",
                        size = format_size(n.bytes, style),
                        count = n.files
                    ))
                    .font(FontId::proportional(12.5))
                    .color(p.text_secondary),
                );
            });
        });
        if let Some(s) = &self.scanning {
            let frac = (s.total > 0).then(|| s.done as f32 / s.total as f32);
            progress_bar(ui, ui.available_width(), frac);
            let cur = if s.current.as_os_str().is_empty() {
                tr!("tools-treemap-listing").to_string()
            } else {
                soyuz_core::entry::display_name(&s.current)
            };
            ui.label(
                egui::RichText::new(tr!(
                    "tools-treemap-scanning",
                    done = s.done.min(s.total),
                    total = s.total,
                    size = format_size(self.tree.nodes[0].bytes, style),
                    current = cur
                ))
                .font(FontId::proportional(11.5))
                .color(p.text_tertiary),
            );
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        } else if let Some(e) = &self.failed {
            ui.label(egui::RichText::new(tr!("tools-treemap-failed", error = e)).color(p.danger));
        } else if let Some(secs) = self.finished_in {
            let secs = soyuz_core::locale::format_decimal(f64::from(secs), 1);
            let s = if self.errors > 0 {
                tr!("tools-treemap-scanned-errors", secs = secs, errors = self.errors)
            } else {
                tr!("tools-treemap-scanned", secs = secs)
            };
            ui.label(egui::RichText::new(s).font(FontId::proportional(11.5)).color(p.text_tertiary));
        }
        ui.add_space(4.0);

        let area = ui.available_rect_before_wrap();
        let legend_h = 24.0;
        let confirm_h = if self.confirm_delete.is_some() { 40.0 } else { 0.0 };
        let map =
            Rect::from_min_max(area.min, pos2(area.right() - SIDE_W - 10.0, area.bottom() - legend_h - confirm_h));
        let side = Rect::from_min_max(pos2(map.right() + 10.0, area.top()), pos2(area.right(), map.bottom()));
        ui.allocate_rect(area, Sense::hover());
        self.treemap(ui, map, app);
        self.side(ui, side, style);
        // Legend.
        let lr = Rect::from_min_max(pos2(area.left(), map.bottom() + 6.0), pos2(area.right(), map.bottom() + legend_h));
        let mut x = lr.left() + 2.0;
        for class in LEGEND {
            let label = legend_label(class);
            let c = class_color(class, &p);
            ui.painter().rect_filled(
                Rect::from_center_size(pos2(x + 5.0, lr.center().y), vec2(10.0, 10.0)),
                CornerRadius::same(3),
                c,
            );
            let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(11.5), p.text_secondary);
            let w = g.size().x;
            ui.painter().galley(pos2(x + 14.0, lr.center().y - g.size().y / 2.0), g, p.text_secondary);
            x += w + 30.0;
        }
        // Delete confirmation bar.
        if let Some(id) = self.confirm_delete {
            let cr = Rect::from_min_max(pos2(area.left(), area.bottom() - confirm_h + 4.0), area.max);
            ui.painter().rect_filled(cr, CornerRadius::same(8), p.danger.gamma_multiply(0.12));
            let n = &self.tree.nodes[id];
            let msg = tr!("tools-treemap-confirm-trash", name = n.label(), size = format_size(n.bytes, style));
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
                let path = n.path.clone();
                app.cmds.push(Cmd::FileOp(FileOpCmd::Delete { paths: vec![path], mode: DeleteMode::Trash }));
                if self.tree.is_under(self.current, id) {
                    self.current = self.tree.nodes[id].parent.unwrap_or(0);
                }
                self.tree.remove(id);
                self.selected = None;
                self.confirm_delete = None;
            }
            if button(&mut bu, None, tr!("common-cancel"), 0, true).clicked() {
                self.confirm_delete = None;
            }
        }
    }

    fn treemap(&mut self, ui: &mut Ui, rect: Rect, app: &mut SoyuzApp) {
        let p = palette(ui.ctx());
        ui.painter().rect_filled(rect, CornerRadius::same(8), p.bg_sunken);
        let resp = ui.interact(rect, ui.id().with("treemap.area"), Sense::click());
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
            style: app.config.general.size_style,
        };
        self.layout_level(ui, &p, inner, self.current, 0, &mut out);
        let painter = ui.painter_at(rect);
        // Frame backgrounds, then the cushions, then frame borders and labels.
        painter.extend(bgs);
        painter.add(Shape::mesh(mesh));
        painter.extend(borders);
        painter.extend(labels);
        // Screen readers: the map is a list of its tiles (name and size).
        if crate::ui::a11y::active(ui.ctx()) {
            crate::ui::a11y::container(ui.ctx(), resp.id, egui::accesskit::Role::ListBox, tr!("tools-treemap-title"));
            let style = app.config.general.size_style;
            let total = hits.len();
            for (pos, (r, id)) in hits.iter().enumerate() {
                let n = &self.tree.nodes[*id];
                let label = tr!("tools-treemap-tile-a11y", name = n.label(), size = format_size(n.bytes, style));
                let aid = ui.id().with(("treemap.tile", *id));
                crate::ui::a11y::list_item(ui, aid, *r, &label, self.selected == Some(*id), pos, total);
            }
        }
        // Live highlights of what the assistant / MCP clients read or change. A folder
        // tile whose contents are not drawn shows the strongest mark inside it.
        if crate::ai::touch::any() {
            let shown: std::collections::HashSet<usize> = hits.iter().map(|(_, id)| *id).collect();
            let reduced = app.ui.reduced_motion;
            for (r, id) in &hits {
                let n = &self.tree.nodes[*id];
                let opened = n.children.iter().any(|c| shown.contains(c));
                let t = if n.is_dir && !opened {
                    crate::ai::touch::get_within(&n.path)
                } else {
                    crate::ai::touch::get(&n.path)
                };
                if let Some(t) = t {
                    crate::ai::touch::paint_touch(&painter, &p, *r, t, reduced);
                }
            }
        }

        let n = &self.tree.nodes[self.current];
        if n.children.is_empty() {
            let (glyph, text) = if n.loading || self.scanning.is_some() {
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
        highlight(self.selected, p.accent, 2.0);
        highlight(self.hover, Color32::WHITE.gamma_multiply(0.9), 1.5);
        if let Some(id) = self.hover {
            let parent_bytes = self.tree.nodes[id].parent.map(|pp| self.tree.nodes[pp].bytes).unwrap_or(0);
            let cur_bytes = self.tree.nodes[self.current].bytes;
            let tree = &self.tree;
            let style = app.config.general.size_style;
            resp.clone().on_hover_ui_at_pointer(|ui| tooltip(ui, tree, id, parent_bytes, cur_bytes, style));
        }
        if resp.clicked()
            && let Some(id) = self.hover
        {
            // Clicking a tile drills into the top-level child of the current view that contains it.
            let chain = self.tree.chain(id);
            let pos = chain.iter().position(|&c| c == self.current).unwrap_or(0);
            let target = if self.tree.nodes[id].is_dir { id } else { chain.get(pos + 1).copied().unwrap_or(id) };
            if self.tree.nodes[target].is_dir && !self.tree.nodes[target].other {
                self.enter(target);
            } else {
                self.selected = Some(id);
            }
        }
        if resp.double_clicked()
            && let Some(id) = self.hover
            && !self.tree.nodes[id].is_dir
            && !self.tree.nodes[id].other
        {
            let path = self.tree.nodes[id].path.clone();
            crate::ops::launch(app, &path);
        }
        if resp.secondary_clicked() {
            self.menu_target = self.hover;
        }
        if let Some(target) = self.menu_target {
            resp.context_menu(|ui| self.menu(ui, app, target));
        }
    }

    fn layout_level(&self, ui: &Ui, p: &Palette, rect: Rect, node: usize, depth: usize, out: &mut Out<'_>) {
        let style = out.style;
        let kids: Vec<usize> =
            self.tree.nodes[node].children.iter().copied().filter(|&k| self.tree.nodes[k].bytes > 0).collect();
        if kids.is_empty() || rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        let weights: Vec<f64> = kids.iter().map(|&k| self.tree.nodes[k].bytes as f64).collect();
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
            let n = &self.tree.nodes[*k];
            let nested =
                n.is_dir && !n.children.is_empty() && depth < MAX_NEST && r.width() > 44.0 && r.height() > 34.0;
            out.hits.push((r, *k));
            if nested {
                let fc = p.glyph_folder;
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
                let base = class_color(n.class, p);
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
                let mut on = self.tab == t;
                let r = super::toggle_chip(ui, label, &mut on, p.accent);
                if r.clicked() {
                    self.tab = t;
                }
            }
        });
        ui.add_space(4.0);
        let items: Vec<usize> = match self.tab {
            SideTab::Contents => self.tree.nodes[self.current].children.clone(),
            SideTab::Largest => self.tree.largest_files(self.current, 100),
        };
        if items.is_empty() {
            let msg = if self.tree.nodes[self.current].loading {
                tr!("tools-treemap-measuring")
            } else {
                tr!("tools-treemap-no-files")
            };
            ui.add_space(12.0);
            ui.label(egui::RichText::new(msg).font(FontId::proportional(12.0)).color(p.text_tertiary));
        }
        let total = self.tree.nodes[self.current].bytes.max(1);
        let mut hover = None;
        let mut click = None;
        egui::ScrollArea::vertical().id_salt("treemap.side").auto_shrink([false, false]).show_rows(
            ui,
            40.0,
            items.len(),
            |ui, range| {
                for &id in &items[range] {
                    let n = &self.tree.nodes[id];
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    let painter = ui.painter();
                    let sel = self.selected == Some(id);
                    if sel {
                        painter.rect_filled(r, CornerRadius::same(6), p.accent_subtle);
                    } else if resp.hovered() {
                        painter.rect_filled(r, CornerRadius::same(6), p.bg_surface3);
                    }
                    if resp.hovered() {
                        hover = Some(id);
                    }
                    let c = class_color(n.class, &p);
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
                    if self.tab == SideTab::Largest {
                        let rel = n
                            .path
                            .strip_prefix(&self.tree.nodes[self.current].path)
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
            if self.tree.nodes[id].is_dir && !self.tree.nodes[id].other {
                self.enter(id);
            } else {
                self.selected = Some(id);
            }
        }
    }

    fn menu(&mut self, ui: &mut Ui, app: &mut SoyuzApp, id: usize) {
        ui.set_width(230.0);
        let n = self.tree.nodes[id].clone();
        if n.other {
            menu_item(ui, Some(ph::INFO), tr!("tools-treemap-many-small"), None, false);
            return;
        }
        if n.is_dir {
            if menu_item(ui, Some(ph::MAGNIFYING_GLASS_PLUS), tr!("tools-treemap-zoom-in"), None, true).clicked() {
                self.enter(id);
                ui.close();
            }
            if menu_item(ui, Some(ph::FOLDER_OPEN), tr!("tools-open-in-pane"), None, true).clicked() {
                let pane = app.ws.pane().id;
                app.cmds.push(Cmd::Navigate { pane, location: Location::Dir(n.path.clone()), select: None });
                ui.close();
            }
        } else if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("tools-open"), None, true).clicked() {
            crate::ops::launch(app, &n.path);
            ui.close();
        }
        if menu_item(ui, Some(ph::ARROW_SQUARE_RIGHT), tr!("tools-shelf-reveal"), None, true).clicked() {
            app.cmds.push(Cmd::RevealPath(n.path.clone()));
            ui.close();
        }
        if menu_item(ui, Some(ph::ARROW_SQUARE_UP_RIGHT), tr!("tools-reveal-in-system"), None, true).clicked() {
            app.cmds.push(Cmd::RevealInSystem(n.path.clone()));
            ui.close();
        }
        if menu_item(ui, Some(ph::CLIPBOARD_TEXT), tr!("tools-copy-path"), None, true).clicked() {
            app.cmds.push(Cmd::CopyText(super::tags_ui::show_path(&n.path)));
            ui.close();
        }
        if menu_item(ui, Some(ph::TRAY_ARROW_DOWN), tr!("tools-menu-add-to-shelf"), None, true).clicked() {
            super::queue(super::ToolCmd::ShelfAdd(vec![n.path.clone()]));
            ui.close();
        }
        menu_separator(ui);
        if menu_item(ui, Some(ph::TRASH), tr!("tools-move-to-trash-dialog"), None, id != 0).clicked() {
            self.confirm_delete = Some(id);
            ui.close();
        }
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

fn tooltip(ui: &mut Ui, tree: &Tree, id: usize, parent_bytes: u64, cur_bytes: u64, style: SizeStyle) {
    let p = palette(ui.ctx());
    let n = &tree.nodes[id];
    ui.set_max_width(360.0);
    ui.horizontal(|ui| {
        ui.label(
            icons::icon(if n.is_dir { ph::FOLDER_SIMPLE } else { ph::FILE }, 14.0).color(class_color(n.class, &p)),
        );
        ui.label(egui::RichText::new(n.label()).font(crate::ui::views::flow::semibold(13.0)).color(p.text_primary));
    });
    if !n.other {
        ui.label(
            egui::RichText::new(super::tags_ui::show_path(&n.path))
                .font(FontId::proportional(11.5))
                .color(p.text_tertiary),
        );
    }
    let pct = |b: u64, of: u64| if of == 0 { 0.0 } else { b as f64 * 100.0 / of as f64 };
    ui.label(
        egui::RichText::new(format_size(n.bytes, style))
            .font(crate::ui::views::flow::semibold(14.0))
            .color(p.text_primary),
    );
    ui.label(
        egui::RichText::new(tr!(
            "tools-treemap-share",
            parent = percent(pct(n.bytes, parent_bytes)),
            view = percent(pct(n.bytes, cur_bytes))
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

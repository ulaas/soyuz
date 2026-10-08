//! Duplicate finder (PLAN M6/M12): pick roots and a minimum size, then
//! `soyuz_search::find_duplicates` streams groups of identical files (size → head hash →
//! full BLAKE3) while it runs. Each group offers keep-one helpers (newest, oldest,
//! shortest path); marked files go to the trash through the jobs engine. A group can
//! never have all of its copies marked. Files leave the list only once the trash job has
//! finished and they are really gone (failed / skipped items stay, still marked).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use crossbeam_channel::Receiver;
use egui::{Align, Align2, CornerRadius, FontId, Layout, Rect, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2};
use soyuz_core::{EntryInfo, OpControl, SizeStyle, format_size, format_time};
use soyuz_jobs::{DeleteMode, JobId, JobKind};
use soyuz_search::DupeEvent;

use super::{button, plural, progress_bar, text_field, tool_window};
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::services::jobs::{JobMeta, OpKind};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::toasts::Toast;
use crate::ui::views::icons as file_icons;
use crate::ui::widgets::trunc_galley;

/// Which copy a helper keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    Newest,
    Oldest,
    ShortestPath,
}

impl Keep {
    pub fn label(self) -> &'static str {
        match self {
            Keep::Newest => "Keep newest",
            Keep::Oldest => "Keep oldest",
            Keep::ShortestPath => "Keep shortest path",
        }
    }
}

/// Index of the file to keep in a group.
pub fn keeper(files: &[EntryInfo], keep: Keep) -> usize {
    let t = |e: &EntryInfo| e.modified.unwrap_or(SystemTime::UNIX_EPOCH);
    let len = |e: &EntryInfo| e.path.as_os_str().len();
    let idx = 0..files.len();
    match keep {
        Keep::Newest => idx.max_by(|&a, &b| t(&files[a]).cmp(&t(&files[b])).then(b.cmp(&a))),
        Keep::Oldest => idx.min_by(|&a, &b| t(&files[a]).cmp(&t(&files[b])).then(a.cmp(&b))),
        Keep::ShortestPath => idx.min_by(|&a, &b| len(&files[a]).cmp(&len(&files[b])).then(a.cmp(&b))),
    }
    .unwrap_or(0)
}

pub struct Group {
    pub files: Vec<EntryInfo>,
    pub marked: Vec<bool>,
    pub collapsed: bool,
}

impl Group {
    pub fn size(&self) -> u64 {
        self.files.first().map_or(0, |e| e.size)
    }

    pub fn wasted(&self) -> u64 {
        self.size() * self.files.len().saturating_sub(1) as u64
    }

    /// Mark everything except the keeper.
    pub fn apply(&mut self, keep: Keep) {
        let k = keeper(&self.files, keep);
        for (i, m) in self.marked.iter_mut().enumerate() {
            *m = i != k;
        }
    }

    /// Toggle a mark, refusing to mark the last unmarked copy. Returns false when refused.
    pub fn toggle(&mut self, i: usize) -> bool {
        if !self.marked[i] && self.marked.iter().filter(|m| !**m).count() <= 1 {
            return false;
        }
        self.marked[i] = !self.marked[i];
        true
    }
}

const SIZES: [(&str, u64); 5] =
    [("Any", 1), ("≥ 100 KB", 100_000), ("≥ 1 MB", 1_000_000), ("≥ 10 MB", 10_000_000), ("≥ 100 MB", 100_000_000)];

struct Running {
    rx: Receiver<DupeEvent>,
    ctl: Arc<OpControl>,
    started: Instant,
    hashed: u64,
    total: u64,
    bytes: u64,
}

pub struct DupesWindow {
    pub open: bool,
    roots: Vec<PathBuf>,
    new_root: String,
    min_idx: usize,
    running: Option<Running>,
    groups: Vec<Group>,
    done: Option<(u64, u64, f32)>,
    confirm: bool,
    refused: Option<(usize, usize, Instant)>,
    /// Files sent to the trash from this window.
    removed: usize,
    /// Trash jobs in flight and the paths each was asked to remove.
    trashing: Vec<(JobId, Vec<PathBuf>)>,
}

impl DupesWindow {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        let mut w = DupesWindow {
            open: true,
            roots,
            new_root: String::new(),
            min_idx: 1,
            running: None,
            groups: Vec::new(),
            done: None,
            confirm: false,
            refused: None,
            removed: 0,
            trashing: Vec::new(),
        };
        if !w.roots.is_empty() {
            w.start();
        }
        w
    }

    fn start(&mut self) {
        self.stop();
        self.removed = 0;
        self.groups.clear();
        self.done = None;
        self.confirm = false;
        if self.roots.is_empty() {
            return;
        }
        let (rx, ctl) = soyuz_search::find_duplicates(self.roots.clone(), SIZES[self.min_idx].1, super::wake());
        self.running = Some(Running { rx, ctl, started: Instant::now(), hashed: 0, total: 0, bytes: 0 });
    }

    fn stop(&mut self) {
        if let Some(r) = self.running.take() {
            r.ctl.cancel();
        }
    }

    fn poll(&mut self) {
        let Some(r) = &mut self.running else { return };
        let mut finished = None;
        loop {
            match r.rx.try_recv() {
                Ok(DupeEvent::Progress { hashed, total, bytes }) => {
                    r.hashed = hashed;
                    r.total = total;
                    r.bytes = bytes;
                }
                Ok(DupeEvent::Group(mut files)) => {
                    files.sort_by(|a, b| a.path.cmp(&b.path));
                    let n = files.len();
                    self.groups.push(Group { files, marked: vec![false; n], collapsed: false });
                }
                Ok(DupeEvent::Done { groups, wasted_bytes }) => {
                    finished = Some((groups, wasted_bytes, r.started.elapsed().as_secs_f32()));
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    finished = Some((self.groups.len() as u64, 0, r.started.elapsed().as_secs_f32()));
                    break;
                }
            }
        }
        if let Some(f) = finished {
            self.running = None;
            self.done = Some(f);
            self.groups.sort_by_key(|g| std::cmp::Reverse(g.wasted()));
        }
    }

    /// Drop the files of finished trash jobs that are really gone (one existence check
    /// per file, once per finished job).
    fn poll_trash(&mut self, app: &SoyuzApp) {
        let finished: Vec<usize> = (0..self.trashing.len())
            .filter(|&i| app.fileops.jobs.snapshot(self.trashing[i].0).is_none_or(|s| s.state.is_terminal()))
            .collect();
        for i in finished.into_iter().rev() {
            let (_, paths) = self.trashing.remove(i);
            let gone: HashSet<PathBuf> = paths.into_iter().filter(|p| !soyuz_fs::exists_nofollow(p)).collect();
            self.removed += gone.len();
            for g in &mut self.groups {
                let keep: Vec<(EntryInfo, bool)> =
                    g.files.drain(..).zip(g.marked.drain(..)).filter(|(e, _)| !gone.contains(&e.path)).collect();
                for (e, m) in keep {
                    g.files.push(e);
                    g.marked.push(m);
                }
            }
            self.groups.retain(|g| g.files.len() > 1);
        }
    }

    fn marked(&self) -> (usize, u64, Vec<PathBuf>) {
        let mut n = 0;
        let mut bytes = 0;
        let mut paths = Vec::new();
        for g in &self.groups {
            for (i, m) in g.marked.iter().enumerate() {
                if *m {
                    n += 1;
                    bytes += g.files[i].size;
                    paths.push(g.files[i].path.clone());
                }
            }
        }
        (n, bytes, paths)
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        self.poll();
        self.poll_trash(app);
        let sub = match self.roots.as_slice() {
            [] => String::new(),
            [one] => super::tags_ui::show_path(one),
            many => format!("{} folders", many.len()),
        };
        let keep =
            tool_window(ctx, "dupes", "Duplicate finder", &sub, ph::FILES, vec2(900.0, 640.0), |ui| self.body(ui, app));
        if !keep {
            self.stop();
            self.open = false;
        }
    }

    fn body(&mut self, ui: &mut Ui, app: &mut SoyuzApp) {
        let p = palette(ui.ctx());
        // Roots.
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Search in").font(FontId::proportional(12.0)).color(p.text_tertiary));
            let mut remove = None;
            for (i, r) in self.roots.iter().enumerate() {
                let label = format!("{}  ×", soyuz_core::entry::display_name(r));
                if super::chip(ui, &label, p.accent)
                    .on_hover_text(format!("{}\nClick to remove", r.display()))
                    .clicked()
                {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                self.roots.remove(i);
            }
            let r = text_field(ui, &mut self.new_root, "Add a folder path…", 220.0);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (enter || button(ui, Some(ph::PLUS), "Add", 0, !self.new_root.trim().is_empty()).clicked())
                && !self.new_root.trim().is_empty()
            {
                let pb = PathBuf::from(self.new_root.trim());
                if !self.roots.contains(&pb) {
                    self.roots.push(pb);
                }
                self.new_root.clear();
            }
            if let Some(other) = other_pane_dir(app)
                && !self.roots.contains(&other)
                && button(ui, Some(ph::SQUARE_SPLIT_HORIZONTAL), "Add other pane", 0, true).clicked()
            {
                self.roots.push(other);
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Minimum size").font(FontId::proportional(12.0)).color(p.text_tertiary));
            for (i, (label, _)) in SIZES.iter().enumerate() {
                let mut on = self.min_idx == i;
                if super::toggle_chip(ui, label, &mut on, p.accent).clicked() {
                    self.min_idx = i;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.running.is_some() {
                    if button(ui, Some(ph::STOP), "Stop", 0, true).clicked() {
                        self.stop();
                        self.done = Some((self.groups.len() as u64, 0, 0.0));
                        self.groups.sort_by_key(|g| std::cmp::Reverse(g.wasted()));
                    }
                } else if button(ui, Some(ph::MAGNIFYING_GLASS), "Find duplicates", 1, !self.roots.is_empty()).clicked()
                {
                    self.start();
                }
            });
        });
        ui.add_space(4.0);
        // Progress / summary line.
        let wasted: u64 = self.groups.iter().map(Group::wasted).sum();
        if let Some(r) = &self.running {
            let frac = (r.total > 0).then(|| r.hashed as f32 / r.total as f32);
            progress_bar(ui, ui.available_width(), frac);
            ui.label(
                RichText::new(format!(
                    "Hashing {} of {} candidates · {} read · {} groups so far · {:.0} s",
                    r.hashed,
                    r.total,
                    format_size(r.bytes, SizeStyle::Binary),
                    self.groups.len(),
                    r.started.elapsed().as_secs_f32()
                ))
                .font(FontId::proportional(12.0))
                .color(p.text_secondary),
            );
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        } else if let Some((_, _, secs)) = self.done {
            ui.label(
                RichText::new(format!(
                    "{} · {} reclaimable{}",
                    plural(self.groups.len(), "group of duplicates", "groups of duplicates"),
                    format_size(wasted, SizeStyle::Binary),
                    if secs >= 1.0 {
                        format!(" · {secs:.1} s")
                    } else if secs > 0.0 {
                        format!(" · {} ms", (secs * 1000.0).round())
                    } else {
                        String::new()
                    }
                ))
                .font(FontId::proportional(12.5))
                .color(p.text_primary),
            );
        }
        if self.groups.is_empty() {
            let (g, t, s) = if self.running.is_some() {
                (ph::SPINNER, "Looking for duplicates…", "Groups appear here as soon as they are confirmed.")
            } else if self.done.is_some() && self.removed > 0 {
                (
                    ph::SPARKLE,
                    "All duplicates handled",
                    "The marked copies were moved to the trash (Undo is in the toast).",
                )
            } else if self.done.is_some() {
                (ph::CHECK_CIRCLE, "No duplicates found", "Every file under these folders is unique.")
            } else {
                (ph::FILES, "Find identical files", "Choose folders and a minimum size, then start.")
            };
            super::compare::empty_note(ui, g, t, s);
            return;
        }
        // Bulk helpers.
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Select in every group:").font(FontId::proportional(12.0)).color(p.text_tertiary));
            for k in [Keep::Newest, Keep::Oldest, Keep::ShortestPath] {
                if button(ui, None, k.label(), 0, true).clicked() {
                    for g in &mut self.groups {
                        g.apply(k);
                    }
                    self.confirm = false;
                }
            }
            if button(ui, None, "Clear", 0, true).clicked() {
                for g in &mut self.groups {
                    g.marked.iter_mut().for_each(|m| *m = false);
                }
                self.confirm = false;
            }
        });
        ui.add_space(4.0);
        let footer = 44.0;
        let h = (ui.available_height() - footer).max(80.0);
        self.list(ui, app, h);
        // Footer.
        let (n, bytes, paths) = self.marked();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let text = if n == 0 {
                "Mark the copies you want to remove".to_string()
            } else {
                format!("{} marked · {}", plural(n, "file", "files"), format_size(bytes, SizeStyle::Binary))
            };
            ui.label(RichText::new(text).font(FontId::proportional(12.5)).color(if n > 0 {
                p.text_primary
            } else {
                p.text_tertiary
            }));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.confirm {
                    let busy = !self.trashing.is_empty();
                    if button(ui, Some(ph::TRASH), &format!("Move {n} to trash"), 2, n > 0 && !busy).clicked() {
                        let spec =
                            app.fileops.jobs.spec(JobKind::Delete { paths: paths.clone(), mode: DeleteMode::Trash });
                        let id = app.fileops.jobs.submit(spec, JobMeta::new(OpKind::Trash, &paths, None));
                        self.trashing.push((id, paths));
                        self.confirm = false;
                        app.ui
                            .toasts
                            .push(Toast::info(format!("Moving {} to the trash", plural(n, "duplicate", "duplicates"))));
                    }
                    if button(ui, None, "Cancel", 0, true).clicked() {
                        self.confirm = false;
                    }
                } else if button(ui, Some(ph::TRASH), "Delete marked…", 2, n > 0).clicked() {
                    self.confirm = true;
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
            .groups
            .iter()
            .map(|g| header_h + if g.collapsed { 0.0 } else { g.files.len() as f32 * row_h } + gap)
            .collect();
        let total: f32 = heights.iter().sum();
        let cfg = app.config.general.clone();
        let roots = self.roots.clone();
        let mut reveal = None;
        ScrollArea::vertical().id_salt("dupes.list").max_height(height).auto_shrink([false, false]).show_viewport(
            ui,
            |ui, viewport| {
                let width = ui.available_width();
                let (area, _) = ui.allocate_exact_size(vec2(width, total), Sense::hover());
                let mut y = 0.0;
                for (gi, g) in self.groups.iter_mut().enumerate() {
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
                    let hresp = ui.interact(hr, ui.id().with(("dupes.h", gi)), Sense::click());
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
                    let title = format!("{} copies · {} each", g.files.len(), format_size(g.size(), cfg.size_style));
                    painter.text(
                        pos2(hr.left() + 46.0, hr.center().y),
                        Align2::LEFT_CENTER,
                        title,
                        crate::ui::views::flow::semibold(12.5),
                        p.text_primary,
                    );
                    painter.text(
                        pos2(hr.left() + 230.0, hr.center().y),
                        Align2::LEFT_CENTER,
                        format!("{} reclaimable", format_size(g.wasted(), cfg.size_style)),
                        FontId::proportional(12.0),
                        p.success,
                    );
                    // Keep helpers (right side of the header).
                    let mut x = hr.right() - 8.0;
                    for k in [Keep::ShortestPath, Keep::Oldest, Keep::Newest] {
                        let label = match k {
                            Keep::Newest => "Newest",
                            Keep::Oldest => "Oldest",
                            Keep::ShortestPath => "Shortest path",
                        };
                        let gal = ui.painter().layout_no_wrap(
                            label.to_string(),
                            FontId::proportional(11.5),
                            p.text_secondary,
                        );
                        let w = gal.size().x + 16.0;
                        let r = Rect::from_min_size(pos2(x - w, hr.center().y - 11.0), vec2(w, 22.0));
                        x -= w + 4.0;
                        let resp = ui.interact(r, ui.id().with(("dupes.k", gi, label)), Sense::click());
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
                        "Keep",
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
                        let resp = ui.interact(rr, ui.id().with(("dupes.r", gi, fi)), Sense::click());
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
                        let refused =
                            self.refused.is_some_and(|(a, b, t)| a == gi && b == fi && t.elapsed().as_secs_f32() < 1.2);
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
                                "Keep at least one copy",
                                FontId::proportional(11.0),
                                p.warning,
                            );
                        }
                        if resp.clicked() && !g.toggle(fi) {
                            self.refused = Some((gi, fi, Instant::now()));
                        }
                        if resp.double_clicked() {
                            reveal = Some(e.path.clone());
                        }
                        let path = e.path.clone();
                        resp.on_hover_text(format!("{}\nClick to mark · double-click to reveal", path.display()))
                            .context_menu(|ui| {
                                if crate::ui::widgets::menu_item(
                                    ui,
                                    Some(ph::FOLDER_OPEN),
                                    "Reveal in pane",
                                    None,
                                    true,
                                )
                                .clicked()
                                {
                                    reveal = Some(path.clone());
                                    ui.close();
                                }
                                if crate::ui::widgets::menu_item(ui, Some(ph::CLIPBOARD_TEXT), "Copy path", None, true)
                                    .clicked()
                                {
                                    ui.ctx().copy_text(super::tags_ui::show_path(&path));
                                    ui.close();
                                }
                            });
                    }
                    y += gh;
                }
            },
        );
        if let Some(p) = reveal {
            app.cmds.push(Cmd::RevealPath(p));
        }
    }
}

/// A folder shown relative to the search root it lives in (`root\sub\dir`).
pub fn short_folder(roots: &[PathBuf], folder: &std::path::Path) -> String {
    for r in roots {
        if let Ok(rel) = folder.strip_prefix(r) {
            let name = soyuz_core::entry::display_name(r);
            return if rel.as_os_str().is_empty() {
                name
            } else {
                format!("{name}{}{}", std::path::MAIN_SEPARATOR, rel.display())
            };
        }
    }
    super::tags_ui::show_path(folder)
}

fn other_pane_dir(app: &SoyuzApp) -> Option<PathBuf> {
    let tab = app.ws.tab();
    if !tab.dual || tab.panes.len() < 2 {
        return None;
    }
    match &tab.panes[1 - tab.focused.min(1)].location {
        soyuz_core::Location::Dir(d) => Some(d.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn f(path: &str, secs: u64) -> EntryInfo {
        let mut e = EntryInfo::minimal(PathBuf::from(path), soyuz_core::EntryKind::File);
        e.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs));
        e.size = 100;
        e
    }

    #[test]
    fn keep_helpers_pick_one_and_mark_the_rest() {
        let files = vec![f("/a/long/path/x.jpg", 50), f("/b/x.jpg", 10), f("/c/d/x.jpg", 90)];
        assert_eq!(keeper(&files, Keep::Newest), 2);
        assert_eq!(keeper(&files, Keep::Oldest), 1);
        assert_eq!(keeper(&files, Keep::ShortestPath), 1);
        let mut g = Group { files, marked: vec![false; 3], collapsed: false };
        g.apply(Keep::Newest);
        assert_eq!(g.marked, vec![true, true, false]);
        assert_eq!(g.wasted(), 200);
        // The last unmarked copy cannot be marked.
        assert!(!g.toggle(2));
        assert!(g.toggle(0));
        assert!(g.toggle(2));
        assert_eq!(g.marked, vec![false, true, true]);
    }
}

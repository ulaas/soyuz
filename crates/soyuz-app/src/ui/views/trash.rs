//! The Trash location (`trash:`): items of every bin via `soyuz_fs::trash::list` (on a
//! worker thread), with original location, deletion date and size columns. Restore,
//! Empty trash and deleting individual items permanently (`JobKind::PurgeTrash`) run as
//! jobs, serialized with each other by the engine.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{Align2, CornerRadius, FontId, Id, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use soyuz_core::{EntryInfo, Location, format_size, format_time};
use soyuz_fs::TrashItem;

use super::ViewCtx;
use super::flow::{selection_fill, semibold};
use super::icons;
use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;
use crate::state::pane::Pane;
use crate::theme::tokens::radius;
use crate::theme::{icons as ticons, icons::ph, palette};
use crate::ui::empty_state::EmptyState;
use crate::ui::widgets::{menu_item, menu_separator, pill_button, spinner, trunc_galley};

enum TrashMsg {
    Listed(Result<Vec<TrashItem>, String>),
}

pub struct TrashState {
    items: Vec<TrashItem>,
    selected: BTreeSet<String>,
    anchor: Option<String>,
    loading: bool,
    loaded: bool,
    stale: bool,
    error: Option<String>,
    tx: Sender<TrashMsg>,
    rx: Receiver<TrashMsg>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// A message for the toast stack (set by `poll`, taken by the view).
    notice: Option<crate::ui::toasts::Toast>,
    last_load: Option<Instant>,
}

fn key(i: &TrashItem) -> String {
    format!("{}|{}", i.trash_root.display(), i.id)
}

impl TrashState {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = unbounded();
        let c = ctx.clone();
        TrashState {
            items: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            loading: false,
            loaded: false,
            stale: true,
            error: None,
            tx,
            rx,
            wake: Arc::new(move || c.request_repaint()),
            notice: None,
            last_load: None,
        }
    }

    pub fn items(&self) -> &[TrashItem] {
        &self.items
    }

    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// Load when shown for the first time or after a trash-related job.
    pub fn ensure_loaded(&mut self) {
        if (self.stale || !self.loaded) && !self.loading {
            self.reload();
        }
    }

    pub fn reload(&mut self) {
        self.loading = true;
        self.stale = false;
        self.last_load = Some(Instant::now());
        let tx = self.tx.clone();
        let wake = self.wake.clone();
        let spawned = std::thread::Builder::new().name("soyuz-trash-list".into()).spawn(move || {
            let r = soyuz_fs::trash::list().map_err(|e| e.to_string());
            let _ = tx.send(TrashMsg::Listed(r));
            wake();
        });
        if spawned.is_err() {
            self.loading = false;
            self.error = Some("Couldn't start the trash reader".into());
        }
    }

    pub fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                TrashMsg::Listed(Ok(items)) => {
                    self.items = items;
                    let keys: BTreeSet<String> = self.items.iter().map(key).collect();
                    self.selected.retain(|k| keys.contains(k));
                    self.loading = false;
                    self.loaded = true;
                    self.error = None;
                }
                TrashMsg::Listed(Err(e)) => {
                    self.loading = false;
                    self.loaded = true;
                    self.error = Some(e);
                }
            }
        }
    }

    pub fn take_notice(&mut self) -> Option<crate::ui::toasts::Toast> {
        self.notice.take()
    }

    pub fn selected_items(&self) -> Vec<TrashItem> {
        self.items.iter().filter(|i| self.selected.contains(&key(i))).cloned().collect()
    }

    fn click(&mut self, idx: usize, mods: egui::Modifiers) {
        let k = key(&self.items[idx]);
        if mods.shift
            && let Some(a) = self.anchor.clone()
            && let Some(ai) = self.items.iter().position(|i| key(i) == a)
        {
            let (lo, hi) = if ai <= idx { (ai, idx) } else { (idx, ai) };
            if !mods.command {
                self.selected.clear();
            }
            for i in &self.items[lo..=hi] {
                self.selected.insert(key(i));
            }
            return;
        }
        if mods.command {
            if !self.selected.remove(&k) {
                self.selected.insert(k.clone());
            }
        } else {
            self.selected.clear();
            self.selected.insert(k.clone());
        }
        self.anchor = Some(k);
    }
}

const HEADER_H: f32 = 56.0;
const COLS_H: f32 = 30.0;

pub fn show(ui: &mut Ui, rect: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    let p = palette(ui.ctx());
    if let Some(t) = v.fileops.trash.take_notice() {
        v.cmds.push(Cmd::Toast(t));
    }
    let bg = ui.interact(rect, Id::new(("soyuz.trash.bg", pane.id.0)), Sense::click());
    if bg.clicked() || bg.secondary_clicked() {
        v.cmds.push(Cmd::FocusPane(pane.id));
        if bg.clicked() {
            v.fileops.trash.selected.clear();
        }
    }
    let ts = &mut v.fileops.trash;

    // ---- header: title, counts, actions ----
    let hr = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H));
    let painter = ui.painter_at(rect);
    let icon_r = Rect::from_min_size(hr.min + vec2(14.0, 12.0), vec2(32.0, 32.0));
    painter.rect_filled(icon_r, CornerRadius::same(9), p.danger.gamma_multiply(0.12));
    painter.text(icon_r.center(), Align2::CENTER_CENTER, ph::TRASH, ticons::font_fill(17.0), p.danger);
    painter.text(
        pos2(icon_r.right() + 12.0, hr.top() + 20.0),
        Align2::LEFT_CENTER,
        "Trash",
        semibold(15.0),
        p.text_primary,
    );
    let total: u64 = ts.items.iter().map(|i| i.size).sum();
    let sub = if ts.loading && !ts.loaded {
        "Reading the trash…".to_string()
    } else if ts.items.is_empty() {
        "Empty".to_string()
    } else {
        let n = ts.items.len();
        let mut s = format!(
            "{} item{} \u{b7} {}",
            crate::ui::statusbar::group_thousands(n),
            if n == 1 { "" } else { "s" },
            format_size(total, v.config.general.size_style)
        );
        if !ts.selected.is_empty() {
            s.push_str(&format!(" \u{b7} {} selected", ts.selected.len()));
        }
        s
    };
    painter.text(
        pos2(icon_r.right() + 12.0, hr.top() + 38.0),
        Align2::LEFT_CENTER,
        sub,
        FontId::proportional(11.5),
        p.text_tertiary,
    );
    let has_sel = !ts.selected.is_empty();
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(icon_r.right() + 120.0, hr.top() + 14.0),
                pos2(hr.right() - 12.0, hr.top() + 42.0),
            ))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 8.0;
    // Narrow panes get icon-only actions.
    let compact = hr.width() < 640.0;
    let icon = |ui: &mut Ui, g: &str, tip: &str| crate::ui::widgets::icon_button(ui, g, tip, true);
    let empty_clicked = !ts.items.is_empty()
        && if compact {
            icon(&mut bar, ph::TRASH_SIMPLE, "Empty trash").clicked()
        } else {
            danger_pill(&mut bar, ph::TRASH_SIMPLE, "Empty trash", true)
                .on_hover_text("Delete everything in the trash permanently")
                .clicked()
        };
    if empty_clicked {
        v.cmds.push(Cmd::FileOp(FileOpCmd::ConfirmEmptyTrash));
    }
    let ts = &mut v.fileops.trash;
    if has_sel {
        let del = if compact {
            icon(&mut bar, ph::X_CIRCLE, "Delete permanently").clicked()
        } else {
            danger_pill(&mut bar, ph::X_CIRCLE, "Delete permanently", false).clicked()
        };
        if del {
            v.cmds.push(Cmd::FileOp(FileOpCmd::ConfirmPurge(ts.selected_items())));
        }
        let ts = &mut v.fileops.trash;
        let restore = if compact {
            pill_button(&mut bar, Some(ph::ARROW_COUNTER_CLOCKWISE), "Restore", true).clicked()
        } else {
            pill_button(&mut bar, Some(ph::ARROW_COUNTER_CLOCKWISE), "Restore selected", true).clicked()
        };
        if restore {
            v.cmds.push(Cmd::FileOp(FileOpCmd::TrashRestore(ts.selected_items())));
            ts.selected.clear();
        }
    }
    let ts = &mut v.fileops.trash;
    let refresh = crate::ui::widgets::icon_button(&mut bar, ph::ARROW_CLOCKWISE, "Refresh", !ts.loading);
    if refresh.clicked() {
        ts.reload();
    }
    if ts.loading {
        let (r, _) = bar.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        spinner(&bar, r.center(), 6.0, p.accent);
    }

    let body = Rect::from_min_max(pos2(rect.left(), hr.bottom()), rect.max);
    painter.line_segment(
        [pos2(rect.left() + 4.0, hr.bottom() - 0.5), pos2(rect.right() - 4.0, hr.bottom() - 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );
    if let Some(err) = ts.error.clone() {
        let mut child = ui.new_child(UiBuilder::new().max_rect(body));
        EmptyState {
            glyph: ph::WARNING_CIRCLE,
            glyph_color: Some(p.danger),
            title: "Can't read the trash",
            subtitle: &err,
            hints: &[],
        }
        .show(&mut child);
        return;
    }
    if ts.items.is_empty() {
        if ts.loaded {
            let mut child = ui.new_child(UiBuilder::new().max_rect(body));
            EmptyState {
                glyph: ph::TRASH,
                glyph_color: Some(p.text_secondary),
                title: "Trash is empty",
                subtitle: "Items you delete stay here until you empty the trash.",
                hints: &[],
            }
            .show(&mut child);
        } else {
            spinner(ui, body.center(), 11.0, p.accent);
        }
        return;
    }

    // ---- column header ----
    let w = body.width();
    let cols = [
        ("Name", body.left() + 10.0, w * 0.30),
        ("Original location", body.left() + 10.0 + w * 0.30, w * 0.38),
        ("Date deleted", body.left() + 10.0 + w * 0.68, w * 0.18),
        ("Size", body.left() + 10.0 + w * 0.86, w * 0.14 - 20.0),
    ];
    let ch = Rect::from_min_size(body.min, vec2(w, COLS_H));
    for (i, (label, x, cw)) in cols.iter().enumerate() {
        let align = if i == 3 { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
        let tx = if i == 3 { x + cw } else { x + 8.0 };
        painter.text(pos2(tx, ch.center().y), align, *label, FontId::proportional(12.0), p.text_secondary);
        if i > 0 {
            painter.line_segment(
                [pos2(*x - 1.0, ch.top() + 9.0), pos2(*x - 1.0, ch.bottom() - 9.0)],
                Stroke::new(1.0, p.border_subtle),
            );
        }
    }
    painter.line_segment(
        [pos2(body.left(), ch.bottom() - 0.5), pos2(body.right(), ch.bottom() - 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );

    // ---- rows ----
    let list = Rect::from_min_max(pos2(body.left(), ch.bottom()), body.max);
    let row_h = v.config.appearance.density.row_height();
    let mut child = ui.new_child(UiBuilder::new().max_rect(list));
    let n = ts.items.len();
    let focused = v.focused;
    let mut clicked: Option<(usize, egui::Modifiers)> = None;
    let mut menu_for: Option<(usize, egui::Response)> = None;
    ScrollArea::vertical().id_salt(("soyuz.trash.rows", pane.id.0)).auto_shrink([false, false]).show_rows(
        &mut child,
        row_h,
        n,
        |ui, range| {
            for i in range {
                let it = &ts.items[i];
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                let r = r.shrink2(vec2(4.0, 0.0));
                let selected = ts.selected.contains(&key(it));
                if selected {
                    ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), selection_fill(&p, focused));
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface2);
                }
                let name_path =
                    if it.original.as_os_str().is_empty() { PathBuf::from(&it.id) } else { it.original.clone() };
                let e = EntryInfo::minimal(name_path.clone(), it.kind);
                let cy = r.center().y;
                let font = if e.is_dir_like() { ticons::font_fill(16.0) } else { ticons::font(16.0) };
                ui.painter().text(
                    pos2(cols[0].1 + 16.0, cy),
                    Align2::CENTER_CENTER,
                    icons::glyph(&e),
                    font,
                    icons::color(&e, &p),
                );
                let g = trunc_galley(
                    ui.painter(),
                    &e.name,
                    FontId::proportional(13.0),
                    p.text_primary,
                    cols[0].2 - 40.0 - if soyuz_core::naming::has_hidden_controls(&e.name) { 14.0 } else { 0.0 },
                );
                let gw = g.size().x;
                ui.painter().galley(pos2(cols[0].1 + 32.0, cy - g.size().y / 2.0), g, p.text_primary);
                let resp = if crate::ui::widgets::paint_hidden_badge(
                    ui.painter(),
                    &p,
                    pos2(cols[0].1 + 32.0 + gw, cy),
                    &e.name,
                ) > 0.0
                {
                    resp.on_hover_text(crate::ui::widgets::HIDDEN_CONTROLS_TIP)
                } else {
                    resp
                };
                let origin = name_path
                    .parent()
                    .map(|d| soyuz_fs::strip_verbatim(d).to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty() && !it.original.as_os_str().is_empty())
                    .unwrap_or_else(|| "Unknown".into());
                let g =
                    trunc_galley(ui.painter(), &origin, FontId::proportional(12.5), p.text_secondary, cols[1].2 - 18.0);
                ui.painter().galley(pos2(cols[1].1 + 8.0, cy - g.size().y / 2.0), g, p.text_secondary);
                let when = format_time(it.deleted_at, &v.config.general.date_format);
                let g =
                    trunc_galley(ui.painter(), &when, FontId::proportional(12.5), p.text_secondary, cols[2].2 - 18.0);
                ui.painter().galley(pos2(cols[2].1 + 8.0, cy - g.size().y / 2.0), g, p.text_secondary);
                let size = format_size(it.size, v.config.general.size_style);
                ui.painter().text(
                    pos2(cols[3].1 + cols[3].2, cy),
                    Align2::RIGHT_CENTER,
                    size,
                    FontId::proportional(12.5),
                    p.text_secondary,
                );
                if resp.clicked() {
                    clicked = Some((i, ui.input(|inp| inp.modifiers)));
                }
                if resp.secondary_clicked() {
                    if !selected {
                        clicked = Some((i, egui::Modifiers::NONE));
                    }
                    menu_for = Some((i, resp.clone()));
                }
                let resp = resp.on_hover_text(soyuz_fs::strip_verbatim(&it.original).display().to_string());
                if menu_for.as_ref().is_some_and(|(mi, _)| *mi == i) || resp.context_menu_opened() {
                    menu_for = Some((i, resp));
                }
            }
        },
    );
    if let Some((i, mods)) = clicked {
        ts.click(i, mods);
        v.cmds.push(Cmd::FocusPane(pane.id));
    }
    if let Some((i, resp)) = menu_for {
        let original = ts.items.get(i).map(|it| it.original.clone()).unwrap_or_default();
        let cmds = &mut *v.cmds;
        let selected = ts.selected_items();
        let pane_id = pane.id;
        resp.context_menu(|ui| {
            ui.set_width(230.0);
            if menu_item(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), "Restore", None, true).clicked() {
                cmds.push(Cmd::FileOp(FileOpCmd::TrashRestore(selected.clone())));
                ui.close();
            }
            if let Some(dir) = original.parent().filter(|d| !d.as_os_str().is_empty())
                && menu_item(ui, Some(ph::FOLDER_OPEN), "Open original location", None, true).clicked()
            {
                cmds.push(Cmd::Navigate { pane: pane_id, location: Location::Dir(dir.to_path_buf()), select: None });
                ui.close();
            }
            if menu_item(ui, Some(ph::CLIPBOARD_TEXT), "Copy original path", None, true).clicked() {
                cmds.push(Cmd::CopyText(soyuz_fs::strip_verbatim(&original).to_string_lossy().into_owned()));
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::X_CIRCLE), "Delete permanently", None, true).clicked() {
                cmds.push(Cmd::FileOp(FileOpCmd::ConfirmPurge(selected.clone())));
                ui.close();
            }
        });
    }
}

/// Pill button in the danger style (filled when `strong`).
pub fn danger_pill(ui: &mut Ui, icon: &str, label: &str, strong: bool) -> egui::Response {
    let p = palette(ui.ctx());
    let font = FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, p.danger);
    let size = vec2(galley.size().x + 44.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hover = resp.hovered();
    let (bg, fg) = if strong {
        (if hover { p.danger } else { p.danger.gamma_multiply(0.85) }, egui::Color32::WHITE)
    } else {
        (if hover { p.danger.gamma_multiply(0.18) } else { p.danger.gamma_multiply(0.10) }, p.danger)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), bg);
    if !strong {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::ROW),
            Stroke::new(1.0, p.danger.gamma_multiply(0.4)),
            StrokeKind::Inside,
        );
    }
    ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, icon, ticons::font(14.0), fg);
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), fg);
    ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - g.size().y / 2.0), g, fg);
    resp
}

//! Two-file diff view (roadmap F3): shown by the preview pane and Quick Look when exactly
//! two text files are selected (`[preview] diff`). Unified or side-by-side, virtualized
//! rows, changed words emphasized. The diff itself runs on a worker
//! ([`crate::services::preview_diff`]).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontId, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use soyuz_core::EntryInfo;

use crate::app::SoyuzApp;
use crate::services::preview::{Kind, kind_by_name};
use crate::services::preview_diff::{DiffClient, DiffModel, Line, Row, SideRow, Tag};
use crate::theme::icons::ph;
use crate::theme::{Palette, palette};
use crate::ui::widgets::{segmented, spinner, trunc_galley};

type Key = (PathBuf, Option<SystemTime>, u64, PathBuf, Option<SystemTime>, u64);

/// One consumer's diff state.
#[derive(Default)]
pub struct DiffView {
    client: DiffClient,
    key: Option<Key>,
    result: Option<Result<Arc<DiffModel>, String>>,
    /// `None` = automatic (side by side when wide).
    side: Option<bool>,
    /// Compare the other way round (newer → older).
    swapped: bool,
}

/// The two selected files of the focused pane, when the diff preview applies.
pub fn pair(app: &SoyuzApp) -> Option<(EntryInfo, EntryInfo)> {
    if !app.config.preview.diff {
        return None;
    }
    let pane = app.ws.pane();
    if pane.selection.len() != 2 {
        return None;
    }
    let sel = pane.selected_entries();
    let [a, b] = sel.as_slice() else { return None };
    // Older file on the left (the "before"), when the times tell.
    let (a, b) = match (a.modified, b.modified) {
        (Some(ta), Some(tb)) if tb < ta => (b, a),
        _ => (a, b),
    };
    let texty = |e: &EntryInfo| {
        !e.is_dir_like() && matches!(kind_by_name(&e.name), None | Some(Kind::Text | Kind::Markdown | Kind::Svg))
    };
    (texty(a) && texty(b)).then(|| ((*a).clone(), (*b).clone()))
}

impl DiffView {
    pub fn clear(&mut self) {
        self.client.cancel();
        self.key = None;
        self.result = None;
    }

    fn sync(&mut self, a: &EntryInfo, b: &EntryInfo, cap: u64, ctx: &egui::Context) {
        let key = (a.path.clone(), a.modified, a.size, b.path.clone(), b.modified, b.size);
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.result = None;
            self.client.request(a.path.clone(), b.path.clone(), cap, ctx);
        }
        while let Some(r) = self.client.try_recv() {
            if r.id == self.client.latest() {
                self.result = Some(r.result);
            }
        }
    }
}

/// Draw the diff of `a` → `b` into `rect`.
pub fn show(ui: &mut Ui, rect: Rect, view: &mut DiffView, a: &EntryInfo, b: &EntryInfo, cap_kib: u64) {
    let p = palette(ui.ctx());
    let (a, b) = if view.swapped { (b, a) } else { (a, b) };
    view.sync(a, b, cap_kib.max(4) * 1024, ui.ctx());
    ui.painter().rect_filled(rect, 0.0, p.bg_sunken);
    // Header: names, stats, mode toggle.
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0));
    ui.painter().rect_filled(bar, 0.0, p.bg_surface1);
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, p.border_subtle));
    let wide = rect.width() > 760.0;
    let side = view.side.unwrap_or(wide);
    let toggle_w = 72.0 + 34.0;
    let names = format!("{}  →  {}", a.name, b.name);
    let ng =
        trunc_galley(ui.painter(), &names, FontId::proportional(12.0), p.text_primary, bar.width() - toggle_w - 150.0);
    ui.painter().galley(pos2(bar.left() + 12.0, bar.center().y - ng.size().y / 2.0), ng, p.text_primary);
    if let Some(Ok(m)) = &view.result {
        let stats_x = bar.right() - toggle_w - 20.0;
        let g_add = ui.painter().layout_no_wrap(format!("+{}", m.added), FontId::monospace(11.5), p.success);
        let g_del = ui.painter().layout_no_wrap(format!("−{}", m.removed), FontId::monospace(11.5), p.danger);
        let y = bar.center().y;
        let dw = g_del.size().x;
        ui.painter().galley(pos2(stats_x - dw, y - g_del.size().y / 2.0), g_del, p.danger);
        let aw = g_add.size().x;
        ui.painter().galley(pos2(stats_x - dw - 10.0 - aw, y - g_add.size().y / 2.0), g_add, p.success);
    }
    let swap_r = Rect::from_min_size(pos2(bar.right() - toggle_w - 8.0, bar.top() + 5.0), vec2(30.0, 30.0));
    let mut sc = ui.new_child(UiBuilder::new().max_rect(swap_r));
    if crate::ui::widgets::icon_toggle(&mut sc, ph::ARROWS_LEFT_RIGHT, tr!("views-diff-swap"), true, view.swapped)
        .clicked()
    {
        view.swapped = !view.swapped;
    }
    let tr = Rect::from_min_size(pos2(bar.right() - 72.0 - 8.0, bar.top() + 7.0), vec2(72.0, 26.0));
    let mut tc = ui.new_child(UiBuilder::new().max_rect(tr));
    if let Some(i) = segmented(
        &mut tc,
        &[(ph::ROWS, tr!("views-diff-unified")), (ph::COLUMNS, tr!("views-diff-side-by-side"))],
        usize::from(side),
        vec2(32.0, 22.0),
    ) {
        view.side = Some(i == 1);
    }
    let body = Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max);
    let m = match &view.result {
        None => {
            spinner(ui, body.center(), 11.0, p.accent);
            return;
        }
        Some(Err(e)) => {
            message(ui, body, tr!("views-diff-failed"), e, &p);
            return;
        }
        Some(Ok(m)) => m.clone(),
    };
    if m.binary {
        let t = if m.identical { tr!("views-diff-identical") } else { tr!("views-diff-binary-differ") };
        message(ui, body, t, tr!("views-diff-binary-sub"), &p);
        return;
    }
    if m.identical {
        message(ui, body, tr!("views-diff-none"), tr!("views-diff-none-sub"), &p);
        return;
    }
    let row_h = 18.0;
    let font = FontId::monospace(12.0);
    let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
    let mut c = ui.new_child(UiBuilder::new().max_rect(body));
    c.set_clip_rect(body.intersect(ui.clip_rect()));
    let n = if side { m.side.len() } else { m.unified.len() } + usize::from(m.truncated);
    let widest = m
        .unified
        .iter()
        .map(|r| match r {
            Row::Line(l) => l.segs.iter().map(|(_, s)| s.chars().count()).sum::<usize>(),
            Row::Hunk(h) => h.len(),
        })
        .max()
        .unwrap_or(0)
        .min(400);
    let gutter = char_w * 5.0 + 10.0;
    ScrollArea::both().id_salt(("diff", side, Arc::as_ptr(&m) as usize)).auto_shrink([false, false]).show_rows(
        &mut c,
        row_h,
        n,
        |ui, range| {
            let avail = ui.available_width();
            let content_w = if side {
                avail.max(2.0 * (gutter + 16.0 + char_w * widest as f32))
            } else {
                avail.max(2.0 * gutter + 24.0 + char_w * widest as f32)
            };
            for i in range {
                let (r, _) = ui.allocate_exact_size(vec2(content_w, row_h), Sense::hover());
                let rows = if side { m.side.len() } else { m.unified.len() };
                if i >= rows {
                    ui.painter().text(
                        pos2(r.left() + 12.0, r.center().y),
                        Align2::LEFT_CENTER,
                        tr!("views-diff-truncated"),
                        FontId::proportional(11.0),
                        p.text_tertiary,
                    );
                    continue;
                }
                if side {
                    match &m.side[i] {
                        SideRow::Hunk(h) => hunk(ui, r, h, &p),
                        SideRow::Pair(l, rr) => {
                            let half = r.width() / 2.0;
                            let lr = Rect::from_min_size(r.min, vec2(half, row_h));
                            let rr_r = Rect::from_min_size(pos2(r.left() + half, r.top()), vec2(half, row_h));
                            side_cell(ui, lr, l.as_ref(), true, gutter, &font, &p);
                            side_cell(ui, rr_r, rr.as_ref(), false, gutter, &font, &p);
                            ui.painter().line_segment(
                                [pos2(rr_r.left(), r.top()), pos2(rr_r.left(), r.bottom())],
                                Stroke::new(1.0, p.border_subtle),
                            );
                        }
                    }
                } else {
                    match &m.unified[i] {
                        Row::Hunk(h) => hunk(ui, r, h, &p),
                        Row::Line(l) => unified_line(ui, r, l, gutter, &font, &p),
                    }
                }
            }
        },
    );
}

fn message(ui: &Ui, r: Rect, title: &str, body: &str, p: &Palette) {
    let c = r.center();
    ui.painter().text(
        c - vec2(0.0, 26.0),
        Align2::CENTER_CENTER,
        ph::GIT_DIFF,
        crate::theme::icons::font_fill(30.0),
        p.text_tertiary,
    );
    ui.painter().text(
        c + vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        title,
        crate::ui::views::flow::semibold(13.0),
        p.text_primary,
    );
    ui.painter().text(c + vec2(0.0, 28.0), Align2::CENTER_CENTER, body, FontId::proportional(11.5), p.text_tertiary);
}

fn hunk(ui: &Ui, r: Rect, h: &str, p: &Palette) {
    ui.painter().rect_filled(r, 0.0, p.accent_subtle.gamma_multiply(0.7));
    ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, h, FontId::monospace(11.0), p.accent);
}

fn colors(tag: Tag, p: &Palette) -> (Color32, Color32) {
    let a = if p.dark { 0.16 } else { 0.12 };
    match tag {
        Tag::Equal => (Color32::TRANSPARENT, Color32::TRANSPARENT),
        Tag::Delete => (p.danger.gamma_multiply(a), p.danger.gamma_multiply(a * 2.6)),
        Tag::Insert => (p.success.gamma_multiply(a), p.success.gamma_multiply(a * 2.6)),
    }
}

fn line_job(l: &Line, font: &FontId, p: &Palette) -> LayoutJob {
    let (_, strong) = colors(l.tag, p);
    let mut job = LayoutJob::default();
    for (emph, s) in &l.segs {
        let mut f = TextFormat::simple(font.clone(), p.text_primary);
        if *emph && l.tag != Tag::Equal {
            f.background = strong;
        }
        job.append(&s.replace('\t', "    "), 0.0, f);
    }
    job
}

fn unified_line(ui: &Ui, r: Rect, l: &Line, gutter: f32, font: &FontId, p: &Palette) {
    let (bg, _) = colors(l.tag, p);
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(r, 0.0, bg);
    }
    let num = |n: Option<usize>| n.map(|n| n.to_string()).unwrap_or_default();
    let small = FontId::monospace(11.0);
    let dim = p.text_tertiary.gamma_multiply(0.8);
    ui.painter().text(
        pos2(r.left() + gutter - 6.0, r.center().y),
        Align2::RIGHT_CENTER,
        num(l.old),
        small.clone(),
        dim,
    );
    ui.painter().text(pos2(r.left() + 2.0 * gutter - 6.0, r.center().y), Align2::RIGHT_CENTER, num(l.new), small, dim);
    let sign = match l.tag {
        Tag::Equal => " ",
        Tag::Delete => "−",
        Tag::Insert => "+",
    };
    let sc = match l.tag {
        Tag::Delete => p.danger,
        Tag::Insert => p.success,
        Tag::Equal => p.text_tertiary,
    };
    ui.painter().text(pos2(r.left() + 2.0 * gutter + 4.0, r.center().y), Align2::LEFT_CENTER, sign, font.clone(), sc);
    let g = ui.painter().layout_job(line_job(l, font, p));
    ui.painter().galley(pos2(r.left() + 2.0 * gutter + 18.0, r.center().y - g.size().y / 2.0), g, p.text_primary);
}

fn side_cell(ui: &Ui, r: Rect, l: Option<&Line>, old: bool, gutter: f32, font: &FontId, p: &Palette) {
    let Some(l) = l else {
        // Filler on the side without a counterpart.
        ui.painter().rect_filled(r, 0.0, p.bg_surface2.gamma_multiply(0.6));
        return;
    };
    let (bg, _) = colors(l.tag, p);
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(r, 0.0, bg);
    }
    let n = if old { l.old } else { l.new };
    ui.painter().text(
        pos2(r.left() + gutter - 6.0, r.center().y),
        Align2::RIGHT_CENTER,
        n.map(|n| n.to_string()).unwrap_or_default(),
        FontId::monospace(11.0),
        p.text_tertiary.gamma_multiply(0.8),
    );
    let g = ui.painter().layout_job(line_job(l, font, p));
    let clip = r.intersect(ui.clip_rect());
    ui.painter().with_clip_rect(clip).galley(
        pos2(r.left() + gutter + 6.0, r.center().y - g.size().y / 2.0),
        g,
        p.text_primary,
    );
}

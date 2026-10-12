//! Plan previews: a before / after tree diff (added, moved, renamed and
//! deleted nodes colour-coded like the folder-compare tool; click a row to reveal and
//! preview it) and the rename diff table with the changed characters highlighted.

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use soyuz_ai::tools::Plan;
use soyuz_core::{EntryInfo, EntryKind};

use super::PanelAct;
use crate::theme::Palette;
use crate::theme::icons::{self, ph};
use crate::ui::views::icons as file_icons;

pub use crate::ai::diff::{Change, Node, char_diff, common_root, trees};
use crate::ai::diff::{changes, name_of};

/// Colour of a change (the folder-compare tool's colours).
pub fn change_color(c: Change, p: &Palette) -> Color32 {
    match c {
        Change::Added => p.success,
        Change::Moved => p.accent,
        Change::Renamed => p.info,
        Change::Deleted => p.danger,
        Change::Changed => p.warning,
        Change::Context => p.text_secondary,
    }
}

const ROW: f32 = 21.0;
const MAX_ROWS: usize = 160;

fn rows(ui: &mut Ui, p: &Palette, node: &Node, depth: usize, budget: &mut usize, acts: &mut Vec<PanelAct>, width: f32) {
    for (name, child) in &node.children {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let (r, resp) = ui.allocate_exact_size(vec2(width, ROW), Sense::click());
        let change = child.change.unwrap_or(Change::Context);
        let color = change_color(change, p);
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(4), p.bg_surface3);
        }
        if child.change.is_some() {
            ui.painter().rect_filled(
                Rect::from_min_size(r.min, vec2(2.5, r.height())),
                CornerRadius::same(1),
                color.gamma_multiply(0.9),
            );
            ui.painter().rect_filled(r, CornerRadius::same(4), color.gamma_multiply(0.06));
        }
        let x = r.left() + 6.0 + depth as f32 * 14.0;
        let e = EntryInfo::minimal(child.path.clone(), if child.dir { EntryKind::Dir } else { EntryKind::File });
        let (glyph, gcolor) = (file_icons::glyph(&e), file_icons::color(&e, p));
        ui.painter().text(pos2(x + 7.0, r.center().y), Align2::CENTER_CENTER, glyph, icons::font_fill(12.5), gcolor);
        let text_color = if change == Change::Context { p.text_secondary } else { p.text_primary };
        let mut job = LayoutJob::default();
        job.append(
            name,
            0.0,
            TextFormat {
                font_id: FontId::proportional(12.0),
                color: text_color,
                strikethrough: if change == Change::Deleted { Stroke::new(1.0, p.danger) } else { Stroke::NONE },
                ..Default::default()
            },
        );
        job.wrap.max_width = (r.right() - x - 76.0).max(20.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x + 18.0, r.center().y - g.size().y / 2.0), g, text_color);
        if change != Change::Context {
            ui.painter().text(
                pos2(r.right() - 6.0, r.center().y),
                Align2::RIGHT_CENTER,
                change.label(),
                FontId::proportional(10.5),
                color,
            );
        }
        let resp = resp.on_hover_text(tr!("ai-diff-reveal-tip", path = child.path.display().to_string()));
        if resp.clicked() {
            acts.push(PanelAct::Reveal(child.path.clone()));
        }
        rows(ui, p, child, depth + 1, budget, acts, width);
    }
}

fn column(ui: &mut Ui, p: &Palette, title: &str, node: &Node, acts: &mut Vec<PanelAct>, width: f32) {
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.label(egui::RichText::new(title).size(10.5).color(p.text_tertiary));
        if node.children.is_empty() {
            ui.label(egui::RichText::new("\u{2014}").color(p.text_tertiary));
            return;
        }
        let mut budget = MAX_ROWS;
        rows(ui, p, node, 0, &mut budget, acts, width);
        let total = node.count();
        if total > MAX_ROWS {
            ui.label(
                egui::RichText::new(tr!("ai-diff-more", count = total - MAX_ROWS)).size(11.0).color(p.text_tertiary),
            );
        }
    });
}

/// Before / after tree diff (two columns when there is room).
pub fn tree_diff(ui: &mut Ui, p: &Palette, plan: &Plan, acts: &mut Vec<PanelAct>) {
    let (root, before, after) = trees(plan);
    let (hr, hresp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    ui.painter().text(
        pos2(hr.left() + 6.0, hr.center().y),
        Align2::CENTER_CENTER,
        ph::FOLDER_OPEN,
        icons::font(11.5),
        p.text_tertiary,
    );
    let root_text = root.display().to_string();
    let g = crate::ui::widgets::trunc_galley(
        ui.painter(),
        &root_text,
        FontId::proportional(11.0),
        p.text_tertiary,
        hr.width() - 18.0,
    );
    ui.painter().galley(pos2(hr.left() + 16.0, hr.center().y - g.size().y / 2.0), g, p.text_tertiary);
    hresp.on_hover_text(root_text);
    let w = ui.available_width();
    let only_after = before.children.is_empty();
    let only_before = after.children.is_empty();
    if only_after || only_before {
        let title = if only_after { tr!("ai-diff-after") } else { tr!("ai-diff-before") };
        column(ui, p, title, if only_after { &after } else { &before }, acts, w);
    } else if w >= 420.0 {
        ui.horizontal_top(|ui| {
            let cw = (w - 12.0) / 2.0;
            column(ui, p, tr!("ai-diff-before"), &before, acts, cw);
            let (r, _) = ui.allocate_exact_size(vec2(1.0, 10.0), Sense::hover());
            ui.painter()
                .line_segment([r.center_top(), pos2(r.center().x, r.top() + 400.0)], Stroke::new(1.0, p.border_subtle));
            column(ui, p, tr!("ai-diff-after"), &after, acts, cw);
        });
    } else {
        column(ui, p, tr!("ai-diff-before"), &before, acts, w);
        ui.add_space(4.0);
        column(ui, p, tr!("ai-diff-after"), &after, acts, w);
    }
    legend(ui, p, plan);
}

fn legend(ui: &mut Ui, p: &Palette, plan: &Plan) {
    let seen = changes(plan);
    if seen.len() < 2 {
        return;
    }
    ui.horizontal(|ui| {
        for c in seen {
            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 3.5, change_color(c, p));
            ui.label(egui::RichText::new(c.label()).size(10.5).color(p.text_tertiary));
        }
    });
}

fn diff_job(parts: (&str, &str, &str), base: Color32, hl: Color32, bg: Color32, strike: bool, max_w: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let font = FontId::proportional(12.0);
    let plain = TextFormat { font_id: font.clone(), color: base, ..Default::default() };
    job.append(parts.0, 0.0, plain.clone());
    job.append(
        parts.1,
        0.0,
        TextFormat {
            font_id: font,
            color: hl,
            background: bg,
            strikethrough: if strike { Stroke::new(1.0, hl) } else { Stroke::NONE },
            ..Default::default()
        },
    );
    job.append(parts.2, 0.0, plain);
    job.wrap.max_width = max_w;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

/// Before / after rename table with the changed characters highlighted.
pub fn rename_table(ui: &mut Ui, p: &Palette, plan: &Plan, acts: &mut Vec<PanelAct>) {
    let w = ui.available_width();
    let col = (w - 30.0) / 2.0;
    let (h, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    ui.painter().text(
        pos2(h.left() + 24.0, h.center().y),
        Align2::LEFT_CENTER,
        tr!("ai-diff-current-name"),
        FontId::proportional(10.5),
        p.text_tertiary,
    );
    ui.painter().text(
        pos2(h.left() + 30.0 + col, h.center().y),
        Align2::LEFT_CENTER,
        tr!("ai-diff-new-name"),
        FontId::proportional(10.5),
        p.text_tertiary,
    );
    for (n, it) in plan.items.iter().enumerate().take(MAX_ROWS) {
        let (r, resp) = ui.allocate_exact_size(vec2(w, ROW + 2.0), Sense::click());
        if n % 2 == 1 {
            ui.painter().rect_filled(r, CornerRadius::same(4), p.bg_surface2.gamma_multiply(0.5));
        }
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(4), p.bg_surface3);
        }
        let (glyph, color) = match &it.conflict {
            Some(c) if c.is_blocking() => (ph::WARNING, p.danger),
            Some(_) => (ph::WARNING, p.warning),
            None => (ph::ARROW_RIGHT, p.text_tertiary),
        };
        ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::CENTER_CENTER, glyph, icons::font(11.5), color);
        let old = name_of(&it.src);
        let new = name_of(&it.dst);
        let (o, nn) = char_diff(&old, &new);
        let og = ui.painter().layout_job(diff_job(
            o,
            p.text_secondary,
            p.danger,
            p.danger.gamma_multiply(0.12),
            true,
            col - 6.0,
        ));
        ui.painter().galley(pos2(r.left() + 24.0, r.center().y - og.size().y / 2.0), og, p.text_secondary);
        let ng = ui.painter().layout_job(diff_job(
            nn,
            p.text_primary,
            p.success,
            p.success.gamma_multiply(0.14),
            false,
            col - 6.0,
        ));
        ui.painter().galley(pos2(r.left() + 30.0 + col, r.center().y - ng.size().y / 2.0), ng, p.text_primary);
        let mut tip = format!("{old}\n\u{2192} {new}");
        if let Some(c) = &it.conflict {
            tip.push_str(&format!("\n\n{c:?}"));
        }
        let resp = resp.on_hover_text(tip);
        if resp.clicked()
            && let Some(s) = &it.src
        {
            acts.push(PanelAct::Reveal(s.clone()));
        }
    }
    if plan.items.len() > MAX_ROWS {
        ui.label(
            egui::RichText::new(tr!("ai-diff-more", count = plan.items.len() - MAX_ROWS))
                .size(11.0)
                .color(p.text_tertiary),
        );
    }
}

//! Markdown rendering for the preview (blocks come from
//! `services::preview_text::parse_markdown`, parsed on the worker).

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Frame, Margin, Stroke, Ui, vec2};

use crate::services::preview_text::{Block, Span};
use crate::theme::Palette;

fn spans_job(spans: &[Span], size: f32, color: Color32, heading: bool, p: &Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    for s in spans {
        let font_id = if s.code {
            crate::theme::font::mono(size * 0.92)
        } else if heading {
            crate::theme::font::heading(size)
        } else if s.bold {
            crate::theme::font::semibold(size)
        } else {
            crate::theme::font::ui(size)
        };
        let mut fmt = TextFormat {
            font_id,
            color: if s.link { p.accent } else { color },
            italics: s.italic,
            ..Default::default()
        };
        if s.code {
            fmt.background = p.bg_surface2;
            fmt.color = p.text_primary;
        }
        if s.link {
            fmt.underline = Stroke::new(1.0, p.accent.gamma_multiply(0.5));
        }
        job.append(&s.text, 0.0, fmt);
    }
    job
}

/// Render `blocks` into `ui` (call inside a vertical scroll area).
pub fn show(ui: &mut Ui, blocks: &[Block], p: &Palette) {
    let width = ui.available_width();
    ui.spacing_mut().item_spacing.y = 6.0;
    for b in blocks {
        match b {
            Block::Heading(level, spans) => {
                let size = match level {
                    1 => 21.0,
                    2 => 17.5,
                    3 => 15.0,
                    _ => 13.5,
                };
                ui.add_space(if *level <= 2 { 8.0 } else { 4.0 });
                let mut job = spans_job(spans, size, p.text_primary, true, p);
                job.wrap.max_width = width;
                ui.label(job);
                if *level <= 2 {
                    let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 0.0, p.border_subtle);
                }
            }
            Block::Para(spans) => {
                let mut job = spans_job(spans, 13.0, p.text_secondary, false, p);
                job.wrap.max_width = width;
                ui.label(job);
            }
            Block::Bullet(depth, spans) | Block::Numbered(depth, _, spans) | Block::Task(depth, _, spans) => {
                ui.horizontal_top(|ui| {
                    ui.add_space(6.0 + *depth as f32 * 16.0);
                    let marker = match b {
                        Block::Numbered(_, n, _) => n.clone(),
                        Block::Task(_, true, _) => "☑".into(),
                        Block::Task(_, false, _) => "☐".into(),
                        _ => if *depth == 0 { "•" } else { "◦" }.into(),
                    };
                    ui.label(egui::RichText::new(marker).color(p.text_tertiary).size(crate::theme::font::px(13.0)));
                    let mut job = spans_job(spans, 13.0, p.text_secondary, false, p);
                    job.wrap.max_width = ui.available_width();
                    ui.label(job);
                });
            }
            Block::Quote(spans) => {
                Frame::new().inner_margin(Margin { left: 12, right: 4, top: 2, bottom: 2 }).show(ui, |ui| {
                    let mut job = spans_job(spans, 13.0, p.text_tertiary, false, p);
                    job.wrap.max_width = ui.available_width();
                    let r = ui.label(job).rect;
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(r.left() - 10.0, r.top()),
                            egui::pos2(r.left() - 7.0, r.bottom()),
                        ),
                        1.0,
                        p.accent.gamma_multiply(0.6),
                    );
                });
            }
            Block::Code { lang, lines } => {
                Frame::new()
                    .fill(p.bg_sunken)
                    .stroke(Stroke::new(1.0, p.border_subtle))
                    .corner_radius(crate::theme::tokens::radius::same(6))
                    .inner_margin(Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(width - 22.0);
                        if !lang.is_empty() {
                            ui.label(
                                egui::RichText::new(lang).size(crate::theme::font::px(10.5)).color(p.text_tertiary),
                            );
                        }
                        egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                            for l in lines {
                                ui.label(
                                    egui::RichText::new(l)
                                        .monospace()
                                        .size(crate::theme::font::px(12.0))
                                        .color(p.text_primary),
                                );
                            }
                        });
                    });
            }
            Block::Table(rows) => {
                Frame::new()
                    .fill(p.bg_surface2.gamma_multiply(0.5))
                    .corner_radius(crate::theme::tokens::radius::same(6))
                    .inner_margin(Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                            for (i, r) in rows.iter().enumerate() {
                                let c = if i == 0 { p.text_primary } else { p.text_secondary };
                                ui.label(
                                    egui::RichText::new(r).monospace().size(crate::theme::font::px(11.5)).color(c),
                                );
                            }
                        });
                    });
            }
            Block::Rule => {
                ui.add_space(4.0);
                let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 0.0, p.border_strong);
                ui.add_space(4.0);
            }
        }
    }
}

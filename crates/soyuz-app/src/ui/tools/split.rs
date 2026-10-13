//! File split / join window. It draws `soyuz_appcore::models::split::SplitModel` (the
//! split / join functions, the presets, the worker and its disk checks, shared with the
//! terminal UI's split dialog). Splitting writes `name.ext.001`, `.002`, … plus a
//! b3sum-compatible checksum file `name.ext.blake3`; joining verifies against it when
//! present. Closing the window while it runs cancels; partial output is removed.

use std::path::PathBuf;

use egui::{Align, FontId, Layout, RichText, Ui, vec2};
use soyuz_core::{SizeStyle, format_size};

use super::{button, progress_bar, tool_window};
use crate::app::SoyuzApp;
use crate::models::split::{Mode, PRESETS, SplitModel, preset_label};
use crate::theme::{icons::ph, palette};

pub struct SplitWindow {
    pub open: bool,
    model: SplitModel,
}

impl SplitWindow {
    pub fn split(path: PathBuf) -> Self {
        SplitWindow { open: true, model: SplitModel::split(path) }
    }

    pub fn join(path: PathBuf) -> Self {
        SplitWindow { open: true, model: SplitModel::join(path) }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        if let Some(toast) = self.model.poll() {
            app.core.ui.toasts.push(toast);
        }
        let m = &self.model;
        let title = if m.mode == Mode::Split { tr!("tools-split-title") } else { tr!("tools-join-title") };
        let icon = if m.mode == Mode::Split { ph::ARROWS_SPLIT } else { ph::ARROWS_MERGE };
        let name = soyuz_core::entry::display_name(&m.path);
        let keep = tool_window(ctx, "split", title, &name, icon, vec2(520.0, 330.0), |ui| self.body(ui));
        if !keep {
            self.model.cancel();
            self.open = false;
        }
    }

    fn body(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let label = |ui: &mut Ui, s: &str| {
            ui.label(RichText::new(s).font(FontId::proportional(11.5)).color(p.text_tertiary));
        };
        let m = &mut self.model;
        match m.mode {
            Mode::Split => {
                label(ui, tr!("tools-split-label-file"));
                let size = match m.size {
                    Some(s) => format_size(s, SizeStyle::Binary),
                    // The size is read off the UI thread (a frame or so).
                    None if m.probing() => "…".into(),
                    None => tr!("tools-split-unknown-size").into(),
                };
                ui.label(
                    RichText::new(format!("{}  ·  {size}", super::tags_ui::show_path(&m.path))).color(p.text_primary),
                );
                ui.add_space(8.0);
                label(ui, tr!("tools-split-label-part-size"));
                ui.horizontal_wrapped(|ui| {
                    for i in 0..PRESETS.len() {
                        let mut on = m.preset == i;
                        if super::toggle_chip(ui, preset_label(i), &mut on, p.accent).clicked() {
                            m.preset = i;
                        }
                    }
                });
                if PRESETS[m.preset].1 == 0 {
                    super::text_field(ui, &mut m.custom, tr!("tools-split-custom-hint"), 160.0);
                }
                if let Some(plan) = m.plan_text() {
                    ui.label(RichText::new(plan).font(FontId::proportional(12.0)).color(p.text_secondary));
                }
            }
            Mode::Join => {
                label(ui, tr!("tools-split-label-parts"));
                match &m.parts {
                    Some(Ok((base, parts))) => {
                        let has_ck = m.has_checksum;
                        ui.label(
                            RichText::new(tr!(
                                "tools-join-plan",
                                count = parts.len(),
                                name = soyuz_core::entry::display_name(base)
                            ))
                            .color(p.text_primary),
                        );
                        ui.label(
                            RichText::new(if has_ck {
                                tr!("tools-join-has-checksum")
                            } else {
                                tr!("tools-join-no-checksum")
                            })
                            .font(FontId::proportional(12.0))
                            .color(if has_ck { p.success } else { p.text_tertiary }),
                        );
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(p.danger));
                    }
                    None => {}
                }
            }
        }
        ui.add_space(8.0);
        label(ui, tr!("tools-split-label-output"));
        super::text_field(ui, &mut m.out_dir, tr!("tools-split-folder-hint"), ui.available_width());
        ui.add_space(10.0);
        if let (Some((done, total, _)), Some(text)) = (m.progress(), m.progress_text()) {
            let frac = if total > 0 { Some(done as f32 / total as f32) } else { None };
            progress_bar(ui, ui.available_width(), frac);
            ui.label(RichText::new(text).font(FontId::proportional(12.0)).color(p.text_secondary));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(r) = &m.result {
            let (text, color) = match r {
                Ok(msg) => (msg.as_str(), p.success),
                Err(e) => (e.as_str(), p.danger),
            };
            ui.label(RichText::new(text).color(color));
        }
        let mut close = false;
        ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if m.running() {
                        if button(ui, Some(ph::STOP), tr!("common-cancel"), 0, true).clicked() {
                            m.cancel();
                        }
                    } else if m.finished_ok() {
                        if button(ui, Some(ph::CHECK), tr!("tools-done"), 1, true).clicked() {
                            close = true;
                        }
                    } else {
                        let label =
                            if m.mode == Mode::Split { tr!("tools-split-start") } else { tr!("tools-join-start") };
                        if button(ui, Some(ph::PLAY), label, 1, m.can_start()).clicked() {
                            m.start();
                        }
                    }
                });
            });
        });
        if close {
            self.open = false;
        }
    }
}

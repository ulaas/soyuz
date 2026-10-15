//! The media tools dialog (Phase V): Convert, Extract audio, Trim, Save frame. The state,
//! the validation and the jobs are the core's [`MediaToolsModel`]; this file draws it:
//! the source and its probed summary, a tool switcher, the tool's fields (preset /
//! codec / trim window / time and picture format), the output name with the conflict
//! choice when the name is taken, the dry-run plan card and the Run / Cancel footer.
//! Enter runs, Esc (the host) cancels.

use egui::{
    Align2, CornerRadius, FontId, Frame, Key, Margin, RichText, Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2,
};

use super::DialogCtx;
use super::conflict::{Btn, button};
use crate::models::media_tools::{
    AUDIO_CODECS, ConflictChoice, ImageFormat, MediaTool, MediaToolsModel, PlanTone, codec_label, preset_label,
};
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons::ph, palette};
use crate::ui::widgets::dropdown as dd;

pub(crate) fn icon(m: &MediaToolsModel) -> &'static str {
    match m.tool {
        MediaTool::Convert => ph::FILM_STRIP,
        MediaTool::ExtractAudio => ph::MUSIC_NOTES,
        MediaTool::Trim => ph::SCISSORS,
        MediaTool::SaveFrame => ph::IMAGE,
    }
}

/// Small caption above a control.
fn caption(ui: &mut Ui, text: &str, p: &Palette) {
    ui.label(RichText::new(text).size(11.5).color(p.text_tertiary));
    ui.add_space(2.0);
}

/// A text segmented control (like the archive dialog's format picker); returns the
/// clicked index.
fn text_segmented(ui: &mut Ui, id: &str, labels: &[&str], selected: usize, cell_w: f32) -> Option<usize> {
    let p = palette(ui.ctx());
    let cell = vec2(cell_w, 26.0);
    let (rect, _) = ui.allocate_exact_size(vec2(cell.x * labels.len() as f32 + 4.0, cell.y + 4.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), p.bg_surface2);
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let r = egui::Rect::from_min_size(pos2(rect.left() + 2.0 + i as f32 * cell.x, rect.top() + 2.0), cell);
        let resp = ui.interact(r, ui.id().with((id, i)), Sense::click());
        crate::ui::a11y::toggle(&resp, label, i == selected);
        let sel = i == selected;
        if sel {
            ui.painter().rect_filled(r, CornerRadius::same(radius::CONTROL + 1), p.bg_surface1);
            ui.painter().rect_stroke(
                r,
                CornerRadius::same(radius::CONTROL + 1),
                Stroke::new(1.0, p.border_subtle),
                StrokeKind::Inside,
            );
        }
        let c = if sel {
            p.text_primary
        } else if resp.hovered() {
            p.text_secondary
        } else {
            p.text_tertiary
        };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *label, FontId::proportional(12.0), c);
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// A one-line text field with a hint; returns whether it changed.
fn text_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32, focus: &mut bool) -> bool {
    let r = ui.add(TextEdit::singleline(text).hint_text(hint).desired_width(width).margin(Margin::symmetric(10, 7)));
    if *focus {
        r.request_focus();
        *focus = false;
    }
    r.changed()
}

/// A card of toned lines (the input summary, the plan).
fn card(ui: &mut Ui, title: &str, lines: &[(PlanTone, String)], p: &Palette) {
    caption(ui, title, p);
    Frame::new()
        .fill(p.bg_surface2)
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (tone, line) in lines {
                let (color, glyph) = match tone {
                    PlanTone::Text => (p.text_primary, None),
                    PlanTone::Muted => (p.text_secondary, None),
                    PlanTone::Warning => (p.warning, Some(ph::WARNING)),
                    PlanTone::Danger => (p.danger, Some(ph::WARNING_CIRCLE)),
                };
                ui.horizontal_wrapped(|ui| {
                    if let Some(g) = glyph {
                        ui.label(crate::theme::icons::icon(g, 12.5).color(color));
                    }
                    // Names and encoder messages come from disk / the libraries: plain text.
                    ui.add(egui::Label::new(RichText::new(line).size(12.0).color(color)).wrap());
                });
            }
        });
}

/// Draw the dialog. Returns `false` to close.
pub(crate) fn show(ui: &mut Ui, m: &mut MediaToolsModel, ctx: &mut DialogCtx<'_>) -> bool {
    m.sync();
    let p = palette(ui.ctx());
    ui.label(RichText::new(m.source_line()).color(p.text_secondary));
    ui.add_space(8.0);
    // The tool switcher.
    let labels: Vec<&str> = MediaTool::ALL.iter().map(|t| t.label()).collect();
    let selected = MediaTool::ALL.iter().position(|t| *t == m.tool).unwrap_or(0);
    if let Some(i) = text_segmented(ui, "tool", &labels, selected, 112.0) {
        m.set_tool(MediaTool::ALL[i]);
    }
    ui.add_space(10.0);
    card(ui, tr!("dialogs-media-input"), &m.input_lines(), &p);
    ui.add_space(10.0);
    match m.tool {
        MediaTool::Convert => {
            caption(ui, tr!("dialogs-media-preset"), &p);
            let items: Vec<dd::Item<usize>> =
                m.presets.iter().enumerate().map(|(i, pr)| dd::Item::new(i, preset_label(pr.id))).collect();
            let mut value = m.preset;
            let out = dd::Dropdown::new("dialogs.media.preset")
                .width(ui.available_width())
                .name(tr!("dialogs-media-preset"))
                .show(ui, &mut value, &items);
            if out.changed {
                m.set_preset(value);
            }
        }
        MediaTool::ExtractAudio => {
            caption(ui, tr!("dialogs-media-codec"), &p);
            let labels: Vec<&str> = AUDIO_CODECS.iter().map(|c| codec_label(*c)).collect();
            let selected = AUDIO_CODECS.iter().position(|c| *c == m.codec).unwrap_or(0);
            if let Some(i) = text_segmented(ui, "codec", &labels, selected, 88.0) {
                m.set_codec(AUDIO_CODECS[i]);
            }
        }
        MediaTool::Trim => {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    caption(ui, tr!("dialogs-media-start"), &p);
                    let mut s = m.start.clone();
                    let mut no_focus = false;
                    if text_field(ui, &mut s, tr!("dialogs-media-time-hint"), 120.0, &mut no_focus) {
                        m.set_start(&s);
                    }
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    caption(ui, tr!("dialogs-media-end"), &p);
                    let mut e = m.end.clone();
                    let mut no_focus = false;
                    if text_field(ui, &mut e, tr!("dialogs-media-time-hint"), 120.0, &mut no_focus) {
                        m.set_end(&e);
                    }
                });
            });
            ui.add_space(6.0);
            let mut on = m.stream_copy;
            if ui.checkbox(&mut on, RichText::new(tr!("dialogs-media-stream-copy")).size(12.0)).changed() {
                m.set_stream_copy(on);
            }
        }
        MediaTool::SaveFrame => {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    caption(ui, tr!("dialogs-media-at"), &p);
                    let mut a = m.at.clone();
                    let mut no_focus = false;
                    if text_field(ui, &mut a, tr!("dialogs-media-time-hint"), 120.0, &mut no_focus) {
                        m.set_at(&a);
                    }
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    caption(ui, tr!("dialogs-media-format"), &p);
                    let formats = [ImageFormat::Png, ImageFormat::Jpeg];
                    let selected = formats.iter().position(|f| *f == m.format).unwrap_or(0);
                    if let Some(i) = text_segmented(ui, "format", &["PNG", "JPEG"], selected, 72.0) {
                        m.set_format(formats[i]);
                    }
                });
            });
        }
    }
    ui.add_space(10.0);
    caption(ui, tr!("dialogs-media-name"), &p);
    let mut name = m.name.clone();
    let mut focus = m.focus;
    let changed = text_field(ui, &mut name, tr!("dialogs-media-name-hint"), f32::INFINITY, &mut focus);
    m.focus = focus;
    if changed {
        m.set_name(&name);
    }
    if m.name_exists() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(crate::theme::icons::icon(ph::WARNING, 12.5).color(p.warning));
            ui.label(RichText::new(tr!("dialogs-media-exists")).size(11.5).color(p.warning));
        });
        ui.add_space(2.0);
        let labels: Vec<&str> = ConflictChoice::ALL.iter().map(|c| c.label()).collect();
        let selected = ConflictChoice::ALL.iter().position(|c| *c == m.conflict).unwrap_or(0);
        if let Some(i) = text_segmented(ui, "conflict", &labels, selected, 84.0) {
            m.set_conflict(ConflictChoice::ALL[i]);
        }
    }
    let plan = m.plan_lines();
    if !plan.is_empty() {
        ui.add_space(10.0);
        card(ui, tr!("dialogs-media-plan"), &plan, &p);
    }
    let error = m.error();
    if let Some(e) = &error {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(e).size(11.5).color(p.danger)).wrap());
    }
    ui.add_space(16.0);
    let mut go = false;
    let mut cancel = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        go = ui.add_enabled_ui(error.is_none(), |ui| button(ui, m.tool.go_label(), Btn::Primary)).inner.clicked();
        cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
    });
    if ui.input(|i| i.key_pressed(Key::Enter)) && error.is_none() {
        go = true;
    }
    if go {
        match m.submit() {
            Ok(cmd) => {
                ctx.cmds.push(cmd);
                return false;
            }
            Err(_) => return true,
        }
    }
    !cancel
}

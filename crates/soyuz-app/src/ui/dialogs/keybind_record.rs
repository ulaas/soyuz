//! "Record shortcut" modal used by the keybinding editor: captures the next chord (or a
//! sequence of up to three chords), shows which actions already use it, and returns the
//! binding string. Enter saves, Esc cancels; every other key is recorded.

use std::time::Instant;

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Id, Margin, Modal, RichText, Sense, Stroke, vec2};
use soyuz_core::config::{Action, KeyChord, Keymap, format_sequence};

use crate::keys::conflicts::users_of;
use crate::theme::icons::ph;
use crate::theme::{icons, palette};
use crate::ui::palette::widgets::{chips_width_scaled, paint_chips_scaled};
use crate::ui::widgets::pill_button;

/// What the recorded binding is for.
#[derive(Clone, Debug, PartialEq)]
pub enum RecordTarget {
    Action(Action),
    Bookmark(usize),
    Command(usize),
}

pub enum RecordOutcome {
    Pending,
    Save(String),
    Cancel,
}

pub struct Recorder {
    pub target: RecordTarget,
    title: String,
    chords: Vec<KeyChord>,
    opened: Option<Instant>,
}

const MAX_CHORDS: usize = 3;

impl Recorder {
    pub fn new(target: RecordTarget, title: impl Into<String>) -> Self {
        Recorder { target, title: title.into(), chords: Vec::new(), opened: None }
    }

    pub fn binding(&self) -> String {
        format_sequence(&self.chords)
    }

    /// Feed raw key events; returns the outcome of this frame.
    fn capture(&mut self, ctx: &egui::Context, cmd_is_ctrl: bool) -> RecordOutcome {
        let mut out = RecordOutcome::Pending;
        let chords = &mut self.chords;
        ctx.input_mut(|i| {
            let mods = i.modifiers;
            i.events.retain(|ev| {
                let Some(chord) = crate::keys::event_chord(ev, mods, cmd_is_ctrl) else {
                    // Swallow typed text too, so nothing leaks into fields behind.
                    return !matches!(ev, egui::Event::Text(_));
                };
                if chord.mods.is_empty() && chord.key == soyuz_core::config::KeyName::Escape {
                    out = RecordOutcome::Cancel;
                } else if chord.mods.is_empty() && chord.key == soyuz_core::config::KeyName::Enter && !chords.is_empty()
                {
                    out = RecordOutcome::Save(format_sequence(chords));
                } else if chords.len() < MAX_CHORDS {
                    chords.push(chord);
                }
                false
            });
        });
        out
    }

    /// Draw the modal. `km` is the effective key map (for "already used by").
    pub fn show(&mut self, ctx: &egui::Context, km: &Keymap, cmd_is_ctrl: bool) -> RecordOutcome {
        let opened = *self.opened.get_or_insert_with(Instant::now);
        // Ignore the click / key that opened the recorder.
        let mut outcome =
            if opened.elapsed().as_millis() > 120 { self.capture(ctx, cmd_is_ctrl) } else { RecordOutcome::Pending };
        let p = palette(ctx);
        let id = Id::new("soyuz.keybind.record");
        let frame = Frame::new()
            .fill(p.bg_surface3)
            .stroke(Stroke::new(1.0, p.border_strong))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(20))
            .shadow(egui::epaint::Shadow {
                offset: [0, 12],
                blur: 40,
                spread: 0,
                color: Color32::from_black_alpha(110),
            });
        let binding = self.binding();
        let users: Vec<(soyuz_core::config::KeyContext, Action)> = if self.chords.is_empty() {
            Vec::new()
        } else {
            users_of(km, &self.chords)
                .into_iter()
                .filter(|(_, a)| !matches!(&self.target, RecordTarget::Action(t) if t == a))
                .collect()
        };
        let mut clear = false;
        let resp = Modal::new(id).frame(frame).backdrop_color(Color32::from_black_alpha(60)).show(ctx, |ui| {
            ui.set_width(420.0);
            crate::ui::a11y::container(
                ui.ctx(),
                ui.unique_id(),
                egui::accesskit::Role::Dialog,
                tr!("dialogs-record-title"),
            );
            ui.horizontal(|ui| {
                ui.label(icons::icon(ph::KEYBOARD, 18.0).color(p.accent));
                ui.label(
                    RichText::new(tr!("dialogs-record-title"))
                        .font(FontId::new(16.0, egui::FontFamily::Name(crate::theme::fonts::SEMIBOLD.into())))
                        .color(p.text_primary),
                );
            });
            ui.label(RichText::new(&self.title).color(p.text_secondary));
            ui.add_space(14.0);
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(10), p.bg_sunken);
            let pulse = (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
            painter.rect_stroke(
                rect,
                CornerRadius::same(10),
                Stroke::new(1.5, p.accent.gamma_multiply(0.35 + 0.4 * pulse)),
                egui::StrokeKind::Inside,
            );
            if binding.is_empty() {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    tr!("dialogs-record-press"),
                    FontId::proportional(14.0),
                    p.text_tertiary,
                );
                ui.ctx().request_repaint();
            } else {
                let w = chips_width_scaled(ui, &binding, 1.6);
                paint_chips_scaled(ui, &p, &binding, rect.center().x + w / 2.0, rect.center().y, true, 1.6);
            }
            ui.add_space(10.0);
            if users.is_empty() {
                let msg = if self.chords.is_empty() {
                    tr!("dialogs-record-sequence-hint")
                } else {
                    tr!("dialogs-record-unused")
                };
                ui.label(RichText::new(msg).size(11.5).color(p.text_tertiary));
            } else {
                let names: Vec<String> = users
                    .iter()
                    .map(|(c, a)| {
                        tr!(
                            "dialogs-record-user",
                            action = crate::palette::catalog::title(a),
                            context = crate::i18n::labels::key_context(*c)
                        )
                    })
                    .collect();
                ui.horizontal_wrapped(|ui| {
                    ui.label(icons::icon(ph::WARNING, 13.0).color(p.warning));
                    ui.label(
                        RichText::new(tr!("dialogs-record-conflict", users = names.join(", ")))
                            .size(11.5)
                            .color(p.warning),
                    );
                });
            }
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr!("dialogs-record-keys-hint")).size(11.0).color(p.text_tertiary));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if pill_button(ui, Some(ph::CHECK), tr!("common-save"), true).clicked() && !self.chords.is_empty() {
                        outcome = RecordOutcome::Save(binding.clone());
                    }
                    ui.add_space(6.0);
                    if pill_button(ui, None, tr!("common-cancel"), false).clicked() {
                        outcome = RecordOutcome::Cancel;
                    }
                    ui.add_space(6.0);
                    if pill_button(ui, Some(ph::BACKSPACE), tr!("dialogs-record-clear"), false).clicked() {
                        clear = true;
                    }
                });
            });
        });
        if clear {
            self.chords.clear();
        }
        if resp.backdrop_response.clicked() {
            outcome = RecordOutcome::Cancel;
        }
        outcome
    }
}

//! Bodies of the non-conflict job prompts: error (Retry / Skip / Skip all / Abort), not
//! enough space, trash unsupported (offer permanent delete), read-only item and archive
//! password. Hosted by [`PromptDialog`](super::conflict::PromptDialog).

use egui::{Align, CornerRadius, FontId, Key, Layout, Margin, RichText, Sense, TextEdit, Ui, vec2};
use soyuz_core::{ErrorAction, SizeStyle, format_size};
use soyuz_jobs::kinds::extract::ARCHIVE_LIMIT_OP;
use soyuz_jobs::{JobPrompt, PromptResponse};

use super::conflict::{Answer, Btn, PromptDialog, button};
use crate::theme::Palette;

/// Display name for the messages (they add the quotes themselves).
fn name(p: &std::path::Path) -> String {
    crate::ui::widgets::shown_name(&soyuz_core::entry::display_name(p))
}

/// Path line in a sunken box.
fn path_box(ui: &mut Ui, path: &std::path::Path, p: &Palette) {
    egui::Frame::new()
        .fill(p.bg_sunken)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(soyuz_fs::strip_verbatim(path).display().to_string())
                    .font(FontId::monospace(11.5))
                    .color(p.text_secondary),
            );
        });
}

pub(crate) fn body(ui: &mut Ui, prompt: &JobPrompt, d: &mut PromptDialog, p: &Palette) -> Option<Answer> {
    let mut out = None;
    match prompt {
        JobPrompt::Error { path, error, .. } if error.op == ARCHIVE_LIMIT_OP => {
            // Decompression-bomb guard: Continue lifts the limit, Abort (default) stops.
            ui.label(RichText::new(tr!("dialogs-prompt-archive-limit", name = name(path))).color(p.text_primary));
            ui.add_space(4.0);
            if !error.message.is_empty() {
                ui.label(RichText::new(&error.message).font(FontId::proportional(12.0)).color(p.warning));
            }
            ui.label(
                RichText::new(tr!("dialogs-prompt-archive-limit-hint"))
                    .font(FontId::proportional(11.5))
                    .color(p.text_tertiary),
            );
            ui.add_space(8.0);
            path_box(ui, path, p);
            ui.add_space(16.0);
            let respond = |a| Some(Answer::Respond(PromptResponse::Error { action: a, apply_to_all: false }));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if button(ui, tr!("dialogs-prompt-abort"), Btn::Primary)
                    .on_hover_text(tr!("dialogs-prompt-abort-extract-tip"))
                    .clicked()
                {
                    out = respond(ErrorAction::Abort);
                }
                if button(ui, tr!("dialogs-prompt-continue"), Btn::Secondary)
                    .on_hover_text(tr!("dialogs-prompt-continue-tip"))
                    .clicked()
                {
                    out = respond(ErrorAction::Skip);
                }
            });
            if out.is_none() && ui.input(|i| i.key_pressed(Key::Enter)) {
                out = respond(ErrorAction::Abort);
            }
        }
        JobPrompt::Error { path, error, retryable } => {
            ui.label(RichText::new(tr!("dialogs-prompt-error", name = name(path))).color(p.text_primary));
            ui.add_space(4.0);
            ui.label(RichText::new(capitalize(error.kind.describe())).color(p.danger));
            if !error.message.is_empty() && error.message != error.kind.describe() {
                ui.label(RichText::new(&error.message).font(FontId::proportional(12.0)).color(p.text_secondary));
            }
            ui.add_space(8.0);
            path_box(ui, path, p);
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if button(ui, tr!("dialogs-prompt-abort"), Btn::Ghost)
                    .on_hover_text(tr!("dialogs-prompt-abort-tip"))
                    .clicked()
                {
                    out = Some(Answer::Respond(PromptResponse::Error {
                        action: ErrorAction::Abort,
                        apply_to_all: false,
                    }));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let respond = |a| Some(Answer::Respond(PromptResponse::Error { action: a, apply_to_all: false }));
                    if *retryable {
                        if button(ui, tr!("common-retry"), Btn::Primary).clicked() {
                            out = respond(ErrorAction::Retry);
                        }
                        if button(ui, tr!("common-skip"), Btn::Secondary).clicked() {
                            out = respond(ErrorAction::Skip);
                        }
                    } else if button(ui, tr!("common-skip"), Btn::Primary).clicked() {
                        out = respond(ErrorAction::Skip);
                    }
                    if button(ui, tr!("dialogs-prompt-skip-all"), Btn::Secondary)
                        .on_hover_text(tr!("dialogs-prompt-skip-all-tip"))
                        .clicked()
                    {
                        out = respond(ErrorAction::SkipAll);
                    }
                });
            });
            if out.is_none() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let a = if *retryable { ErrorAction::Retry } else { ErrorAction::Skip };
                out = Some(Answer::Respond(PromptResponse::Error { action: a, apply_to_all: false }));
            }
        }
        JobPrompt::NotEnoughSpace { needed, free, dest } => {
            ui.label(
                RichText::new(tr!(
                    "dialogs-prompt-space",
                    name = name(dest),
                    needed = format_size(*needed, SizeStyle::Binary),
                    free = format_size(*free, SizeStyle::Binary)
                ))
                .color(p.text_primary),
            );
            ui.add_space(10.0);
            // Needed vs free bar.
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(4), p.bg_sunken);
            let frac = if *needed == 0 { 1.0 } else { (*free as f32 / *needed as f32).clamp(0.0, 1.0) };
            let mut fr = r;
            fr.set_width((r.width() * frac).max(4.0));
            ui.painter().rect_filled(fr, CornerRadius::same(4), p.warning);
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr!("dialogs-prompt-space-hint")).font(FontId::proportional(11.5)).color(p.text_tertiary),
            );
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if button(ui, tr!("dialogs-prompt-continue-anyway"), Btn::Primary).clicked() {
                    out = Some(Answer::Respond(PromptResponse::Space { proceed: true }));
                }
                if button(ui, tr!("dialogs-prompt-cancel-op"), Btn::Secondary).clicked() {
                    out = Some(Answer::Respond(PromptResponse::Space { proceed: false }));
                }
            });
        }
        JobPrompt::TrashUnsupported { path, reason } => {
            ui.label(RichText::new(tr!("dialogs-prompt-trash", name = name(path))).color(p.text_primary));
            if !reason.is_empty() {
                ui.label(RichText::new(reason).font(FontId::proportional(12.0)).color(p.text_secondary));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(tr!("dialogs-prompt-trash-permanent")).color(p.danger));
            ui.add_space(10.0);
            ui.checkbox(&mut d.apply_all, tr!("dialogs-prompt-apply-all-items"));
            ui.add_space(14.0);
            let all = d.apply_all;
            ui.horizontal(|ui| {
                if button(ui, tr!("dialogs-prompt-cancel-op"), Btn::Ghost).clicked() {
                    out = Some(Answer::CancelJob);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if button(ui, tr!("dialogs-prompt-delete-permanently"), Btn::Danger).clicked() {
                        out = Some(Answer::Respond(PromptResponse::TrashUnsupported {
                            permanent: true,
                            apply_to_all: all,
                        }));
                    }
                    if button(ui, tr!("common-skip"), Btn::Secondary).clicked() {
                        out = Some(Answer::Respond(PromptResponse::TrashUnsupported {
                            permanent: false,
                            apply_to_all: all,
                        }));
                    }
                });
            });
        }
        JobPrompt::ReadOnlyItem { path } => {
            ui.label(RichText::new(tr!("dialogs-prompt-read-only", name = name(path))).color(p.text_primary));
            ui.label(RichText::new(tr!("dialogs-prompt-read-only-ask")).color(p.text_secondary));
            ui.add_space(8.0);
            path_box(ui, path, p);
            ui.add_space(10.0);
            ui.checkbox(&mut d.apply_all, tr!("dialogs-prompt-apply-all-read-only"));
            ui.add_space(14.0);
            let all = d.apply_all;
            ui.horizontal(|ui| {
                if button(ui, tr!("dialogs-prompt-cancel-op"), Btn::Ghost).clicked() {
                    out = Some(Answer::CancelJob);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if button(ui, tr!("dialogs-prompt-change-anyway"), Btn::Danger).clicked() {
                        out = Some(Answer::Respond(PromptResponse::ReadOnly { force: true, apply_to_all: all }));
                    }
                    if button(ui, tr!("common-skip"), Btn::Secondary).clicked() {
                        out = Some(Answer::Respond(PromptResponse::ReadOnly { force: false, apply_to_all: all }));
                    }
                });
            });
        }
        JobPrompt::Password { archive, wrong } => {
            ui.label(RichText::new(tr!("dialogs-prompt-encrypted", name = name(archive))).color(p.text_primary));
            if *wrong {
                ui.label(RichText::new(tr!("dialogs-prompt-wrong-password")).color(p.danger));
            }
            ui.add_space(8.0);
            let r = ui.add(
                TextEdit::singleline(&mut d.password)
                    .password(true)
                    .hint_text(tr!("dialogs-prompt-password"))
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(10, 7)),
            );
            if d.focus_password {
                r.request_focus();
                d.focus_password = false;
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if (button(ui, tr!("dialogs-prompt-unlock"), Btn::Primary).clicked() || enter) && !d.password.is_empty()
                {
                    out = Some(Answer::Respond(PromptResponse::Password(Some(d.password.clone()))));
                }
                if button(ui, tr!("common-cancel"), Btn::Secondary).clicked() {
                    out = Some(Answer::Respond(PromptResponse::Password(None)));
                }
            });
        }
        JobPrompt::Conflict(_) => {}
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

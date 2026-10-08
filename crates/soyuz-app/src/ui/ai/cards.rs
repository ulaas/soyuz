//! Plan cards (roadmap A6): one card per proposed operation with an icon, the summary
//! ("Move 214 files into 9 folders"), totals, conflicts and warnings, expandable details
//! (tree diff or rename table) and Approve / Edit / Reject / "Approve all of this kind".

use egui::{Align2, CornerRadius, FontId, Frame, Margin, RichText, Sense, Stroke, TextEdit, Ui, pos2, vec2};
use serde_json::{Value, json};
use soyuz_ai::tools::{Plan, Risk};

use super::{PanelAct, Tone, button};
use crate::ai::agent::Decision;
use crate::ai::state::{CardState, PlanCard, PlanEdit};
use crate::theme::Palette;
use crate::theme::icons::{self, ph};
use crate::theme::tokens::radius;
use crate::ui::views::flow::semibold;

/// Icon and colour of a tool.
pub fn tool_style(tool: &str, p: &Palette) -> (&'static str, egui::Color32) {
    match tool {
        "copy" => (ph::COPY, p.accent),
        "move" => (ph::ARROW_BEND_DOWN_RIGHT, p.accent),
        "rename" | "batch_rename" => (ph::PENCIL_SIMPLE, p.info),
        "mkdir" => (ph::FOLDER_PLUS, p.success),
        "create_file" => (ph::FILE_PLUS, p.success),
        "trash" => (ph::TRASH, p.danger),
        "delete_permanent" | "empty_trash" => (ph::X_CIRCLE, p.danger),
        "compress" | "extract" => (ph::ARCHIVE, p.glyph_archive),
        "set_attributes" => (ph::SLIDERS, p.warning),
        "tag" => (ph::TAG, p.tags[6]),
        "sync_apply" => (ph::ARROWS_CLOCKWISE, p.warning),
        "restore_from_trash" | "undo" => (ph::ARROW_COUNTER_CLOCKWISE, p.success),
        "run_custom_command" => (ph::TERMINAL_WINDOW, p.warning),
        _ => (ph::LIGHTNING, p.accent),
    }
}

fn is_rename(tool: &str) -> bool {
    matches!(tool, "rename" | "batch_rename")
}

fn totals(plan: &Plan) -> String {
    let mut s = format!("{} item{}", plan.items.len(), if plan.items.len() == 1 { "" } else { "s" });
    if plan.total_bytes > 0 {
        s.push_str(&format!(" \u{00b7} {}", soyuz_core::format_size(plan.total_bytes, soyuz_core::SizeStyle::Binary)));
    }
    if plan.total_entries > plan.items.len() as u64 {
        s.push_str(&format!(" \u{00b7} {} entries", plan.total_entries));
    }
    if plan.conflicts > 0 {
        s.push_str(&format!(" \u{00b7} {} conflict{}", plan.conflicts, if plan.conflicts == 1 { "" } else { "s" }));
    }
    s
}

fn state_badge(ui: &mut Ui, p: &Palette, state: &CardState, rect: egui::Rect) {
    let (icon, label, color) = match state {
        CardState::Pending => (ph::HOURGLASS, "Waiting for you".to_string(), p.warning),
        CardState::Approved { auto: true } => (ph::CHECKS, "Auto-approved".into(), p.accent),
        CardState::Approved { auto: false } => (ph::CHECK, "Approved".into(), p.accent),
        CardState::Running(_) => (ph::ARROWS_CLOCKWISE, "Running".into(), p.accent),
        CardState::Done { ok: true, .. } => (ph::CHECK_CIRCLE, "Done".into(), p.success),
        CardState::Done { ok: false, .. } => (ph::WARNING, "Failed".into(), p.danger),
        CardState::Rejected => (ph::PROHIBIT, "Rejected".into(), p.text_tertiary),
        CardState::Edited => (ph::PENCIL_SIMPLE, "Replaced by your edit".into(), p.text_tertiary),
        CardState::Preview => (ph::EYE, "Preview only".into(), p.text_tertiary),
    };
    let g = ui.painter().layout_no_wrap(label, FontId::proportional(10.5), color);
    let w = g.size().x + 26.0;
    let r = egui::Rect::from_min_size(pos2(rect.right() - w, rect.top()), vec2(w, 20.0));
    ui.painter().rect_filled(r, CornerRadius::same(10), color.gamma_multiply(0.12));
    if matches!(state, CardState::Running(_)) {
        crate::ui::widgets::spinner(ui, pos2(r.left() + 10.0, r.center().y), 5.0, color);
    } else {
        ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::CENTER_CENTER, icon, icons::font(11.0), color);
    }
    ui.painter().galley(pos2(r.left() + 19.0, r.center().y - g.size().y / 2.0), g, color);
}

/// Start editing: rename plans get a name table, others a JSON editor.
fn begin_edit(card: &mut PlanCard) {
    let names = if is_rename(&card.tool) {
        card.plan
            .items
            .iter()
            .map(|i| {
                i.dst.as_ref().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            })
            .collect()
    } else {
        Vec::new()
    };
    let json = serde_json::to_string_pretty(&card.args).unwrap_or_default();
    card.edit = Some(PlanEdit { json, names, error: None });
    card.expanded = true;
}

/// The edited arguments, or an error to show.
fn edited_args(card: &PlanCard, e: &PlanEdit) -> Result<Value, String> {
    if is_rename(&card.tool) && !e.names.is_empty() {
        let pairs: Vec<Value> = card
            .plan
            .items
            .iter()
            .zip(&e.names)
            .filter_map(|(i, n)| i.src.as_ref().map(|s| json!({"path": s, "new_name": n.trim()})))
            .collect();
        if pairs.iter().any(|p| p["new_name"].as_str().is_some_and(str::is_empty)) {
            return Err("names cannot be empty".into());
        }
        return Ok(if card.tool == "rename" && pairs.len() == 1 { pairs[0].clone() } else { json!({"pairs": pairs}) });
    }
    serde_json::from_str::<Value>(&e.json).map_err(|err| format!("not valid JSON: {err}"))
}

pub fn show(ui: &mut Ui, p: &Palette, card: &mut PlanCard, live: bool, acts: &mut Vec<PanelAct>) {
    let (icon, color) = tool_style(&card.tool, p);
    let destructive = card.plan.risk == Risk::Destructive;
    let pending = card.state == CardState::Pending && live;
    let border = if pending {
        if card.tainted || destructive { p.warning.gamma_multiply(0.7) } else { p.accent.gamma_multiply(0.55) }
    } else {
        p.border_subtle
    };
    Frame::new()
        .fill(p.bg_window)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Header.
            let w = ui.available_width();
            let (hr, hresp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
            ui.painter().circle_filled(pos2(hr.left() + 17.0, hr.center().y), 17.0, color.gamma_multiply(0.13));
            ui.painter().text(pos2(hr.left() + 17.0, hr.center().y), Align2::CENTER_CENTER, icon, icons::font(16.0), color);
            let tx = hr.left() + 44.0;
            let tw = (hr.right() - tx - 120.0).max(60.0);
            let summary = card.plan.summary.trim_start_matches("AI: ").to_string();
            let g = crate::ui::widgets::trunc_galley(ui.painter(), &summary, semibold(13.0), p.text_primary, tw);
            ui.painter().galley(pos2(tx, hr.top() + 2.0), g, p.text_primary);
            let sub = format!("{} \u{00b7} {}", card.tool, totals(&card.plan));
            let sg = crate::ui::widgets::trunc_galley(ui.painter(), &sub, FontId::proportional(11.0), p.text_tertiary, tw);
            ui.painter().galley(pos2(tx, hr.top() + 22.0), sg, p.text_tertiary);
            state_badge(ui, p, &card.state, hr);
            let caret = if card.expanded { ph::CARET_UP } else { ph::CARET_DOWN };
            ui.painter().text(pos2(hr.right() - 8.0, hr.bottom() - 6.0), Align2::RIGHT_BOTTOM, caret, icons::font(12.0), p.text_tertiary);
            if hresp.on_hover_text(&summary).clicked() {
                card.expanded = !card.expanded;
            }
            // Risk, taint and warnings.
            if destructive && card.state != CardState::Preview {
                banner(ui, p, ph::SHIELD_WARNING, p.danger, "Destructive: items leave this folder (trash is restorable; permanent delete is not).");
            }
            if card.tainted && card.state != CardState::Preview {
                banner(
                    ui,
                    p,
                    ph::SHIELD_WARNING,
                    p.warning,
                    "Proposed right after the assistant read file contents. Text inside files can try to steer it — check this plan before approving.",
                );
            }
            for wmsg in &card.plan.warnings {
                banner(ui, p, ph::WARNING, p.warning, wmsg);
            }
            if !card.plan.executable && card.state == CardState::Pending {
                banner(ui, p, ph::PROHIBIT, p.danger, "Blocked by conflicts (see the items marked below). Edit or reject it.");
            }
            // Details.
            if card.expanded {
                ui.add_space(6.0);
                if let Some(edit) = &mut card.edit {
                    editor(ui, p, &card.tool, &card.plan, edit);
                } else if is_rename(&card.tool) {
                    super::diff::rename_table(ui, p, &card.plan, acts);
                } else {
                    super::diff::tree_diff(ui, p, &card.plan, acts);
                }
            }
            if let CardState::Done { message, ok } = &card.state {
                ui.add_space(4.0);
                ui.label(RichText::new(message).size(11.5).color(if *ok { p.success } else { p.danger }));
            }
            // Buttons.
            if pending {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    if let Some(edit) = card.edit.clone() {
                        let res = edited_args(card, &edit);
                        if button(ui, Some(ph::CHECK), "Use these changes", Tone::Primary, res.is_ok()).clicked()
                            && let Ok(args) = res
                        {
                            acts.push(PanelAct::Decide(card.id, card.task, Decision::Edit(args)));
                        }
                        if button(ui, None, "Cancel edit", Tone::Quiet, true).clicked() {
                            card.edit = None;
                        }
                        if let Err(e) = edited_args(card, &edit) {
                            ui.label(RichText::new(e).size(11.0).color(p.danger));
                        }
                        return;
                    }
                    let can = card.plan.executable;
                    let ok = button(ui, Some(ph::CHECK), "Approve", Tone::Primary, can);
                    let ok = if can { ok } else { ok.on_hover_text("This plan has blocking conflicts") };
                    if ok.clicked() {
                        acts.push(PanelAct::Decide(card.id, card.task, Decision::Approve));
                    }
                    if button(ui, Some(ph::PENCIL_SIMPLE), "Edit", Tone::Normal, true).clicked() {
                        begin_edit(card);
                    }
                    if button(ui, Some(ph::X), "Reject", Tone::Danger, true).clicked() {
                        acts.push(PanelAct::Decide(card.id, card.task, Decision::Reject(String::new())));
                    }
                    let label = format!("Approve all \u{201c}{}\u{201d}", card.tool);
                    let r = button(ui, Some(ph::CHECKS), &label, Tone::Quiet, can).on_hover_text(
                        "Approve this plan and every later plan of the same kind in this chat (never for plans proposed right after reading file contents).",
                    );
                    if r.clicked() {
                        acts.push(PanelAct::ApproveKind(card.id, card.task, card.tool.clone()));
                    }
                });
            }
        });
}

fn banner(ui: &mut Ui, p: &Palette, icon: &str, color: egui::Color32, text: &str) {
    ui.add_space(4.0);
    Frame::new()
        .fill(color.gamma_multiply(0.08))
        .corner_radius(CornerRadius::same(radius::ROW))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.label(icons::icon(icon, 12.5).color(color));
                ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.text_secondary)).wrap());
            });
        });
}

fn editor(ui: &mut Ui, p: &Palette, tool: &str, plan: &Plan, edit: &mut PlanEdit) {
    if is_rename(tool) && !edit.names.is_empty() {
        ui.label(
            RichText::new("Edit the new names, then use them to rebuild the plan.").size(11.0).color(p.text_tertiary),
        );
        for (i, item) in plan.items.iter().enumerate() {
            let Some(name) = edit.names.get_mut(i) else { break };
            ui.horizontal(|ui| {
                let old = item
                    .src
                    .as_ref()
                    .and_then(|s| s.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let w = ui.available_width();
                ui.add_sized(
                    vec2(w * 0.45, 20.0),
                    egui::Label::new(RichText::new(old).size(12.0).color(p.text_secondary)).truncate(),
                );
                ui.label(icons::icon(ph::ARROW_RIGHT, 11.0).color(p.text_tertiary));
                ui.add(TextEdit::singleline(name).desired_width(f32::INFINITY).font(FontId::proportional(12.0)));
            });
        }
    } else {
        ui.label(
            RichText::new("Edit the tool arguments (JSON). The plan is rebuilt with a fresh dry run.")
                .size(11.0)
                .color(p.text_tertiary),
        );
        ui.add(
            TextEdit::multiline(&mut edit.json)
                .code_editor()
                .desired_rows(6)
                .desired_width(f32::INFINITY)
                .font(FontId::monospace(11.5)),
        );
    }
    if let Some(e) = &edit.error {
        ui.label(RichText::new(e).size(11.0).color(p.danger));
    }
}

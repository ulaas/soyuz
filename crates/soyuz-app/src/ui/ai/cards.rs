//! Plan cards: one card per proposed operation with an icon, the summary
//! ("Move 214 files into 9 folders"), totals, conflicts and warnings, expandable details
//! (tree diff or rename table) and Approve / Edit / Reject / "Approve all of this kind".
//!
//! The same widget renders the assistant's plans (chat) and plans of external MCP
//! clients (approvals area / modal card): those carry an "External client" badge with
//! the client name and a countdown. The widget never decides anything itself; it returns
//! a [`CardAction`] the caller routes to the agent or to the MCP bridge.

use egui::{Align2, CornerRadius, FontId, Frame, Margin, RichText, Sense, Stroke, TextEdit, Ui, pos2, vec2};
use serde_json::{Value, json};
use soyuz_ai::tools::{Plan, Risk};

use super::{PanelAct, Tone, button};
use crate::ai::agent::Decision;
use crate::ai::state::{CardOrigin, CardState, PlanCard, PlanEdit};
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

/// What the user did on a card.
#[derive(Clone, Debug, PartialEq)]
pub enum CardAction {
    Decide(Decision),
    /// "Approve all of this kind" (this chat / this MCP client).
    ApproveKind,
}

pub use crate::ai::mcp::irreversible;

fn is_rename(tool: &str) -> bool {
    matches!(tool, "rename" | "batch_rename")
}

fn totals(plan: &Plan) -> String {
    let mut parts = vec![tr!("ai-card-items", count = plan.items.len())];
    if plan.total_bytes > 0 {
        parts.push(soyuz_core::format_size(plan.total_bytes, soyuz_core::SizeStyle::Binary));
    }
    if plan.total_entries > plan.items.len() as u64 {
        parts.push(tr!("ai-card-entries", count = plan.total_entries));
    }
    if plan.conflicts > 0 {
        parts.push(tr!("ai-card-conflicts", count = plan.conflicts));
    }
    parts.join(" \u{00b7} ")
}

fn state_badge(ui: &mut Ui, p: &Palette, state: &CardState, rect: egui::Rect) {
    let (icon, label, color) = match state {
        CardState::Pending => (ph::HOURGLASS, tr!("ai-card-state-pending").to_string(), p.warning),
        CardState::Approved { auto: true } => (ph::CHECKS, tr!("ai-card-state-auto").into(), p.accent),
        CardState::Approved { auto: false } => (ph::CHECK, tr!("ai-card-state-approved").into(), p.accent),
        CardState::Running(_) => (ph::ARROWS_CLOCKWISE, tr!("ai-card-state-running").into(), p.accent),
        CardState::Done { ok: true, .. } => (ph::CHECK_CIRCLE, tr!("ai-card-state-done").into(), p.success),
        CardState::Done { ok: false, .. } => (ph::WARNING, tr!("ai-card-state-failed").into(), p.danger),
        CardState::Rejected => (ph::PROHIBIT, tr!("ai-card-state-rejected").into(), p.text_tertiary),
        CardState::Edited => (ph::PENCIL_SIMPLE, tr!("ai-card-state-edited").into(), p.text_tertiary),
        CardState::Preview => (ph::EYE, tr!("ai-card-state-preview").into(), p.text_tertiary),
        CardState::Withdrawn(why) => (ph::CLOCK_COUNTDOWN, why.clone(), p.text_tertiary),
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
            return Err(tr!("ai-card-names-empty").into());
        }
        return Ok(if card.tool == "rename" && pairs.len() == 1 { pairs[0].clone() } else { json!({"pairs": pairs}) });
    }
    serde_json::from_str::<Value>(&e.json).map_err(|err| tr!("ai-card-invalid-json", error = err.to_string()))
}

/// The "External client" strip of MCP cards: badge, client, transport and countdown.
fn origin_strip(ui: &mut Ui, p: &Palette, card: &PlanCard) {
    let CardOrigin::Mcp { client, via } = &card.origin else { return };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let font = FontId::proportional(10.5);
        let g = ui.painter().layout_no_wrap(tr!("ai-card-external").into(), semibold(10.5), p.warning);
        let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 26.0, 18.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(9), p.warning.gamma_multiply(0.14));
        ui.painter().text(
            pos2(r.left() + 10.0, r.center().y),
            Align2::CENTER_CENTER,
            ph::PLUGS_CONNECTED,
            icons::font(11.0),
            p.warning,
        );
        ui.painter().galley(pos2(r.left() + 19.0, r.center().y - g.size().y / 2.0), g, p.warning);
        ui.label(RichText::new(client).font(semibold(12.0)).color(p.text_primary))
            .on_hover_text(tr!("ai-card-client-tip"));
        ui.label(RichText::new(tr!("ai-card-via", transport = via)).font(font.clone()).color(p.text_tertiary));
        if card.state == CardState::Pending
            && let Some(exp) = card.expires
        {
            let left = exp.saturating_duration_since(std::time::Instant::now()).as_secs();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(tr!("ai-card-left", time = format!("{}:{:02}", left / 60, left % 60)))
                        .font(font)
                        .color(if left < 30 { p.danger } else { p.text_tertiary }),
                )
                .on_hover_text(tr!("ai-card-timeout-tip"));
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    });
    ui.add_space(4.0);
}

/// Draw a card; returns what the user clicked (diff clicks go to `acts`).
pub fn show(ui: &mut Ui, p: &Palette, card: &mut PlanCard, live: bool, acts: &mut Vec<PanelAct>) -> Option<CardAction> {
    let mut action = None;
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
            origin_strip(ui, p, card);
            // Header.
            let w = ui.available_width();
            let (hr, hresp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
            ui.painter().circle_filled(pos2(hr.left() + 17.0, hr.center().y), 17.0, color.gamma_multiply(0.13));
            ui.painter().text(
                pos2(hr.left() + 17.0, hr.center().y),
                Align2::CENTER_CENTER,
                icon,
                icons::font(16.0),
                color,
            );
            let tx = hr.left() + 44.0;
            let tw = (hr.right() - tx - 120.0).max(60.0);
            let summary = card.plan.summary.trim_start_matches("AI: ").to_string();
            let g = crate::ui::widgets::trunc_galley(ui.painter(), &summary, semibold(13.0), p.text_primary, tw);
            ui.painter().galley(pos2(tx, hr.top() + 2.0), g, p.text_primary);
            let sub = format!("{} \u{00b7} {}", card.tool, totals(&card.plan));
            let sg =
                crate::ui::widgets::trunc_galley(ui.painter(), &sub, FontId::proportional(11.0), p.text_tertiary, tw);
            ui.painter().galley(pos2(tx, hr.top() + 22.0), sg, p.text_tertiary);
            state_badge(ui, p, &card.state, hr);
            let caret = if card.expanded { ph::CARET_UP } else { ph::CARET_DOWN };
            ui.painter().text(
                pos2(hr.right() - 8.0, hr.bottom() - 6.0),
                Align2::RIGHT_BOTTOM,
                caret,
                icons::font(12.0),
                p.text_tertiary,
            );
            // Screen readers: the card is a named group; its header toggles the details.
            crate::ui::a11y::container(ui.ctx(), ui.unique_id(), egui::accesskit::Role::Group, &summary);
            crate::ui::a11y::with(ui.ctx(), hresp.id, |n| {
                n.set_role(egui::accesskit::Role::Button);
                n.set_label(summary.as_str());
                n.set_description(sub.as_str());
                n.set_expanded(card.expanded);
            });
            if hresp.on_hover_text(&summary).clicked() {
                card.expanded = !card.expanded;
            }
            // Risk, taint and warnings.
            if destructive && card.state != CardState::Preview {
                banner(ui, p, ph::SHIELD_WARNING, p.danger, tr!("ai-card-destructive"));
            }
            if card.origin != CardOrigin::Assistant && card.state == CardState::Pending {
                banner(ui, p, ph::SHIELD_WARNING, p.warning, tr!("ai-card-external-warning"));
            }
            if card.tainted && card.state != CardState::Preview {
                banner(ui, p, ph::SHIELD_WARNING, p.warning, tr!("ai-card-tainted"));
            }
            for wmsg in &card.plan.warnings {
                banner(ui, p, ph::WARNING, p.warning, wmsg);
            }
            if !card.plan.executable && card.state == CardState::Pending {
                banner(ui, p, ph::PROHIBIT, p.danger, tr!("ai-card-blocked"));
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
                        if button(ui, Some(ph::CHECK), tr!("ai-card-use-edit"), Tone::Primary, res.is_ok()).clicked()
                            && let Ok(args) = res
                        {
                            action = Some(CardAction::Decide(Decision::Edit(args)));
                        }
                        if button(ui, None, tr!("ai-card-cancel-edit"), Tone::Quiet, true).clicked() {
                            card.edit = None;
                        }
                        if let Err(e) = edited_args(card, &edit) {
                            ui.label(RichText::new(e).size(11.0).color(p.danger));
                        }
                        return;
                    }
                    let can = card.plan.executable;
                    let ok = button(ui, Some(ph::CHECK), tr!("ai-card-approve"), Tone::Primary, can);
                    let ok = if can { ok } else { ok.on_hover_text(tr!("ai-card-blocked-tip")) };
                    if ok.clicked() {
                        action = Some(CardAction::Decide(Decision::Approve));
                    }
                    if button(ui, Some(ph::PENCIL_SIMPLE), tr!("ai-card-edit"), Tone::Normal, true).clicked() {
                        begin_edit(card);
                    }
                    if button(ui, Some(ph::X), tr!("ai-card-reject"), Tone::Danger, true).clicked() {
                        action = Some(CardAction::Decide(Decision::Reject(String::new())));
                    }
                    let (label, tip) = match &card.origin {
                        CardOrigin::Assistant => (
                            tr!("ai-card-approve-kind", tool = &card.tool),
                            tr!("ai-card-approve-kind-tip").to_string(),
                        ),
                        CardOrigin::Mcp { client, .. } => (
                            tr!("ai-card-approve-kind-client", tool = &card.tool, client = client),
                            tr!("ai-card-approve-kind-client-tip", tool = &card.tool, client = client),
                        ),
                    };
                    let bulk_ok = card.origin == CardOrigin::Assistant || !irreversible(&card.tool);
                    if bulk_ok {
                        let r = button(ui, Some(ph::CHECKS), &label, Tone::Quiet, can).on_hover_text(tip);
                        if r.clicked() {
                            action = Some(CardAction::ApproveKind);
                        }
                    }
                });
            }
        });
    action
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
        ui.label(RichText::new(tr!("ai-card-edit-names-hint")).size(11.0).color(p.text_tertiary));
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
        ui.label(RichText::new(tr!("ai-card-edit-json-hint")).size(11.0).color(p.text_tertiary));
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

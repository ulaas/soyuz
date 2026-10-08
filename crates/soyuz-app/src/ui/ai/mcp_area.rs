//! The approvals area of the assistant panel: plan cards of external MCP clients (the
//! same card widget as the assistant's plans, with an "External client" badge), above
//! the chat. It also offers "Undo everything <client> did" for every client that changed
//! something in this session.

use egui::{Align, Align2, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke, Ui, pos2};

use super::cards::{self, CardAction};
use super::{PanelAct, Tone, button};
use crate::app::SoyuzApp;
use crate::theme::Palette;
use crate::theme::icons::{self, ph};
use crate::theme::tokens::radius;
use crate::ui::views::flow::semibold;

/// Whether the area has anything to show.
pub fn visible(app: &SoyuzApp) -> bool {
    !app.services.mcp.approvals.lock().cards.is_empty()
        || !crate::ai::mcp::clients_with_history(app.fileops.jobs.history()).is_empty()
}

/// Draw the area as a top panel of the chat tab.
pub fn show(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    if !visible(app) {
        return;
    }
    let max_h = (ui.available_height() * 0.62).max(160.0);
    let flash = app.ai.panel.mcp_flash.map(|t| t.elapsed().as_secs_f32()).filter(|s| *s < 1.6);
    if flash.is_some() {
        ui.ctx().request_repaint();
    }
    ui.scope(|ui| {
        let approvals = app.services.mcp.approvals.clone();
        let mut a = approvals.lock();
        let waiting = a.pending();
        let undo = crate::ai::mcp::clients_with_history(app.fileops.jobs.history());
        let border = match flash {
            Some(t) => p.warning.gamma_multiply(0.35 + 0.65 * (1.0 - t / 1.6)),
            None if waiting > 0 => p.warning.gamma_multiply(0.45),
            None => p.border_subtle,
        };
        Frame::new()
            .fill(p.bg_surface2.gamma_multiply(0.6))
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(radius::CARD))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 11.0, p.warning.gamma_multiply(0.14));
                    ui.painter().text(
                        pos2(r.center().x, r.center().y),
                        Align2::CENTER_CENTER,
                        ph::PLUGS_CONNECTED,
                        icons::font(13.0),
                        p.warning,
                    );
                    ui.label(RichText::new(tr!("ai-mcp-title")).font(semibold(12.5)).color(p.text_primary));
                    let sub = match waiting {
                        0 => tr!("ai-mcp-clients").to_string(),
                        n => tr!("ai-mcp-waiting", count = n),
                    };
                    ui.label(RichText::new(sub).size(11.0).color(if waiting > 0 {
                        p.warning
                    } else {
                        p.text_tertiary
                    }));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if a.cards.iter().any(|c| c.resolved.is_some())
                            && button(ui, None, tr!("ai-mcp-clear"), Tone::Quiet, true)
                                .on_hover_text(tr!("ai-mcp-clear-tip"))
                                .clicked()
                        {
                            acts.push(PanelAct::McpClear);
                        }
                    });
                });
                if !undo.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                        for (client, n) in &undo {
                            let busy = app.ai.mcp_undo_running(client);
                            let label = tr!("ai-mcp-undo-client", client = client);
                            let r = button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), &label, Tone::Normal, !busy)
                                .on_hover_text(tr!("ai-mcp-undo-client-tip", count = *n, client = client));
                            if r.clicked() {
                                acts.push(PanelAct::McpUndoClient(client.clone()));
                            }
                            if busy {
                                ui.spinner();
                            }
                        }
                    });
                }
                if a.cards.is_empty() {
                    return;
                }
                ui.add_space(6.0);
                ScrollArea::vertical().id_salt("soyuz.ai.mcp.cards").max_height(max_h).auto_shrink([false, true]).show(
                    ui,
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        // Pending first (oldest first), then the finished ones.
                        let mut order: Vec<usize> = (0..a.cards.len()).collect();
                        order.sort_by_key(|&i| (a.cards[i].resolved.is_some(), a.cards[i].card.id));
                        for i in order {
                            let c = &mut a.cards[i];
                            let id = c.card.id;
                            if let Some(act) = cards::show(ui, p, &mut c.card, true, acts) {
                                acts.push(PanelAct::Mcp(id, act));
                            }
                        }
                    },
                );
            });
    });
    ui.add_space(8.0);
}

/// Apply a card action to the MCP approvals.
pub fn apply(app: &mut SoyuzApp, id: u64, act: CardAction) {
    let mut a = app.services.mcp.approvals.lock();
    match act {
        CardAction::Decide(d) => a.decide(id, d),
        CardAction::ApproveKind => a.approve_kind(id),
    }
}

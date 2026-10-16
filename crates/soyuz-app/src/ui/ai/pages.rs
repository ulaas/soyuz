//! Panel pages: saved recipes (re-run without a model, same approvals), the audit log
//! viewer (`ai-audit.jsonl` shared by the assistant and MCP clients; filter by source —
//! assistant or MCP client —, session, model and text) and the smart
//! search card (natural language → editable query pills).

use egui::{
    Align, Align2, CornerRadius, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke, StrokeKind,
    TextEdit, Ui, pos2, vec2,
};

use super::{PanelAct, Tone, button, chip};
use crate::ai::audit::{self, AuditRecord};
use crate::ai::smart_search::{self, Pill, PillKey};
use crate::app::SoyuzApp;
use crate::theme::Palette;
use crate::theme::icons::{self, ph};
use crate::theme::tokens::radius;
use crate::ui::views::flow::semibold;

fn card(ui: &mut Ui, p: &Palette, body: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(p.bg_window)
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        });
}

pub fn recipes(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    if !app.core.ai.recipes.loaded {
        app.core.ai.load_recipes();
    }
    ui.label(RichText::new(tr!("ai-recipes-intro")).size(11.5).color(p.text_tertiary));
    ui.add_space(6.0);
    ScrollArea::vertical().id_salt("soyuz.ai.recipes").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        if app.core.ai.recipes.loading.is_some() && app.core.ai.recipes.list.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr!("common-loading")).color(p.text_tertiary));
            });
            return;
        }
        if app.core.ai.recipes.list.is_empty() {
            crate::ui::tools::compare::empty_note(
                ui,
                ph::BOOK_OPEN,
                tr!("ai-recipes-empty-title"),
                tr!("ai-recipes-empty-body"),
            );
            return;
        }
        let dir = crate::ai::recipe::dir();
        for (path, r) in app.core.ai.recipes.list.clone() {
            card(ui, p, |ui| match &r {
                Ok(rec) => {
                    ui.horizontal(|ui| {
                        ui.label(icons::icon(ph::BOOK_OPEN, 16.0).color(p.accent));
                        ui.label(RichText::new(&rec.name).font(semibold(13.0)).color(p.text_primary));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if button(ui, Some(ph::TRASH), "", Tone::Quiet, true)
                                .on_hover_text(tr!("ai-recipe-delete-tip"))
                                .clicked()
                            {
                                acts.push(PanelAct::DeleteRecipe(path.clone()));
                            }
                            if button(
                                ui,
                                Some(ph::PLAY),
                                tr!("ai-recipe-run"),
                                Tone::Primary,
                                app.core.ai.running.is_none(),
                            )
                            .on_hover_text(tr!("ai-recipe-run-tip"))
                            .clicked()
                            {
                                acts.push(PanelAct::RunRecipe(Box::new(rec.clone())));
                            }
                        });
                    });
                    if !rec.description.is_empty() {
                        ui.label(RichText::new(&rec.description).size(11.5).color(p.text_secondary));
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for s in &rec.steps {
                            let (icon, _) = super::cards::tool_style(&s.tool, p);
                            chip(ui, Some(icon), &s.tool, false);
                        }
                    });
                    ui.label(
                        RichText::new(format!(
                            "{} \u{00b7} {}",
                            path.strip_prefix(&dir).unwrap_or(&path).display(),
                            rec.created.get(..16).unwrap_or(&rec.created).replace('T', " ")
                        ))
                        .size(10.5)
                        .color(p.text_tertiary),
                    );
                }
                Err(e) => {
                    ui.horizontal(|ui| {
                        ui.label(icons::icon(ph::WARNING, 14.0).color(p.warning));
                        ui.add(
                            egui::Label::new(
                                RichText::new(path.display().to_string()).size(12.0).color(p.text_primary),
                            )
                            .truncate(),
                        );
                    });
                    ui.label(RichText::new(e).size(11.0).color(p.danger));
                }
            });
        }
    });
}

pub fn audit(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    if !app.core.ai.audit.loaded {
        app.core.ai.load_audit();
    }
    let a = &mut app.core.ai.audit;
    let uniq = |f: &dyn Fn(&AuditRecord) -> String| {
        let mut v: Vec<String> = a.events.iter().map(f).filter(|s| !s.is_empty()).collect();
        v.sort();
        v.dedup();
        v
    };
    let sessions = uniq(&|e| e.session.clone());
    let sources = uniq(&audit::source_label);
    let models = uniq(&|e| if e.source == audit::SOURCE_MCP { String::new() } else { e.via.clone() });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.add(TextEdit::singleline(&mut a.filter).hint_text(tr!("ai-audit-filter-hint")).desired_width(140.0));
        for (id, label, value, opts) in [
            ("ai.audit.source", tr!("ai-audit-source"), &mut a.source, &sources),
            ("ai.audit.session", tr!("ai-audit-session"), &mut a.session, &sessions),
            ("ai.audit.model", tr!("ai-audit-model"), &mut a.model, &models),
        ] {
            let shown = if value.is_empty() {
                tr!("ai-audit-chip-all", label = label)
            } else {
                tr!("ai-audit-chip", label = label, value = &*value)
            };
            let r = super::chip_menu(ui, None, &shown, !value.is_empty());
            crate::ui::native::popup::Menu::menu(&r).id(egui::Id::unique(id)).show(|ui| {
                ui.set_width(240.0);
                if crate::ui::widgets::menu_item(ui, None, tr!("ai-audit-all"), None, true).clicked() {
                    value.clear();
                    ui.close();
                }
                for o in opts.iter().rev().take(40) {
                    if crate::ui::widgets::menu_item(ui, None, o, None, true).clicked() {
                        *value = o.clone();
                        ui.close();
                    }
                }
            });
        }
        if button(ui, Some(ph::ARROWS_CLOCKWISE), "", Tone::Quiet, true).on_hover_text(tr!("ai-audit-reload")).clicked()
        {
            acts.push(PanelAct::ReloadAudit);
        }
    });
    ui.add_space(6.0);
    if let Some(e) = &a.error {
        ui.label(RichText::new(e).color(p.danger));
    }
    let f = a.filter.to_lowercase();
    let rows: Vec<&AuditRecord> = a
        .events
        .iter()
        .rev()
        .filter(|e| a.session.is_empty() || e.session == a.session)
        .filter(|e| a.source.is_empty() || audit::source_label(e) == a.source)
        .filter(|e| a.model.is_empty() || e.via == a.model)
        .filter(|e| {
            f.is_empty()
                || e.detail.to_lowercase().contains(&f)
                || e.args.to_lowercase().contains(&f)
                || e.event.contains(&f)
                || e.outcome.contains(&f)
                || e.tool.contains(&f)
                || e.client.to_lowercase().contains(&f)
        })
        .collect();
    ui.label(
        RichText::new(tr!(
            "ai-audit-count",
            shown = rows.len(),
            total = a.events.len(),
            path = audit::default_path().display().to_string()
        ))
        .size(10.5)
        .color(p.text_tertiary),
    );
    ScrollArea::vertical().id_salt("soyuz.ai.audit").auto_shrink([false, false]).show_rows(
        ui,
        38.0,
        rows.len(),
        |ui, range| {
            for e in &rows[range] {
                let w = ui.available_width();
                let (r, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface2);
                }
                let mcp = e.source == audit::SOURCE_MCP;
                let (icon, color) = match (e.event.as_str(), e.outcome.as_str()) {
                    (_, "rejected" | "refused" | "timed_out" | "cancelled") => (ph::PROHIBIT, p.text_tertiary),
                    (_, "error") => (ph::WARNING, p.danger),
                    ("prompt", _) => (ph::CHAT_CIRCLE_DOTS, p.accent),
                    ("tool_call", "plan") | ("plan", _) => (ph::LIST_CHECKS, p.warning),
                    ("tool_call", _) | ("resource_read", _) => (ph::LIGHTNING, p.info),
                    ("approval", _) => (ph::CHECK, p.success),
                    ("rejection" | "edit", _) => (ph::PROHIBIT, p.text_tertiary),
                    ("job" | "job_result", _) => (ph::PULSE, p.accent),
                    ("taint", _) => (ph::SHIELD_WARNING, p.warning),
                    ("client_registered", _) => (ph::PLUGS_CONNECTED, p.warning),
                    _ => (ph::DOT_OUTLINE, p.text_tertiary),
                };
                ui.painter().text(
                    pos2(r.left() + 12.0, r.top() + 11.0),
                    Align2::CENTER_CENTER,
                    icon,
                    icons::font(12.5),
                    color,
                );
                let tool = if e.tool.is_empty() { String::new() } else { format!(" \u{00b7} {}", e.tool) };
                let outcome = if e.outcome.is_empty() || e.outcome == "ok" || e.outcome == e.event {
                    String::new()
                } else {
                    format!(" \u{2192} {}", e.outcome)
                };
                let head = format!(
                    "{}  {}{tool}{outcome}{}",
                    local_time(&e.ts),
                    e.event,
                    e.job_id
                        .map(|j| format!(" \u{00b7} {}", tr!("ai-audit-job", id = j.to_string())))
                        .unwrap_or_default()
                );
                // Source badge: who did it (the assistant or an external MCP client).
                let (badge, badge_color) = if mcp {
                    (format!("MCP \u{00b7} {}", if e.client.is_empty() { "?" } else { &e.client }), p.warning)
                } else {
                    (
                        if e.client == "recipe" {
                            tr!("ai-audit-badge-recipe")
                        } else {
                            tr!("ai-audit-badge-assistant")
                        }
                        .to_string(),
                        p.accent,
                    )
                };
                let bg = crate::ui::widgets::trunc_galley(
                    ui.painter(),
                    &badge,
                    FontId::proportional(10.0),
                    badge_color,
                    (w * 0.35).max(60.0),
                );
                let bw = bg.size().x + 12.0;
                let br = egui::Rect::from_min_size(pos2(r.right() - 4.0 - bw, r.top() + 3.0), vec2(bw, 16.0));
                ui.painter().rect_filled(br, CornerRadius::same(8), badge_color.gamma_multiply(0.12));
                ui.painter().galley(pos2(br.left() + 6.0, br.center().y - bg.size().y / 2.0), bg, badge_color);
                let mw = bw + 6.0;
                let hg = crate::ui::widgets::trunc_galley(
                    ui.painter(),
                    &head,
                    FontId::proportional(11.5),
                    p.text_primary,
                    (w - 26.0 - mw - 10.0).max(40.0),
                );
                ui.painter().galley(pos2(r.left() + 26.0, r.top() + 11.0 - hg.size().y / 2.0), hg, p.text_primary);
                let body = if !e.detail.is_empty() {
                    e.detail.replace('\n', " ")
                } else if !e.args.is_empty() {
                    e.args.clone()
                } else {
                    e.via.clone()
                };
                let g = crate::ui::widgets::trunc_galley(
                    ui.painter(),
                    &body,
                    FontId::proportional(11.0),
                    p.text_secondary,
                    w - 32.0,
                );
                ui.painter().galley(pos2(r.left() + 26.0, r.top() + 20.0), g, p.text_secondary);
                let mut tip = format!("{}\n{}", e.ts, badge);
                for (k, v) in [("session", &e.session), ("via", &e.via), ("args", &e.args), ("detail", &e.detail)] {
                    if !v.is_empty() {
                        tip.push_str(&format!("\n{k}: {v}"));
                    }
                }
                resp.on_hover_text(tip);
            }
        },
    );
}

/// `HH:MM:SS` in local time (records are stamped in UTC).
fn local_time(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
        .unwrap_or_else(|_| ts.get(11..19).unwrap_or(ts).to_string())
}

fn pill_ui(ui: &mut Ui, p: &Palette, pill: &Pill) -> bool {
    let key = pill.key.word();
    let font = FontId::proportional(11.5);
    let kg = ui.painter().layout_no_wrap(format!("{key}:"), font.clone(), p.accent);
    let vg = ui.painter().layout_no_wrap(pill.value.clone(), font, p.text_primary);
    let w = kg.size().x + vg.size().x + 34.0;
    let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    ui.painter().rect_filled(r, CornerRadius::same(12), p.accent_subtle);
    if resp.hovered() {
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(12),
            Stroke::new(1.0, p.accent.gamma_multiply(0.5)),
            StrokeKind::Inside,
        );
    }
    let x = r.left() + 10.0;
    let kw = kg.size().x;
    ui.painter().galley(pos2(x, r.center().y - kg.size().y / 2.0), kg, p.accent);
    ui.painter().galley(pos2(x + kw + 2.0, r.center().y - vg.size().y / 2.0), vg, p.text_primary);
    ui.painter().text(
        pos2(r.right() - 11.0, r.center().y),
        Align2::CENTER_CENTER,
        ph::X,
        icons::font(10.0),
        p.text_tertiary,
    );
    resp.on_hover_text(tr!("ai-smart-remove-tip")).clicked()
}

/// Natural language → pills → structured query (editable before it runs).
pub fn smart_card(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    let s = &mut app.core.ai.smart;
    Frame::new()
        .fill(p.bg_window)
        .stroke(Stroke::new(1.0, p.accent.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(icons::icon(ph::MAGNIFYING_GLASS, 14.0).color(p.accent));
                ui.label(RichText::new(tr!("ai-smart-title")).font(semibold(12.5)).color(p.text_primary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, Some(ph::X), "", Tone::Quiet, true).clicked() {
                        s.open = false;
                    }
                });
            });
            let resp =
                ui.add(TextEdit::singleline(&mut s.input).hint_text(tr!("ai-smart-hint")).desired_width(f32::INFINITY));
            if std::mem::take(&mut s.focus) {
                resp.request_focus();
            }
            if resp.changed() {
                acts.push(PanelAct::SmartParse);
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                acts.push(PanelAct::SmartRun);
            }
            ui.add_space(4.0);
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for (i, pill) in s.pills.iter().enumerate() {
                    if pill_ui(ui, p, pill) {
                        remove = Some(i);
                    }
                }
                if s.pills.is_empty() {
                    ui.label(RichText::new(tr!("ai-smart-empty")).size(11.0).color(p.text_tertiary));
                }
            });
            if let Some(i) = remove {
                s.pills.remove(i);
                s.query_text = smart_search::to_text(&s.pills);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr!("ai-smart-query")).size(10.5).color(p.text_tertiary));
                let r = ui.add(
                    TextEdit::singleline(&mut s.query_text)
                        .font(FontId::monospace(11.5))
                        .desired_width(f32::INFINITY)
                        .hint_text(PillKey::ALL.iter().map(|k| format!("{}:", k.word())).collect::<Vec<_>>().join(" ")),
                );
                if r.changed() {
                    s.pills = smart_search::parse_text(&s.query_text);
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    acts.push(PanelAct::SmartRun);
                }
            });
            if let Some(e) = &s.error {
                ui.label(RichText::new(e).size(11.0).color(p.danger));
            }
            ui.horizontal(|ui| {
                if button(
                    ui,
                    Some(ph::MAGNIFYING_GLASS),
                    tr!("ai-smart-search"),
                    Tone::Primary,
                    !s.query_text.trim().is_empty(),
                )
                .clicked()
                {
                    acts.push(PanelAct::SmartRun);
                }
                if button(ui, Some(ph::SPARKLE), tr!("ai-smart-ask"), Tone::Quiet, !s.input.trim().is_empty())
                    .on_hover_text(tr!("ai-smart-ask-tip"))
                    .clicked()
                {
                    acts.push(PanelAct::SmartAskModel);
                }
            });
        });
}

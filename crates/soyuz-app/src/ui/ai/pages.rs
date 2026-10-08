//! Panel pages: saved recipes (re-run without a model, same approvals), the audit log
//! viewer (`ai-audit.jsonl`, filter by session, client, model and text) and the smart
//! search card (natural language → editable query pills).

use egui::{
    Align, Align2, CornerRadius, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke, StrokeKind,
    TextEdit, Ui, pos2, vec2,
};

use super::{PanelAct, Tone, button, chip};
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
    if !app.ai.recipes.loaded {
        app.ai.load_recipes();
    }
    ui.label(
        RichText::new("Recipes replay saved steps in the current folder with fresh plans — no model, same approvals.")
            .size(11.5)
            .color(p.text_tertiary),
    );
    ui.add_space(6.0);
    ScrollArea::vertical().id_salt("soyuz.ai.recipes").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        if app.ai.recipes.loading.is_some() && app.ai.recipes.list.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Loading\u{2026}").color(p.text_tertiary));
            });
            return;
        }
        if app.ai.recipes.list.is_empty() {
            crate::ui::tools::compare::empty_note(
                ui,
                ph::BOOK_OPEN,
                "No recipes yet",
                "After the assistant finished a task, choose “Save as recipe” to keep its steps.",
            );
            return;
        }
        let dir = crate::ai::recipe::dir();
        for (path, r) in app.ai.recipes.list.clone() {
            card(ui, p, |ui| match &r {
                Ok(rec) => {
                    ui.horizontal(|ui| {
                        ui.label(icons::icon(ph::BOOK_OPEN, 16.0).color(p.accent));
                        ui.label(RichText::new(&rec.name).font(semibold(13.0)).color(p.text_primary));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if button(ui, Some(ph::TRASH), "", Tone::Quiet, true)
                                .on_hover_text("Delete this recipe")
                                .clicked()
                            {
                                acts.push(PanelAct::DeleteRecipe(path.clone()));
                            }
                            if button(ui, Some(ph::PLAY), "Run here", Tone::Primary, app.ai.running.is_none())
                                .on_hover_text(
                                    "Plan every step again in the current folder; each plan waits for your approval.",
                                )
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
    if !app.ai.audit.loaded {
        app.ai.load_audit();
    }
    let a = &mut app.ai.audit;
    let mut sessions: Vec<String> = a.events.iter().map(|e| e.session.clone()).collect();
    sessions.sort();
    sessions.dedup();
    let mut clients: Vec<String> = a.events.iter().map(|e| e.client.clone()).collect();
    clients.sort();
    clients.dedup();
    let mut models: Vec<String> = a.events.iter().map(|e| e.model.clone()).filter(|m| !m.is_empty()).collect();
    models.sort();
    models.dedup();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.add(TextEdit::singleline(&mut a.filter).hint_text("Filter text").desired_width(140.0));
        for (id, label, value, opts) in [
            ("ai.audit.session", "Session", &mut a.session, &sessions),
            ("ai.audit.client", "Client", &mut a.client, &clients),
            ("ai.audit.model", "Model", &mut a.model, &models),
        ] {
            let shown = if value.is_empty() { format!("{label}: all") } else { format!("{label}: {value}") };
            let r = super::chip_menu(ui, None, &shown, !value.is_empty());
            egui::Popup::menu(&r).id(egui::Id::new(id)).show(|ui| {
                ui.set_width(240.0);
                if crate::ui::widgets::menu_item(ui, None, "All", None, true).clicked() {
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
        if button(ui, Some(ph::ARROWS_CLOCKWISE), "", Tone::Quiet, true).on_hover_text("Reload").clicked() {
            acts.push(PanelAct::ReloadAudit);
        }
    });
    ui.add_space(6.0);
    if let Some(e) = &a.error {
        ui.label(RichText::new(e).color(p.danger));
    }
    let f = a.filter.to_lowercase();
    let rows: Vec<&crate::ai::audit::AuditEvent> = a
        .events
        .iter()
        .rev()
        .filter(|e| a.session.is_empty() || e.session == a.session)
        .filter(|e| a.client.is_empty() || e.client == a.client)
        .filter(|e| a.model.is_empty() || e.model == a.model)
        .filter(|e| {
            f.is_empty()
                || e.detail.to_lowercase().contains(&f)
                || e.kind.contains(&f)
                || e.tool.as_deref().is_some_and(|t| t.contains(&f))
        })
        .collect();
    ui.label(
        RichText::new(format!(
            "{} of {} events \u{00b7} {}",
            rows.len(),
            a.events.len(),
            crate::ai::audit::default_path().display()
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
                let (icon, color) = match e.kind.as_str() {
                    "prompt" => (ph::CHAT_CIRCLE_DOTS, p.accent),
                    "tool_call" => (ph::LIGHTNING, p.info),
                    "plan" => (ph::LIST_CHECKS, p.warning),
                    "approval" => (ph::CHECK, p.success),
                    "rejection" | "edit" => (ph::PROHIBIT, p.text_tertiary),
                    "job" | "job_result" => (ph::PULSE, p.accent),
                    "error" => (ph::WARNING, p.danger),
                    "taint" => (ph::SHIELD_WARNING, p.warning),
                    _ => (ph::DOT_OUTLINE, p.text_tertiary),
                };
                ui.painter().text(
                    pos2(r.left() + 12.0, r.top() + 11.0),
                    Align2::CENTER_CENTER,
                    icon,
                    icons::font(12.5),
                    color,
                );
                let head = format!(
                    "{}  {}{}{}",
                    e.ts.get(11..19).unwrap_or(&e.ts),
                    e.kind,
                    e.tool.as_deref().map(|t| format!(" \u{00b7} {t}")).unwrap_or_default(),
                    e.job.map(|j| format!(" \u{00b7} job {j}")).unwrap_or_default()
                );
                let meta = format!("{} \u{00b7} {}", e.client, e.model);
                let mg = crate::ui::widgets::trunc_galley(
                    ui.painter(),
                    &meta,
                    FontId::proportional(10.5),
                    p.text_tertiary,
                    (w * 0.4).max(60.0),
                );
                let mw = mg.size().x;
                ui.painter().galley(
                    pos2(r.right() - 6.0 - mw, r.top() + 11.0 - mg.size().y / 2.0),
                    mg,
                    p.text_tertiary,
                );
                let hg = crate::ui::widgets::trunc_galley(
                    ui.painter(),
                    &head,
                    FontId::proportional(11.5),
                    p.text_primary,
                    (w - 26.0 - mw - 16.0).max(40.0),
                );
                ui.painter().galley(pos2(r.left() + 26.0, r.top() + 11.0 - hg.size().y / 2.0), hg, p.text_primary);
                let g = crate::ui::widgets::trunc_galley(
                    ui.painter(),
                    &e.detail.replace('\n', " "),
                    FontId::proportional(11.0),
                    p.text_secondary,
                    w - 32.0,
                );
                ui.painter().galley(pos2(r.left() + 26.0, r.top() + 20.0), g, p.text_secondary);
                resp.on_hover_text(format!("{}\nsession {}\n\n{}", e.ts, e.session, e.detail));
            }
        },
    );
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
    resp.on_hover_text("Remove this filter").clicked()
}

/// Natural language → pills → structured query (editable before it runs).
pub fn smart_card(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    let s = &mut app.ai.smart;
    Frame::new()
        .fill(p.bg_window)
        .stroke(Stroke::new(1.0, p.accent.gamma_multiply(0.45)))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(icons::icon(ph::MAGNIFYING_GLASS, 14.0).color(p.accent));
                ui.label(RichText::new("Smart search").font(semibold(12.5)).color(p.text_primary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, Some(ph::X), "", Tone::Quiet, true).clicked() {
                        s.open = false;
                    }
                });
            });
            let resp = ui.add(
                TextEdit::singleline(&mut s.input)
                    .hint_text("e.g. photos from last summer larger than 5 MB")
                    .desired_width(f32::INFINITY),
            );
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
                    ui.label(RichText::new("Filters appear here as you type.").size(11.0).color(p.text_tertiary));
                }
            });
            if let Some(i) = remove {
                s.pills.remove(i);
                s.query_text = smart_search::to_text(&s.pills);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Query").size(10.5).color(p.text_tertiary));
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
                if button(ui, Some(ph::MAGNIFYING_GLASS), "Search", Tone::Primary, !s.query_text.trim().is_empty())
                    .clicked()
                {
                    acts.push(PanelAct::SmartRun);
                }
                if button(ui, Some(ph::SPARKLE), "Ask the assistant", Tone::Quiet, !s.input.trim().is_empty())
                    .on_hover_text("Let the model build the query and pick the best matches")
                    .clicked()
                {
                    acts.push(PanelAct::SmartAskModel);
                }
            });
        });
}

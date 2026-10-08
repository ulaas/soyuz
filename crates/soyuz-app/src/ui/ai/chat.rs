//! Chat tab: the conversation (markdown-ish assistant text with code blocks, tool rows,
//! plan cards, notices, task summaries with "Undo everything" and "Save as recipe") and
//! the composer with @-mentions (selection, folders, tabs, files; drop files onto it) and
//! slash commands.

use std::path::PathBuf;

use egui::{
    Align, Align2, CornerRadius, DragAndDrop, FontId, Frame, Id, Key, Layout, Margin, Modifiers, Panel, RichText,
    ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2,
};
use soyuz_core::Location;

use super::{PanelAct, Tone, button, chip, tokens};
use crate::ai::agent::Finish;
use crate::ai::state::{self, CardState, ChatItem, TaskEnd, ToolStatus, UndoState};
use crate::app::SoyuzApp;
use crate::ops::fileops::dnd::DragPayload;
use crate::theme::Palette;
use crate::theme::icons::{self, ph};
use crate::theme::tokens::radius;
use crate::ui::views::flow::semibold;

const COMPOSER_ID: &str = "soyuz.ai.composer";

pub fn show(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    Panel::bottom("soyuz.ai.footer")
        .show_separator_line(false)
        .frame(Frame::new().inner_margin(Margin { left: 0, right: 0, top: 8, bottom: 0 }))
        .show(ui, |ui| footer(ui, app, p, acts));
    let stick = std::mem::take(&mut app.ai.panel.scroll_to_bottom);
    let scroll = ScrollArea::vertical().id_salt("soyuz.ai.chat").auto_shrink([false, false]).stick_to_bottom(true);
    scroll.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;
        if app.ai.chat.is_empty() {
            empty(ui, app, p, acts);
            return;
        }
        let running = app.ai.running.as_ref().map(|r| r.task);
        let reduced = app.ui.reduced_motion;
        let n = app.ai.chat.len();
        for i in 0..n {
            let item = &mut app.ai.chat[i];
            match item {
                ChatItem::User { text, mentions } => user(ui, p, text, mentions),
                ChatItem::Assistant { text, reasoning, streaming, .. } => {
                    assistant(ui, p, text, reasoning, *streaming, reduced)
                }
                ChatItem::Tool { tool, detail, status, .. } => tool_row(ui, p, tool, detail, status),
                ChatItem::Plan(card) => super::cards::show(ui, p, card, running == Some(card.task), acts),
                ChatItem::Notice { text, warn } => notice(ui, p, text, *warn),
                ChatItem::Confirm { estimate, model, .. } => confirm(ui, p, *estimate, model, acts),
                ChatItem::TaskEnd(t) => task_end(ui, p, t, acts),
            }
        }
        ui.add_space(6.0);
        if stick {
            ui.scroll_to_cursor(Some(Align::BOTTOM));
        }
    });
}

fn empty(ui: &mut Ui, app: &SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(icons::icon_fill(ph::SPARKLE, 30.0).color(p.accent));
        ui.add_space(4.0);
        ui.label(RichText::new("Ask about this folder").font(semibold(15.0)).color(p.text_primary));
        ui.label(
            RichText::new("The assistant looks around with read tools and proposes every change as a plan you approve. Approved plans run as normal, undoable jobs.")
                .size(11.5)
                .color(p.text_tertiary),
        );
    });
    ui.add_space(14.0);
    let has_sel = !app.ws.pane().selection.is_empty();
    let mut ideas: Vec<(&str, &str, String)> = vec![
        (ph::MAGIC_WAND, "Organize this folder", "/organize".into()),
        (ph::QUESTION, "What is in here?", "/explain".into()),
        (ph::COPY, "Find duplicates", "/dedupe".into()),
    ];
    if has_sel {
        ideas.insert(0, (ph::TEXT_T, "Suggest better names for the selection", "/rename".into()));
    }
    for (icon, label, prompt) in ideas {
        let w = ui.available_width();
        let (r, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
        let hover = resp.hovered();
        ui.painter().rect_filled(
            r,
            CornerRadius::same(radius::CARD),
            if hover { p.bg_surface3 } else { p.bg_surface2 },
        );
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(radius::CARD),
            Stroke::new(1.0, p.border_subtle),
            StrokeKind::Inside,
        );
        ui.painter().text(pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, icon, icons::font(14.0), p.accent);
        ui.painter().text(
            pos2(r.left() + 36.0, r.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.5),
            p.text_primary,
        );
        ui.painter().text(
            pos2(r.right() - 12.0, r.center().y),
            Align2::RIGHT_CENTER,
            &prompt,
            FontId::monospace(11.0),
            p.text_tertiary,
        );
        if resp.clicked() {
            acts.push(PanelAct::Prompt(prompt));
        }
    }
}

fn user(ui: &mut Ui, p: &Palette, text: &str, mentions: &[PathBuf]) {
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        let max = ui.available_width() * 0.86;
        Frame::new()
            .fill(p.accent_subtle)
            .corner_radius(CornerRadius { nw: 12, ne: 4, sw: 12, se: 12 })
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_max_width(max);
                ui.vertical(|ui| {
                    ui.label(RichText::new(text).size(13.0).color(p.text_primary));
                    if !mentions.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            for m in mentions.iter().take(8) {
                                let name = soyuz_core::entry::display_name(m);
                                ui.label(RichText::new(format!("@{name}")).size(11.0).color(p.accent));
                            }
                            if mentions.len() > 8 {
                                ui.label(
                                    RichText::new(format!("+{}", mentions.len() - 8)).size(11.0).color(p.text_tertiary),
                                );
                            }
                        });
                    }
                });
            });
    });
}

fn assistant(ui: &mut Ui, p: &Palette, text: &str, reasoning: &str, streaming: bool, reduced: bool) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 11.0, p.accent.gamma_multiply(0.14));
        ui.painter().text(r.center(), Align2::CENTER_CENTER, ph::SPARKLE, icons::font_fill(11.0), p.accent);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            if !reasoning.trim().is_empty() {
                egui::CollapsingHeader::new(RichText::new("Thinking").size(11.0).color(p.text_tertiary))
                    .id_salt(ui.next_auto_id())
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(RichText::new(reasoning.trim()).size(11.5).italics().color(p.text_tertiary));
                    });
            }
            if text.trim().is_empty() && streaming {
                dots(ui, p, reduced);
                return;
            }
            let blocks = crate::services::preview_text::parse_markdown(text);
            crate::ui::preview::markdown::show(ui, &blocks, p);
            if streaming {
                let t = ui.input(|i| i.time);
                let (cr, _) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
                if reduced || (t * 2.0) as i64 % 2 == 0 {
                    ui.painter().rect_filled(cr.shrink2(vec2(1.0, 0.0)), CornerRadius::same(1), p.accent);
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }
        });
    });
}

fn dots(ui: &mut Ui, p: &Palette, reduced: bool) {
    let (r, _) = ui.allocate_exact_size(vec2(40.0, 18.0), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    for k in 0..3 {
        let phase = if reduced { 1.0 } else { ((t * 4.0 - k as f32 * 0.6).sin() * 0.5 + 0.5).max(0.25) };
        ui.painter().circle_filled(
            pos2(r.left() + 6.0 + k as f32 * 11.0, r.center().y),
            3.2,
            p.text_tertiary.gamma_multiply(phase),
        );
    }
    if !reduced {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
    }
}

fn tool_row(ui: &mut Ui, p: &Palette, tool: &str, detail: &str, status: &ToolStatus) {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    let x0 = r.left() + 30.0;
    match status {
        ToolStatus::Running => crate::ui::widgets::spinner(ui, pos2(r.left() + 11.0, r.center().y), 6.0, p.info),
        ToolStatus::Ok(_) => {
            ui.painter().text(
                pos2(r.left() + 11.0, r.center().y),
                Align2::CENTER_CENTER,
                ph::CHECK,
                icons::font(12.0),
                p.success,
            );
        }
        ToolStatus::Failed(_) => {
            ui.painter().text(
                pos2(r.left() + 11.0, r.center().y),
                Align2::CENTER_CENTER,
                ph::WARNING,
                icons::font(12.0),
                p.warning,
            );
        }
    }
    let name = ui.painter().layout_no_wrap(tool.to_string(), FontId::monospace(11.5), p.text_secondary);
    let nw = name.size().x;
    ui.painter().galley(pos2(x0, r.center().y - name.size().y / 2.0), name, p.text_secondary);
    let brief = match status {
        ToolStatus::Ok(b) | ToolStatus::Failed(b) => b.as_str(),
        ToolStatus::Running => detail,
    };
    let color = if matches!(status, ToolStatus::Failed(_)) { p.warning } else { p.text_tertiary };
    let g = crate::ui::widgets::trunc_galley(
        ui.painter(),
        brief,
        FontId::proportional(11.5),
        color,
        (r.right() - x0 - nw - 10.0).max(20.0),
    );
    ui.painter().galley(pos2(x0 + nw + 8.0, r.center().y - g.size().y / 2.0), g, color);
    if !detail.is_empty() {
        resp.on_hover_text(format!("{tool}({detail})\n{brief}"));
    }
}

fn notice(ui: &mut Ui, p: &Palette, text: &str, warn: bool) {
    let c = if warn { p.warning } else { p.text_tertiary };
    ui.horizontal_top(|ui| {
        ui.add_space(4.0);
        ui.label(icons::icon(if warn { ph::WARNING } else { ph::INFO }, 12.0).color(c));
        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(c)).wrap());
    });
}

fn confirm(ui: &mut Ui, p: &Palette, estimate: f64, model: &str, acts: &mut Vec<PanelAct>) {
    Frame::new()
        .fill(p.warning.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0, p.warning.gamma_multiply(0.4)))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(icons::icon(ph::COINS, 16.0).color(p.warning));
                ui.label(
                    RichText::new(format!("This task may cost about ${estimate:.2}"))
                        .font(semibold(13.0))
                        .color(p.text_primary),
                );
            });
            ui.label(
                RichText::new(format!("Estimated for {model} over a full run. You can stop it at any time with Esc."))
                    .size(11.5)
                    .color(p.text_secondary),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if button(ui, Some(ph::PLAY), "Start", Tone::Primary, true).clicked() {
                    acts.push(PanelAct::Confirm(true));
                }
                if button(ui, None, "Cancel", Tone::Normal, true).clicked() {
                    acts.push(PanelAct::Confirm(false));
                }
            });
        });
}

fn task_end(ui: &mut Ui, p: &Palette, t: &mut TaskEnd, acts: &mut Vec<PanelAct>) {
    let ok = matches!(t.reason, Finish::Done);
    let (icon, color) = match &t.reason {
        Finish::Done => (ph::CHECK_CIRCLE, p.success),
        Finish::Cancelled => (ph::STOP_CIRCLE, p.text_tertiary),
        Finish::Error(_) => (ph::WARNING_OCTAGON, p.danger),
        _ => (ph::WARNING, p.warning),
    };
    ui.add_space(2.0);
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    ui.painter()
        .line_segment([pos2(r.left(), r.center().y), pos2(r.right(), r.center().y)], Stroke::new(1.0, p.border_subtle));
    let changes = t.executed.len();
    let mut label = t.reason.describe();
    if changes > 0 {
        label.push_str(&format!(" \u{00b7} {changes} change{}", if changes == 1 { "" } else { "s" }));
    }
    if t.usage.total() > 0 {
        label.push_str(&format!(" \u{00b7} {} tokens", tokens(t.usage.total())));
    }
    if let Some(c) = t.cost {
        label.push_str(&format!(" \u{00b7} ${c:.3}"));
    }
    let g = ui.painter().layout_no_wrap(label, FontId::proportional(11.0), color);
    let gw = g.size().x + 26.0;
    let pill = egui::Rect::from_center_size(r.center(), vec2(gw.min(w), 18.0));
    ui.painter().rect_filled(pill, CornerRadius::same(9), p.bg_surface1);
    ui.painter().text(pos2(pill.left() + 9.0, pill.center().y), Align2::CENTER_CENTER, icon, icons::font(11.0), color);
    ui.painter().galley(pos2(pill.left() + 19.0, pill.center().y - g.size().y / 2.0), g, color);
    if let Finish::Error(e) = &t.reason
        && e.contains("Settings")
    {
        ui.horizontal(|ui| {
            if button(ui, Some(ph::GEAR_SIX), "Open AI settings", Tone::Normal, true).clicked() {
                acts.push(PanelAct::OpenSettings);
            }
        });
    }
    if changes == 0 {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        match &t.undo {
            UndoState::None => {
                if button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), "Undo everything the assistant did", Tone::Normal, true)
                    .on_hover_text("Undo every job of this task, newest first (history undo).")
                    .clicked()
                {
                    acts.push(PanelAct::Undo(t.task));
                }
            }
            UndoState::Running => {
                button(ui, Some(ph::HOURGLASS), "Undoing\u{2026}", Tone::Normal, false);
            }
            UndoState::Done(msg) => {
                ui.label(RichText::new(msg).size(11.5).color(p.text_secondary));
            }
        }
        if !t.recipe_run && ok {
            match &mut t.recipe_name {
                None => {
                    if t.saved.is_none()
                        && button(ui, Some(ph::FLOPPY_DISK), "Save as recipe", Tone::Quiet, true)
                            .on_hover_text("Save these steps as a TOML recipe you can re-run later without a model (same preview and approval).")
                            .clicked()
                    {
                        t.recipe_name = Some(String::new());
                    }
                }
                Some(name) => {
                    let resp = ui.add(
                        TextEdit::singleline(name).hint_text("Recipe name").desired_width(150.0).font(FontId::proportional(12.0)),
                    );
                    resp.request_focus();
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if (button(ui, Some(ph::CHECK), "Save", Tone::Primary, !name.trim().is_empty()).clicked() || enter)
                        && !name.trim().is_empty()
                    {
                        acts.push(PanelAct::SaveRecipe(t.task, name.trim().to_string()));
                        t.recipe_name = None;
                        t.saved = Some("Saving\u{2026}".into());
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        t.recipe_name = None;
                    }
                }
            }
        }
        if let Some(s) = &t.saved {
            ui.label(RichText::new(s).size(11.5).color(p.success));
        }
    });
}

// ---------------------------------------------------------------------------------------
// Footer: pending bar, smart search, mentions, composer.

fn footer(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    if app.ai.smart.open {
        super::pages::smart_card(ui, app, p, acts);
        ui.add_space(6.0);
    }
    if let Some(r) = &app.ai.running {
        let task = r.task;
        let pending: Vec<bool> = app
            .ai
            .chat
            .iter()
            .filter_map(|c| match c {
                ChatItem::Plan(x) if x.task == task && x.state == CardState::Pending => Some(x.plan.executable),
                _ => None,
            })
            .collect();
        if pending.len() > 1 {
            Frame::new()
                .fill(p.warning.gamma_multiply(0.08))
                .corner_radius(CornerRadius::same(radius::CARD))
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(icons::icon(ph::HOURGLASS, 13.0).color(p.warning));
                        ui.label(
                            RichText::new(format!("{} plans wait for your approval", pending.len()))
                                .size(12.0)
                                .color(p.text_primary),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if button(ui, Some(ph::CHECKS), "Approve all", Tone::Primary, pending.iter().any(|e| *e))
                                .clicked()
                            {
                                acts.push(PanelAct::ApproveAllPending(task));
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        }
    }
    composer(ui, app, p, acts);
}

/// Candidates for an `@query` mention.
fn mention_candidates(app: &SoyuzApp, q: &str) -> Vec<(String, String, Vec<PathBuf>)> {
    let q = q.to_lowercase();
    let pane = app.ws.pane();
    let mut out: Vec<(String, String, Vec<PathBuf>)> = Vec::new();
    let sel: Vec<PathBuf> = pane.selected_entries().iter().map(|e| e.path.clone()).collect();
    if !sel.is_empty() && "selection".contains(&q) {
        out.push((ph::SELECTION_ALL.into(), format!("selection ({} items)", sel.len()), sel));
    }
    if let Location::Dir(d) = &pane.location
        && ("folder".contains(&q) || "here".contains(&q))
    {
        out.push((
            ph::FOLDER_OPEN.into(),
            format!("this folder: {}", soyuz_core::entry::display_name(d)),
            vec![d.clone()],
        ));
    }
    for t in &app.ws.tabs {
        if let Location::Dir(d) = &t.active().location {
            let name = soyuz_core::entry::display_name(d);
            if (q.is_empty() || name.to_lowercase().contains(&q) || "tab".contains(&q))
                && !out.iter().any(|(_, _, v)| v.len() == 1 && &v[0] == d)
            {
                out.push((ph::BROWSERS.into(), format!("tab: {name}"), vec![d.clone()]));
            }
        }
    }
    if !q.is_empty() {
        for e in pane.entries.iter().filter(|e| e.name.to_lowercase().contains(&q)).take(8) {
            let icon = if e.is_dir_like() { ph::FOLDER_SIMPLE } else { ph::FILE };
            out.push((icon.into(), e.name.clone(), vec![e.path.clone()]));
        }
    }
    out.truncate(10);
    out
}

fn composer(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    let id = Id::new(COMPOSER_ID);
    // Mentions.
    if !app.ai.panel.mentions.is_empty() {
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for (i, m) in app.ai.panel.mentions.iter().enumerate() {
                let name = soyuz_core::entry::display_name(m);
                if chip(ui, Some(ph::AT), &format!("{name}  \u{00d7}"), true)
                    .on_hover_text(m.display().to_string())
                    .clicked()
                {
                    remove = Some(i);
                }
            }
        });
        if let Some(i) = remove {
            app.ai.panel.mentions.remove(i);
        }
        ui.add_space(4.0);
    }
    // Popups above the composer: slash commands / mentions.
    let text = app.ai.panel.composer.clone();
    let last_tok = text.rsplit(char::is_whitespace).next().unwrap_or("");
    let slash = text.starts_with('/') && !text.contains(char::is_whitespace);
    let mention = last_tok.starts_with('@');
    let has_focus = ui.memory(|m| m.has_focus(id));
    // Keep a popup alive for the frame in which a click on it takes the focus away.
    let pop_id = id.with("popup");
    let was_open = ui.data(|d| d.get_temp::<bool>(pop_id).unwrap_or(false));
    let focused = has_focus || (was_open && (slash || mention) && !ui.input(|i| i.key_pressed(Key::Escape)));
    ui.data_mut(|d| d.insert_temp(pop_id, focused && (slash || mention)));
    if focused && slash {
        let matches: Vec<_> = state::SLASH.iter().filter(|(c, _, _)| c.starts_with(&text.to_lowercase())).collect();
        if !matches.is_empty() {
            popup_frame(ui, p, |ui| {
                for (c, d, _) in &matches {
                    if pick_row(ui, p, ph::LIGHTNING, c, d) {
                        app.ai.panel.composer = format!("{c} ");
                        app.ai.panel.focus_composer = true;
                    }
                }
            });
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                app.ai.panel.composer = format!("{} ", matches[0].0);
            }
        }
    }
    if focused && mention {
        let cands = mention_candidates(app, &last_tok[1..]);
        if !cands.is_empty() {
            let mut chosen = None;
            popup_frame(ui, p, |ui| {
                for (icon, label, paths) in &cands {
                    if pick_row(ui, p, icon, label, "") {
                        chosen = Some(paths.clone());
                    }
                }
            });
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                chosen = Some(cands[0].2.clone());
            }
            if let Some(paths) = chosen {
                let cut = app.ai.panel.composer.len() - last_tok.len();
                app.ai.panel.composer.truncate(cut);
                for p in paths {
                    if !app.ai.panel.mentions.contains(&p) {
                        app.ai.panel.mentions.push(p);
                    }
                }
                app.ai.panel.focus_composer = true;
            }
        }
    }
    // Enter sends (Shift+Enter = new line).
    let popup = (focused && slash) || (focused && mention);
    if has_focus
        && !popup
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
        && !app.ai.panel.composer.trim().is_empty()
        && app.ai.running.is_none()
    {
        acts.push(PanelAct::Send);
    }
    let running = app.ai.running.is_some();
    let frame_rect = Frame::new()
        .fill(p.bg_sunken)
        .stroke(Stroke::new(1.0, if has_focus { p.accent.gamma_multiply(0.7) } else { p.border_subtle }))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin { left: 10, right: 6, top: 8, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let hint = if running {
                "The assistant is working\u{2026}  (Esc stops it)"
            } else {
                "Ask or give a task\u{2026}   / commands \u{00b7} @ mention"
            };
            let resp = ui.add(
                TextEdit::multiline(&mut app.ai.panel.composer)
                    .id(id)
                    .hint_text(RichText::new(hint).color(p.text_tertiary))
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .frame(Frame::NONE)
                    .font(FontId::proportional(13.0)),
            );
            if std::mem::take(&mut app.ai.panel.focus_composer) {
                resp.request_focus();
            }
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Enter to send \u{00b7} Shift+Enter new line").size(10.5).color(p.text_tertiary),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if running {
                        if button(ui, Some(ph::STOP), "Stop", Tone::Danger, true).clicked() {
                            acts.push(PanelAct::Stop);
                        }
                    } else if button(
                        ui,
                        Some(ph::PAPER_PLANE_RIGHT),
                        "Send",
                        Tone::Primary,
                        !app.ai.panel.composer.trim().is_empty(),
                    )
                    .clicked()
                    {
                        acts.push(PanelAct::Send);
                    }
                    if chip(ui, Some(ph::MAGNIFYING_GLASS), "Smart search", app.ai.smart.open).clicked() {
                        app.ai.smart.open = !app.ai.smart.open;
                        app.ai.smart.focus = app.ai.smart.open;
                    }
                });
            });
        })
        .response
        .rect;
    // Drop files from the panes onto the composer.
    if let Some(payload) = DragAndDrop::payload::<DragPayload>(ui.ctx()) {
        let hover = ui.input(|i| i.pointer.hover_pos()).is_some_and(|pos| frame_rect.contains(pos));
        if hover {
            ui.painter().rect_stroke(
                frame_rect.expand(2.0),
                CornerRadius::same(radius::CARD + 2),
                Stroke::new(2.0, p.accent),
                StrokeKind::Outside,
            );
            ui.painter().rect_filled(frame_rect, CornerRadius::same(radius::CARD), p.accent.gamma_multiply(0.06));
            let n = payload.paths.len();
            ui.painter().text(
                frame_rect.center(),
                Align2::CENTER_CENTER,
                format!("Mention {n} item{}", if n == 1 { "" } else { "s" }),
                semibold(12.5),
                p.accent,
            );
            if ui.input(|i| i.pointer.any_released())
                && let Some(pl) = DragAndDrop::take_payload::<DragPayload>(ui.ctx())
            {
                for path in pl.paths.iter() {
                    if !app.ai.panel.mentions.contains(path) {
                        app.ai.panel.mentions.push(path.clone());
                    }
                }
                app.ai.panel.focus_composer = true;
            }
        }
    }
}

fn popup_frame(ui: &mut Ui, p: &Palette, body: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(p.bg_surface2)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        });
    ui.add_space(4.0);
}

fn pick_row(ui: &mut Ui, p: &Palette, icon: &str, label: &str, desc: &str) -> bool {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface3);
    }
    ui.painter().text(pos2(r.left() + 12.0, r.center().y), Align2::CENTER_CENTER, icon, icons::font(12.5), p.accent);
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), p.text_primary);
    let lw = g.size().x;
    ui.painter().galley(pos2(r.left() + 26.0, r.center().y - g.size().y / 2.0), g, p.text_primary);
    if !desc.is_empty() {
        let d = crate::ui::widgets::trunc_galley(
            ui.painter(),
            desc,
            FontId::proportional(11.0),
            p.text_tertiary,
            (w - lw - 40.0).max(10.0),
        );
        ui.painter().galley(pos2(r.left() + 34.0 + lw, r.center().y - d.size().y / 2.0), d, p.text_tertiary);
    }
    resp.clicked()
}

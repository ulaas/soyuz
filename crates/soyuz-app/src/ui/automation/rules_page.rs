//! Settings → Automations: global switches, the CLI pipe, and one card per rule with
//! its filters, steps, a live preview of what it would match right now, Run now and the
//! rule's run log.

use std::path::PathBuf;

use egui::{Align, Grid, Layout, RichText, Ui, vec2};
use soyuz_core::config::{AutomationRule, RuleAction, RuleConflict, RuleKind};
use soyuz_core::{SizeStyle, format_size};
use soyuz_script::rules::worker::LogKind;

use super::card;
use crate::app::SoyuzApp;
use crate::automation::AutoCmd;
use crate::cmd::Cmd;
use crate::theme::icons::ph;
use crate::theme::{Palette, icons, palette};
use crate::ui::settings::widgets::{code, dropdown, group, matches, number, segmented, small_button, switch, text};

const CTX: &str = "Automations rules watch folder";

const TAGS: [&str; 8] = ["red", "orange", "yellow", "green", "cyan", "blue", "purple", "gray"];

fn label(ui: &mut Ui, p: &Palette, s: &str) {
    ui.label(RichText::new(s).size(12.0).color(p.text_secondary));
}

pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let mut any = false;
    let cli_status = app.automation.cli_status.clone();
    let note = app.automation.rules_note.clone();
    let d = &mut app.settings.draft;
    any |= group(ui, q, CTX, "Automations", |r| {
        r.row("Run rules", "Act on new items in watched folders while Soyuz is open.", |ui| {
            switch(ui, &mut d.automation.enabled);
        });
        r.row_kw("Dry run", "Every rule only writes what it would do to its run log.", "test simulate", |ui| {
            switch(ui, &mut d.automation.dry_run);
        });
        r.row_kw("Run log size", "Entries kept per rule.", "history", |ui| {
            number(ui, &mut d.automation.log_entries, 10..=5_000, "");
        });
        r.row_kw(
            "Accept commands from the terminal",
            "soyuz cp | mv | rm | trash | open | reveal | jobs send their jobs to this window (current user only).",
            "cli command line pipe",
            |ui| {
                switch(ui, &mut d.cli.server_enabled);
                let p = palette(ui.ctx());
                match &cli_status {
                    Some(Ok(name)) => {
                        ui.label(RichText::new(name).monospace().size(10.5).color(p.text_tertiary));
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).size(11.0).color(p.warning));
                    }
                    None => {}
                }
            },
        );
    });
    let searching = !q.trim().is_empty();
    let p = palette(ui.ctx());
    if let Some(n) = &note
        && !searching
    {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(icons::icon(ph::INFO, 13.0).color(p.info));
            ui.label(RichText::new(n).size(12.0).color(p.text_secondary));
        });
    }
    if !searching {
        any = true;
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if small_button(ui, Some(ph::PLUS), "New rule", false).clicked() {
                let watch = app.ws.pane().location.as_fs_path().map(|p| p.to_path_buf()).unwrap_or_default();
                app.settings.draft.automation.rules.push(AutomationRule {
                    name: format!("Rule {}", app.settings.draft.automation.rules.len() + 1),
                    watch,
                    ..Default::default()
                });
            }
            ui.label(RichText::new("or start from").size(11.5).color(p.text_tertiary));
            if small_button(ui, Some(ph::FILE_PDF), "PDFs from Downloads → Documents/PDF + blue tag", false).clicked()
            {
                let home = soyuz_fs::home();
                app.settings.draft.automation.rules.push(AutomationRule {
                    name: "File PDFs".into(),
                    watch: home.join("Downloads"),
                    glob: "*.pdf".into(),
                    actions: vec![
                        RuleAction::Move { to: home.join("Documents").join("PDF") },
                        RuleAction::Tag { tag: "blue".into() },
                    ],
                    ..Default::default()
                });
            }
            if small_button(ui, Some(ph::BROOM), "Trash old installers", false).clicked() {
                app.settings.draft.automation.rules.push(AutomationRule {
                    name: "Old installers".into(),
                    watch: soyuz_fs::home().join("Downloads"),
                    glob: "*.msi;*.exe".into(),
                    min_age_minutes: 30 * 24 * 60,
                    apply_to_existing: true,
                    dry_run: true,
                    actions: vec![RuleAction::Trash],
                    ..Default::default()
                });
            }
        });
        ui.label(
            RichText::new(
                "Each step runs as a job (activity panel, undo). Rules wait until an item stops changing, and never act on \
                 partial downloads.",
            )
            .size(11.5)
            .color(p.text_tertiary),
        );
    }
    let n = app.settings.draft.automation.rules.len();
    let mut remove = None;
    let mut moves = None;
    for i in 0..n {
        let r = &app.settings.draft.automation.rules[i];
        if searching && !matches(q, &[&r.name, &r.glob, &r.watch.to_string_lossy(), "rule automation"]) {
            continue;
        }
        any = true;
        ui.add_space(10.0);
        let rule_now = r.clone();
        let preview = crate::automation::rule_preview(app, i, &rule_now);
        let status = app.automation.status.get(&rule_now.name).cloned();
        let log: Vec<soyuz_script::rules::worker::LogEntry> = app
            .automation
            .logs
            .get(&rule_now.name)
            .map(|l| l.iter().rev().take(8).cloned().collect())
            .unwrap_or_default();
        let mut cmds: Vec<Cmd> = Vec::new();
        let r = &mut app.settings.draft.automation.rules[i];
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(icons::icon(ph::ROBOT, 16.0).color(p.accent));
                ui.add(
                    egui::TextEdit::singleline(&mut r.name)
                        .hint_text("Rule name")
                        .font(egui::FontId::proportional(14.0))
                        .desired_width(220.0)
                        .margin(egui::Margin::symmetric(8, 4)),
                );
                switch(ui, &mut r.enabled);
                let (dot, tip) = match (&status, r.enabled) {
                    (_, false) => (p.text_tertiary, "Off".to_string()),
                    (Some(Ok(())), _) => (p.success, "Watching".to_string()),
                    (Some(Err(e)), _) => (p.danger, e.clone()),
                    (None, _) => (p.text_tertiary, "Not running yet".to_string()),
                };
                let (rect, resp) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.5, dot);
                resp.on_hover_text(&tip);
                if let Some(Err(e)) = &status {
                    ui.label(RichText::new(e).size(11.0).color(p.danger));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if small_button(ui, Some(ph::TRASH), "", true).on_hover_text("Delete rule").clicked() {
                        remove = Some(i);
                    }
                    if i + 1 < n && small_button(ui, Some(ph::ARROW_DOWN), "", false).clicked() {
                        moves = Some((i, i + 1));
                    }
                    if i > 0 && small_button(ui, Some(ph::ARROW_UP), "", false).clicked() {
                        moves = Some((i - 1, i));
                    }
                    if small_button(ui, Some(ph::PLAY), "Run now", false)
                        .on_hover_text("Act on everything that matches right now")
                        .clicked()
                    {
                        cmds.push(Cmd::Automation(AutoCmd::RunRule(r.name.clone())));
                    }
                });
            });
            ui.add_space(8.0);
            Grid::new(("soyuz.rule", i)).num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                label(ui, &p, "Watch");
                ui.horizontal(|ui| {
                    let mut s = r.watch.to_string_lossy().into_owned();
                    if code(ui, &mut s, "C:\\Users\\me\\Downloads", 300.0).changed() {
                        r.watch = PathBuf::from(s.trim());
                    }
                    ui.checkbox(&mut r.recursive, "Subfolders");
                });
                ui.end_row();
                label(ui, &p, "Names");
                ui.horizontal(|ui| {
                    code(ui, &mut r.glob, "*.pdf;*.epub  (empty = any)", 180.0);
                    code(ui, &mut r.regex, "regex (optional)", 140.0);
                });
                ui.end_row();
                label(ui, &p, "Items");
                segmented(
                    ui,
                    &mut r.kind,
                    &[(RuleKind::Files, "Files"), (RuleKind::Folders, "Folders"), (RuleKind::Any, "Both")],
                );
                ui.end_row();
                label(ui, &p, "Size (KiB)");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut r.min_size_kib).prefix("≥ ").speed(16));
                    ui.add(egui::DragValue::new(&mut r.max_size_kib).prefix("≤ ").speed(16));
                    ui.label(RichText::new("0 = no limit").size(11.0).color(p.text_tertiary));
                });
                ui.end_row();
                label(ui, &p, "Age (minutes)");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut r.min_age_minutes).prefix("≥ ").speed(1));
                    ui.add(egui::DragValue::new(&mut r.max_age_minutes).prefix("≤ ").speed(1));
                    ui.label(RichText::new("by modification time").size(11.0).color(p.text_tertiary));
                });
                ui.end_row();
                label(ui, &p, "Wait");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut r.delay_secs).range(0..=3600).suffix(" s"));
                    ui.label(RichText::new("after the last change").size(11.0).color(p.text_tertiary));
                    ui.checkbox(&mut r.apply_to_existing, "Also existing items");
                });
                ui.end_row();
                label(ui, &p, "Conflicts");
                ui.horizontal(|ui| {
                    let opts = [
                        (RuleConflict::KeepBoth, RuleConflict::KeepBoth.label()),
                        (RuleConflict::Skip, RuleConflict::Skip.label()),
                        (RuleConflict::Overwrite, RuleConflict::Overwrite.label()),
                        (RuleConflict::Ask, RuleConflict::Ask.label()),
                    ];
                    dropdown(ui, &format!("rule_conflict_{i}"), &mut r.conflict, &opts, 110.0);
                    ui.checkbox(&mut r.dry_run, "Dry run");
                });
                ui.end_row();
                label(ui, &p, "Then");
                ui.vertical(|ui| steps(ui, &p, i, &mut r.actions));
                ui.end_row();
            });
            ui.add_space(8.0);
            preview_block(ui, &p, preview.as_ref());
            if !log.is_empty() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("RUN LOG").size(10.5).color(p.text_tertiary));
                    if small_button(ui, None, "Clear", false).clicked() {
                        cmds.push(Cmd::Automation(AutoCmd::ClearLog(r.name.clone())));
                    }
                });
                for e in &log {
                    log_row(ui, &p, e);
                }
            }
        });
        app.cmds.extend(cmds);
    }
    if let Some(i) = remove {
        app.settings.draft.automation.rules.remove(i);
        app.automation.previews.clear();
    }
    if let Some((a, b)) = moves {
        app.settings.draft.automation.rules.swap(a, b);
        app.automation.previews.clear();
    }
    any
}

/// Editor of a rule's steps.
fn steps(ui: &mut Ui, p: &Palette, i: usize, actions: &mut Vec<RuleAction>) {
    let mut remove = None;
    let scripts: Vec<String> = crate::automation::scripts::current()
        .map(|s| s.active().filter(|x| x.has_rule).map(|x| x.name.clone()).collect())
        .unwrap_or_default();
    for (k, a) in actions.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}.", k + 1)).size(12.0).color(p.text_tertiary));
            ui.label(RichText::new(a.label()).size(12.5).color(p.text_primary));
            match a {
                RuleAction::Move { to } | RuleAction::Copy { to } => {
                    let mut s = to.to_string_lossy().into_owned();
                    if code(ui, &mut s, "destination folder", 260.0).changed() {
                        *to = PathBuf::from(s.trim());
                    }
                }
                RuleAction::Rename { pattern } => {
                    code(ui, pattern, "{mdate}-{name}", 200.0)
                        .on_hover_text("{name} {stem} {ext} {date} {time} {mdate} {parent}");
                }
                RuleAction::Tag { tag } => {
                    let opts: Vec<(String, &str)> = TAGS.iter().map(|t| (t.to_string(), *t)).collect();
                    dropdown(ui, &format!("rule_tag_{i}_{k}"), tag, &opts, 100.0);
                }
                RuleAction::Script { script } => {
                    if scripts.is_empty() {
                        text(ui, script, "script name", 160.0)
                            .on_hover_text("A script in the scripts folder that defines fn rule(path, ctx)");
                    } else {
                        let opts: Vec<(String, &str)> = scripts.iter().map(|s| (s.clone(), s.as_str())).collect();
                        dropdown(ui, &format!("rule_script_{i}_{k}"), script, &opts, 160.0);
                    }
                }
                RuleAction::Trash => {}
            }
            if small_button(ui, Some(ph::X), "", true).on_hover_text("Remove step").clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        actions.remove(k);
    }
    ui.horizontal_wrapped(|ui| {
        let home = soyuz_fs::home();
        for (glyph, name, make) in [
            (ph::ARROW_SQUARE_RIGHT, "Move", RuleAction::Move { to: home.join("Documents") }),
            (ph::COPY, "Copy", RuleAction::Copy { to: home.join("Documents") }),
            (ph::TEXT_T, "Rename", RuleAction::Rename { pattern: "{mdate}-{name}".into() }),
            (ph::TAG, "Tag", RuleAction::Tag { tag: "blue".into() }),
            (ph::TRASH, "Trash", RuleAction::Trash),
            (ph::CODE, "Script", RuleAction::Script { script: scripts.first().cloned().unwrap_or_default() }),
        ] {
            if small_button(ui, Some(glyph), name, false).on_hover_text("Add a step").clicked() {
                actions.push(make);
            }
        }
    });
}

fn preview_block(ui: &mut Ui, p: &Palette, preview: Option<&crate::automation::RulePreview>) {
    let Some(pv) = preview else { return };
    ui.horizontal(|ui| {
        ui.label(RichText::new("MATCHES NOW").size(10.5).color(p.text_tertiary));
        if pv.loading {
            ui.add(egui::Spinner::new().size(10.0));
        }
    });
    match &pv.result {
        None => {}
        Some(Err(e)) => {
            ui.label(RichText::new(e).size(12.0).color(p.warning));
        }
        Some(Ok((items, total))) => {
            if *total == 0 {
                ui.label(RichText::new("Nothing in the folder matches right now.").size(12.0).color(p.text_tertiary));
            }
            for it in items.iter().take(6) {
                ui.horizontal(|ui| {
                    ui.label(icons::icon(if it.is_dir { ph::FOLDER } else { ph::FILE }, 13.0).color(p.text_secondary));
                    let name = it.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(RichText::new(name).size(12.0).color(p.text_primary));
                    if !it.is_dir {
                        ui.label(
                            RichText::new(format_size(it.size, SizeStyle::Binary)).size(11.0).color(p.text_tertiary),
                        );
                    }
                    ui.add(
                        egui::Label::new(RichText::new(it.steps.join(" → ")).size(11.0).color(p.text_tertiary))
                            .truncate(),
                    );
                });
            }
            if *total > 6 {
                ui.label(RichText::new(format!("…and {} more", total - 6)).size(11.5).color(p.text_tertiary));
            }
        }
    }
}

fn log_row(ui: &mut Ui, p: &Palette, e: &soyuz_script::rules::worker::LogEntry) {
    let (glyph, color) = match e.kind {
        LogKind::Done => (ph::CHECK, p.success),
        LogKind::DryRun => (ph::EYE, p.info),
        LogKind::Error => (ph::WARNING, p.danger),
        LogKind::Info => (ph::INFO, p.text_secondary),
    };
    let when = {
        let dt: chrono::DateTime<chrono::Local> = e.when.into();
        let today = chrono::Local::now().date_naive() == dt.date_naive();
        if today { dt.format("%H:%M:%S").to_string() } else { dt.format("%Y-%m-%d %H:%M").to_string() }
    };
    ui.horizontal(|ui| {
        ui.label(icons::icon(glyph, 12.0).color(color));
        ui.label(RichText::new(when).monospace().size(11.0).color(p.text_tertiary));
        let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        ui.label(RichText::new(name).size(12.0).color(p.text_primary));
        ui.add(egui::Label::new(RichText::new(&e.message).size(11.5).color(p.text_secondary)).truncate());
    });
}

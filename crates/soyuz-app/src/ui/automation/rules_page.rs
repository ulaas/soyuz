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
use crate::i18n::labels::{rule_action, rule_conflict, rule_kind};
use crate::theme::icons::ph;
use crate::theme::{Palette, icons, palette};
use crate::ui::settings::widgets::{code, dropdown, group, matches, number, segmented, small_button, switch, text};

const CTX: &str = "Automations rules watch folder otomasyonlar kurallar izle klasör";

const TAGS: [&str; 8] = ["red", "orange", "yellow", "green", "cyan", "blue", "purple", "gray"];

/// Display name of a tag color in [`TAGS`] (the user's name for it, if renamed).
fn tag_name(tag: &str) -> String {
    crate::services::tags::display_name(tag)
}

fn label(ui: &mut Ui, p: &Palette, s: &str) {
    ui.label(RichText::new(s).size(12.0).color(p.text_secondary));
}

pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let mut any = false;
    let cli_status = app.core.automation.cli_status.clone();
    let note = app.core.automation.rules_note.clone();
    let d = &mut app.gui.settings.draft;
    any |= group(ui, q, CTX, tr!("automation-rules-group"), |r| {
        r.row(tr!("automation-rules-run"), tr!("automation-rules-run-desc"), |ui| {
            switch(ui, &mut d.automation.enabled);
        });
        r.row_kw(tr!("automation-rules-dry-run"), tr!("automation-rules-dry-run-desc"), "test simulate", |ui| {
            switch(ui, &mut d.automation.dry_run);
        });
        r.row_kw(tr!("automation-rules-log-size"), tr!("automation-rules-log-size-desc"), "history", |ui| {
            number(ui, &mut d.automation.log_entries, 10..=5_000, "");
        });
        r.row_kw(tr!("automation-rules-cli"), tr!("automation-rules-cli-desc"), "cli command line pipe", |ui| {
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
        });
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
            if small_button(ui, Some(ph::PLUS), tr!("automation-rules-new"), false).clicked() {
                let watch = app.core.ws.pane().location.as_fs_path().map(|p| p.to_path_buf()).unwrap_or_default();
                app.gui.settings.draft.automation.rules.push(AutomationRule {
                    name: tr!(
                        "automation-rules-default-name",
                        n = (app.gui.settings.draft.automation.rules.len() + 1).to_string()
                    ),
                    watch,
                    ..Default::default()
                });
            }
            ui.label(RichText::new(tr!("automation-rules-start-from")).size(11.5).color(p.text_tertiary));
            if small_button(ui, Some(ph::FILE_PDF), tr!("automation-rules-preset-pdf"), false).clicked() {
                let home = soyuz_fs::home();
                app.gui.settings.draft.automation.rules.push(AutomationRule {
                    name: tr!("automation-rules-preset-pdf-name").into(),
                    watch: home.join("Downloads"),
                    glob: "*.pdf".into(),
                    actions: vec![
                        RuleAction::Move { to: home.join("Documents").join("PDF") },
                        RuleAction::Tag { tag: "blue".into() },
                    ],
                    ..Default::default()
                });
            }
            if small_button(ui, Some(ph::BROOM), tr!("automation-rules-preset-installers"), false).clicked() {
                app.gui.settings.draft.automation.rules.push(AutomationRule {
                    name: tr!("automation-rules-preset-installers-name").into(),
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
        ui.label(RichText::new(tr!("automation-rules-intro")).size(11.5).color(p.text_tertiary));
    }
    let n = app.gui.settings.draft.automation.rules.len();
    let mut remove = None;
    let mut moves = None;
    for i in 0..n {
        let r = &app.gui.settings.draft.automation.rules[i];
        if searching && !matches(q, &[&r.name, &r.glob, &r.watch.to_string_lossy(), "rule automation"]) {
            continue;
        }
        any = true;
        ui.add_space(10.0);
        let rule_now = r.clone();
        let preview = crate::automation::rule_preview(&mut app.core, i, &rule_now);
        let status = app.core.automation.status.get(&rule_now.name).cloned();
        let log: Vec<soyuz_script::rules::worker::LogEntry> = app
            .core
            .automation
            .logs
            .get(&rule_now.name)
            .map(|l| l.iter().rev().take(8).cloned().collect())
            .unwrap_or_default();
        let mut cmds: Vec<Cmd> = Vec::new();
        let r = &mut app.gui.settings.draft.automation.rules[i];
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(icons::icon(ph::ROBOT, 16.0).color(p.accent));
                ui.add(
                    egui::TextEdit::singleline(&mut r.name)
                        .hint_text(tr!("automation-rules-name-hint"))
                        .font(egui::FontId::proportional(14.0))
                        .desired_width(220.0)
                        .margin(egui::Margin::symmetric(8, 4)),
                );
                switch(ui, &mut r.enabled);
                let (dot, tip) = match (&status, r.enabled) {
                    (_, false) => (p.text_tertiary, tr!("automation-rules-status-off").to_string()),
                    (Some(Ok(())), _) => (p.success, tr!("automation-rules-status-watching").to_string()),
                    (Some(Err(e)), _) => (p.danger, e.clone()),
                    (None, _) => (p.text_tertiary, tr!("automation-rules-status-not-running").to_string()),
                };
                let (rect, resp) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.5, dot);
                resp.on_hover_text(&tip);
                if let Some(Err(e)) = &status {
                    ui.label(RichText::new(e).size(11.0).color(p.danger));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if small_button(ui, Some(ph::TRASH), "", true)
                        .on_hover_text(tr!("automation-rules-delete"))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    if i + 1 < n
                        && small_button(ui, Some(ph::ARROW_DOWN), "", false)
                            .on_hover_text(tr!("automation-rules-move-down"))
                            .clicked()
                    {
                        moves = Some((i, i + 1));
                    }
                    if i > 0
                        && small_button(ui, Some(ph::ARROW_UP), "", false)
                            .on_hover_text(tr!("automation-rules-move-up"))
                            .clicked()
                    {
                        moves = Some((i - 1, i));
                    }
                    if small_button(ui, Some(ph::PLAY), tr!("automation-rules-run-now"), false)
                        .on_hover_text(tr!("automation-rules-run-now-tip"))
                        .clicked()
                    {
                        cmds.push(Cmd::Automation(AutoCmd::RunRule(r.name.clone())));
                    }
                });
            });
            ui.add_space(8.0);
            Grid::new(("soyuz.rule", i)).num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                label(ui, &p, tr!("automation-rules-watch"));
                ui.horizontal(|ui| {
                    let mut s = r.watch.to_string_lossy().into_owned();
                    if code(ui, &mut s, "C:\\Users\\me\\Downloads", 300.0).changed() {
                        r.watch = PathBuf::from(s.trim());
                    }
                    ui.checkbox(&mut r.recursive, tr!("automation-rules-subfolders"));
                });
                ui.end_row();
                label(ui, &p, tr!("automation-rules-names"));
                ui.horizontal(|ui| {
                    code(ui, &mut r.glob, tr!("automation-rules-glob-hint"), 180.0);
                    code(ui, &mut r.regex, tr!("automation-rules-regex-hint"), 140.0);
                });
                ui.end_row();
                label(ui, &p, tr!("automation-rules-items"));
                segmented(
                    ui,
                    &mut r.kind,
                    &[RuleKind::Files, RuleKind::Folders, RuleKind::Any].map(|k| (k, rule_kind(k))),
                );
                ui.end_row();
                label(ui, &p, tr!("automation-rules-size"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut r.min_size_kib).prefix("≥ ").speed(16));
                    ui.add(egui::DragValue::new(&mut r.max_size_kib).prefix("≤ ").speed(16));
                    ui.label(RichText::new(tr!("automation-rules-no-limit")).size(11.0).color(p.text_tertiary));
                });
                ui.end_row();
                label(ui, &p, tr!("automation-rules-age"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut r.min_age_minutes).prefix("≥ ").speed(1));
                    ui.add(egui::DragValue::new(&mut r.max_age_minutes).prefix("≤ ").speed(1));
                    ui.label(RichText::new(tr!("automation-rules-by-mtime")).size(11.0).color(p.text_tertiary));
                });
                ui.end_row();
                label(ui, &p, tr!("automation-rules-wait"));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut r.delay_secs)
                            .range(0..=3600)
                            .suffix(tr!("automation-rules-seconds-suffix")),
                    );
                    ui.label(RichText::new(tr!("automation-rules-after-change")).size(11.0).color(p.text_tertiary));
                    ui.checkbox(&mut r.apply_to_existing, tr!("automation-rules-existing"));
                });
                ui.end_row();
                label(ui, &p, tr!("automation-rules-conflicts"));
                ui.horizontal(|ui| {
                    let opts = [RuleConflict::KeepBoth, RuleConflict::Skip, RuleConflict::Overwrite, RuleConflict::Ask]
                        .map(|c| (c, rule_conflict(c)));
                    dropdown(ui, &format!("rule_conflict_{i}"), &mut r.conflict, &opts, 110.0);
                    ui.checkbox(&mut r.dry_run, tr!("automation-rules-dry-run"));
                });
                ui.end_row();
                label(ui, &p, tr!("automation-rules-then"));
                ui.vertical(|ui| steps(ui, &p, i, &mut r.actions));
                ui.end_row();
            });
            ui.add_space(8.0);
            preview_block(ui, &p, preview.as_ref());
            if !log.is_empty() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr!("automation-rules-run-log")).size(10.5).color(p.text_tertiary));
                    if small_button(ui, None, tr!("automation-rules-clear"), false).clicked() {
                        cmds.push(Cmd::Automation(AutoCmd::ClearLog(r.name.clone())));
                    }
                });
                for e in &log {
                    log_row(ui, &p, e);
                }
            }
        });
        app.core.cmds.extend(cmds);
    }
    if let Some(i) = remove {
        app.gui.settings.draft.automation.rules.remove(i);
        app.core.automation.previews.clear();
    }
    if let Some((a, b)) = moves {
        app.gui.settings.draft.automation.rules.swap(a, b);
        app.core.automation.previews.clear();
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
            ui.label(RichText::new(rule_action(a)).size(12.5).color(p.text_primary));
            match a {
                RuleAction::Move { to } | RuleAction::Copy { to } => {
                    let mut s = to.to_string_lossy().into_owned();
                    if code(ui, &mut s, tr!("automation-rules-dest-hint"), 260.0).changed() {
                        *to = PathBuf::from(s.trim());
                    }
                }
                RuleAction::Rename { pattern } => {
                    code(ui, pattern, "{mdate}-{name}", 200.0)
                        .on_hover_text("{name} {stem} {ext} {date} {time} {mdate} {parent}");
                }
                RuleAction::Tag { tag } => {
                    let names: Vec<String> = TAGS.iter().map(|t| tag_name(t)).collect();
                    let opts: Vec<(String, &str)> =
                        TAGS.iter().zip(&names).map(|(t, n)| (t.to_string(), n.as_str())).collect();
                    dropdown(ui, &format!("rule_tag_{i}_{k}"), tag, &opts, 100.0);
                }
                RuleAction::Script { script } => {
                    if scripts.is_empty() {
                        text(ui, script, tr!("automation-rules-script-hint"), 160.0)
                            .on_hover_text(tr!("automation-rules-script-tip"));
                    } else {
                        let opts: Vec<(String, &str)> = scripts.iter().map(|s| (s.clone(), s.as_str())).collect();
                        dropdown(ui, &format!("rule_script_{i}_{k}"), script, &opts, 160.0);
                    }
                }
                RuleAction::Trash => {}
            }
            if small_button(ui, Some(ph::X), "", true).on_hover_text(tr!("automation-rules-remove-step")).clicked() {
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
            (ph::ARROW_SQUARE_RIGHT, tr!("automation-step-move"), RuleAction::Move { to: home.join("Documents") }),
            (ph::COPY, tr!("automation-step-copy"), RuleAction::Copy { to: home.join("Documents") }),
            (ph::TEXT_T, tr!("automation-step-rename"), RuleAction::Rename { pattern: "{mdate}-{name}".into() }),
            (ph::TAG, tr!("automation-step-tag"), RuleAction::Tag { tag: "blue".into() }),
            (ph::TRASH, tr!("automation-step-trash"), RuleAction::Trash),
            (
                ph::CODE,
                tr!("automation-step-script"),
                RuleAction::Script { script: scripts.first().cloned().unwrap_or_default() },
            ),
        ] {
            if small_button(ui, Some(glyph), name, false).on_hover_text(tr!("automation-rules-add-step")).clicked() {
                actions.push(make);
            }
        }
    });
}

fn preview_block(ui: &mut Ui, p: &Palette, preview: Option<&crate::automation::RulePreview>) {
    let Some(pv) = preview else { return };
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr!("automation-rules-matches-now")).size(10.5).color(p.text_tertiary));
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
                ui.label(RichText::new(tr!("automation-rules-no-matches")).size(12.0).color(p.text_tertiary));
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
                ui.label(
                    RichText::new(tr!("automation-and-more", count = total - 6)).size(11.5).color(p.text_tertiary),
                );
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

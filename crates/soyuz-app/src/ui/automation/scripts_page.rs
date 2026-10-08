//! Settings → Scripts: sandbox settings, the scripts folder, every script with what it
//! registers, its errors (line / column) and an enable switch.

use egui::{Align, Layout, RichText, Ui};

use super::card;
use crate::app::SoyuzApp;
use crate::automation::{AutoCmd, TEMPLATES, template_title};
use crate::cmd::Cmd;
use crate::theme::icons::ph;
use crate::theme::{icons, palette};
use crate::ui::settings::widgets::{group, matches, number, small_button, switch};

const CTX: &str = "Scripts rhai betikler";

pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let mut any = false;
    let d = &mut app.settings.draft.scripting;
    any |= group(ui, q, CTX, tr!("automation-scripts-group"), |r| {
        r.row(tr!("automation-scripts-run"), tr!("automation-scripts-run-desc"), |ui| {
            switch(ui, &mut d.enabled);
        });
        r.row_kw(
            tr!("automation-scripts-approve"),
            tr!("automation-scripts-approve-desc"),
            "confirm preview plan",
            |ui| {
                switch(ui, &mut d.approve_writes);
            },
        );
        r.row_kw(tr!("automation-scripts-read"), tr!("automation-scripts-read-desc"), "sandbox privacy", |ui| {
            switch(ui, &mut d.allow_read);
        });
        r.row_kw(tr!("automation-scripts-timeout"), tr!("automation-scripts-timeout-desc"), "timeout sandbox", |ui| {
            number(ui, &mut d.timeout_ms, 100..=60_000, " ms");
        });
        r.row_kw(
            tr!("automation-scripts-cell-timeout"),
            tr!("automation-scripts-cell-timeout-desc"),
            "timeout cell",
            |ui| {
                number(ui, &mut d.cell_timeout_ms, 20..=5_000, " ms");
            },
        );
        r.row_kw(tr!("automation-scripts-ops"), tr!("automation-scripts-ops-desc"), "sandbox ops", |ui| {
            number(ui, &mut d.max_operations, 100_000..=100_000_000, "");
        });
        r.row_kw(tr!("automation-scripts-read-max"), tr!("automation-scripts-read-max-desc"), "sandbox kib", |ui| {
            number(ui, &mut d.max_read_kib, 4..=65_536, " KiB");
        });
        r.row_kw(tr!("automation-scripts-list-max"), tr!("automation-scripts-list-max-desc"), "sandbox", |ui| {
            number(ui, &mut d.max_list_entries, 100..=200_000, "");
        });
    });
    let searching = !q.trim().is_empty();
    let show_folder = !searching || matches(q, &["scripts folder reload template new", CTX]);
    let p = palette(ui.ctx());
    let dir = soyuz_script::scripts_dir();
    if show_folder {
        any = true;
        ui.add_space(14.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(icons::icon(ph::FOLDER_SIMPLE, 16.0).color(p.accent));
                ui.add(
                    egui::Label::new(
                        RichText::new(dir.display().to_string()).monospace().size(12.0).color(p.text_primary),
                    )
                    .truncate(),
                );
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if small_button(ui, Some(ph::FOLDER_OPEN), tr!("automation-scripts-show-folder"), false).clicked() {
                    app.cmds.push(Cmd::Automation(AutoCmd::OpenScriptsFolder));
                }
                if small_button(ui, Some(ph::ARROWS_CLOCKWISE), tr!("automation-scripts-reload"), false).clicked() {
                    app.cmds.push(Cmd::Automation(AutoCmd::ReloadScripts));
                }
                ui.label(RichText::new(tr!("automation-scripts-new-from")).size(11.5).color(p.text_tertiary));
                for (file, _) in TEMPLATES {
                    if small_button(ui, Some(ph::FILE_PLUS), template_title(file), false).clicked() {
                        app.cmds.push(Cmd::Automation(AutoCmd::NewScript(file)));
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new(tr!("automation-scripts-folder-note")).size(11.5).color(p.text_tertiary));
        });
    }
    let Some(set) = app.automation.set.clone() else {
        if show_folder {
            ui.add_space(10.0);
            ui.label(RichText::new(tr!("automation-scripts-loading")).color(p.text_tertiary));
        }
        return any;
    };
    if set.scripts.is_empty() && show_folder {
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("automation-scripts-empty")).color(p.text_tertiary));
        return any;
    }
    let (actions, columns, previewers) = set
        .active()
        .fold((0, 0, 0), |(a, c, pv), s| (a + s.actions.len(), c + s.columns.len(), pv + s.previewers.len()));
    if show_folder {
        ui.add_space(10.0);
        let summary = tr!(
            "automation-scripts-summary",
            scripts = set.scripts.len(),
            actions = actions,
            columns = columns,
            previewers = previewers
        );
        ui.label(RichText::new(summary).size(11.5).color(p.text_tertiary));
        if columns > 4 {
            ui.label(RichText::new(tr!("automation-scripts-columns-capped")).size(11.5).color(p.warning));
        }
    }
    let mut toggle: Option<(String, bool)> = None;
    for sc in &set.scripts {
        if searching && !matches(q, &[&sc.file, "script rhai"]) {
            continue;
        }
        any = true;
        ui.add_space(10.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let (glyph, color) = if !sc.enabled {
                    (ph::PROHIBIT, p.text_tertiary)
                } else if sc.error.is_some() {
                    (ph::WARNING_CIRCLE, p.danger)
                } else {
                    (ph::CHECK_CIRCLE, p.success)
                };
                ui.label(icons::icon(glyph, 16.0).color(color));
                ui.label(RichText::new(&sc.file).monospace().size(13.5).color(p.text_primary));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut on = sc.enabled;
                    if switch(ui, &mut on).changed() {
                        toggle = Some((sc.file.clone(), on));
                    }
                });
            });
            if let Some(e) = &sc.error {
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(p.danger.gamma_multiply(0.12))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(RichText::new(e.to_string()).monospace().size(12.0).color(p.danger))
                                .wrap(),
                        );
                    });
            }
            let rt: Vec<&String> =
                sc.actions.iter().filter_map(|a| app.automation.runtime_errors.get(&a.title)).collect();
            for e in rt {
                ui.add(
                    egui::Label::new(
                        RichText::new(tr!("automation-scripts-last-run", error = e))
                            .monospace()
                            .size(11.5)
                            .color(p.warning),
                    )
                    .wrap(),
                );
            }
            if !sc.enabled {
                return;
            }
            ui.add_space(6.0);
            for a in &sc.actions {
                ui.horizontal(|ui| {
                    ui.label(icons::icon(if a.menu { ph::LIST } else { ph::LIGHTNING }, 13.0).color(p.accent));
                    ui.label(RichText::new(&a.title).size(12.5).color(p.text_primary));
                    ui.label(
                        RichText::new(format!("run_script:{}", sc.action_id(a)))
                            .monospace()
                            .size(11.0)
                            .color(p.text_tertiary),
                    );
                    if !a.hotkey.is_empty() {
                        ui.label(RichText::new(&a.hotkey).size(11.0).color(p.text_secondary));
                    }
                    if a.menu {
                        ui.label(
                            RichText::new(tr!("automation-scripts-context-menu")).size(11.0).color(p.text_tertiary),
                        );
                    }
                });
            }
            for c in &sc.columns {
                ui.horizontal(|ui| {
                    ui.label(icons::icon(ph::COLUMNS, 13.0).color(p.accent));
                    ui.label(
                        RichText::new(tr!("automation-scripts-column", title = &c.title))
                            .size(12.5)
                            .color(p.text_primary),
                    );
                });
            }
            for pv in &sc.previewers {
                ui.horizontal(|ui| {
                    ui.label(icons::icon(ph::EYE, 13.0).color(p.accent));
                    ui.label(
                        RichText::new(tr!(
                            "automation-scripts-previewer",
                            title = &pv.title,
                            exts = pv.extensions.join(", .")
                        ))
                        .size(12.5)
                        .color(p.text_primary),
                    );
                });
            }
            if sc.has_rule {
                ui.horizontal(|ui| {
                    ui.label(icons::icon(ph::ROBOT, 13.0).color(p.accent));
                    ui.label(RichText::new(tr!("automation-scripts-rule-step")).size(12.5).color(p.text_primary));
                });
            }
        });
    }
    if let Some((file, on)) = toggle {
        let d = &mut app.settings.draft.scripting.disabled;
        d.retain(|f| !f.eq_ignore_ascii_case(&file));
        if !on {
            d.push(file);
        }
    }
    any
}

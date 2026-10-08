//! Custom commands and bookmarks editors.

use egui::{Align, CornerRadius, Frame, Layout, Margin, RichText, Stroke, Ui};
use soyuz_core::config::{Bookmark, CommandContext, CustomCommand};

use super::widgets::{code, group, matches, small_button, subheading, text};
use super::{binding_chip, record};
use crate::app::SoyuzApp;
use crate::theme::icons::ph;
use crate::theme::{icons, palette};
use crate::ui::dialogs::keybind_record::RecordTarget;

/// Starter commands: (name, command, arguments, run in terminal).
fn templates() -> [(&'static str, &'static str, &'static str, bool); 4] {
    [
        (tr!("settings-commands-template-vscode"), "code", "%s", false),
        (tr!("settings-commands-template-git"), "git status", "", true),
        (
            tr!("settings-commands-template-copy-names"),
            if cfg!(windows) { "cmd /C echo %n | clip" } else { "sh -c \"echo %n | xclip -sel c\"" },
            "",
            false,
        ),
        (
            tr!("settings-commands-template-list"),
            if cfg!(windows) { "Get-ChildItem %d" } else { "ls -la %d" },
            "",
            true,
        ),
    ]
}

fn placeholders() -> [(&'static str, &'static str); 6] {
    [
        ("%p", tr!("settings-commands-ph-focused")),
        ("%s", tr!("settings-commands-ph-selected")),
        ("%n", tr!("settings-commands-ph-names")),
        ("%d", tr!("settings-commands-ph-dir")),
        ("%o", tr!("settings-commands-ph-other")),
        ("%%", tr!("settings-commands-ph-percent")),
    ]
}

fn card(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    Frame::new()
        .fill(p.bg_surface2.gamma_multiply(if p.dark { 0.7 } else { 1.0 }))
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        });
}

pub fn commands(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let searching = !q.trim().is_empty();
    let mut any = false;
    // Help + templates (not part of search).
    if !searching {
        any = true;
        ui.add_space(14.0);
        card(ui, |ui| {
            subheading(ui, tr!("settings-commands-placeholders"));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (k, v) in placeholders() {
                    ui.label(RichText::new(k).monospace().color(p.accent));
                    ui.label(RichText::new(v).size(11.5).color(p.text_secondary));
                    ui.add_space(10.0);
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("settings-commands-help")).size(11.5).color(p.text_tertiary));
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            if small_button(ui, Some(ph::PLUS), tr!("settings-commands-new"), false).clicked() {
                app.settings
                    .draft
                    .custom_commands
                    .push(CustomCommand { name: tr!("settings-commands-new").into(), ..Default::default() });
            }
            ui.label(RichText::new(tr!("settings-commands-or-template")).size(11.5).color(p.text_tertiary));
            for (name, cmd, args, term) in templates() {
                if small_button(ui, Some(if term { ph::TERMINAL } else { ph::LIGHTNING }), name, false).clicked() {
                    app.settings.draft.custom_commands.push(CustomCommand {
                        name: name.into(),
                        command: cmd.into(),
                        args: crate::commands::split_args(args),
                        terminal: term,
                        ..Default::default()
                    });
                }
            }
        });
    }
    let n = app.settings.draft.custom_commands.len();
    if app.settings.args_text.len() != n {
        app.settings.args_text =
            app.settings.draft.custom_commands.iter().map(|c| crate::commands::join_args(&c.args)).collect();
    }
    let mut remove = None;
    let mut run = None;
    let mut rec = None;
    let mut moves = None;
    for i in 0..n {
        let c = &app.settings.draft.custom_commands[i];
        if searching && !matches(q, &[&c.name, &c.command, "custom command", tr!("settings-section-commands")]) {
            continue;
        }
        any = true;
        ui.add_space(10.0);
        let s = &mut app.settings;
        card(ui, |ui| {
            let c = &mut s.draft.custom_commands[i];
            ui.horizontal(|ui| {
                ui.label(icons::icon(if c.terminal { ph::TERMINAL } else { ph::LIGHTNING }, 16.0).color(p.accent));
                ui.add(
                    egui::TextEdit::singleline(&mut c.name)
                        .hint_text(tr!("settings-name"))
                        .font(egui::FontId::proportional(14.0))
                        .desired_width(220.0)
                        .margin(Margin::symmetric(8, 4)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if small_button(ui, Some(ph::TRASH), "", true)
                        .on_hover_text(tr!("settings-commands-delete"))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                    if i + 1 < n && small_button(ui, Some(ph::ARROW_DOWN), "", false).clicked() {
                        moves = Some((i, i + 1));
                    }
                    if i > 0 && small_button(ui, Some(ph::ARROW_UP), "", false).clicked() {
                        moves = Some((i - 1, i));
                    }
                    if !c.command.trim().is_empty()
                        && small_button(ui, Some(ph::PLAY), tr!("settings-commands-run"), false).clicked()
                    {
                        run = Some(c.clone());
                    }
                });
            });
            ui.add_space(8.0);
            egui::Grid::new(("soyuz.cc", i)).num_columns(2).spacing(egui::vec2(12.0, 8.0)).show(ui, |ui| {
                ui.label(RichText::new(tr!("settings-commands-command")).size(12.0).color(p.text_secondary));
                code(ui, &mut c.command, tr!("settings-commands-command-hint"), ui.available_width().min(460.0));
                ui.end_row();
                ui.label(RichText::new(tr!("settings-terminal-args")).size(12.0).color(p.text_secondary));
                let buf = &mut s.args_text[i];
                if code(ui, buf, tr!("settings-commands-args-hint"), ui.available_width().min(460.0)).changed() {
                    c.args = crate::commands::split_args(buf);
                }
                ui.end_row();
                ui.label(RichText::new(tr!("settings-commands-show-for")).size(12.0).color(p.text_secondary));
                ui.horizontal(|ui| {
                    for (k, label) in [
                        (CommandContext::File, tr!("settings-general-files")),
                        (CommandContext::Dir, tr!("settings-views-folders")),
                        (CommandContext::Background, tr!("settings-commands-background-ctx")),
                    ] {
                        let mut on = c.context.is_empty() || c.context.contains(&k);
                        if ui.checkbox(&mut on, label).changed() {
                            if c.context.is_empty() {
                                c.context = vec![CommandContext::File, CommandContext::Dir, CommandContext::Background];
                            }
                            if on {
                                c.context.push(k);
                            } else {
                                c.context.retain(|x| *x != k);
                            }
                            c.context.sort_by_key(|x| *x as u8);
                            c.context.dedup();
                            if c.context.len() == 3 {
                                c.context.clear();
                            }
                        }
                        ui.add_space(6.0);
                    }
                });
                ui.end_row();
                ui.label(RichText::new(tr!("settings-commands-run-in")).size(12.0).color(p.text_secondary));
                super::widgets::segmented(
                    ui,
                    &mut c.terminal,
                    &[(false, tr!("settings-commands-run-background")), (true, tr!("settings-commands-run-terminal"))],
                );
                ui.end_row();
                ui.label(RichText::new(tr!("settings-commands-hotkey")).size(12.0).color(p.text_secondary));
                ui.horizontal(|ui| {
                    if c.hotkey.trim().is_empty() {
                        if small_button(ui, Some(ph::KEYBOARD), tr!("settings-commands-record"), false).clicked() {
                            rec = Some(i);
                        }
                    } else {
                        let (clicked, removed) = binding_chip(ui, &p, &c.hotkey.clone(), false, true);
                        if removed {
                            c.hotkey.clear();
                        } else if clicked {
                            rec = Some(i);
                        }
                    }
                });
                ui.end_row();
            });
            // Live preview of the expanded command.
            if !c.command.trim().is_empty() {
                let pc = crate::commands::PlaceholderCtx {
                    focused: Some("C:/demo/report.pdf".into()),
                    selected: vec!["C:/demo/report.pdf".into(), "C:/demo/data 1.csv".into()],
                    dir: Some("C:/demo".into()),
                    other_dir: Some("D:/backup".into()),
                };
                let line = crate::commands::command_line(c, &pc, crate::commands::Quoting::for_shell(""));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(icons::icon(ph::EYE, 12.0).color(p.text_tertiary));
                    ui.add(
                        egui::Label::new(RichText::new(line).monospace().size(11.0).color(p.text_tertiary)).truncate(),
                    );
                });
            }
        });
    }
    if let Some(i) = remove {
        app.settings.draft.custom_commands.remove(i);
        app.settings.args_text.clear();
    }
    if let Some((a, b)) = moves {
        app.settings.draft.custom_commands.swap(a, b);
        app.settings.args_text.clear();
    }
    if let Some(i) = rec {
        let name = app.settings.draft.custom_commands[i].name.clone();
        record(app, RecordTarget::Command(i), tr!("settings-commands-record-title", name = name));
    }
    if let Some(c) = run {
        let ctx = ui.ctx().clone();
        crate::commands::run(app, &c, &ctx);
    }
    if !searching && n == 0 {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(icons::icon(ph::LIGHTNING, 28.0).color(p.text_tertiary));
            ui.label(RichText::new(tr!("settings-commands-empty")).color(p.text_secondary));
        });
    }
    any
}

const BOOKMARK_ICONS: [(&str, &str); 12] = [
    ("", ph::FOLDER_SIMPLE_STAR),
    ("code", ph::CODE),
    ("star", ph::STAR),
    ("home", ph::HOUSE),
    ("downloads", ph::DOWNLOAD_SIMPLE),
    ("music", ph::MUSIC_NOTES),
    ("pictures", ph::IMAGE),
    ("video", ph::FILM_STRIP),
    ("cloud", ph::CLOUD),
    ("terminal", ph::TERMINAL_WINDOW),
    ("archive", ph::PACKAGE),
    ("database", ph::DATABASE),
];

pub fn bookmarks(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let searching = !q.trim().is_empty();
    let current = app.ws.pane().location.as_fs_path().map(soyuz_fs::strip_verbatim);
    let mut any = false;
    if !searching {
        any = true;
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if let Some(cur) = &current
                && small_button(ui, Some(ph::PLUS), tr!("settings-bookmarks-add-current"), false).clicked()
            {
                let name = cur
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| cur.display().to_string());
                app.settings.draft.bookmarks.push(Bookmark {
                    name,
                    path: cur.clone(),
                    hotkey: String::new(),
                    icon: String::new(),
                });
            }
            if small_button(ui, Some(ph::PLUS), tr!("settings-bookmarks-add-empty"), false).clicked() {
                app.settings
                    .draft
                    .bookmarks
                    .push(Bookmark { name: tr!("settings-bookmarks-default-name").into(), ..Default::default() });
            }
            ui.label(RichText::new(tr!("settings-bookmarks-help")).size(11.5).color(p.text_tertiary));
        });
    }
    let n = app.settings.draft.bookmarks.len();
    let mut remove = None;
    let mut moves = None;
    let mut rec = None;
    let rows_drawn = {
        let mut drawn = false;
        let list: Vec<usize> = (0..n)
            .filter(|i| {
                let b = &app.settings.draft.bookmarks[*i];
                !searching
                    || matches(q, &[&b.name, &b.path.to_string_lossy(), "bookmark", tr!("settings-section-bookmarks")])
            })
            .collect();
        if !list.is_empty() {
            drawn = true;
            group(ui, "", "", "", |r| {
                for &i in &list {
                    r.block("", "", |ui| {
                        let b = &mut app.settings.draft.bookmarks[i];
                        ui.horizontal(|ui| {
                            let glyph = BOOKMARK_ICONS
                                .iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case(b.icon.trim()))
                                .map(|(_, g)| *g)
                                .unwrap_or(ph::FOLDER_SIMPLE_STAR);
                            egui::ComboBox::from_id_salt(("soyuz.bm.icon", i))
                                .selected_text(icons::icon(glyph, 15.0).color(p.accent))
                                .width(44.0)
                                .show_ui(ui, |ui| {
                                    for (k, g) in BOOKMARK_ICONS {
                                        let label =
                                            if k.is_empty() { tr!("settings-bookmarks-icon-default") } else { k };
                                        if ui.selectable_label(b.icon == k, format!("{g}  {label}")).clicked() {
                                            b.icon = k.to_string();
                                        }
                                    }
                                });
                            if i < 9 {
                                ui.label(RichText::new(format!("{}", i + 1)).monospace().color(p.text_tertiary));
                            }
                            text(ui, &mut b.name, tr!("settings-name"), 150.0);
                            let mut path = b.path.to_string_lossy().into_owned();
                            if code(
                                ui,
                                &mut path,
                                tr!("settings-bookmarks-path-hint"),
                                (ui.available_width() - 110.0).max(120.0),
                            )
                            .changed()
                            {
                                b.path = path.into();
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if small_button(ui, Some(ph::TRASH), "", true).clicked() {
                                    remove = Some(i);
                                }
                                if i + 1 < n && small_button(ui, Some(ph::ARROW_DOWN), "", false).clicked() {
                                    moves = Some((i, i + 1));
                                }
                                if i > 0 && small_button(ui, Some(ph::ARROW_UP), "", false).clicked() {
                                    moves = Some((i - 1, i));
                                }
                                if b.hotkey.trim().is_empty() {
                                    if small_button(ui, Some(ph::KEYBOARD), "", false)
                                        .on_hover_text(tr!("settings-bookmarks-record"))
                                        .clicked()
                                    {
                                        rec = Some(i);
                                    }
                                } else {
                                    let (clicked, removed) = binding_chip(ui, &p, &b.hotkey.clone(), false, true);
                                    if removed {
                                        b.hotkey.clear();
                                    } else if clicked {
                                        rec = Some(i);
                                    }
                                }
                            });
                        });
                    });
                }
            });
        }
        drawn
    };
    any |= rows_drawn;
    if let Some(i) = remove {
        app.settings.draft.bookmarks.remove(i);
    }
    if let Some((a, b)) = moves {
        app.settings.draft.bookmarks.swap(a, b);
    }
    if let Some(i) = rec {
        let name = app.settings.draft.bookmarks[i].name.clone();
        record(app, RecordTarget::Bookmark(i), tr!("settings-bookmarks-record-title", name = name));
    }
    if !searching && n == 0 {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(icons::icon(ph::BOOKMARK_SIMPLE, 28.0).color(p.text_tertiary));
            ui.label(RichText::new(tr!("settings-bookmarks-empty")).color(p.text_secondary));
        });
    }
    any
}

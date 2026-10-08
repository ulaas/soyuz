//! Settings pages for the plain config sections: general, views & layout, behaviour,
//! file operations, terminal, preview / search / git, and the config file page.

use std::time::SystemTime;

use egui::{RichText, Ui};
use soyuz_core::config::{
    ActivityPlacement, DragDefault, MiddleClick, OpenFoldersIn, Orientation, PasteConflict, PreserveItem, ReflinkPref,
    RenameSelects, SidebarSection, TabStripMode, TrashUnsupportedPolicy, TypeAhead,
};
use soyuz_core::{Column, DateFormat, DateFormatName, SizeStyle, ViewMode, format_size, format_time};

use super::widgets::{code, dropdown, group, number, segmented, small_button, switch, text};
use crate::app::SoyuzApp;
use crate::theme::icons::ph;
use crate::theme::palette;

const CTX_GENERAL: &str = "General";

pub fn general(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.settings.draft;
    let mut any = false;
    any |= group(ui, q, CTX_GENERAL, "Startup", |r| {
        r.row_kw(
            "Start in",
            "Folder shown when Soyuz starts without a saved session.",
            "start location home last",
            |ui| {
                let mut mode = match d.general.start_location.trim().to_ascii_lowercase().as_str() {
                    "" | "home" => 0,
                    "last" => 1,
                    _ => 2,
                };
                let before = mode;
                if mode == 2 {
                    text(ui, &mut d.general.start_location, "C:\\Users\\me\\Projects", 200.0);
                }
                segmented(ui, &mut mode, &[(0, "Home"), (1, "Last folder"), (2, "Folder")]);
                if mode != before {
                    d.general.start_location = match mode {
                        0 => "home".into(),
                        1 => "last".into(),
                        _ => soyuz_fs::home().to_string_lossy().into_owned(),
                    };
                }
            },
        );
        r.row("Restore tabs on startup", "Reopen the tabs, panes and folders of the last session.", |ui| {
            switch(ui, &mut d.behaviour.restore_session);
        });
        r.row_kw("Language", "Only English is available for now.", "locale translation", |ui| {
            ui.add_enabled_ui(false, |ui| text(ui, &mut d.general.language, "en", 80.0));
        });
    });
    any |= group(ui, q, CTX_GENERAL, "Files", |r| {
        r.row_kw(
            "Show hidden files",
            "Dotfiles and items with the hidden attribute (Ctrl+H toggles).",
            "dotfiles",
            |ui| {
                switch(ui, &mut d.general.show_hidden);
            },
        );
        r.row("Show system files", "Windows protected operating-system files.", |ui| {
            switch(ui, &mut d.general.show_system);
        });
        r.row("Show file extensions", "Display “.txt”, “.rs” … after names.", |ui| {
            switch(ui, &mut d.general.show_extensions);
        });
        r.row_kw(
            "Strict Windows names",
            "Apply Windows file-name rules (no <>:\"|?*, reserved names) on every OS.",
            "validate portable",
            |ui| {
                switch(ui, &mut d.general.strict_windows_names);
            },
        );
    });
    any |= group(ui, q, CTX_GENERAL, "Formats", |r| {
        let sample = 1_288_490_189u64;
        r.row_kw(
            "File sizes",
            &format!("1.2 GB file shows as {}", format_size(sample, d.general.size_style)),
            "units kib mb",
            |ui| {
                segmented(
                    ui,
                    &mut d.general.size_style,
                    &[(SizeStyle::Binary, "KiB, MiB"), (SizeStyle::Decimal, "kB, MB"), (SizeStyle::Bytes, "Bytes")],
                );
            },
        );
        let preview =
            format_time(SystemTime::now() - std::time::Duration::from_secs(3600 * 26), &d.general.date_format);
        r.row_kw("Dates", &format!("Yesterday's file shows as “{preview}”"), "time format strftime", |ui| {
            let mut custom = match &d.general.date_format {
                DateFormat::Custom(s) => Some(s.clone()),
                _ => None,
            };
            if let Some(s) = custom.as_mut()
                && code(ui, s, "%Y-%m-%d %H:%M", 140.0).changed()
            {
                d.general.date_format = DateFormat::Custom(s.clone());
            }
            let mut sel = match &d.general.date_format {
                DateFormat::Named(n) => Some(*n),
                DateFormat::Custom(_) => None,
            };
            let opts = [
                (Some(DateFormatName::Relative), "Relative"),
                (Some(DateFormatName::Short), "Short"),
                (Some(DateFormatName::Long), "Long"),
                (Some(DateFormatName::Iso), "ISO 8601"),
                (None, "Custom…"),
            ];
            if dropdown(ui, "date_format", &mut sel, &opts, 120.0) {
                d.general.date_format = match sel {
                    Some(n) => DateFormat::Named(n),
                    None => DateFormat::Custom("%Y-%m-%d %H:%M".into()),
                };
            }
        });
    });
    any
}

const MODES: [(ViewMode, &str); 5] = [
    (ViewMode::Details, "Details"),
    (ViewMode::Grid, "Grid"),
    (ViewMode::List, "List"),
    (ViewMode::Miller, "Columns"),
    (ViewMode::Gallery, "Gallery"),
];

pub fn views(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.settings.draft;
    let ctx = "Views layout";
    let mut any = false;
    any |= group(ui, q, ctx, "Folders", |r| {
        r.row("Default view", "Used for folders you haven't customized.", |ui| {
            segmented(ui, &mut d.general.default_view, &MODES);
        });
        r.row("Folders first", "Keep folders above files when sorting.", |ui| {
            switch(ui, &mut d.general.folders_first);
        });
        r.row_kw("Natural sorting", "file2 comes before file10.", "numeric", |ui| {
            switch(ui, &mut d.general.natural_sort);
        });
        r.row("Case-insensitive sorting", "“apple” and “Apple” sort together.", |ui| {
            switch(ui, &mut d.general.case_insensitive_sort);
        });
        r.row_kw("Remember view per folder", "Each folder keeps its own view, sort and grouping.", "memory", |ui| {
            switch(ui, &mut d.behaviour.remember_folder_views);
        });
    });
    any |= group(ui, q, ctx, "Details columns", |r| {
        r.block(
            "",
            "Columns of the details view for folders without their own layout. Drag widths in the view itself.",
            |ui| {
                columns_editor(ui, &mut d.columns.details, &mut d.columns.widths);
            },
        );
    });
    any |= group(ui, q, ctx, "Window layout", |r| {
        r.row("Sidebar", "Favorites, devices and the folder tree.", |ui| {
            switch(ui, &mut d.layout.sidebar);
        });
        r.row("Status bar", "Item counts, selection size and free space.", |ui| {
            switch(ui, &mut d.layout.status_bar);
        });
        r.row("Preview pane", "Show the preview pane at startup.", |ui| {
            switch(ui, &mut d.layout.preview_pane);
        });
        r.row_kw("Dual pane at startup", "Open new windows with two panes.", "split commander", |ui| {
            switch(ui, &mut d.layout.dual_pane);
        });
        r.row("Dual pane orientation", "", |ui| {
            segmented(
                ui,
                &mut d.layout.dual_pane_orientation,
                &[(Orientation::Vertical, "Side by side"), (Orientation::Horizontal, "Stacked")],
            );
        });
        r.row("Tab strip", "", |ui| {
            segmented(
                ui,
                &mut d.layout.tab_strip,
                &[(TabStripMode::Always, "Always"), (TabStripMode::Multiple, "With 2+ tabs")],
            );
        });
        r.row("Activity panel", "Where running file operations are shown.", |ui| {
            segmented(
                ui,
                &mut d.layout.activity_panel,
                &[
                    (ActivityPlacement::Right, "Right"),
                    (ActivityPlacement::Bottom, "Bottom"),
                    (ActivityPlacement::Floating, "Floating"),
                ],
            );
        });
        r.row("Terminal height", "Default height of the bottom panel.", |ui| {
            number(ui, &mut d.layout.terminal_height, 120.0..=800.0, " px");
        });
    });
    any |= group(ui, q, ctx, "Sidebar sections", |r| {
        r.block("", "Choose and order the sections of the sidebar.", |ui| {
            sidebar_sections(ui, &mut d.sidebar.sections);
        });
    });
    any |= group(ui, q, ctx, "Toolbar & context menu", |r| {
        r.block(
            "Toolbar items",
            "Action names (e.g. back, up, new_tab), plus | separator, spacer, breadcrumb, filter, view_menu, sort_menu, activity. One per line.",
            |ui| {
                lines_editor(ui, "toolbar", &mut d.toolbar.items, 5);
            },
        );
        r.block("Hidden context-menu entries", "Action names to hide from right-click menus. One per line.", |ui| {
            lines_editor(ui, "ctx_hide", &mut d.context_menu.hide, 3);
        });
        r.block("Context-menu order", "Optional explicit order of action names. One per line.", |ui| {
            lines_editor(ui, "ctx_order", &mut d.context_menu.order, 3);
        });
    });
    any
}

fn columns_editor(ui: &mut Ui, details: &mut Vec<Column>, widths: &mut std::collections::BTreeMap<String, f32>) {
    let p = palette(ui.ctx());
    let mut action: Option<(usize, i32)> = None;
    // Shown columns, in order.
    for (i, col) in details.clone().iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(
                    crate::palette::catalog::title(&soyuz_core::config::Action::SortBy(*col)).replace("Sort by: ", ""),
                )
                .color(p.text_primary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if *col != Column::Name
                    && small_button(ui, Some(ph::X), "", true).on_hover_text("Hide column").clicked()
                {
                    action = Some((i, 0));
                }
                if i + 1 < details.len() && small_button(ui, Some(ph::ARROW_DOWN), "", false).clicked() {
                    action = Some((i, 1));
                }
                if i > 0 && small_button(ui, Some(ph::ARROW_UP), "", false).clicked() {
                    action = Some((i, -1));
                }
                let key = col.to_string();
                let mut w = widths.get(&key).copied().unwrap_or(col.default_width());
                if ui.add(egui::DragValue::new(&mut w).range(40.0..=800.0).suffix(" px").speed(1.0)).changed() {
                    widths.insert(key, w);
                }
            });
        });
    }
    if let Some((i, op)) = action {
        match op {
            0 => {
                details.remove(i);
            }
            -1 => details.swap(i, i - 1),
            _ => details.swap(i, i + 1),
        }
    }
    let hidden: Vec<Column> = Column::ALL.into_iter().filter(|c| !details.contains(c)).collect();
    if !hidden.is_empty() {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Add:").size(11.5).color(p.text_tertiary));
            for c in hidden {
                if small_button(ui, Some(ph::PLUS), c.label(), false).clicked() {
                    details.push(c);
                }
            }
        });
    }
}

fn sidebar_sections(ui: &mut Ui, sections: &mut Vec<SidebarSection>) {
    let p = palette(ui.ctx());
    let all = [
        (SidebarSection::Favorites, "Favorites"),
        (SidebarSection::Devices, "Devices"),
        (SidebarSection::Cloud, "Cloud folders"),
        (SidebarSection::Tags, "Tags"),
        (SidebarSection::Network, "Network"),
        (SidebarSection::Trash, "Trash"),
        (SidebarSection::Tree, "Folder tree"),
    ];
    let mut action: Option<(usize, bool)> = None;
    for (i, s) in sections.clone().iter().enumerate() {
        let label = all.iter().find(|(x, _)| x == s).map(|(_, l)| *l).unwrap_or("?");
        ui.horizontal(|ui| {
            let mut on = true;
            if switch(ui, &mut on).changed() {
                action = Some((i, false));
            }
            ui.label(RichText::new(label).color(p.text_primary));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if i + 1 < sections.len() && small_button(ui, Some(ph::ARROW_DOWN), "", false).clicked() {
                    action = Some((i, true));
                }
                if i > 0 && small_button(ui, Some(ph::ARROW_UP), "", false).clicked() {
                    action = Some((i - 1, true));
                }
            });
        });
    }
    if let Some((i, swap)) = action {
        if swap {
            sections.swap(i, i + 1);
        } else {
            sections.remove(i);
        }
    }
    for (s, label) in all {
        if sections.contains(&s) {
            continue;
        }
        ui.horizontal(|ui| {
            let mut on = false;
            if switch(ui, &mut on).changed() {
                sections.push(s);
            }
            ui.label(RichText::new(label).color(p.text_tertiary));
        });
    }
}

/// Edit a list of strings as lines of text.
fn lines_editor(ui: &mut Ui, id: &str, items: &mut Vec<String>, rows: usize) {
    let mut text = items.join("\n");
    let resp = ui.add(
        egui::TextEdit::multiline(&mut text)
            .id_salt(("soyuz.settings.lines", id))
            .font(egui::TextStyle::Monospace)
            .desired_rows(rows)
            .desired_width(f32::INFINITY),
    );
    if resp.changed() {
        *items = text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    }
}

pub fn behaviour(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.settings.draft;
    let ctx = "Behaviour";
    let mut any = false;
    any |= group(ui, q, ctx, "Opening", |r| {
        r.row_kw("Open items with a single click", "Otherwise a double click opens.", "click", |ui| {
            switch(ui, &mut d.general.single_click_open);
        });
        r.row("Double-click speed", "Maximum time between the two clicks.", |ui| {
            number(ui, &mut d.general.double_click_speed_ms, 150..=900, " ms");
        });
        r.row("Open folders in", "", |ui| {
            segmented(
                ui,
                &mut d.general.open_folders_in,
                &[(OpenFoldersIn::SameTab, "Same tab"), (OpenFoldersIn::NewTab, "New tab")],
            );
        });
        r.row("Middle click on a folder", "", |ui| {
            segmented(
                ui,
                &mut d.behaviour.middle_click,
                &[
                    (MiddleClick::NewTab, "New tab"),
                    (MiddleClick::OtherPane, "Other pane"),
                    (MiddleClick::Nothing, "Nothing"),
                ],
            );
        });
    });
    any |= group(ui, q, ctx, "Confirmations", |r| {
        r.row_kw("Confirm moving to trash", "Ask before Delete sends items to the trash.", "delete recycle", |ui| {
            switch(ui, &mut d.general.confirm_delete);
        });
        r.row_kw("Confirm permanent delete", "Ask before Shift+Delete removes items for good.", "delete", |ui| {
            switch(ui, &mut d.general.confirm_permanent_delete);
        });
        r.row("When pasted items already exist", "", |ui| {
            dropdown(
                ui,
                "paste_conflict",
                &mut d.behaviour.paste_conflict,
                &[
                    (PasteConflict::Ask, "Ask"),
                    (PasteConflict::Overwrite, "Overwrite"),
                    (PasteConflict::Skip, "Skip"),
                    (PasteConflict::KeepBoth, "Keep both"),
                ],
                130.0,
            );
        });
    });
    any |= group(ui, q, ctx, "Drag and drop", |r| {
        r.row_kw(
            "Default drop action",
            "Ctrl copies, Shift moves, Alt links regardless of this.",
            "drag drop move copy",
            |ui| {
                segmented(
                    ui,
                    &mut d.behaviour.drag_default,
                    &[
                        (DragDefault::Auto, "Auto"),
                        (DragDefault::Copy, "Copy"),
                        (DragDefault::Move, "Move"),
                        (DragDefault::Ask, "Ask"),
                    ],
                );
            },
        );
    });
    any |= group(ui, q, ctx, "Typing & renaming", |r| {
        r.row_kw("Typing in the list", "Jump to the first match, or filter the list.", "type ahead", |ui| {
            segmented(ui, &mut d.behaviour.type_ahead, &[(TypeAhead::Jump, "Jump"), (TypeAhead::Filter, "Filter")]);
        });
        r.row("Rename selects", "What is selected when renaming starts (F2 cycles).", |ui| {
            segmented(
                ui,
                &mut d.behaviour.rename_selects,
                &[(RenameSelects::Stem, "Name only"), (RenameSelects::All, "Everything")],
            );
        });
    });
    any |= group(ui, q, ctx, "Refreshing", |r| {
        r.row_kw(
            "Refresh automatically",
            "Watch open folders for changes made by other programs.",
            "watch auto refresh",
            |ui| {
                switch(ui, &mut d.behaviour.auto_refresh);
            },
        );
        r.row("Change debounce", "How long to wait for a burst of changes to settle.", |ui| {
            number(ui, &mut d.behaviour.watch_debounce_ms, 50..=2000, " ms");
        });
    });
    any
}

pub fn jobs(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.settings.draft;
    let ctx = "Jobs file operations copy move";
    let mut any = false;
    any |= group(ui, q, ctx, "Queue", |r| {
        r.row_kw(
            "Parallel jobs",
            "How many copy / move / delete jobs run at once. Applies after restart.",
            "parallelism threads",
            |ui| {
                number(ui, &mut d.jobs.max_parallel, 1..=16, "");
            },
        );
        r.row_kw(
            "Queue mode",
            "Per device runs one job per disk pair at a time, which is fastest on HDDs. Applies after restart.",
            "serial",
            |ui| {
                segmented(ui, &mut d.jobs.per_device_serial, &[(true, "Per device"), (false, "Parallel")]);
            },
        );
        r.row("Open the activity panel automatically", "When a job runs longer than a second.", |ui| {
            switch(ui, &mut d.jobs.auto_open_activity);
        });
        r.row_kw(
            "Reset “apply to all” between jobs",
            "Conflict answers don't carry over to the next job. Applies after restart.",
            "conflict",
            |ui| {
                switch(ui, &mut d.jobs.reset_apply_all_between_jobs);
            },
        );
        r.row("History size", "Completed operations kept for undo. Applies after restart.", |ui| {
            number(ui, &mut d.jobs.history_limit, 10..=5000, "");
        });
    });
    any |= group(ui, q, ctx, "Copying", |r| {
        r.row_kw(
            "Verify after copy",
            "Hash source and destination (BLAKE3) and compare.",
            "checksum integrity",
            |ui| {
                switch(ui, &mut d.jobs.verify_after_copy);
            },
        );
        r.row_kw(
            "Copy-on-write clones",
            "Instant copies on ReFS / Btrfs / APFS when possible.",
            "reflink block clone",
            |ui| {
                segmented(
                    ui,
                    &mut d.jobs.reflink,
                    &[(ReflinkPref::Auto, "Auto"), (ReflinkPref::Never, "Never"), (ReflinkPref::Always, "Always")],
                );
            },
        );
        r.row("Copy buffer", "", |ui| {
            number(ui, &mut d.jobs.copy_buffer_mib, 1..=128, " MiB");
        });
        r.row_kw("Unbuffered I/O above", "Large files bypass the OS cache.", "direct", |ui| {
            number(ui, &mut d.jobs.unbuffered_threshold_mib, 16..=4096, " MiB");
        });
        r.block("Preserve", "Metadata carried over to copies.", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (item, label) in [
                    (PreserveItem::Times, "Timestamps"),
                    (PreserveItem::Attrs, "Attributes"),
                    (PreserveItem::Xattr, "Extended attributes"),
                    (PreserveItem::Acl, "ACLs"),
                    (PreserveItem::Owner, "Owner"),
                    (PreserveItem::Sparse, "Sparse files"),
                ] {
                    let mut on = d.jobs.preserve.contains(&item);
                    if ui.checkbox(&mut on, label).changed() {
                        if on {
                            d.jobs.preserve.push(item);
                        } else {
                            d.jobs.preserve.retain(|x| *x != item);
                        }
                    }
                    ui.add_space(8.0);
                }
            });
        });
    });
    any |= group(ui, q, ctx, "Trash", |r| {
        r.row_kw("When the trash isn't available", "e.g. network drives and FAT volumes.", "recycle", |ui| {
            segmented(
                ui,
                &mut d.jobs.trash_unsupported,
                &[
                    (TrashUnsupportedPolicy::Ask, "Ask"),
                    (TrashUnsupportedPolicy::Permanent, "Delete"),
                    (TrashUnsupportedPolicy::Skip, "Skip"),
                ],
            );
        });
    });
    any
}

pub fn terminal(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.settings.draft;
    let ctx = "Terminal shell";
    let mut any = false;
    any |= group(ui, q, ctx, "Shell", |r| {
        r.row_kw("Shell", "Leave empty to detect automatically. Applies to new sessions.", "pwsh bash zsh cmd", |ui| {
            code(ui, &mut d.terminal.shell, "auto", 160.0);
            let picks: &[&str] = if cfg!(windows) { &["pwsh", "powershell", "cmd"] } else { &["bash", "zsh", "fish"] };
            for pick in picks.iter().rev() {
                if small_button(ui, None, pick, false).clicked() {
                    d.terminal.shell = pick.to_string();
                }
            }
        });
        r.row("Arguments", "Extra arguments for the shell.", |ui| {
            let mut s = crate::commands::join_args(&d.terminal.args);
            if code(ui, &mut s, "-NoLogo", 220.0).changed() {
                d.terminal.args = crate::commands::split_args(&s);
            }
        });
        r.row_kw("Shell integration", "Inject a prompt hook so the terminal reports its folder.", "cwd prompt", |ui| {
            switch(ui, &mut d.terminal.shell_integration);
        });
        r.row("Scrollback", "Lines kept per session.", |ui| {
            number(ui, &mut d.terminal.scrollback, 1000..=100_000, " lines");
        });
    });
    any |= group(ui, q, ctx, "Look", |r| {
        r.row_kw("Font size", "", "terminal text", |ui| {
            number(ui, &mut d.terminal.font_size, 8.0..=24.0, " pt");
        });
        r.row_kw("Colors", "Auto derives the palette from the app theme.", "theme", |ui| {
            let mut v = match d.terminal.theme.as_str() {
                "dark" => 1,
                "light" => 2,
                _ => 0,
            };
            if segmented(ui, &mut v, &[(0, "Auto"), (1, "Dark"), (2, "Light")]) {
                d.terminal.theme = ["auto", "dark", "light"][v].to_string();
            }
        });
    });
    any |= group(ui, q, ctx, "Following", |r| {
        r.row_kw("Follow navigation", "cd the terminal when the focused pane changes folder.", "sync", |ui| {
            switch(ui, &mut d.terminal.follow_navigation);
        });
        r.row_kw("Follow the shell", "Navigate the pane when you cd in the shell.", "reverse sync", |ui| {
            switch(ui, &mut d.terminal.follow_shell);
        });
        r.row("cd on tab switch", "", |ui| {
            switch(ui, &mut d.terminal.cd_on_tab_switch);
        });
        r.row("One terminal per tab", "Otherwise all tabs share the same sessions.", |ui| {
            switch(ui, &mut d.terminal.per_tab);
        });
        r.row_kw("Esc returns focus", "Esc on an empty prompt goes back to the file list.", "escape", |ui| {
            switch(ui, &mut d.terminal.escape_returns_focus);
        });
    });
    any
}

pub fn preview(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.settings.draft;
    let ctx = "Preview search git";
    let mut any = false;
    any |= group(ui, q, ctx, "Preview", |r| {
        r.row_kw("Text preview limit", "Larger files show the first part only.", "max size", |ui| {
            number(ui, &mut d.preview.max_text_kib, 16..=16_384, " KiB");
        });
        r.row_kw("Image size limit", "Larger images aren't decoded for previews.", "megapixels", |ui| {
            number(ui, &mut d.preview.max_image_mp, 4..=400, " MP");
        });
        r.row_kw("Syntax theme", "auto follows the app theme, or a syntect theme name.", "highlight code", |ui| {
            text(ui, &mut d.preview.syntax_theme, "auto", 160.0);
        });
        r.row("PDF preview", "Needs a build with the pdf feature.", |ui| {
            switch(ui, &mut d.preview.pdf);
        });
    });
    any |= group(ui, q, ctx, "Search", |r| {
        r.row_kw("Search threads", "0 uses every core.", "parallel", |ui| {
            number(ui, &mut d.search.threads, 0..=64, "");
        });
        r.row_kw("Respect .gitignore", "Skip ignored files by default.", "ignore", |ui| {
            switch(ui, &mut d.search.respect_gitignore);
        });
        r.row_kw("Content search limit", "Files larger than this are skipped when searching contents.", "grep", |ui| {
            number(ui, &mut d.search.content_max_mib, 1..=1024, " MiB");
        });
        if !d.search.saved.is_empty() {
            r.block("Saved searches from config.toml", "", |ui| {
                let mut remove = None;
                for (i, s) in d.search.saved.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(crate::theme::icons::icon(ph::MAGNIFYING_GLASS, 13.0));
                        ui.label(&s.name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if small_button(ui, Some(ph::TRASH), "", true).clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if let Some(i) = remove {
                    d.search.saved.remove(i);
                }
            });
        }
    });
    any |= group(ui, q, ctx, "Git", |r| {
        r.row_kw("Git status column", "Show modified / untracked markers in repositories.", "vcs", |ui| {
            switch(ui, &mut d.git.status_column);
        });
        r.row("Refresh interval", "", |ui| {
            number(ui, &mut d.git.refresh_secs, 1..=120, " s");
        });
    });
    any
}

pub fn advanced(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let path = app.config_watch.path().to_path_buf();
    let mut edit = false;
    let mut reload = false;
    let mut export = false;
    let mut import = false;
    let mut reset_all = false;
    let mut copy = false;
    let mut any = false;
    any |= group(ui, q, "Config file advanced", "config.toml", |r| {
        r.block("", "Every setting lives in one TOML file. Edits made in an editor apply within a second; parse errors show where they are and keep the previous settings.", |ui| {
            ui.horizontal(|ui| {
                ui.label(crate::theme::icons::icon(ph::FILE_CODE, 14.0).color(p.accent));
                ui.add(egui::Label::new(RichText::new(path.display().to_string()).monospace().color(p.text_secondary)).truncate());
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                edit = small_button(ui, Some(ph::PENCIL_SIMPLE), "Edit config file", false).clicked();
                reload = small_button(ui, Some(ph::ARROW_CLOCKWISE), "Reload from disk", false).clicked();
                copy = small_button(ui, Some(ph::CLIPBOARD_TEXT), "Copy as TOML", false).clicked();
            });
        });
    });
    any |= group(ui, q, "Config file advanced", "Export & import", |r| {
        r.row_kw("Export settings", "Save a copy of config.toml anywhere you like.", "backup save", |ui| {
            export = small_button(ui, Some(ph::EXPORT), "Export…", false).clicked();
        });
        r.row_kw("Import settings", "Load a config file (replaces the current settings).", "restore load", |ui| {
            import = small_button(ui, Some(ph::DOWNLOAD_SIMPLE), "Import…", false).clicked();
        });
    });
    any |= group(ui, q, "Config file advanced", "Danger zone", |r| {
        r.row_kw("Reset everything", "Restore every setting to its default.", "defaults factory", |ui| {
            reset_all = small_button(ui, Some(ph::WARNING), "Reset all settings", true).clicked();
        });
    });
    if edit {
        app.cmds.push(crate::cmd::Cmd::Action(soyuz_core::config::Action::EditConfigFile));
        super::close(app);
    }
    if reload {
        app.cmds.push(crate::cmd::Cmd::Action(soyuz_core::config::Action::ReloadConfig));
    }
    if copy && let Ok(t) = soyuz_core::config::to_pretty_string(&app.config) {
        app.cmds.push(crate::cmd::Cmd::CopyText(t));
    }
    if export {
        super::start_export(app);
    }
    if import {
        super::start_import(app);
    }
    if reset_all {
        app.settings.draft = soyuz_core::config::Config::default();
        app.ui.toasts.push(crate::ui::toasts::Toast::info("All settings reset to defaults"));
    }
    any
}

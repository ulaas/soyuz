//! Settings pages for the plain config sections: general, views & layout, behaviour,
//! file operations, terminal, terminal UI, preview / search / git, and the config file page.

use std::time::SystemTime;

use egui::{RichText, Ui};
use soyuz_core::config::{
    ActivityPlacement, DragDefault, MiddleClick, OpenFoldersIn, Orientation, Osc52Policy, PasteConflict, PreserveItem,
    ReflinkPref, RenameSelects, SidebarMode, SidebarSection, TabStripMode, TrashUnsupportedPolicy, TypeAhead,
};
use soyuz_core::{Column, DateFormat, DateFormatName, SizeStyle, ViewMode, format_size, format_time};

use super::widgets::{choice, code, dd, dropdown, group, number, segmented, small_button, switch, text, text_choice};
use crate::app::SoyuzApp;
use crate::theme::icons::ph;
use crate::theme::palette;

const CTX_GENERAL: &str = "General";

pub fn general(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.gui.settings.draft;
    let mut any = false;
    any |= group(ui, q, CTX_GENERAL, tr!("settings-general-startup"), |r| {
        r.row_kw(
            tr!("settings-general-start-in"),
            tr!("settings-general-start-in-desc"),
            "start location home last",
            |ui| {
                let mut mode = match d.general.start_location.trim().to_ascii_lowercase().as_str() {
                    "" | "home" => 0,
                    "last" => 1,
                    _ => 2,
                };
                let before = mode;
                if mode == 2 {
                    let hint = if cfg!(windows) { "C:\\Users\\me\\Projects" } else { "~/Projects" };
                    text(ui, &mut d.general.start_location, hint, 200.0);
                }
                segmented(
                    ui,
                    &mut mode,
                    &[
                        (0, tr!("settings-general-start-home")),
                        (1, tr!("settings-general-start-last")),
                        (2, tr!("settings-general-start-folder")),
                    ],
                );
                if mode != before {
                    d.general.start_location = match mode {
                        0 => "home".into(),
                        1 => "last".into(),
                        _ => soyuz_fs::home().to_string_lossy().into_owned(),
                    };
                }
            },
        );
        r.row(tr!("settings-general-restore-tabs"), tr!("settings-general-restore-tabs-desc"), |ui| {
            switch(ui, &mut d.behaviour.restore_session);
        });
        r.row_kw(
            tr!("settings-general-language"),
            tr!("settings-general-language-desc"),
            "language locale translation dil türkçe english",
            |ui| {
                let choices = crate::i18n::language_choices();
                // "en" / "tr" (older configs) select their full tag.
                let mut sel = match crate::i18n::negotiate(&d.general.language) {
                    _ if d.general.language.trim().eq_ignore_ascii_case("auto") => "auto",
                    Some(l) => l.tag(),
                    None => "auto",
                };
                let opts: Vec<(&'static str, &str)> = choices.iter().map(|(v, l)| (*v, l.as_str())).collect();
                if dropdown(ui, "language", &mut sel, &opts, 160.0) {
                    d.general.language = sel.to_string();
                }
            },
        );
    });
    any |= group(ui, q, CTX_GENERAL, tr!("settings-general-files"), |r| {
        let hidden_desc = if cfg!(unix) {
            tr!("settings-general-show-hidden-desc-unix")
        } else {
            tr!("settings-general-show-hidden-desc")
        };
        r.row_kw(tr!("settings-general-show-hidden"), hidden_desc, "dotfiles", |ui| {
            switch(ui, &mut d.general.show_hidden);
        });
        // `.hidden` files are a GNOME / KDE convention; Windows has its own hidden attribute.
        if cfg!(unix) {
            r.row_kw(
                tr!("settings-general-dot-hidden"),
                tr!("settings-general-dot-hidden-desc"),
                "hidden dotfiles nautilus dolphin",
                |ui| {
                    switch(ui, &mut d.general.honour_dot_hidden);
                },
            );
        }
        // Protected operating-system files are a Windows attribute.
        if cfg!(windows) {
            r.row(tr!("settings-general-show-system"), tr!("settings-general-show-system-desc"), |ui| {
                switch(ui, &mut d.general.show_system);
            });
        }
        r.row(tr!("settings-general-show-ext"), tr!("settings-general-show-ext-desc"), |ui| {
            switch(ui, &mut d.general.show_extensions);
        });
        r.row_kw(
            tr!("settings-general-strict-names"),
            if cfg!(windows) {
                tr!("settings-general-strict-names-desc")
            } else {
                tr!("settings-general-strict-names-desc-unix")
            },
            "validate portable fat exfat ntfs",
            |ui| {
                switch(ui, &mut d.general.strict_windows_names);
            },
        );
    });
    if cfg!(target_os = "linux") {
        any |= linux_group(ui, q, d);
    }
    any |= group(ui, q, CTX_GENERAL, tr!("settings-general-formats"), |r| {
        let sample = 1_288_490_189u64;
        r.row_kw(
            tr!("settings-general-sizes"),
            &tr!("settings-general-sizes-desc", sample = format_size(sample, d.general.size_style)),
            "units kib mb",
            |ui| {
                segmented(
                    ui,
                    &mut d.general.size_style,
                    &[
                        (SizeStyle::Binary, "KiB, MiB"),
                        (SizeStyle::Decimal, "kB, MB"),
                        (SizeStyle::Bytes, tr!("settings-general-sizes-bytes")),
                    ],
                );
            },
        );
        let preview =
            format_time(SystemTime::now() - std::time::Duration::from_secs(3600 * 26), &d.general.date_format);
        r.row_kw(
            tr!("settings-general-dates"),
            &tr!("settings-general-dates-desc", preview = preview),
            "time format strftime",
            |ui| {
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
                // Each entry shows what it looks like.
                let then = SystemTime::now() - std::time::Duration::from_secs(3600 * 26);
                let named = |n: DateFormatName, label: &str| {
                    dd::Item::new(Some(n), label).secondary(format_time(then, &DateFormat::Named(n)))
                };
                let opts = [
                    named(DateFormatName::Relative, tr!("settings-general-date-relative")),
                    named(DateFormatName::Short, tr!("settings-general-date-short")),
                    named(DateFormatName::Long, tr!("settings-general-date-long")),
                    named(DateFormatName::Iso, "ISO 8601"),
                    dd::Item::new(None, tr!("settings-general-date-custom"))
                        .secondary("strftime")
                        .icon(ph::PENCIL_SIMPLE),
                ];
                if choice(ui, "date_format", &mut sel, &opts, 150.0) {
                    d.general.date_format = match sel {
                        Some(n) => DateFormat::Named(n),
                        None => DateFormat::Custom("%Y-%m-%d %H:%M".into()),
                    };
                }
            },
        );
    });
    any
}

/// `[linux]`: Linux desktop integration (General page, Linux only).
fn linux_group(ui: &mut Ui, q: &str, d: &mut soyuz_core::config::Config) -> bool {
    use soyuz_core::config::WindowBackend;
    group(ui, q, "General linux desktop wayland x11", tr!("settings-linux"), |r| {
        r.row_kw(
            tr!("settings-linux-window-backend"),
            tr!("settings-linux-window-backend-desc"),
            "wayland x11 xwayland display server drag drop",
            |ui| {
                segmented(
                    ui,
                    &mut d.linux.window_backend,
                    &[
                        (WindowBackend::Auto, tr!("settings-auto")),
                        (WindowBackend::Wayland, "Wayland"),
                        (WindowBackend::X11, "X11"),
                    ],
                );
            },
        );
        r.row_kw(
            tr!("settings-linux-file-manager1"),
            tr!("settings-linux-file-manager1-desc"),
            "dbus filemanager1 show in folder reveal browser",
            |ui| {
                switch(ui, &mut d.linux.own_file_manager1);
                fm1_status(ui, d.linux.own_file_manager1);
            },
        );
        r.row_kw(
            tr!("settings-linux-clipboard-tools"),
            tr!("settings-linux-clipboard-tools-desc"),
            "clipboard wl-copy xclip copy paste",
            |ui| {
                switch(ui, &mut d.linux.clipboard_fallback_tools);
            },
        );
    })
}

/// State of the FileManager1 service next to its switch (a snapshot; never blocks).
fn fm1_status(ui: &mut Ui, wanted: bool) {
    let p = palette(ui.ctx());
    let (text, color) = match crate::services::filemanager1::status() {
        Some(Ok(())) => (tr!("settings-linux-fm1-serving").to_string(), p.success),
        Some(Err(why)) => (why, p.warning),
        None if wanted => {
            // Starting (or the change is not applied yet): look again shortly.
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            (tr!("settings-linux-fm1-pending").to_string(), p.text_tertiary)
        }
        None => (tr!("settings-linux-fm1-off").to_string(), p.text_tertiary),
    };
    ui.label(RichText::new(text).size(11.5).color(color));
}

fn modes() -> [dd::Item<ViewMode>; 5] {
    [
        dd::Item::new(ViewMode::Details, tr!("settings-view-details")).icon(ph::ROWS),
        dd::Item::new(ViewMode::Grid, tr!("settings-view-grid")).icon(ph::SQUARES_FOUR),
        dd::Item::new(ViewMode::List, tr!("settings-view-list")).icon(ph::LIST),
        dd::Item::new(ViewMode::Miller, tr!("settings-view-columns")).icon(ph::COLUMNS),
        dd::Item::new(ViewMode::Gallery, tr!("settings-view-gallery")).icon(ph::IMAGES),
    ]
}

pub fn views(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.gui.settings.draft;
    let ctx = "Views layout";
    let mut any = false;
    any |= group(ui, q, ctx, tr!("settings-views-folders"), |r| {
        r.row(tr!("settings-views-default-view"), tr!("settings-views-default-view-desc"), |ui| {
            choice(ui, "default_view", &mut d.general.default_view, &modes(), 160.0);
        });
        r.row(tr!("settings-views-folders-first"), tr!("settings-views-folders-first-desc"), |ui| {
            switch(ui, &mut d.general.folders_first);
        });
        r.row_kw(tr!("settings-views-natural-sort"), tr!("settings-views-natural-sort-desc"), "numeric", |ui| {
            switch(ui, &mut d.general.natural_sort);
        });
        r.row(tr!("settings-views-case-sort"), tr!("settings-views-case-sort-desc"), |ui| {
            switch(ui, &mut d.general.case_insensitive_sort);
        });
        r.row_kw(tr!("settings-views-remember"), tr!("settings-views-remember-desc"), "memory", |ui| {
            switch(ui, &mut d.behaviour.remember_folder_views);
        });
    });
    any |= group(ui, q, ctx, tr!("settings-views-details-columns"), |r| {
        r.block("", tr!("settings-views-details-columns-desc"), |ui| {
            columns_editor(ui, &mut d.columns.details, &mut d.columns.widths);
        });
    });
    any |= group(ui, q, ctx, tr!("settings-views-window-layout"), |r| {
        r.row(tr!("settings-views-sidebar"), tr!("settings-views-sidebar-desc"), |ui| {
            switch(ui, &mut d.layout.sidebar);
        });
        r.row_kw(
            tr!("settings-views-sidebar-content"),
            tr!("settings-views-sidebar-content-desc"),
            "folder tree places navigation pane",
            |ui| {
                segmented(
                    ui,
                    &mut d.sidebar.mode,
                    &[
                        (SidebarMode::Places, tr!("settings-views-sidebar-places")),
                        (SidebarMode::Tree, tr!("settings-views-folder-tree")),
                        (SidebarMode::Both, tr!("settings-views-sidebar-both")),
                    ],
                );
            },
        );
        r.row_kw(tr!("settings-views-tree-follow"), tr!("settings-views-tree-follow-desc"), "sync reveal tree", |ui| {
            switch(ui, &mut d.sidebar.tree_follow);
        });
        r.row_kw(
            tr!("settings-views-tree-drag"),
            tr!("settings-views-tree-drag-desc"),
            "drag drop spring loaded tree",
            |ui| {
                number(ui, &mut d.sidebar.tree_drag_expand_ms, 0..=3000, " ms");
            },
        );
        r.row(tr!("settings-views-status-bar"), tr!("settings-views-status-bar-desc"), |ui| {
            switch(ui, &mut d.layout.status_bar);
        });
        r.row(tr!("settings-views-preview-pane"), tr!("settings-views-preview-pane-desc"), |ui| {
            switch(ui, &mut d.layout.preview_pane);
        });
        r.row_kw(tr!("settings-views-dual-pane"), tr!("settings-views-dual-pane-desc"), "split commander", |ui| {
            switch(ui, &mut d.layout.dual_pane);
        });
        r.row(tr!("settings-views-dual-orientation"), "", |ui| {
            segmented(
                ui,
                &mut d.layout.dual_pane_orientation,
                &[
                    (Orientation::Vertical, tr!("settings-views-side-by-side")),
                    (Orientation::Horizontal, tr!("settings-views-stacked")),
                ],
            );
        });
        r.row(tr!("settings-views-tab-strip"), "", |ui| {
            segmented(
                ui,
                &mut d.layout.tab_strip,
                &[
                    (TabStripMode::Always, tr!("settings-always")),
                    (TabStripMode::Multiple, tr!("settings-views-tab-strip-multiple")),
                ],
            );
        });
        r.row(tr!("settings-views-activity-panel"), tr!("settings-views-activity-panel-desc"), |ui| {
            segmented(
                ui,
                &mut d.layout.activity_panel,
                &[
                    (ActivityPlacement::Right, tr!("settings-views-activity-right")),
                    (ActivityPlacement::Bottom, tr!("settings-views-activity-bottom")),
                    (ActivityPlacement::Floating, tr!("settings-views-activity-floating")),
                ],
            );
        });
        r.block(tr!("settings-views-focus-mode"), tr!("settings-views-focus-mode-desc"), |ui| {
            focus_mode_parts(ui, &mut d.layout.focus_mode_hides);
        });
        r.row(tr!("settings-views-terminal-height"), tr!("settings-views-terminal-height-desc"), |ui| {
            number(ui, &mut d.layout.terminal_height, 120.0..=800.0, " px");
        });
    });
    any |= group(ui, q, ctx, tr!("settings-views-windows"), |r| {
        use soyuz_core::config::{CloseWindowTabs, NewWindowMode};
        r.row_kw(
            tr!("settings-views-new-window"),
            tr!("settings-views-new-window-desc"),
            "multi window process",
            |ui| {
                segmented(
                    ui,
                    &mut d.workspace.new_window,
                    &[
                        (NewWindowMode::InProcess, tr!("settings-views-new-window-window")),
                        (NewWindowMode::Process, tr!("settings-views-new-window-process")),
                    ],
                );
            },
        );
        r.row_kw(tr!("settings-views-tear-off"), tr!("settings-views-tear-off-desc"), "detach drag", |ui| {
            switch(ui, &mut d.workspace.tear_off);
        });
        r.row_kw(tr!("settings-views-tear-off-distance"), tr!("settings-views-tear-off-distance-desc"), "drag", |ui| {
            number(ui, &mut d.workspace.tear_off_distance, 16.0..=200.0, " px");
        });
        r.row_kw(
            tr!("settings-views-close-window"),
            tr!("settings-views-close-window-desc"),
            "merge close window",
            |ui| {
                segmented(
                    ui,
                    &mut d.workspace.close_window_tabs,
                    &[
                        (CloseWindowTabs::Ask, tr!("settings-ask")),
                        (CloseWindowTabs::Merge, tr!("settings-views-move-tabs")),
                        (CloseWindowTabs::Close, tr!("settings-views-close-tabs")),
                    ],
                );
            },
        );
        r.row_kw(tr!("settings-views-restore-windows"), tr!("settings-views-restore-windows-desc"), "session", |ui| {
            switch(ui, &mut d.workspace.restore_windows);
        });
        r.row_kw(
            tr!("settings-views-sync-relative"),
            tr!("settings-views-sync-relative-desc"),
            "sync navigation follow",
            |ui| {
                switch(ui, &mut d.workspace.sync_relative);
            },
        );
        r.row_kw(
            tr!("settings-views-history-graph"),
            tr!("settings-views-history-graph-desc"),
            "history graph",
            |ui| {
                number(ui, &mut d.workspace.history_graph_limit, 20..=2000, "");
            },
        );
        r.row_kw(tr!("settings-views-timeline-range"), tr!("settings-views-timeline-range-desc"), "timeline", |ui| {
            number(ui, &mut d.workspace.timeline_days, 1..=60, &format!(" {}", tr!("settings-unit-days")));
        });
        r.row_kw(
            tr!("settings-views-timeline-folders"),
            tr!("settings-views-timeline-folders-desc"),
            "timeline scan",
            |ui| {
                number(ui, &mut d.workspace.timeline_scan_folders, 0..=300, "");
            },
        );
        r.row_kw(
            tr!("settings-views-timeline-items"),
            tr!("settings-views-timeline-items-desc"),
            "timeline limit",
            |ui| {
                number(ui, &mut d.workspace.timeline_max_items, 50..=10_000, "");
            },
        );
        r.row_kw(
            tr!("settings-views-timeline-index"),
            tr!("settings-views-timeline-index-desc"),
            "timeline index",
            |ui| {
                switch(ui, &mut d.workspace.timeline_use_index);
            },
        );
    });
    any |= group(ui, q, ctx, tr!("settings-views-sidebar-sections"), |r| {
        r.block("", tr!("settings-views-sidebar-sections-desc"), |ui| {
            sidebar_sections(ui, &mut d.sidebar.sections);
        });
        // GTK bookmarks are a Linux desktop convention.
        if cfg!(target_os = "linux") {
            r.row_kw(
                tr!("settings-views-gtk-bookmarks"),
                tr!("settings-views-gtk-bookmarks-desc"),
                "gtk bookmarks nautilus favorites places",
                |ui| {
                    switch(ui, &mut d.sidebar.gtk_bookmarks);
                },
            );
            // Unmounted disks under Devices (UDisks2).
            r.row_kw(
                tr!("settings-views-unmounted-devices"),
                tr!("settings-views-unmounted-devices-desc"),
                "unmounted devices disks partitions mount unlock bitlocker luks ntfs udisks",
                |ui| {
                    use soyuz_core::config::UnmountedDevices as U;
                    segmented(
                        ui,
                        &mut d.sidebar.unmounted_devices,
                        &[
                            (U::All, tr!("settings-views-unmounted-all")),
                            (U::Removable, tr!("settings-views-unmounted-removable")),
                            (U::Off, tr!("settings-views-unmounted-off")),
                        ],
                    );
                },
            );
            r.block(tr!("settings-views-hidden-devices"), tr!("settings-views-hidden-devices-desc"), |ui| {
                lines_editor(ui, "hidden_devices", &mut d.sidebar.hidden_devices, 2);
                ui.add_space(4.0);
                let any = !d.sidebar.hidden_devices.is_empty();
                let clear = ui.add_enabled_ui(any, |ui| {
                    small_button(ui, Some(ph::EYE), tr!("settings-views-hidden-devices-clear"), false)
                });
                if clear.inner.clicked() {
                    d.sidebar.hidden_devices.clear();
                }
            });
        }
    });
    any |= group(ui, q, ctx, tr!("settings-views-sysfolders"), |r| {
        r.row_kw(
            tr!("settings-views-sysfolders-show"),
            tr!("settings-views-sysfolders-show-desc"),
            "system folders appdata programdata temp program files config cache etc",
            |ui| {
                switch(ui, &mut d.sidebar.system_folders);
            },
        );
        if !crate::models::system_folders::ids().is_empty() {
            let on = d.sidebar.system_folders;
            r.block(tr!("settings-views-sysfolders-list"), tr!("settings-views-sysfolders-list-desc"), |ui| {
                ui.add_enabled_ui(on, |ui| system_folder_list(ui, &mut d.sidebar.system_folder_list));
            });
        }
    });
    any |= group(ui, q, ctx, tr!("settings-views-portal"), |r| {
        r.block("", tr!("settings-views-portal-desc"), |ui| {
            portal_pins(ui, &mut d.sidebar.portal);
        });
    });
    any |= group(ui, q, ctx, tr!("settings-views-tag-names"), |r| {
        r.block("", tr!("settings-views-tag-names-desc"), |_| {});
        tag_name_rows(r, &mut d.tags.names);
        // Extended attributes shared with Dolphin & co. are a Linux convention.
        if cfg!(target_os = "linux") {
            r.row_kw(
                tr!("settings-views-tags-xdg"),
                tr!("settings-views-tags-xdg-desc"),
                "tags xattr dolphin baloo kde user.xdg.tags",
                |ui| {
                    switch(ui, &mut d.tags.sync_xdg_tags);
                },
            );
        }
    });
    any |= group(ui, q, ctx, tr!("settings-views-toolbar"), |r| {
        r.block(tr!("settings-views-toolbar-items"), tr!("settings-views-toolbar-items-desc"), |ui| {
            lines_editor(ui, "toolbar", &mut d.toolbar.items, 5);
        });
        r.block(tr!("settings-views-ctx-hide"), tr!("settings-views-ctx-hide-desc"), |ui| {
            lines_editor(ui, "ctx_hide", &mut d.context_menu.hide, 3);
        });
        // `[context_menu] order` is not offered: the menus have a fixed, grouped layout
        // and don't read it yet (the key stays accepted in config.toml).
    });
    any
}

/// `[tags] names`: one row per color (dot + name field, the color name as placeholder).
fn tag_name_rows(r: &mut super::widgets::Rows<'_>, names: &mut Vec<String>) {
    use crate::services::tags::{TAG_COUNT, default_name};
    for t in 0..TAG_COUNT {
        let color_name = default_name(t as u8);
        r.row_kw(color_name, "", "tag tags label rename name etiket", |ui| {
            let mut s = names.get(t).cloned().unwrap_or_default();
            let resp = text(ui, &mut s, color_name, 200.0);
            if resp.changed() {
                let s: String = s.chars().take(soyuz_core::config::TAG_NAME_MAX_CHARS).collect();
                if names.len() <= t {
                    names.resize(TAG_COUNT, String::new());
                }
                names[t] = s;
            }
            let (dot, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 5.5, palette(ui.ctx()).tags[t]);
        });
    }
}

fn columns_editor(ui: &mut Ui, details: &mut Vec<Column>, widths: &mut std::collections::BTreeMap<String, f32>) {
    let p = palette(ui.ctx());
    let mut action: Option<(usize, i32)> = None;
    // Shown columns, in order.
    for (i, col) in details.clone().iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::ui::views::columns::header_label(*col)).color(p.text_primary));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if *col != Column::Name
                    && small_button(ui, Some(ph::X), "", true)
                        .on_hover_text(tr!("settings-views-hide-column"))
                        .clicked()
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
                let dv = ui.add(egui::DragValue::new(&mut w).range(40.0..=800.0).suffix(" px").speed(1.0));
                super::widgets::named(&dv, &crate::ui::views::columns::header_label(*col));
                if dv.changed() {
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
    let hidden: Vec<Column> = Column::ALL
        .into_iter()
        .filter(|c| !details.contains(c))
        .filter(|c| c.script_slot().is_none() || crate::automation::scripts::column_label(*c).is_some())
        .collect();
    if !hidden.is_empty() {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr!("settings-views-add-column")).size(11.5).color(p.text_tertiary));
            for c in hidden {
                if small_button(ui, Some(ph::PLUS), &crate::ui::views::columns::header_label(c), false).clicked() {
                    details.push(c);
                }
            }
        });
    }
}

/// One switch per part focus mode can hide (`[layout] focus_mode_hides`).
fn focus_mode_parts(ui: &mut Ui, list: &mut Vec<String>) {
    use crate::models::focus_mode as fm;
    let p = palette(ui.ctx());
    for &id in fm::PARTS {
        let mut on = list.iter().any(|x| x == id);
        ui.horizontal(|ui| {
            if switch(ui, &mut on).changed() {
                if on {
                    list.push(id.to_string());
                } else {
                    list.retain(|x| x != id);
                }
            }
            ui.label(RichText::new(fm::label(id)).color(p.text_primary));
        });
    }
}

/// One switch per system folder of this OS, with its path (ids of other OSes in the list
/// are kept).
fn system_folder_list(ui: &mut Ui, list: &mut Vec<String>) {
    use crate::models::system_folders as sf;
    let p = palette(ui.ctx());
    for &id in sf::ids() {
        let mut on = list.iter().any(|x| x == id);
        ui.horizontal(|ui| {
            if switch(ui, &mut on).changed() {
                if on {
                    list.push(id.to_string());
                } else {
                    list.retain(|x| x != id);
                }
            }
            ui.label(RichText::new(sf::label(id)).color(p.text_primary));
            let path = sf::available().iter().find(|f| f.id == id).map(|f| f.path.display().to_string());
            let note = path.unwrap_or_else(|| tr!("settings-views-sysfolders-missing").to_string());
            ui.label(RichText::new(note).size(11.5).color(p.text_tertiary));
        });
    }
}

fn sidebar_sections(ui: &mut Ui, sections: &mut Vec<SidebarSection>) {
    let p = palette(ui.ctx());
    let all = [
        (SidebarSection::Favorites, tr!("settings-views-section-favorites")),
        (SidebarSection::Devices, tr!("settings-views-section-devices")),
        (SidebarSection::Portal, tr!("settings-views-section-portal")),
        (SidebarSection::Tags, tr!("settings-views-section-tags")),
        (SidebarSection::Trash, tr!("settings-views-section-trash")),
        (SidebarSection::Network, tr!("settings-views-section-network")),
        (SidebarSection::Cloud, tr!("settings-views-section-cloud")),
        (SidebarSection::Smart, tr!("settings-views-section-smart")),
        (SidebarSection::Tree, tr!("settings-views-section-tree")),
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

/// The Portal's pinned folders: label (empty = folder name), path, reorder, unpin.
fn portal_pins(ui: &mut Ui, pins: &mut Vec<soyuz_core::config::PortalPin>) {
    let p = palette(ui.ctx());
    if pins.is_empty() {
        ui.label(RichText::new(tr!("settings-views-portal-empty")).color(p.text_tertiary));
        return;
    }
    let mut action: Option<(usize, bool)> = None;
    let n = pins.len();
    for (i, pin) in pins.iter_mut().enumerate() {
        ui.push_id(("soyuz.settings.portal", i), |ui| {
            ui.horizontal(|ui| {
                let hint = soyuz_core::entry::display_name(&pin.path);
                text(ui, &mut pin.name, &hint, 150.0);
                let path = soyuz_fs::strip_verbatim(&pin.path).display().to_string();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_button(ui, Some(ph::PUSH_PIN_SLASH), "", true)
                        .on_hover_text(tr!("settings-views-portal-unpin"))
                        .clicked()
                    {
                        action = Some((i, false));
                    }
                    if i + 1 < n
                        && small_button(ui, Some(ph::ARROW_DOWN), "", false)
                            .on_hover_text(tr!("settings-views-portal-down"))
                            .clicked()
                    {
                        action = Some((i, true));
                    }
                    if i > 0
                        && small_button(ui, Some(ph::ARROW_UP), "", false)
                            .on_hover_text(tr!("settings-views-portal-up"))
                            .clicked()
                    {
                        action = Some((i - 1, true));
                    }
                    ui.add(egui::Label::new(RichText::new(&path).color(p.text_secondary)).truncate())
                        .on_hover_text(&path);
                });
            });
        });
    }
    if let Some((i, swap)) = action {
        if swap {
            pins.swap(i, i + 1);
        } else {
            pins.remove(i);
        }
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
    let d = &mut app.gui.settings.draft;
    let ctx = "Behaviour";
    let mut any = false;
    any |= group(ui, q, ctx, tr!("settings-behaviour-opening"), |r| {
        r.row_kw(tr!("settings-behaviour-single-click"), tr!("settings-behaviour-single-click-desc"), "click", |ui| {
            switch(ui, &mut d.general.single_click_open);
        });
        r.row(tr!("settings-behaviour-double-click"), tr!("settings-behaviour-double-click-desc"), |ui| {
            number(ui, &mut d.general.double_click_speed_ms, 150..=900, " ms");
        });
        r.row(tr!("settings-behaviour-open-folders-in"), "", |ui| {
            segmented(
                ui,
                &mut d.general.open_folders_in,
                &[
                    (OpenFoldersIn::SameTab, tr!("settings-behaviour-same-tab")),
                    (OpenFoldersIn::NewTab, tr!("settings-behaviour-new-tab")),
                ],
            );
        });
        r.row(tr!("settings-behaviour-middle-click"), "", |ui| {
            segmented(
                ui,
                &mut d.behaviour.middle_click,
                &[
                    (MiddleClick::NewTab, tr!("settings-behaviour-new-tab")),
                    (MiddleClick::OtherPane, tr!("settings-behaviour-other-pane")),
                    (MiddleClick::Nothing, tr!("settings-behaviour-nothing")),
                ],
            );
        });
    });
    any |= group(ui, q, ctx, tr!("settings-behaviour-confirmations"), |r| {
        r.row_kw(
            tr!("settings-behaviour-confirm-trash"),
            tr!("settings-behaviour-confirm-trash-desc"),
            "delete recycle",
            |ui| {
                switch(ui, &mut d.general.confirm_delete);
            },
        );
        r.row_kw(
            tr!("settings-behaviour-confirm-permanent"),
            tr!("settings-behaviour-confirm-permanent-desc"),
            "delete",
            |ui| {
                switch(ui, &mut d.general.confirm_permanent_delete);
            },
        );
        r.row(tr!("settings-behaviour-paste-conflict"), "", |ui| {
            dropdown(
                ui,
                "paste_conflict",
                &mut d.behaviour.paste_conflict,
                &[
                    (PasteConflict::Ask, tr!("settings-ask")),
                    (PasteConflict::Overwrite, tr!("settings-behaviour-overwrite")),
                    (PasteConflict::Skip, tr!("settings-skip")),
                    (PasteConflict::KeepBoth, tr!("settings-behaviour-keep-both")),
                ],
                130.0,
            );
        });
        r.row_kw(
            tr!("settings-behaviour-paste-images"),
            tr!("settings-behaviour-paste-images-desc"),
            "paste image picture screenshot clipboard png",
            |ui| {
                switch(ui, &mut d.behaviour.paste_images);
            },
        );
        r.row_kw(
            tr!("settings-behaviour-clipboard-history"),
            tr!("settings-behaviour-clipboard-history-desc"),
            "clipboard history copy cut paste recent",
            |ui| {
                number(ui, &mut d.behaviour.clipboard_history, 0..=50, "");
            },
        );
    });
    any |= group(ui, q, ctx, tr!("settings-behaviour-drag-drop"), |r| {
        r.row_kw(
            tr!("settings-behaviour-drop-action"),
            tr!("settings-behaviour-drop-action-desc"),
            "drag drop move copy",
            |ui| {
                segmented(
                    ui,
                    &mut d.behaviour.drag_default,
                    &[
                        (DragDefault::Auto, tr!("settings-auto")),
                        (DragDefault::Copy, tr!("settings-behaviour-drop-copy")),
                        (DragDefault::Move, tr!("settings-behaviour-drop-move")),
                        (DragDefault::Ask, tr!("settings-ask")),
                    ],
                );
            },
        );
        r.row_kw(
            tr!("settings-behaviour-spring"),
            tr!("settings-behaviour-spring-desc"),
            "drag hover open spring",
            |ui| {
                switch(ui, &mut d.behaviour.spring_loaded_folders);
            },
        );
        r.row_kw(
            tr!("settings-behaviour-spring-delay"),
            tr!("settings-behaviour-spring-delay-desc"),
            "drag hover delay",
            |ui| {
                number(ui, &mut d.behaviour.spring_load_delay_ms, 200..=3000, " ms");
            },
        );
        if drag_out_supported() {
            r.row_kw(
                tr!("settings-behaviour-drag-out"),
                if cfg!(windows) {
                    tr!("settings-behaviour-drag-out-desc")
                } else {
                    tr!("settings-behaviour-drag-out-desc-unix")
                },
                "drag out external desktop explorer other apps",
                |ui| {
                    switch(ui, &mut d.behaviour.drag_out);
                },
            );
        }
    });
    any |= group(ui, q, ctx, tr!("settings-behaviour-typing"), |r| {
        r.row_kw(tr!("settings-behaviour-type-ahead"), tr!("settings-behaviour-type-ahead-desc"), "type ahead", |ui| {
            segmented(
                ui,
                &mut d.behaviour.type_ahead,
                &[
                    (TypeAhead::Jump, tr!("settings-behaviour-jump")),
                    (TypeAhead::Filter, tr!("settings-behaviour-filter")),
                ],
            );
        });
        r.row(tr!("settings-behaviour-rename-selects"), tr!("settings-behaviour-rename-selects-desc"), |ui| {
            segmented(
                ui,
                &mut d.behaviour.rename_selects,
                &[
                    (RenameSelects::Stem, tr!("settings-behaviour-name-only")),
                    (RenameSelects::All, tr!("settings-behaviour-everything")),
                ],
            );
        });
    });
    any |= group(ui, q, ctx, tr!("settings-open-title"), |r| {
        r.row_kw(
            tr!("settings-open-remember"),
            tr!("settings-open-remember-desc"),
            "open with remember last application program",
            |ui| {
                switch(ui, &mut d.open.remember_choice);
            },
        );
        r.row_kw(
            tr!("settings-open-unknown"),
            tr!("settings-open-unknown-desc"),
            "open with unknown unrecognized unregistered type no extension association",
            |ui| {
                switch(ui, &mut d.open.choose_when_unknown);
            },
        );
        r.row_kw(
            tr!("settings-open-run-binaries"),
            tr!("settings-open-run-binaries-desc"),
            "run execute launch program binary executable appimage elf",
            |ui| {
                switch(ui, &mut d.open.run_binaries);
            },
        );
        r.block(tr!("settings-open-defaults"), tr!("settings-open-defaults-desc"), |ui| {
            default_apps_editor(ui, &mut d.open.default_apps);
        });
    });
    any |= group(ui, q, ctx, tr!("settings-behaviour-refreshing"), |r| {
        r.row_kw(
            tr!("settings-behaviour-auto-refresh"),
            tr!("settings-behaviour-auto-refresh-desc"),
            "watch auto refresh",
            |ui| {
                switch(ui, &mut d.behaviour.auto_refresh);
            },
        );
        r.row(tr!("settings-behaviour-debounce"), tr!("settings-behaviour-debounce-desc"), |ui| {
            number(ui, &mut d.behaviour.watch_debounce_ms, 50..=2000, " ms");
        });
        // Only Linux runs out of change notifications (inotify's per-user watch limit).
        if cfg!(target_os = "linux") {
            r.row_kw(
                tr!("settings-behaviour-watch-poll"),
                tr!("settings-behaviour-watch-poll-desc"),
                "inotify max_user_watches poll watch limit",
                |ui| {
                    number(ui, &mut d.behaviour.watch_poll_interval_ms, 500..=60_000, " ms");
                },
            );
        }
    });
    any
}

/// `[[open.default_apps]]`: one row per rule (extensions, name, program, remove). The
/// extensions field keeps its text while focused, so separators can be typed.
fn default_apps_editor(ui: &mut Ui, rules: &mut Vec<soyuz_core::config::DefaultApp>) {
    let p = palette(ui.ctx());
    let ext_id = |i: usize| egui::Id::unique(("settings.open.ext", i));
    if rules.is_empty() {
        ui.label(RichText::new(tr!("settings-open-defaults-empty")).size(11.5).color(p.text_tertiary));
    }
    let mut remove = None;
    for (i, r) in rules.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let mut exts = ui.data(|d| d.get_temp::<String>(ext_id(i))).unwrap_or_else(|| r.extensions.join(" "));
            let resp = text(ui, &mut exts, tr!("settings-open-extensions-hint"), 120.0);
            if resp.changed() {
                r.extensions = exts
                    .split([' ', ',', ';'])
                    .map(soyuz_core::config::OpenSection::norm_key)
                    .filter(|e| !e.is_empty())
                    .collect();
            }
            if resp.has_focus() {
                ui.data_mut(|d| d.insert_temp(ext_id(i), exts));
            } else {
                ui.data_mut(|d| d.remove::<String>(ext_id(i)));
            }
            text(ui, &mut r.name, tr!("settings-name"), 110.0);
            let mut program = r.program.display().to_string();
            let w = (ui.available_width() - 40.0).max(120.0);
            if code(ui, &mut program, crate::models::requests::open_with::program_hint(), w).changed() {
                r.program = program.into();
            }
            if small_button(ui, Some(ph::TRASH), "", true).on_hover_text(tr!("settings-open-remove")).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        rules.remove(i);
        ui.data_mut(|d| (i..=rules.len()).for_each(|j| d.remove::<String>(ext_id(j))));
    }
    ui.add_space(4.0);
    if small_button(ui, Some(ph::PLUS), tr!("settings-open-add"), false).clicked() {
        rules.push(soyuz_core::config::DefaultApp::default());
    }
}

/// Dragging items out to other apps works here (OLE on Windows, XDND on X11, a
/// `wl_data_source` on Wayland once the drag-and-drop thread is up).
fn drag_out_supported() -> bool {
    crate::ui::dnd::dragout::supported()
}

pub fn jobs(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.gui.settings.draft;
    let ctx = "Jobs file operations copy move archive extract";
    let mut any = false;
    any |= group(ui, q, ctx, tr!("settings-jobs-queue"), |r| {
        r.row_kw(tr!("settings-jobs-parallel"), tr!("settings-jobs-parallel-desc"), "parallelism threads", |ui| {
            number(ui, &mut d.jobs.max_parallel, 1..=16, "");
        });
        r.row_kw(tr!("settings-jobs-queue-mode"), tr!("settings-jobs-queue-mode-desc"), "serial", |ui| {
            segmented(
                ui,
                &mut d.jobs.per_device_serial,
                &[(true, tr!("settings-jobs-per-device")), (false, tr!("settings-jobs-parallel-mode"))],
            );
        });
        r.row(tr!("settings-jobs-auto-activity"), tr!("settings-jobs-auto-activity-desc"), |ui| {
            switch(ui, &mut d.jobs.auto_open_activity);
        });
        r.row_kw(tr!("settings-jobs-io-stats"), tr!("settings-jobs-io-stats-desc"), "slow speed throughput", |ui| {
            switch(ui, &mut d.jobs.io_stats);
        });
        r.row_kw(tr!("settings-jobs-reset-apply-all"), tr!("settings-jobs-reset-apply-all-desc"), "conflict", |ui| {
            switch(ui, &mut d.jobs.reset_apply_all_between_jobs);
        });
        r.row(tr!("settings-jobs-history"), tr!("settings-jobs-history-desc"), |ui| {
            number(ui, &mut d.jobs.history_limit, 10..=5000, "");
        });
    });
    any |= group(ui, q, ctx, tr!("settings-jobs-copying"), |r| {
        r.row_kw(tr!("settings-jobs-verify"), tr!("settings-jobs-verify-desc"), "checksum integrity", |ui| {
            switch(ui, &mut d.jobs.verify_after_copy);
        });
        r.row_kw(tr!("settings-jobs-reflink"), tr!("settings-jobs-reflink-desc"), "reflink block clone", |ui| {
            segmented(
                ui,
                &mut d.jobs.reflink,
                &[
                    (ReflinkPref::Auto, tr!("settings-auto")),
                    (ReflinkPref::Never, tr!("settings-never")),
                    (ReflinkPref::Always, tr!("settings-always")),
                ],
            );
        });
        r.row(tr!("settings-jobs-buffer"), "", |ui| {
            number(ui, &mut d.jobs.copy_buffer_mib, 1..=128, " MiB");
        });
        r.row_kw(tr!("settings-jobs-unbuffered"), tr!("settings-jobs-unbuffered-desc"), "direct", |ui| {
            number(ui, &mut d.jobs.unbuffered_threshold_mib, 16..=4096, " MiB");
        });
        r.block(tr!("settings-jobs-preserve"), tr!("settings-jobs-preserve-desc"), |ui| {
            ui.horizontal_wrapped(|ui| {
                for (item, label) in [
                    (PreserveItem::Times, tr!("settings-jobs-preserve-times")),
                    (PreserveItem::Attrs, tr!("settings-jobs-preserve-attrs")),
                    (PreserveItem::Xattr, tr!("settings-jobs-preserve-xattr")),
                    (PreserveItem::Acl, tr!("settings-jobs-preserve-acl")),
                    (PreserveItem::Owner, tr!("settings-jobs-preserve-owner")),
                    (PreserveItem::Sparse, tr!("settings-jobs-preserve-sparse")),
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
    any |= group(ui, q, ctx, tr!("settings-jobs-archive"), |r| {
        r.row_kw(
            tr!("settings-jobs-bomb-ratio"),
            tr!("settings-jobs-bomb-ratio-desc"),
            "decompression bomb zip ratio extract",
            |ui| {
                number(ui, &mut d.archive.bomb_ratio, 0..=1000, "×");
            },
        );
        r.row_kw(tr!("settings-jobs-bomb-floor"), tr!("settings-jobs-bomb-floor-desc"), "bomb floor", |ui| {
            number(ui, &mut d.archive.bomb_floor_mib, 16..=65_536, " MiB");
        });
        r.row_kw(tr!("settings-jobs-max-entries"), tr!("settings-jobs-max-entries-desc"), "bomb files count", |ui| {
            number(ui, &mut d.archive.max_entries, 0..=10_000_000, "");
        });
        r.row_kw(tr!("settings-jobs-max-entry"), tr!("settings-jobs-max-entry-desc"), "bomb size", |ui| {
            number(ui, &mut d.archive.max_entry_mib, 0..=1_048_576, " MiB");
        });
        r.row_kw(
            tr!("settings-jobs-zip-level"),
            tr!("settings-jobs-zip-level-desc"),
            "edit zip add level compress",
            |ui| {
                number(ui, &mut d.archive.edit_level, 0..=9, "");
            },
        );
        r.row_kw(
            tr!("settings-jobs-special-bits"),
            tr!("settings-jobs-special-bits-desc"),
            "permissions mode suid",
            |ui| {
                switch(ui, &mut d.archive.preserve_special_bits);
            },
        );
    });
    any |= group(ui, q, ctx, tr!("settings-views-section-trash"), |r| {
        r.row_kw(
            tr!("settings-jobs-trash-unsupported"),
            tr!("settings-jobs-trash-unsupported-desc"),
            "recycle",
            |ui| {
                segmented(
                    ui,
                    &mut d.jobs.trash_unsupported,
                    &[
                        (TrashUnsupportedPolicy::Ask, tr!("settings-ask")),
                        (TrashUnsupportedPolicy::Permanent, tr!("settings-jobs-trash-delete")),
                        (TrashUnsupportedPolicy::Skip, tr!("settings-skip")),
                    ],
                );
            },
        );
    });
    any |= group(
        ui,
        q,
        "Jobs sync schedule backup snapshot shadow copy previous versions",
        tr!("settings-jobs-sync"),
        |r| {
            r.row_kw(
                tr!("settings-jobs-scheduler"),
                tr!("settings-jobs-scheduler-desc"),
                "scheduler automatic",
                |ui| {
                    switch(ui, &mut d.sync.scheduler);
                },
            );
            r.row_kw(tr!("settings-jobs-dry-run"), tr!("settings-jobs-dry-run-desc"), "preview report", |ui| {
                switch(ui, &mut d.sync.scheduled_dry_run);
            });
            r.row_kw(
                tr!("settings-jobs-change-delay"),
                tr!("settings-jobs-change-delay-desc"),
                "debounce watch",
                |ui| {
                    number(ui, &mut d.sync.change_debounce_secs, 1..=600, " s");
                },
            );
            r.row_kw(tr!("settings-jobs-reports"), tr!("settings-jobs-reports-desc"), "history log", |ui| {
                number(ui, &mut d.sync.reports_kept, 1..=200, "");
            });
            if crate::ui::tools::versions::supported() {
                r.row_kw(
                    tr!("settings-jobs-previous-versions"),
                    if cfg!(windows) {
                        tr!("settings-jobs-previous-versions-desc")
                    } else {
                        tr!("settings-jobs-previous-versions-desc-linux")
                    },
                    "vss snapshot restore snapper zfs timeshift btrfs",
                    |ui| {
                        switch(ui, &mut d.sync.previous_versions);
                    },
                );
            }
            if cfg!(windows) {
                r.row_kw(
                    tr!("settings-jobs-shadow-probe"),
                    tr!("settings-jobs-shadow-probe-desc"),
                    "vss globalroot",
                    |ui| {
                        number(ui, &mut d.sync.shadow_probe_max, 1..=512, "");
                    },
                );
            }
        },
    );
    any |= group(ui, q, "Remote network sftp ssh webdav ftp s3 server cloud", tr!("settings-remote"), |r| {
        r.row_kw(tr!("settings-remote-cache"), tr!("settings-remote-cache-desc"), "cache ttl", |ui| {
            number(ui, &mut d.remote.cache_ttl_secs, 0..=3600, " s");
        });
        r.row_kw(tr!("settings-remote-poll"), tr!("settings-remote-poll-desc"), "poll watch", |ui| {
            number(ui, &mut d.remote.poll_secs, 0..=3600, " s");
        });
        r.row_kw(
            tr!("settings-remote-connect-timeout"),
            tr!("settings-remote-connect-timeout-desc"),
            "connect",
            |ui| {
                number(ui, &mut d.remote.connect_timeout_secs, 1..=300, " s");
            },
        );
        r.row_kw(tr!("settings-remote-io-timeout"), tr!("settings-remote-io-timeout-desc"), "io stall", |ui| {
            number(ui, &mut d.remote.io_timeout_secs, 5..=3600, " s");
        });
        r.row_kw(
            tr!("settings-remote-download-cache"),
            tr!("settings-remote-download-cache-desc"),
            "preview temp",
            |ui| {
                number(ui, &mut d.remote.preview_cache_mb, 16..=65_536, " MB");
            },
        );
        r.row_kw(tr!("settings-remote-preview-max"), tr!("settings-remote-preview-max-desc"), "preview", |ui| {
            number(ui, &mut d.remote.preview_max_mb, 1..=4096, " MB");
        });
        // `\\server` share listing: Windows NetShareEnum; elsewhere srvsvc over SMB.
        let platform = if cfg!(windows) { "windows" } else { "other" };
        r.row_kw(
            tr!("settings-remote-shares"),
            &tr!("settings-remote-shares-desc", platform = platform),
            "smb cifs samba unc netshareenum",
            |ui| {
                switch(ui, &mut d.remote.share_enumeration);
            },
        );
        r.row_kw(
            tr!("settings-remote-discover"),
            tr!("settings-remote-discover-desc"),
            "mdns bonjour zeroconf lan",
            |ui| {
                switch(ui, &mut d.remote.discover);
            },
        );
        r.row_kw(
            tr!("settings-remote-share-minutes"),
            tr!("settings-remote-share-minutes-desc"),
            "quick share qr",
            |ui| {
                number(ui, &mut d.remote.share_minutes, 0..=1440, " min");
            },
        );
        r.row_kw(
            tr!("settings-remote-share-port"),
            tr!("settings-remote-share-port-desc"),
            "quick share firewall",
            |ui| {
                number(ui, &mut d.remote.share_port, 0..=65535, "");
            },
        );
        let hosts = soyuz_vfs::registry::known_hosts().entries();
        r.row_kw(
            tr!("settings-remote-trusted"),
            &tr!("settings-remote-hosts-desc", count = hosts.len()),
            "known_hosts fingerprint host key",
            |ui| {
                ui.vertical(|ui| {
                    for (host, algo, fp) in &hosts {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(host).size(12.0)).on_hover_text(format!(
                                "{algo}
{fp}"
                            ));
                            if small_button(ui, None, tr!("settings-remote-forget"), true).clicked() {
                                let (h, port) = match host.rsplit_once(':') {
                                    Some((h, p)) => (h.to_string(), p.parse().unwrap_or(22)),
                                    None => (host.clone(), 22),
                                };
                                let _ = soyuz_vfs::registry::known_hosts().forget(&h, port);
                            }
                        });
                    }
                });
            },
        );
    });
    any
}

pub fn terminal(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.gui.settings.draft;
    let ctx = "Terminal shell";
    let mut any = false;
    any |= group(ui, q, ctx, tr!("settings-terminal-shell"), |r| {
        r.row_kw(tr!("settings-terminal-shell"), tr!("settings-terminal-shell-desc"), "pwsh bash zsh cmd", |ui| {
            shell_choice(ui, &mut d.terminal.shell, &mut d.terminal.args);
        });
        r.row(tr!("settings-terminal-args"), tr!("settings-terminal-args-desc"), |ui| {
            let mut s = crate::commands::join_args(&d.terminal.args);
            if code(ui, &mut s, "-NoLogo", 220.0).changed() {
                d.terminal.args = crate::commands::split_args(&s);
            }
        });
        r.row_kw(tr!("settings-terminal-integration"), tr!("settings-terminal-integration-desc"), "cwd prompt", |ui| {
            switch(ui, &mut d.terminal.shell_integration);
        });
        r.row(tr!("settings-terminal-scrollback"), tr!("settings-terminal-scrollback-desc"), |ui| {
            number(ui, &mut d.terminal.scrollback, 1000..=100_000, &format!(" {}", tr!("settings-unit-lines")));
        });
    });
    any |= group(ui, q, ctx, tr!("settings-terminal-look"), |r| {
        r.row_kw(tr!("settings-terminal-font-size"), "", "terminal text", |ui| {
            number(ui, &mut d.terminal.font_size, 8.0..=24.0, " pt");
        });
        r.row_kw(tr!("settings-terminal-colors"), tr!("settings-terminal-colors-desc"), "theme", |ui| {
            let mut v = match d.terminal.theme.as_str() {
                "dark" => 1,
                "light" => 2,
                _ => 0,
            };
            if segmented(
                ui,
                &mut v,
                &[(0, tr!("settings-auto")), (1, tr!("settings-dark")), (2, tr!("settings-light"))],
            ) {
                d.terminal.theme = ["auto", "dark", "light"][v].to_string();
            }
        });
    });
    any |= group(ui, q, ctx, tr!("settings-terminal-following"), |r| {
        r.row_kw(tr!("settings-terminal-follow-nav"), tr!("settings-terminal-follow-nav-desc"), "sync", |ui| {
            switch(ui, &mut d.terminal.follow_navigation);
        });
        r.row_kw(
            tr!("settings-terminal-follow-shell"),
            tr!("settings-terminal-follow-shell-desc"),
            "reverse sync",
            |ui| {
                switch(ui, &mut d.terminal.follow_shell);
            },
        );
        r.row(tr!("settings-terminal-cd-tab"), "", |ui| {
            switch(ui, &mut d.terminal.cd_on_tab_switch);
        });
        r.row(tr!("settings-terminal-per-tab"), tr!("settings-terminal-per-tab-desc"), |ui| {
            switch(ui, &mut d.terminal.per_tab);
        });
        r.row_kw(tr!("settings-terminal-esc"), tr!("settings-terminal-esc-desc"), "escape", |ui| {
            switch(ui, &mut d.terminal.escape_returns_focus);
        });
    });
    any |= group(ui, q, ctx, tr!("settings-terminal-security"), |r| {
        r.row_kw(tr!("settings-terminal-osc52"), tr!("settings-terminal-osc52-desc"), "osc52 clipboard paste", |ui| {
            segmented(
                ui,
                &mut d.terminal.osc52,
                &[
                    (Osc52Policy::Ask, tr!("settings-ask")),
                    (Osc52Policy::Allow, tr!("settings-allow")),
                    (Osc52Policy::Deny, tr!("settings-deny")),
                ],
            );
        });
    });
    any
}

/// `[tui]`: the terminal UI started with `soyuz --tui`.
pub fn terminal_ui(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    use soyuz_core::config::{
        KeyboardProtocol, TuiBorders, TuiColorMode, TuiIcons, TuiImages, TuiOsc52, TuiSection, TuiShellMode,
    };
    let d = &mut app.gui.settings.draft;
    group(ui, q, "Terminal UI tui text mode console", tr!("settings-tui-input"), |r| {
        r.row_kw(
            tr!("settings-tui-mouse"),
            tr!("settings-tui-mouse-desc"),
            "mouse click wheel scroll selection",
            |ui| {
                switch(ui, &mut d.tui.mouse);
            },
        );
        r.row_kw(
            tr!("settings-tui-keyboard-protocol"),
            tr!("settings-tui-keyboard-protocol-desc"),
            "kitty keyboard protocol legacy keys modifiers",
            |ui| {
                dropdown(
                    ui,
                    "tui.keyboard_protocol",
                    &mut d.tui.keyboard_protocol,
                    &[
                        (KeyboardProtocol::Auto, tr!("settings-auto")),
                        (KeyboardProtocol::Kitty, "Kitty"),
                        (KeyboardProtocol::Legacy, tr!("settings-tui-legacy")),
                    ],
                    160.0,
                );
            },
        );
    }) | group(ui, q, "Terminal UI tui text mode console", tr!("settings-tui-appearance"), |r| {
        r.row_kw(
            tr!("settings-tui-color-mode"),
            tr!("settings-tui-color-mode-desc"),
            "color colour mode truecolor 24-bit 256 16 ansi mono monochrome palette",
            |ui| {
                dropdown(
                    ui,
                    "tui.color_mode",
                    &mut d.tui.color_mode,
                    &[
                        (TuiColorMode::Auto, tr!("settings-auto")),
                        (TuiColorMode::Truecolor, tr!("settings-tui-truecolor")),
                        (TuiColorMode::Ansi256, tr!("settings-tui-ansi256")),
                        (TuiColorMode::Ansi16, tr!("settings-tui-ansi16")),
                        (TuiColorMode::Mono, tr!("settings-tui-mono")),
                    ],
                    180.0,
                );
            },
        );
        r.row_kw(
            tr!("settings-tui-icons"),
            tr!("settings-tui-icons-desc"),
            "icons glyphs nerd font unicode ascii symbols",
            |ui| {
                dropdown(
                    ui,
                    "tui.icons",
                    &mut d.tui.icons,
                    &[
                        (TuiIcons::Nerd, tr!("settings-tui-icons-nerd")),
                        (TuiIcons::Unicode, tr!("settings-tui-icons-unicode")),
                        (TuiIcons::Ascii, tr!("settings-tui-icons-ascii")),
                    ],
                    180.0,
                );
            },
        );
        r.row_kw(
            tr!("settings-tui-borders"),
            tr!("settings-tui-borders-desc"),
            "borders frame lines box drawing rounded plain square ascii none",
            |ui| {
                dropdown(
                    ui,
                    "tui.borders",
                    &mut d.tui.borders,
                    &[
                        (TuiBorders::Rounded, tr!("settings-tui-borders-rounded")),
                        (TuiBorders::Plain, tr!("settings-tui-borders-plain")),
                        (TuiBorders::Ascii, tr!("settings-tui-borders-ascii")),
                        (TuiBorders::None, tr!("settings-none")),
                    ],
                    180.0,
                );
            },
        );
        r.row_kw(
            tr!("settings-tui-sidebar-width"),
            tr!("settings-tui-sidebar-width-desc"),
            "sidebar width columns panel size",
            |ui| {
                number(
                    ui,
                    &mut d.tui.sidebar_width,
                    TuiSection::SIDEBAR_WIDTH_MIN..=TuiSection::SIDEBAR_WIDTH_MAX,
                    &format!(" {}", tr!("settings-unit-columns")),
                );
            },
        );
        r.row_kw(
            tr!("settings-tui-jobs-rows"),
            tr!("settings-tui-jobs-rows-desc"),
            "jobs activity bottom panel height rows operations progress",
            |ui| {
                number(
                    ui,
                    &mut d.tui.jobs_rows,
                    TuiSection::JOBS_ROWS_MIN..=TuiSection::JOBS_ROWS_MAX,
                    &format!(" {}", tr!("settings-unit-rows")),
                );
            },
        );
        r.row_kw(
            tr!("settings-tui-preview-ratio"),
            tr!("settings-tui-preview-ratio-desc"),
            "preview pane width ratio share size split",
            |ui| {
                number(ui, &mut d.tui.preview_ratio, TuiSection::PREVIEW_RATIO_MIN..=TuiSection::PREVIEW_RATIO_MAX, "");
            },
        );
        r.row_kw(
            tr!("settings-tui-terminal-rows"),
            tr!("settings-tui-terminal-rows-desc"),
            "terminal shell console pane height rows bottom panel",
            |ui| {
                number(
                    ui,
                    &mut d.tui.terminal_rows,
                    TuiSection::TERMINAL_ROWS_MIN..=TuiSection::TERMINAL_ROWS_MAX,
                    &format!(" {}", tr!("settings-unit-rows")),
                );
            },
        );
        r.row_kw(
            tr!("settings-tui-ai-ratio"),
            tr!("settings-tui-ai-ratio-desc"),
            "ai assistant panel width ratio share size split",
            |ui| {
                number(ui, &mut d.tui.ai_ratio, TuiSection::AI_RATIO_MIN..=TuiSection::AI_RATIO_MAX, "");
            },
        );
    }) | group(ui, q, "Terminal UI tui text mode console", tr!("settings-tui-images-group"), |r| {
        r.row_kw(
            tr!("settings-tui-images"),
            tr!("settings-tui-images-desc"),
            "images pictures thumbnails graphics kitty sixel iterm2 half blocks protocol",
            |ui| {
                dropdown(
                    ui,
                    "tui.images",
                    &mut d.tui.images,
                    &[
                        (TuiImages::Auto, tr!("settings-auto")),
                        (TuiImages::Kitty, "Kitty"),
                        (TuiImages::Sixel, "Sixel"),
                        (TuiImages::Iterm2, "iTerm2"),
                        (TuiImages::Halfblocks, tr!("settings-tui-images-halfblocks")),
                        (TuiImages::Off, tr!("settings-off")),
                    ],
                    180.0,
                );
            },
        );
        r.row_kw(
            tr!("settings-tui-cell-pixels"),
            tr!("settings-tui-cell-pixels-desc"),
            "cell size pixels font width height sixel",
            |ui| {
                let name = super::widgets::row_label(ui);
                let range = TuiSection::CELL_PIXELS_MIN..=TuiSection::CELL_PIXELS_MAX;
                let [w, h] = &mut d.tui.cell_pixels;
                let rw = ui.add(egui::DragValue::new(w).range(range.clone()).suffix(" px"));
                super::widgets::named(&rw, &name);
                ui.label("×");
                let rh = ui.add(egui::DragValue::new(h).range(range).suffix(" px"));
                super::widgets::named(&rh, &name);
            },
        );
    }) | group(ui, q, "Terminal UI tui text mode console", tr!("settings-tui-programs"), |r| {
        r.row_kw(
            tr!("settings-tui-shell-mode"),
            tr!("settings-tui-shell-mode-desc"),
            "terminal shell suspend pane embedded console exit",
            |ui| {
                dropdown(
                    ui,
                    "tui.shell_mode",
                    &mut d.tui.shell_mode,
                    &[
                        (TuiShellMode::Pane, tr!("settings-tui-shell-mode-pane")),
                        (TuiShellMode::Suspend, tr!("settings-tui-shell-mode-suspend")),
                    ],
                    180.0,
                );
            },
        );
        r.row_kw(
            tr!("settings-tui-editor"),
            tr!("settings-tui-editor-desc"),
            "editor visual vim nvim nano code config file edit",
            |ui| {
                text(ui, &mut d.tui.editor, "$VISUAL / $EDITOR", 200.0);
            },
        );
    }) | group(ui, q, "Terminal UI tui text mode console", tr!("settings-tui-clipboard"), |r| {
        r.row_kw(
            tr!("settings-tui-osc52"),
            tr!("settings-tui-osc52-desc"),
            "osc 52 osc52 clipboard copy ssh remote",
            |ui| {
                dropdown(
                    ui,
                    "tui.osc52",
                    &mut d.tui.osc52,
                    &[
                        (TuiOsc52::Auto, tr!("settings-auto")),
                        (TuiOsc52::Always, tr!("settings-always")),
                        (TuiOsc52::Never, tr!("settings-never")),
                    ],
                    160.0,
                );
            },
        );
    }) | group(ui, q, "Terminal UI tui text mode console", tr!("settings-tui-accessibility"), |r| {
        r.row_kw(
            tr!("settings-tui-screen-reader"),
            tr!("settings-tui-screen-reader-desc"),
            "screen reader accessibility a11y narrator nvda orca voiceover blind braille",
            |ui| {
                switch(ui, &mut d.tui.screen_reader);
            },
        );
    })
}

/// `[terminal] shell` / `args`: automatic, the detected shells (WSL distributions with
/// their `-d <name>`), or a custom program.
fn shell_choice(ui: &mut Ui, shell: &mut String, args: &mut Vec<String>) {
    use crate::services::shells::{self, ShellKind};
    let c = ui.ctx().clone();
    let found = shells::detected(move || c.request_repaint());
    let list = found.unwrap_or_default();
    let is_auto = shell.trim().is_empty() || shell.trim().eq_ignore_ascii_case("auto");
    let current = shells::matching(list, shell, args);
    // Key: "" = automatic, else the entry's index ("#3"), else the typed program.
    let key = |i: usize| format!("#{i}");
    let mut value = if is_auto {
        String::new()
    } else {
        current.and_then(|cur| list.iter().position(|s| s == cur)).map_or_else(|| shell.clone(), key)
    };
    let mut items = vec![
        dd::Item::new(String::new(), tr!("settings-auto"))
            .secondary(if cfg!(windows) { "pwsh → powershell → cmd" } else { "$SHELL" })
            .icon(ph::MAGIC_WAND),
    ];
    let group = tr!("settings-terminal-detected");
    for (i, s) in list.iter().enumerate() {
        let icon = match s.kind {
            ShellKind::Wsl => ph::LINUX_LOGO,
            ShellKind::Bash => ph::GIT_BRANCH,
            ShellKind::Cmd => ph::TERMINAL,
            _ => ph::TERMINAL_WINDOW,
        };
        items.push(dd::Item::new(key(i), s.label.clone()).secondary(s.path.clone()).icon(icon).group(group));
    }
    if found.is_none() {
        items.push(
            dd::Item::new("?".to_string(), tr!("settings-terminal-detecting"))
                .group(group)
                .disabled(Some(tr!("settings-terminal-detecting").to_string())),
        );
    }
    let before = value.clone();
    if text_choice(ui, "terminal_shell", &mut value, &items, 200.0, r"pwsh, C:\…\shell.exe") || value != before {
        // Arguments that belonged to the previous entry (WSL "-d …", Git Bash login)
        // go with it.
        if current.is_some_and(|c| !c.args.is_empty() && c.args == *args) {
            args.clear();
        }
        if value.is_empty() {
            *shell = "auto".into();
        } else if let Some(s) = value.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()).and_then(|i| list.get(i))
        {
            shell.clone_from(&s.program);
            if !s.args.is_empty() {
                args.clone_from(&s.args);
            }
        } else {
            *shell = value;
        }
    }
}

/// `[preview] syntax_theme`: automatic or one of syntect's themes (with their colors).
fn syntax_theme_choice(ui: &mut Ui, theme: &mut String) {
    let c = ui.ctx().clone();
    let list = crate::services::preview_text::syntax_theme_list(move || c.request_repaint());
    let mut items = vec![
        dd::Item::new("auto".to_string(), tr!("settings-auto"))
            .secondary(tr!("settings-preview-syntax-auto"))
            .icon(ph::CIRCLE_HALF),
    ];
    match &list {
        Some(l) => {
            for (name, bg, fg) in l {
                let (bg, fg) = (crate::ui::gfx::color32(*bg), crate::ui::gfx::color32(*fg));
                items.push(dd::Item::new(name.clone(), name.clone()).swatch(bg).swatch(fg));
            }
        }
        None => items.push(dd::Item::new(theme.clone(), theme.clone())),
    }
    let mut v = if theme.trim().is_empty() { "auto".to_string() } else { theme.clone() };
    if choice(ui, "syntax_theme", &mut v, &items, 200.0) {
        *theme = v;
    }
}

pub fn preview(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.gui.settings.draft;
    let ctx = "Preview search git";
    let mut any = false;
    any |= group(ui, q, ctx, tr!("settings-preview"), |r| {
        r.row_kw(tr!("settings-preview-text-limit"), tr!("settings-preview-text-limit-desc"), "max size", |ui| {
            number(ui, &mut d.preview.max_text_kib, 16..=16_384, " KiB");
        });
        r.row_kw(tr!("settings-preview-image-limit"), tr!("settings-preview-image-limit-desc"), "megapixels", |ui| {
            number(ui, &mut d.preview.max_image_mp, 4..=400, " MP");
        });
        r.row_kw(tr!("settings-preview-syntax"), tr!("settings-preview-syntax-desc"), "highlight code", |ui| {
            syntax_theme_choice(ui, &mut d.preview.syntax_theme);
        });
        let pdf_desc =
            if cfg!(windows) { tr!("settings-preview-pdf-desc") } else { tr!("settings-preview-pdf-desc-unix") };
        r.row_kw(tr!("settings-preview-pdf"), pdf_desc, "pdfium document", |ui| {
            switch(ui, &mut d.preview.pdf);
        });
        let (pdfium_desc, pdfium_hint) = if cfg!(windows) {
            (tr!("settings-preview-pdfium-desc"), tr!("settings-preview-pdfium-hint"))
        } else {
            (tr!("settings-preview-pdfium-desc-unix"), tr!("settings-preview-pdfium-hint-unix"))
        };
        r.row_kw(tr!("settings-preview-pdfium"), pdfium_desc, "pdf dll so library", |ui| {
            text(ui, &mut d.preview.pdfium_dir, pdfium_hint, 220.0);
        });
        r.row_kw(tr!("settings-preview-office"), tr!("settings-preview-office-desc"), "docx xlsx pptx odt ods", |ui| {
            switch(ui, &mut d.preview.office);
        });
        r.row_kw(tr!("settings-preview-sheet-rows"), tr!("settings-preview-sheet-rows-desc"), "xlsx ods table", |ui| {
            number(ui, &mut d.preview.sheet_rows, 10..=100_000, &format!(" {}", tr!("settings-unit-rows")));
        });
        r.row_kw(tr!("settings-preview-epub"), tr!("settings-preview-epub-desc"), "epub book", |ui| {
            switch(ui, &mut d.preview.epub);
        });
        r.row_kw(tr!("settings-preview-doc-limit"), tr!("settings-preview-doc-limit-desc"), "max size", |ui| {
            number(ui, &mut d.preview.max_document_mib, 1..=4096, " MiB");
        });
        r.row_kw(tr!("settings-preview-fonts"), tr!("settings-preview-fonts-desc"), "typeface specimen", |ui| {
            switch(ui, &mut d.preview.fonts);
        });
        r.row_kw(tr!("settings-preview-models"), tr!("settings-preview-models-desc"), "stl gltf obj 3d", |ui| {
            switch(ui, &mut d.preview.models);
        });
        r.row_kw(tr!("settings-preview-spin"), tr!("settings-preview-spin-desc"), "turntable rotate", |ui| {
            switch(ui, &mut d.preview.model_turntable);
        });
        r.row_kw(tr!("settings-preview-model-limit"), tr!("settings-preview-model-limit-desc"), "max size", |ui| {
            number(ui, &mut d.preview.max_model_mib, 1..=4096, " MiB");
        });
        r.row_kw(tr!("settings-preview-waveform"), tr!("settings-preview-waveform-desc"), "wave sound music", |ui| {
            switch(ui, &mut d.preview.audio_waveform);
        });
        r.row_kw(tr!("settings-preview-audio-limit"), tr!("settings-preview-audio-limit-desc"), "max size", |ui| {
            number(ui, &mut d.preview.max_audio_mib, 1..=16_384, " MiB");
        });
        r.row_kw(tr!("settings-preview-playback"), tr!("settings-preview-playback-desc"), "play music sound", |ui| {
            switch(ui, &mut d.preview.audio_playback);
        });
        r.row_kw(tr!("settings-preview-video"), tr!("settings-preview-video-desc"), "ffmpeg video poster", |ui| {
            switch(ui, &mut d.preview.video);
        });
        r.row_kw(tr!("settings-preview-diff"), tr!("settings-preview-diff-desc"), "diff compare", |ui| {
            switch(ui, &mut d.preview.diff);
        });
        r.row_kw(tr!("settings-preview-diff-limit"), tr!("settings-preview-diff-limit-desc"), "max size", |ui| {
            number(ui, &mut d.preview.max_diff_kib, 16..=65_536, " KiB");
        });
        r.row_kw(
            tr!("settings-preview-folder-notes"),
            tr!("settings-preview-folder-notes-desc"),
            "readme notes folder markdown",
            |ui| {
                switch(ui, &mut d.preview.folder_notes);
            },
        );
        let notes_on = d.preview.folder_notes;
        r.block(tr!("settings-preview-folder-notes-files"), tr!("settings-preview-folder-notes-files-desc"), |ui| {
            ui.add_enabled_ui(notes_on, |ui| {
                lines_editor(ui, "folder_notes_files", &mut d.preview.folder_notes_files, 3)
            });
        });
        r.row_kw(tr!("settings-preview-thumb-mem"), tr!("settings-preview-thumb-mem-desc"), "cache ram budget", |ui| {
            number(ui, &mut d.preview.thumb_cache_mib, 8..=4096, " MiB");
        });
        r.row_kw(
            tr!("settings-preview-preview-mem"),
            tr!("settings-preview-preview-mem-desc"),
            "cache ram budget",
            |ui| {
                number(ui, &mut d.preview.preview_cache_mib, 0..=4096, " MiB");
            },
        );
    });
    // `[media]`: the FFmpeg libraries (its own module; it needs the whole app for the
    // status and the folder picker).
    any |= super::media::media(ui, app, q);
    let d = &mut app.gui.settings.draft;
    any |= group(ui, q, ctx, tr!("settings-search"), |r| {
        r.row_kw(tr!("settings-search-threads"), tr!("settings-search-threads-desc"), "parallel", |ui| {
            number(ui, &mut d.search.threads, 0..=64, "");
        });
        r.row_kw(tr!("settings-search-gitignore"), tr!("settings-search-gitignore-desc"), "ignore", |ui| {
            switch(ui, &mut d.search.respect_gitignore);
        });
        r.row_kw(tr!("settings-search-content-limit"), tr!("settings-search-content-limit-desc"), "grep", |ui| {
            number(ui, &mut d.search.content_max_mib, 1..=1024, " MiB");
        });
        r.row_kw(tr!("settings-search-document-text"), tr!("settings-search-document-text-desc"), "pdf", |ui| {
            switch(ui, &mut d.search.document_text);
        });
        if !d.search.saved.is_empty() {
            r.block(tr!("settings-search-saved"), "", |ui| {
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
    any |= crate::ui::smart_settings::index_group(ui, q, d);
    any |= group(ui, q, ctx, "Git", |r| {
        r.row_kw(tr!("settings-git-status-column"), tr!("settings-git-status-column-desc"), "vcs", |ui| {
            switch(ui, &mut d.git.status_column);
        });
        r.row(tr!("settings-git-refresh"), "", |ui| {
            number(ui, &mut d.git.refresh_secs, 1..=120, " s");
        });
    });
    any
}

pub fn advanced(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let path = app.core.config_watch.path().to_path_buf();
    let mut edit = false;
    let mut reload = false;
    let mut export = false;
    let mut import = false;
    let mut reset_all = false;
    let mut copy = false;
    let mut any = false;
    any |= group(ui, q, "Config file advanced", "config.toml", |r| {
        r.block("", tr!("settings-advanced-config-desc"), |ui| {
            ui.horizontal(|ui| {
                ui.label(crate::theme::icons::icon(ph::FILE_CODE, 14.0).color(p.accent));
                ui.add(
                    egui::Label::new(RichText::new(path.display().to_string()).monospace().color(p.text_secondary))
                        .truncate(),
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                edit = small_button(ui, Some(ph::PENCIL_SIMPLE), tr!("settings-advanced-edit"), false).clicked();
                reload = small_button(ui, Some(ph::ARROW_CLOCKWISE), tr!("settings-advanced-reload"), false).clicked();
                copy = small_button(ui, Some(ph::CLIPBOARD_TEXT), tr!("settings-advanced-copy"), false).clicked();
            });
        });
    });
    any |= group(ui, q, "Config file advanced", tr!("settings-advanced-transfer"), |r| {
        r.row_kw(tr!("settings-export-title"), tr!("settings-advanced-export-desc"), "backup save", |ui| {
            export = small_button(ui, Some(ph::EXPORT), tr!("settings-advanced-export"), false).clicked();
        });
        r.row_kw(tr!("settings-import-title"), tr!("settings-advanced-import-desc"), "restore load", |ui| {
            import = small_button(ui, Some(ph::DOWNLOAD_SIMPLE), tr!("settings-advanced-import"), false).clicked();
        });
    });
    any |= group(ui, q, "Config file advanced", tr!("settings-advanced-danger"), |r| {
        r.row_kw(
            tr!("settings-advanced-reset-all"),
            tr!("settings-advanced-reset-all-desc"),
            "defaults factory",
            |ui| {
                reset_all =
                    small_button(ui, Some(ph::WARNING), tr!("settings-advanced-reset-all-button"), true).clicked();
            },
        );
    });
    if edit {
        app.core.cmds.push(crate::cmd::Cmd::Action(soyuz_core::config::Action::EditConfigFile));
        super::close(app);
    }
    if reload {
        app.core.cmds.push(crate::cmd::Cmd::Action(soyuz_core::config::Action::ReloadConfig));
    }
    if copy && let Ok(t) = soyuz_core::config::to_pretty_string(&app.core.config) {
        app.core.cmds.push(crate::cmd::Cmd::CopyText(t));
    }
    if export {
        super::start_export(app);
    }
    if import {
        super::start_import(app);
    }
    if reset_all {
        app.gui.settings.draft = soyuz_core::config::Config::default();
        app.core.ui.toasts.push(crate::ui::toasts::Toast::info(tr!("settings-advanced-reset-all-done")));
    }
    any
}

#[cfg(test)]
mod tests {
    /// Every `[tui]` key has a row on the Terminal UI page (no dead settings: a new key
    /// without a GUI row fails here).
    #[test]
    fn tui_page_covers_every_key() {
        let table = toml::Table::try_from(soyuz_core::config::TuiSection::default()).unwrap();
        let src = include_str!("pages.rs");
        for key in table.keys() {
            assert!(src.contains(&format!("d.tui.{key}")), "[tui] {key} has no row on the Terminal UI page");
        }
        assert!(table.len() >= 15);
    }
}

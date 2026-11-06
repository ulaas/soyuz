//! Settings → System monitor: every `[monitor]` key, grouped as Readings, Process table,
//! History, Snapshots and Alerts.

use egui::Ui;
use soyuz_core::config::SnapshotCommands;
use soyuz_core::config::monitor::{CoreView, CpuScale, MonitorPage, ProcColumn, ProcColumnConfig, ProcView, TempUnit};

use super::widgets::{choice, dd, group, number, segmented, small_button, switch, text};
use crate::app::SoyuzApp;
use crate::models::monitor::labels;
use crate::theme::icons::ph;
use crate::theme::palette;

const CTX: &str = "System monitor processes cpu memory gpu disks network sensors history snapshots alerts";

pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.gui.settings.draft;
    let m = &mut d.monitor;
    let mut any = false;
    any |= group(ui, q, CTX, tr!("settings-monitor-readings"), |r| {
        r.row_kw(
            tr!("settings-monitor-interval"),
            tr!("settings-monitor-interval-desc"),
            "refresh rate sampling",
            |ui| {
                number(ui, &mut m.interval_ms, 250..=10_000, " ms");
            },
        );
        r.row_kw(tr!("settings-monitor-page"), tr!("settings-monitor-page-desc"), "start page overview", |ui| {
            let items: Vec<dd::Item<MonitorPage>> = MonitorPage::ALL
                .iter()
                .map(|p| dd::Item::new(*p, labels::page_title(*p)).icon(crate::ui::views::monitor::page_glyph(*p)))
                .collect();
            choice(ui, "monitor.page", &mut m.page, &items, 200.0);
        });
        r.row_kw(
            tr!("settings-monitor-graph-seconds"),
            tr!("settings-monitor-graph-seconds-desc"),
            "graph span chart",
            |ui| {
                number(ui, &mut m.graph_seconds, 30..=3600, " s");
            },
        );
        r.row_kw(tr!("settings-monitor-cpu-scale"), tr!("settings-monitor-cpu-scale-desc"), "percent cores", |ui| {
            segmented(
                ui,
                &mut m.cpu_scale,
                &[
                    (CpuScale::Machine, tr!("monitor-gui-scale-machine")),
                    (CpuScale::PerCore, tr!("monitor-gui-scale-per-core")),
                ],
            );
        });
        r.row_kw(
            tr!("settings-monitor-core-view"),
            tr!("settings-monitor-core-view-desc"),
            "logical cpus heat bars graphs",
            |ui| {
                segmented(
                    ui,
                    &mut m.core_view,
                    &[
                        (CoreView::Graphs, tr!("monitor-gui-core-graphs")),
                        (CoreView::Heat, tr!("monitor-gui-core-heat")),
                        (CoreView::Bars, tr!("monitor-gui-core-bars")),
                    ],
                );
            },
        );
        r.row_kw(tr!("settings-monitor-temperature-unit"), "", "celsius fahrenheit", |ui| {
            segmented(ui, &mut m.temperature_unit, &[(TempUnit::Celsius, "°C"), (TempUnit::Fahrenheit, "°F")]);
        });
        r.row_kw(
            tr!("settings-monitor-virtual-networks"),
            tr!("settings-monitor-virtual-networks-desc"),
            "loopback tunnel bridge interfaces",
            |ui| {
                switch(ui, &mut m.show_virtual_networks);
            },
        );
    });
    any |= group(ui, q, CTX, tr!("settings-monitor-table"), |r| {
        r.row_kw(
            tr!("settings-monitor-view"),
            tr!("settings-monitor-view-desc"),
            "list tree group program user",
            |ui| {
                let items: Vec<dd::Item<ProcView>> =
                    [ProcView::List, ProcView::Tree, ProcView::ByName, ProcView::ByUser]
                        .iter()
                        .map(|v| dd::Item::new(*v, labels::view_title(*v)))
                        .collect();
                choice(ui, "monitor.view", &mut m.view, &items, 180.0);
            },
        );
        r.row_kw(tr!("settings-monitor-sort"), tr!("settings-monitor-sort-desc"), "order column", |ui| {
            let items: Vec<dd::Item<ProcColumn>> = ProcColumn::ALL
                .iter()
                .map(|c| dd::Item::new(*c, labels::column_title(*c)).secondary(labels::column_desc(*c)))
                .collect();
            choice(ui, "monitor.sort_by", &mut m.sort_by, &items, 220.0);
        });
        r.row(tr!("settings-monitor-sort-descending"), tr!("settings-monitor-sort-descending-desc"), |ui| {
            switch(ui, &mut m.sort_descending);
        });
        r.block(tr!("settings-monitor-columns"), tr!("settings-monitor-columns-desc"), |ui| {
            let p = palette(ui.ctx());
            ui.horizontal_wrapped(|ui| {
                for col in ProcColumn::ALL {
                    let mut on = m.columns.iter().any(|c| c.id == col);
                    let r = ui
                        .add_enabled(col != ProcColumn::Name, egui::Checkbox::new(&mut on, labels::column_title(col)));
                    if r.on_hover_text(labels::column_desc(col)).changed() {
                        if on {
                            m.columns.push(ProcColumnConfig { id: col, width: 0.0 });
                        } else {
                            m.columns.retain(|c| c.id != col);
                        }
                    }
                    ui.add_space(6.0);
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("common-reset"), false).clicked() {
                    m.columns = soyuz_core::config::default_proc_columns();
                }
                ui.label(
                    egui::RichText::new(tr!("settings-monitor-columns-note"))
                        .size(crate::theme::font::px(11.0))
                        .color(p.text_tertiary),
                );
            });
        });
        r.row_kw(tr!("settings-monitor-heat"), tr!("settings-monitor-heat-desc"), "tint color load cells", |ui| {
            switch(ui, &mut m.heat_cells);
        });
        r.row_kw(tr!("settings-monitor-icons"), tr!("settings-monitor-icons-desc"), "program icons exe", |ui| {
            switch(ui, &mut m.process_icons);
        });
        r.row_kw(
            tr!("settings-monitor-process-gpu"),
            tr!("settings-monitor-process-gpu-desc"),
            "gpu per process expensive",
            |ui| {
                switch(ui, &mut m.process_gpu);
            },
        );
        r.row_kw(tr!("settings-monitor-kernel"), tr!("settings-monitor-kernel-desc"), "kernel threads system", |ui| {
            switch(ui, &mut m.show_kernel);
        });
        r.row_kw(
            tr!("settings-monitor-other-users"),
            tr!("settings-monitor-other-users-desc"),
            "users processes",
            |ui| {
                switch(ui, &mut m.show_other_users);
            },
        );
        r.row_kw(
            tr!("settings-monitor-follow"),
            tr!("settings-monitor-follow-desc"),
            "follow selection scroll",
            |ui| {
                switch(ui, &mut m.follow_selected);
            },
        );
        r.row_kw(tr!("settings-monitor-details"), tr!("settings-monitor-details-desc"), "details panel", |ui| {
            switch(ui, &mut m.details_panel);
        });
        r.row(tr!("settings-monitor-details-height"), "", |ui| {
            number(ui, &mut m.details_height, 80.0..=900.0, " pt");
        });
        r.row_kw(
            tr!("settings-monitor-confirm-kill"),
            tr!("settings-monitor-confirm-kill-desc"),
            "ask end kill suspend",
            |ui| {
                switch(ui, &mut m.confirm_kill);
            },
        );
        r.row_kw(
            tr!("settings-monitor-remember-filter"),
            tr!("settings-monitor-remember-filter-desc"),
            "filter search remember",
            |ui| {
                switch(ui, &mut m.remember_filter);
            },
        );
        r.row_kw(tr!("settings-monitor-filter"), tr!("settings-monitor-filter-desc"), "query regex", |ui| {
            text(ui, &mut m.filter, tr!("monitor-gui-filter-hint"), 220.0);
        });
        r.row(tr!("settings-monitor-filter-regex"), tr!("settings-monitor-filter-regex-desc"), |ui| {
            switch(ui, &mut m.filter_regex);
        });
        r.row_kw(
            tr!("settings-monitor-keep-parents"),
            tr!("settings-monitor-keep-parents-desc"),
            "tree filter parents",
            |ui| {
                switch(ui, &mut m.filter_keeps_parents);
            },
        );
    });
    any |= group(ui, q, CTX, tr!("settings-monitor-history"), |r| {
        r.row_kw(tr!("settings-monitor-record"), tr!("settings-monitor-record-desc"), "history start stop", |ui| {
            switch(ui, &mut m.record_history);
        });
        r.row_kw(
            tr!("settings-monitor-background"),
            tr!("settings-monitor-background-desc"),
            "record background closed",
            |ui| {
                switch(ui, &mut m.background_history);
            },
        );
        r.row(tr!("settings-monitor-background-interval"), tr!("settings-monitor-background-interval-desc"), |ui| {
            number(ui, &mut m.background_interval_secs, 2..=3600, " s");
        });
        r.row_kw(tr!("settings-monitor-persist"), tr!("settings-monitor-persist-desc"), "disk save restart", |ui| {
            switch(ui, &mut m.history_persist);
        });
        r.row(tr!("settings-monitor-history-days"), tr!("settings-monitor-history-days-desc"), |ui| {
            number(ui, &mut m.history_days, 1..=90, "");
        });
    });
    any |= group(ui, q, CTX, tr!("settings-monitor-snapshots"), |r| {
        r.row(tr!("settings-monitor-snapshots-kept"), tr!("settings-monitor-snapshots-kept-desc"), |ui| {
            number(ui, &mut m.snapshots_kept, 1..=1000, "");
        });
        r.row_kw(
            tr!("settings-monitor-commands"),
            tr!("settings-monitor-commands-desc"),
            "secrets passwords command line redact",
            |ui| {
                segmented(
                    ui,
                    &mut m.snapshot_commands,
                    &[
                        (SnapshotCommands::None, tr!("settings-monitor-commands-none")),
                        (SnapshotCommands::Redacted, tr!("settings-monitor-commands-redacted")),
                        (SnapshotCommands::Full, tr!("settings-monitor-commands-full")),
                    ],
                );
            },
        );
    });
    any |= group(ui, q, CTX, tr!("settings-monitor-alerts"), |r| {
        r.row_kw(
            tr!("settings-monitor-alert-cpu"),
            tr!("settings-monitor-alert-off-hint"),
            "notification threshold",
            |ui| {
                number(ui, &mut m.alert_cpu_percent, 0..=100, " %");
            },
        );
        r.row_kw(
            tr!("settings-monitor-alert-memory"),
            tr!("settings-monitor-alert-off-hint"),
            "notification threshold",
            |ui| {
                number(ui, &mut m.alert_memory_percent, 0..=100, " %");
            },
        );
        r.row_kw(
            tr!("settings-monitor-alert-temperature"),
            tr!("settings-monitor-alert-off-hint"),
            "notification threshold heat",
            |ui| {
                number(ui, &mut m.alert_temperature_c, 0..=130, " °C");
            },
        );
        r.row(tr!("settings-monitor-alert-sustain"), tr!("settings-monitor-alert-sustain-desc"), |ui| {
            number(ui, &mut m.alert_sustain_secs, 1..=3600, " s");
        });
        r.row(tr!("settings-monitor-alert-snapshot"), tr!("settings-monitor-alert-snapshot-desc"), |ui| {
            switch(ui, &mut m.alert_snapshot);
        });
    });
    any
}

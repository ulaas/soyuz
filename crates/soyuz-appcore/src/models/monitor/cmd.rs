//! What the monitor pages can ask for. Frontends push these through `Cmd::Monitor`; each
//! runs here with the host, so a change to `[monitor]` is saved like any other setting.
//! Process control, service control and file writes run on workers; [`pump_outcomes`] turns
//! their results into toasts.

use std::sync::Arc;

use soyuz_core::config::monitor::{CoreView, CpuScale, MonitorPage, ProcColumn, ProcColumnConfig, ProcView};
use soyuz_core::config::{SnapshotCommands, default_proc_columns};
use soyuz_core::{Location, SizeStyle};
use soyuz_sysmon::fetch::Loader;
use soyuz_sysmon::snapshot;
use soyuz_sysmon::{
    ControlError, DiffSort, Frame, Hub, MatchBy, ProcKey, ProcPriority, ProcRisk, ProcSignal, RowId, ServiceOp,
};

use super::snapshots::{CompareTarget, export_dir};
use super::{AffinityEdit, DetailsTab, data_dir, labels};
use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::host::Host;
use crate::models::frontend::FrontendCmd;
use crate::models::settings::update_config;
use crate::models::toasts::Toast;
use crate::ops;
use crate::state::pane::PaneId;

/// Which processes a signal goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalScope {
    /// The marked processes, else the selected one.
    Selection,
    /// The selected process and all its descendants.
    Tree,
}

/// A monitor page's request.
#[derive(Clone, Debug)]
pub enum MonitorCmd {
    SetPage(MonitorPage),
    SetView(ProcView),
    /// Sort by a column (again: other direction).
    SortBy(ProcColumn),
    /// Show or hide a column.
    ToggleColumn(ProcColumn),
    /// Move a column to position `to` among the shown ones.
    MoveColumn(ProcColumn, usize),
    /// Set a column's width in points (0 = the default).
    SetColumnWidth(ProcColumn, f32),
    ResetColumns,
    /// Open the column chooser.
    ChooseColumns,
    SetFilter(String),
    ToggleFilterRegex,
    FocusFilter,
    /// Collapse or expand a row of the tree or a group.
    ToggleCollapsed(RowId),
    /// Expand every row.
    ExpandAll,
    /// Collapse every row with children.
    CollapseAll,
    Select(Option<RowId>),
    ToggleMark(ProcKey),
    ClearMarks,
    ToggleDetails,
    SetDetailsHeight(f32),
    SetDetailsTab(DetailsTab),
    /// End, kill, suspend, resume; asks first when `[monitor] confirm_kill` says so.
    Signal {
        scope: SignalScope,
        sig: ProcSignal,
    },
    /// The confirmed form of [`MonitorCmd::Signal`].
    SignalNow {
        keys: Vec<ProcKey>,
        tree: bool,
        sig: ProcSignal,
    },
    SetPriority(ProcKey, ProcPriority),
    /// Read a process's CPU set, then show the affinity editor.
    EditAffinity(ProcKey),
    /// Tick a CPU in the affinity editor.
    ToggleAffinityCpu(usize),
    ApplyAffinity,
    CancelAffinity,
    /// Open the folder of a program, in a new tab.
    OpenLocation(ProcKey),
    /// Copy the process's command line, path, pid or a summary line.
    CopyProcess(ProcKey, CopyWhat),
    /// Copy a text of a page (an address, a command of a startup item).
    CopyText(String),
    TogglePause,
    SetInterval(u32),
    SetGraphSeconds(u32),
    SetCpuScale(CpuScale),
    SetCoreView(CoreView),
    ToggleKernel,
    ToggleOtherUsers,
    ToggleHeat,
    ToggleVirtualNets,
    /// Export the process table as CSV.
    ExportProcesses,
    // ---- list pages ----
    ConnectionsSort(u8),
    ConnectionsFilter(String),
    ServicesSort(u8),
    ServicesFilter(String),
    StartupSort(u8),
    StartupFilter(String),
    /// Start, stop or restart a service; asks first.
    Service(String, ServiceOp),
    ServiceNow(String, ServiceOp),
    // ---- history ----
    HistoryRange(u32),
    /// Pan back by (positive) or forward by (negative) seconds; live again at the end.
    HistoryPan(i64),
    HistoryLive,
    HistoryCursor(Option<i64>),
    // ---- snapshots ----
    TakeSnapshot(String),
    SnapshotCursor(usize),
    /// Use a snapshot as the left side of the comparison.
    CompareFrom(String),
    /// Use a snapshot or now as the right side.
    CompareWith(CompareTarget),
    RunCompare,
    SetMatchBy(MatchBy),
    DiffSortBy(DiffSort),
    DiffShowSame(bool),
    DiffFilter(String),
    /// Forget the current comparison.
    CloseCompare,
    StartRename(String),
    RenameText(String),
    CommitRename,
    CancelRename,
    /// Delete a snapshot; asks first.
    DeleteSnapshot(String),
    DeleteSnapshotNow(String),
    ExportSnapshot {
        id: String,
        csv: bool,
    },
}

/// [`MonitorCmd::CopyProcess`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyWhat {
    Pid,
    Name,
    Path,
    Command,
}

/// A worker's result.
pub(super) enum Outcome {
    Toast(Toast),
    Control {
        what: String,
        errors: Vec<(String, ControlError)>,
        offer_kill: Vec<ProcKey>,
        done: usize,
    },
    Affinity(PaneId, AffinityEdit),
    /// The snapshot list changed.
    SnapshotsChanged(Option<Toast>),
    ServicesChanged(Toast),
}

fn spawn(app: &mut AppCore, name: &'static str, work: impl FnOnce() -> Outcome + Send + 'static) {
    let wake = app.wake.clone();
    app.ui.monitor.pending.push(Loader::spawn(name, wake, work));
}

/// Results of finished workers.
pub fn pump_outcomes(app: &mut AppCore) {
    let mut finished = Vec::new();
    app.ui.monitor.pending.retain_mut(|l| match l.poll() {
        Some(o) => {
            finished.push(o);
            false
        }
        None => l.is_pending(),
    });
    for o in finished {
        match o {
            Outcome::Toast(t) => {
                app.ui.toasts.push(t);
            }
            Outcome::Control { what, errors, offer_kill, done } => control_toast(app, what, errors, offer_kill, done),
            Outcome::Affinity(pane, edit) => {
                let cfg = app.config.monitor.clone();
                app.ui.monitor.view(pane, &cfg).affinity = Some(edit);
            }
            Outcome::SnapshotsChanged(t) => {
                for v in app.ui.monitor.views.values_mut() {
                    v.snapshots.stale = true;
                }
                if let Some(t) = t {
                    app.ui.toasts.push(t);
                }
            }
            Outcome::ServicesChanged(t) => {
                for v in app.ui.monitor.views.values_mut() {
                    v.services.stale = true;
                }
                app.ui.toasts.push(t);
            }
        }
    }
}

fn control_toast(
    app: &mut AppCore,
    what: String,
    errors: Vec<(String, ControlError)>,
    offer_kill: Vec<ProcKey>,
    done: usize,
) {
    if errors.is_empty() && offer_kill.is_empty() {
        app.ui.toasts.push(Toast::success(tr!("monitor-signal-done", what = what.as_str(), count = done)));
        return;
    }
    if !offer_kill.is_empty() {
        let pane = app.ws.pane().id;
        let t = Toast::warning(tr!("monitor-no-windows", count = offer_kill.len())).cmd(
            tr!("monitor-action-kill"),
            Cmd::Monitor(pane, MonitorCmd::SignalNow { keys: offer_kill, tree: false, sig: ProcSignal::Kill }),
        );
        app.ui.toasts.push(t);
    }
    if !errors.is_empty() {
        let body = errors
            .iter()
            .take(6)
            .map(|(name, e)| format!("{name}: {}", labels::control_error(e)))
            .collect::<Vec<_>>()
            .join("\n");
        app.ui.toasts.push(Toast::error(tr!("monitor-signal-failed", what = what.as_str())).body(body));
    }
}

/// Run one command.
pub fn apply(app: &mut AppCore, pane: PaneId, cmd: MonitorCmd, host: &dyn Host) {
    use MonitorCmd as C;
    let cfg = app.config.monitor.clone();
    let wake = app.wake.clone();
    match cmd {
        C::SetPage(page) => {
            app.ui.monitor.view(pane, &cfg).page = page;
            update_config(app, host, |c| c.monitor.page = page);
        }
        C::SetView(view) => update_config(app, host, |c| c.monitor.view = view),
        C::SortBy(col) => update_config(app, host, |c| {
            if c.monitor.sort_by == col {
                c.monitor.sort_descending = !c.monitor.sort_descending;
            } else {
                c.monitor.sort_by = col;
                c.monitor.sort_descending = col.descending_first();
            }
        }),
        C::ToggleColumn(col) if col != ProcColumn::Name => update_config(app, host, |c| {
            if let Some(i) = c.monitor.columns.iter().position(|x| x.id == col) {
                c.monitor.columns.remove(i);
            } else {
                c.monitor.columns.push(ProcColumnConfig { id: col, width: 0.0 });
            }
        }),
        C::ToggleColumn(_) => {}
        C::MoveColumn(col, to) => update_config(app, host, |c| {
            if let Some(i) = c.monitor.columns.iter().position(|x| x.id == col) {
                let item = c.monitor.columns.remove(i);
                let to = to.min(c.monitor.columns.len());
                c.monitor.columns.insert(to, item);
            }
        }),
        C::SetColumnWidth(col, width) => update_config(app, host, |c| {
            if let Some(x) = c.monitor.columns.iter_mut().find(|x| x.id == col) {
                x.width = width.clamp(0.0, 2000.0);
            }
        }),
        C::ResetColumns => update_config(app, host, |c| c.monitor.columns = default_proc_columns()),
        C::ChooseColumns => app.frontend.push(FrontendCmd::MonitorColumns(pane)),
        C::SetFilter(text) => {
            app.ui.monitor.view(pane, &cfg).set_filter(text.clone());
            if cfg.remember_filter {
                update_config(app, host, |c| c.monitor.filter = text);
            }
        }
        C::ToggleFilterRegex => {
            let v = app.ui.monitor.view(pane, &cfg);
            v.filter_regex = !v.filter_regex;
            let (text, regex) = (v.filter_text.clone(), v.filter_regex);
            v.set_filter(text);
            if cfg.remember_filter {
                update_config(app, host, |c| c.monitor.filter_regex = regex);
            }
        }
        C::FocusFilter => app.ui.monitor.view(pane, &cfg).focus_filter = true,
        C::ToggleCollapsed(id) => {
            let v = app.ui.monitor.view(pane, &cfg);
            if !v.collapsed.remove(&id) {
                v.collapsed.insert(id);
            }
            v.table_changed();
        }
        C::ExpandAll => {
            let v = app.ui.monitor.view(pane, &cfg);
            v.collapsed.clear();
            v.table_changed();
        }
        C::CollapseAll => {
            let frame = app.ui.monitor.frame.clone();
            let generation = app.ui.monitor.generation;
            let v = app.ui.monitor.view(pane, &cfg);
            if let Some(frame) = frame {
                let ids: Vec<RowId> =
                    v.rows(&frame, generation, &cfg).iter().filter(|r| r.has_children).map(|r| r.id(&frame)).collect();
                v.collapsed.extend(ids);
                v.table_changed();
            }
        }
        C::Select(id) => app.ui.monitor.view(pane, &cfg).selected = id,
        C::ToggleMark(key) => {
            let v = app.ui.monitor.view(pane, &cfg);
            if !v.marked.remove(&key) {
                v.marked.insert(key);
            }
        }
        C::ClearMarks => app.ui.monitor.view(pane, &cfg).marked.clear(),
        C::ToggleDetails => update_config(app, host, |c| c.monitor.details_panel = !c.monitor.details_panel),
        C::SetDetailsHeight(h) => update_config(app, host, |c| c.monitor.details_height = h.clamp(80.0, 900.0)),
        C::SetDetailsTab(t) => app.ui.monitor.view(pane, &cfg).details.tab = t,
        C::Signal { scope, sig } => ask_signal(app, pane, scope, sig),
        C::SignalNow { keys, tree, sig } => run_signal(app, keys, tree, sig),
        C::SetPriority(key, p) => {
            let Some(row) = app.ui.monitor.frame.as_ref().and_then(|f| f.proc(key).cloned()) else { return };
            let name = row.raw.name.to_string();
            let what = labels::priority(p).to_string();
            spawn(app, "priority", move || match soyuz_fs::sysmon::set_priority(key, p) {
                Ok(()) => Outcome::Toast(Toast::success(tr!(
                    "monitor-priority-set",
                    name = name.as_str(),
                    what = what.as_str()
                ))),
                Err(e) => Outcome::Toast(
                    Toast::error(tr!("monitor-priority-failed", name = name.as_str())).body(labels::control_error(&e)),
                ),
            });
        }
        C::EditAffinity(key) => {
            let name = app
                .ui
                .monitor
                .frame
                .as_ref()
                .and_then(|f| f.proc(key).map(|p| p.raw.name.to_string()))
                .unwrap_or_default();
            let total = app.ui.monitor.frame.as_ref().map_or(0, |f| f.ncpu() as usize);
            spawn(app, "affinity", move || match soyuz_fs::sysmon::affinity(key) {
                Ok(on) => {
                    let mut cpus = vec![false; total.max(1)];
                    for c in on {
                        if let Some(s) = cpus.get_mut(c as usize) {
                            *s = true;
                        }
                    }
                    Outcome::Affinity(pane, AffinityEdit { key, name, cpus })
                }
                Err(e) => Outcome::Toast(
                    Toast::error(tr!("monitor-affinity-failed", name = name.as_str())).body(labels::control_error(&e)),
                ),
            });
        }
        C::ToggleAffinityCpu(i) => {
            if let Some(e) = app.ui.monitor.view(pane, &cfg).affinity.as_mut()
                && let Some(c) = e.cpus.get_mut(i)
            {
                *c = !*c;
            }
        }
        C::ApplyAffinity => {
            let Some(e) = app.ui.monitor.view(pane, &cfg).affinity.take() else { return };
            let cpus: Vec<u32> = e.cpus.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i as u32).collect();
            let name = e.name.clone();
            if cpus.is_empty() {
                app.ui.toasts.push(Toast::error(tr!("monitor-affinity-empty")));
                return;
            }
            spawn(app, "affinity", move || match soyuz_fs::sysmon::set_affinity(e.key, &cpus) {
                Ok(()) => Outcome::Toast(Toast::success(tr!("monitor-affinity-set", name = name.as_str()))),
                Err(err) => Outcome::Toast(
                    Toast::error(tr!("monitor-affinity-failed", name = name.as_str()))
                        .body(labels::control_error(&err)),
                ),
            });
        }
        C::CancelAffinity => app.ui.monitor.view(pane, &cfg).affinity = None,
        C::OpenLocation(key) => {
            let exe = app.ui.monitor.frame.as_ref().and_then(|f| f.proc(key).and_then(|p| p.raw.exe.clone()));
            match exe {
                Some(exe) => {
                    if let Some(dir) = exe.parent() {
                        ops::open_in_new_tab(app, Location::Dir(dir.to_path_buf()), true);
                        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned());
                        app.ws.pane_mut().pending_cursor = name;
                    }
                }
                None => {
                    app.ui.toasts.push(Toast::info(tr!("monitor-no-path")));
                }
            }
        }
        C::CopyProcess(key, what) => {
            let Some(row) = app.ui.monitor.frame.as_ref().and_then(|f| f.proc(key).cloned()) else { return };
            let text = match what {
                CopyWhat::Pid => Some(key.pid.to_string()),
                CopyWhat::Name => Some(row.raw.name.to_string()),
                CopyWhat::Path => row.raw.exe.as_ref().map(|p| p.display().to_string()),
                CopyWhat::Command => row.raw.cmdline.as_deref().map(str::to_string),
            };
            if let Some(t) = text {
                host.copy_text(t);
            }
        }
        C::CopyText(t) => host.copy_text(t),
        C::TogglePause => {
            let hub = Hub::global();
            hub.pause(!hub.is_paused());
        }
        C::SetInterval(ms) => update_config(app, host, |c| c.monitor.interval_ms = ms),
        C::SetGraphSeconds(s) => update_config(app, host, |c| c.monitor.graph_seconds = s),
        C::SetCpuScale(s) => update_config(app, host, |c| c.monitor.cpu_scale = s),
        C::SetCoreView(s) => update_config(app, host, |c| c.monitor.core_view = s),
        C::ToggleKernel => update_config(app, host, |c| c.monitor.show_kernel = !c.monitor.show_kernel),
        C::ToggleOtherUsers => update_config(app, host, |c| c.monitor.show_other_users = !c.monitor.show_other_users),
        C::ToggleHeat => update_config(app, host, |c| c.monitor.heat_cells = !c.monitor.heat_cells),
        C::ToggleVirtualNets => {
            update_config(app, host, |c| c.monitor.show_virtual_networks = !c.monitor.show_virtual_networks)
        }
        C::ExportProcesses => export_processes(app, pane),
        C::ConnectionsSort(c) => app.ui.monitor.view(pane, &cfg).connections.sort_by(c),
        C::ConnectionsFilter(t) => app.ui.monitor.view(pane, &cfg).connections.set_filter(t),
        C::ServicesSort(c) => app.ui.monitor.view(pane, &cfg).services.sort_by(c),
        C::ServicesFilter(t) => app.ui.monitor.view(pane, &cfg).services.set_filter(t),
        C::StartupSort(c) => app.ui.monitor.view(pane, &cfg).startup.sort_by(c),
        C::StartupFilter(t) => app.ui.monitor.view(pane, &cfg).startup.set_filter(t),
        C::Service(name, op) => {
            let verb = labels::service_op(op).to_string();
            app.frontend.push(FrontendCmd::Confirm {
                title: tr!("monitor-service-confirm-title", what = verb.as_str(), name = name.as_str()),
                message: tr!("monitor-service-confirm").to_string(),
                confirm: verb,
                on_confirm: Cmd::Monitor(pane, C::ServiceNow(name, op)),
            });
        }
        C::ServiceNow(name, op) => {
            let verb = labels::service_op(op).to_string();
            spawn(app, "service", move || match soyuz_fs::sysmon::service_control(&name, op) {
                Ok(()) => Outcome::ServicesChanged(Toast::success(tr!(
                    "monitor-service-done",
                    what = verb.as_str(),
                    name = name.as_str()
                ))),
                Err(e) => Outcome::ServicesChanged(
                    Toast::error(tr!("monitor-service-failed", what = verb.as_str(), name = name.as_str()))
                        .body(labels::control_error(&e)),
                ),
            });
        }
        C::HistoryRange(s) => app.ui.monitor.view(pane, &cfg).history.range_secs = s,
        C::HistoryPan(by) => {
            let now = app.ui.monitor.frame.as_ref().map_or(0, |f| f.at_ms);
            let h = &mut app.ui.monitor.view(pane, &cfg).history;
            let end = h.end_ms.unwrap_or(now) - by * 1000;
            h.end_ms = (end < now).then_some(end);
        }
        C::HistoryLive => app.ui.monitor.view(pane, &cfg).history.end_ms = None,
        C::HistoryCursor(t) => {
            let h = &mut app.ui.monitor.view(pane, &cfg).history;
            if h.cursor_ms != t {
                h.cursor_ms = t;
                h.tops = t.and_then(|t| Hub::global().with_history(|hist| hist.tops_near(t)));
            }
        }
        C::TakeSnapshot(label) => {
            let label = if label.trim().is_empty() { labels::default_snapshot_label() } else { label };
            take_snapshot(app, label);
        }
        C::SnapshotCursor(i) => app.ui.monitor.view(pane, &cfg).snapshots.cursor = i,
        C::CompareFrom(id) => app.ui.monitor.view(pane, &cfg).snapshots.a = Some(id),
        C::CompareWith(t) => app.ui.monitor.view(pane, &cfg).snapshots.b = t,
        C::RunCompare => {
            let live = app.ui.monitor.frame.clone();
            app.ui.monitor.view(pane, &cfg).snapshots.start_compare(live, cfg.cpu_scale, &wake);
        }
        C::SetMatchBy(m) => {
            let live = app.ui.monitor.frame.clone();
            let s = &mut app.ui.monitor.view(pane, &cfg).snapshots;
            s.match_by = m;
            if s.comparison.is_some() {
                s.start_compare(live, cfg.cpu_scale, &wake);
            }
        }
        C::DiffSortBy(sort) => {
            let s = &mut app.ui.monitor.view(pane, &cfg).snapshots;
            if s.sort == sort {
                s.descending = !s.descending;
            } else {
                s.sort = sort;
                s.descending = !matches!(sort, DiffSort::Name | DiffSort::User | DiffSort::Change);
            }
            s.reshow();
        }
        C::DiffShowSame(on) => {
            let s = &mut app.ui.monitor.view(pane, &cfg).snapshots;
            s.show_same = on;
            s.reshow();
        }
        C::DiffFilter(t) => {
            let s = &mut app.ui.monitor.view(pane, &cfg).snapshots;
            s.filter = t;
            s.reshow();
        }
        C::CloseCompare => {
            app.ui.monitor.view(pane, &cfg).snapshots.close_compare();
        }
        C::StartRename(id) => {
            let s = &mut app.ui.monitor.view(pane, &cfg).snapshots;
            let label = s.label_of(&id).unwrap_or("").to_string();
            s.renaming = Some((id, label));
        }
        C::RenameText(t) => {
            if let Some((_, text)) = app.ui.monitor.view(pane, &cfg).snapshots.renaming.as_mut() {
                *text = t;
            }
        }
        C::CommitRename => {
            let Some((id, label)) = app.ui.monitor.view(pane, &cfg).snapshots.renaming.take() else { return };
            spawn(app, "snapshot-rename", move || match snapshot::rename(&data_dir(), &id, &label) {
                Ok(_) => Outcome::SnapshotsChanged(None),
                Err(e) => {
                    Outcome::SnapshotsChanged(Some(Toast::error(tr!("monitor-snapshot-failed")).body(e.to_string())))
                }
            });
        }
        C::CancelRename => app.ui.monitor.view(pane, &cfg).snapshots.renaming = None,
        C::DeleteSnapshot(id) => {
            let label = app.ui.monitor.view(pane, &cfg).snapshots.label_of(&id).unwrap_or(&id).to_string();
            app.frontend.push(FrontendCmd::Confirm {
                title: tr!("monitor-snapshot-delete-title").to_string(),
                message: tr!("monitor-snapshot-delete", label = label.as_str()),
                confirm: tr!("common-delete").to_string(),
                on_confirm: Cmd::Monitor(pane, C::DeleteSnapshotNow(id)),
            });
        }
        C::DeleteSnapshotNow(id) => {
            let s = &mut app.ui.monitor.view(pane, &cfg).snapshots;
            if s.a.as_deref() == Some(id.as_str()) {
                s.a = None;
            }
            if s.b == CompareTarget::Snapshot(id.clone()) {
                s.b = CompareTarget::Live;
            }
            spawn(app, "snapshot-delete", move || match snapshot::delete(&data_dir(), &id) {
                Ok(()) => Outcome::SnapshotsChanged(None),
                Err(e) => {
                    Outcome::SnapshotsChanged(Some(Toast::error(tr!("monitor-snapshot-failed")).body(e.to_string())))
                }
            });
        }
        C::ExportSnapshot { id, csv } => export_snapshot(app, id, csv, cfg.snapshot_commands),
    }
}

fn ask_signal(app: &mut AppCore, pane: PaneId, scope: SignalScope, sig: ProcSignal) {
    let cfg = app.config.monitor.clone();
    let Some(frame) = app.ui.monitor.frame.clone() else { return };
    let v = app.ui.monitor.view(pane, &cfg);
    let selected = match v.selected {
        Some(RowId::Proc(k)) => Some(k),
        _ => None,
    };
    let (keys, tree) = match scope {
        SignalScope::Tree => (selected.into_iter().collect::<Vec<_>>(), true),
        SignalScope::Selection if !v.marked.is_empty() => (v.marked.iter().copied().collect(), false),
        SignalScope::Selection => (selected.into_iter().collect(), false),
    };
    let me = v.me().map(str::to_string);
    if keys.is_empty() {
        return;
    }
    let rows: Vec<_> = keys.iter().filter_map(|k| frame.proc(*k)).collect();
    let mut risk = ProcRisk::Normal;
    for r in &rows {
        match soyuz_sysmon::risk(&r.raw, me.as_deref()) {
            ProcRisk::Critical => risk = ProcRisk::Critical,
            ProcRisk::OtherUser if risk == ProcRisk::Normal => risk = ProcRisk::OtherUser,
            _ => {}
        }
    }
    let now = Cmd::Monitor(pane, MonitorCmd::SignalNow { keys: keys.clone(), tree, sig });
    let harmless = matches!(sig, ProcSignal::Resume);
    if (!cfg.confirm_kill || harmless) && risk == ProcRisk::Normal {
        app.cmds.push(now);
        return;
    }
    let names = rows.iter().take(5).map(|r| r.raw.name.to_string()).collect::<Vec<_>>().join(", ");
    let verb = labels::signal(sig).to_string();
    let title = tr!("monitor-signal-confirm-title", what = verb.as_str(), count = keys.len());
    let mut message = if tree {
        tr!("monitor-signal-confirm-tree", names = names.as_str())
    } else {
        tr!("monitor-signal-confirm", names = names.as_str())
    };
    match risk {
        ProcRisk::Critical => {
            message.push_str("\n\n");
            message.push_str(tr!("monitor-risk-critical"));
        }
        ProcRisk::OtherUser => {
            message.push_str("\n\n");
            message.push_str(tr!("monitor-risk-other-user"));
        }
        ProcRisk::Normal => {}
    }
    app.frontend.push(FrontendCmd::Confirm { title, message, confirm: verb, on_confirm: now });
}

fn run_signal(app: &mut AppCore, keys: Vec<ProcKey>, tree: bool, sig: ProcSignal) {
    let Some(frame) = app.ui.monitor.frame.clone() else { return };
    let what = labels::signal(sig).to_string();
    let was_terminate = sig == ProcSignal::Terminate;
    spawn(app, "signal", move || run_signal_worker(frame, keys, tree, sig, what, was_terminate));
}

fn run_signal_worker(
    frame: Arc<Frame>,
    keys: Vec<ProcKey>,
    tree: bool,
    sig: ProcSignal,
    what: String,
    was_terminate: bool,
) -> Outcome {
    let mut errors = Vec::new();
    let mut offer_kill = Vec::new();
    let mut done = 0;
    for key in keys {
        let name = frame.proc(key).map_or_else(|| key.pid.to_string(), |p| p.raw.name.to_string());
        if tree {
            let report = soyuz_sysmon::signal_tree(&frame, key, sig);
            done += report.done.len();
            errors.extend(report.failed.into_iter().map(|(k, e)| {
                let n = frame.proc(k).map_or_else(|| k.pid.to_string(), |p| p.raw.name.to_string());
                (n, e)
            }));
            continue;
        }
        let result = match frame.proc(key) {
            Some(row) => soyuz_sysmon::signal_row(row, sig),
            None => Err(ControlError::Gone),
        };
        match result {
            Ok(()) => done += 1,
            Err(ControlError::Unsupported) if was_terminate => offer_kill.push(key),
            Err(e) => errors.push((name, e)),
        }
    }
    Outcome::Control { what, errors, offer_kill, done }
}

fn take_snapshot(app: &mut AppCore, label: String) {
    let wake = app.wake.clone();
    let loader = Hub::global().snapshot_now(label, wake.clone());
    let mut loader = loader;
    spawn(app, "snapshot", move || {
        // `snapshot_now` already runs on a worker: wait for its answer here.
        let result = loop {
            if let Some(r) = loader.poll() {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        match result {
            Ok(Some(m)) => {
                Outcome::SnapshotsChanged(Some(Toast::success(tr!("monitor-snapshot-taken", label = m.label.as_str()))))
            }
            Ok(None) => Outcome::SnapshotsChanged(Some(Toast::error(tr!("monitor-no-data")))),
            Err(e) => Outcome::SnapshotsChanged(Some(Toast::error(tr!("monitor-snapshot-failed")).body(e.to_string()))),
        }
    });
}

fn export_toast(path: std::path::PathBuf, pane: PaneId) -> Toast {
    let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let _ = pane;
    Toast::success(tr!("monitor-exported"))
        .body(path.display().to_string())
        .cmd(tr!("monitor-show-folder"), Cmd::OpenInNewTab { location: Location::Dir(dir), activate: true })
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn export_processes(app: &mut AppCore, pane: PaneId) {
    let cfg = app.config.monitor.clone();
    let Some(frame) = app.ui.monitor.frame.clone() else { return };
    let generation = app.ui.monitor.generation;
    let v = app.ui.monitor.view(pane, &cfg);
    // What the table shows, in its order.
    let rows: Vec<_> = v.rows(&frame, generation, &cfg).iter().filter_map(|r| r.proc(&frame).cloned()).collect();
    let columns: Vec<ProcColumn> = cfg.columns.iter().map(|c| c.id).collect();
    let opts = snapshot::CsvOpts {
        ncpu: frame.ncpu(),
        scale: cfg.cpu_scale,
        now_ms: frame.at_ms,
        commands: cfg.snapshot_commands,
    };
    spawn(app, "export", move || {
        let path = export_dir().join(format!("processes-{}.csv", stamp()));
        let result = write_with(&path, |w| snapshot::export_procs_csv(&rows, &columns, opts, w));
        match result {
            Ok(()) => Outcome::Toast(export_toast(path, pane)),
            Err(e) => Outcome::Toast(Toast::error(tr!("monitor-export-failed")).body(e.to_string())),
        }
    });
}

fn export_snapshot(app: &mut AppCore, id: String, csv: bool, commands: SnapshotCommands) {
    let pane = app.ws.pane().id;
    spawn(app, "export", move || {
        let snap = match snapshot::load(&data_dir(), &id) {
            Ok(s) => s,
            Err(e) => return Outcome::Toast(Toast::error(tr!("monitor-export-failed")).body(e.to_string())),
        };
        let ext = if csv { "csv" } else { "json" };
        let path = export_dir().join(format!("snapshot-{id}.{ext}"));
        let result = write_with(&path, |w| {
            if csv {
                let opts = snapshot::CsvOpts {
                    ncpu: snap.frame.ncpu(),
                    scale: soyuz_core::config::CpuScale::Machine,
                    now_ms: snap.meta.taken_ms,
                    commands,
                };
                snapshot::export_procs_csv(&snap.frame.procs, &soyuz_core::config::monitor::ProcColumn::ALL, opts, w)
            } else {
                snapshot::export_json(&snap, w)
            }
        });
        match result {
            Ok(()) => Outcome::Toast(export_toast(path, pane)),
            Err(e) => Outcome::Toast(Toast::error(tr!("monitor-export-failed")).body(e.to_string())),
        }
    });
}

fn write_with(
    path: &std::path::Path,
    f: impl FnOnce(&mut dyn std::io::Write) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    f(&mut buf)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    soyuz_fs::write_atomic(path, &buf).map_err(|e| std::io::Error::other(e.to_string()))
}

/// The size style of the monitor's texts.
pub fn size_style(app: &AppCore) -> SizeStyle {
    app.config.general.size_style
}

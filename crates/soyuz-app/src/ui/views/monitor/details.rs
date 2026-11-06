//! The details panel under the process table: general facts in two columns with CPU and
//! memory sparklines, and the Threads, Modules, Files, Environment and Sockets tables,
//! each read on a worker only while its tab is open.

use std::sync::Arc;

use egui::{Align2, Id, Rect, Ui, pos2, vec2};
use soyuz_sysmon::{Frame, Hub, ProcKey, ProcRow, Pt};

use super::charts::{Chart, Hue, Series, Unit, hue};
use super::table::{Col, Table, row_bg};
use super::{PageCtx, fact, segmented_text};
use crate::models::monitor::lists::endpoint;
use crate::models::monitor::{DetailsTab, MonitorCmd, labels};
use crate::theme::font::semibold;
use crate::theme::{font, icons::ph};
use crate::ui::widgets::{spinner, trunc_galley};

fn tab_title(t: DetailsTab) -> &'static str {
    match t {
        DetailsTab::General => tr!("monitor-gui-tab-general"),
        DetailsTab::Threads => tr!("monitor-gui-tab-threads"),
        DetailsTab::Modules => tr!("monitor-gui-tab-modules"),
        DetailsTab::Files => tr!("monitor-gui-tab-files"),
        DetailsTab::Environment => tr!("monitor-gui-tab-environment"),
        DetailsTab::Sockets => tr!("monitor-gui-tab-sockets"),
    }
}

/// The per-process trail, read from the hub once per data generation.
fn trail(ui: &Ui, pane: u64, key: ProcKey, generation: u64) -> Arc<(Vec<Pt>, Vec<Pt>)> {
    let id = Id::unique(("soyuz.monitor.details.trail", pane));
    if let Some((k, g, s)) = ui.ctx().data(|d| d.get_temp::<(ProcKey, u64, Arc<(Vec<Pt>, Vec<Pt>)>)>(id))
        && k == key
        && g == generation
    {
        return s;
    }
    let samples = Hub::global().proc_series(key);
    let cpu: Vec<Pt> = samples.iter().map(|s| Pt { t: s.t_ms, avg: s.cpu_percent, max: s.cpu_percent }).collect();
    let mem: Vec<Pt> = samples.iter().map(|s| Pt { t: s.t_ms, avg: s.rss as f32, max: s.rss as f32 }).collect();
    let s = Arc::new((cpu, mem));
    ui.ctx().data_mut(|d| d.insert_temp(id, (key, generation, s.clone())));
    s
}

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>, frame: &Arc<Frame>, pending: &mut Vec<MonitorCmd>) {
    let p = c.p.clone();
    let pane = c.pane.0;
    let painter = ui.painter_at(rect);
    let (tab, selected) = {
        let v = c.view();
        (v.details.tab, v.selected_row(frame).cloned())
    };
    let Some(pr) = selected else {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            tr!("monitor-gui-select-process"),
            font::ui(12.5),
            p.text_tertiary,
        );
        return;
    };
    // Header: name, pid, tabs.
    let hr =
        Rect::from_min_size(pos2(rect.left() + 14.0, rect.top() + 6.0), vec2(rect.width() - 28.0, font::grow(28.0)));
    let title = format!("{}", pr.raw.name);
    let tg = trunc_galley(&painter, &title, semibold(13.0), p.text_primary, hr.width() * 0.4);
    painter.galley(pos2(hr.left(), hr.center().y - tg.size().y / 2.0), tg.clone(), p.text_primary);
    let pid = format!("PID {}", pr.raw.key.pid);
    painter.text(
        pos2(hr.left() + tg.size().x + 8.0, hr.center().y),
        Align2::LEFT_CENTER,
        pid,
        font::numbers(11.5),
        p.text_tertiary,
    );
    let tabs = DetailsTab::available();
    let titles: Vec<&str> = tabs.iter().map(|t| tab_title(*t)).collect();
    let seg_w = super::segmented_width(ui, &titles);
    let seg_r = Rect::from_min_size(pos2(hr.right() - seg_w, hr.top()), vec2(seg_w, hr.height()));
    let sel = tabs.iter().position(|t| *t == tab).unwrap_or(0);
    if let Some(i) = segmented_text(ui, seg_r, Id::unique(("soyuz.monitor.details.tabs", pane)), &titles, sel) {
        pending.push(MonitorCmd::SetDetailsTab(tabs[i]));
    }
    let body =
        Rect::from_min_max(pos2(rect.left() + 14.0, hr.bottom() + 6.0), pos2(rect.right() - 14.0, rect.bottom() - 6.0));
    match tab {
        DetailsTab::General => general(ui, body, c, frame, &pr),
        DetailsTab::Sockets => sockets(ui, body, c, frame, &pr),
        other => list_tab(ui, body, c, other, &pr),
    }
}

fn general(ui: &mut Ui, body: Rect, c: &mut PageCtx<'_>, frame: &Arc<Frame>, pr: &ProcRow) {
    let p = c.p.clone();
    let config = c.config;
    let style = c.style();
    let raw = &pr.raw;
    let dash = PageCtx::dash();
    let ncpu = frame.ncpu();
    let painter = ui.painter_at(body);
    let now_ms = frame.at_ms;
    let opt_b = |v: Option<u64>| v.map_or(dash.to_string(), |b| soyuz_core::format_size(b, style));
    let opt_s = |v: Option<&Arc<str>>| v.map_or(dash.to_string(), |s| s.to_string());
    let cpu_shown = match config.monitor.cpu_scale {
        soyuz_core::config::monitor::CpuScale::Machine => pr.cpu_percent / ncpu.max(1) as f32,
        soyuz_core::config::monitor::CpuScale::PerCore => pr.cpu_percent,
    };
    let facts_a: Vec<(&str, String)> = vec![
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::Pid), raw.key.pid.to_string()),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::Parent),
            raw.parent.map_or(dash.to_string(), |x| x.to_string()),
        ),
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::User), opt_s(raw.user.as_ref())),
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::State), labels::state(raw.state).to_string()),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::Priority),
            raw.priority.map_or(dash.to_string(), |x| labels::priority(x).to_string()),
        ),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::Started),
            if raw.key.start_ms > 0 { labels::time(raw.key.start_ms, config) } else { dash.to_string() },
        ),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::Uptime),
            if raw.key.start_ms > 0 {
                labels::uptime(((now_ms - raw.key.start_ms) / 1000).max(0) as u64)
            } else {
                dash.to_string()
            },
        ),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::CpuTime),
            labels::cpu_time(raw.cpu_user_ms + raw.cpu_system_ms),
        ),
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::Threads), labels::count(u64::from(raw.threads))),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::Handles),
            raw.handles.map_or(dash.to_string(), |h| labels::count(u64::from(h))),
        ),
    ];
    let facts_b: Vec<(&str, String)> = vec![
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::Cpu), labels::percent(cpu_shown)),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::Memory),
            soyuz_core::format_size(raw.rss, style),
        ),
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::Private), opt_b(raw.private)),
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::Shared), opt_b(raw.shared)),
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::Virtual), opt_b(raw.virt)),
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::PeakMemory), opt_b(raw.peak_rss)),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::DiskRead),
            pr.read_rate.map_or(dash.to_string(), |r| labels::rate(r, style)),
        ),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::DiskWrite),
            pr.write_rate.map_or(dash.to_string(), |r| labels::rate(r, style)),
        ),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::Gpu),
            raw.gpu_percent.map_or(dash.to_string(), labels::percent),
        ),
        (labels::column_title(soyuz_core::config::monitor::ProcColumn::Arch), opt_s(raw.arch.as_ref())),
    ];
    let wide = body.width() >= font::grow(760.0);
    let chart_w = if wide { font::grow(220.0) } else { 0.0 };
    let cols_w = body.width() - chart_w - if wide { 16.0 } else { 0.0 };
    let col_w = (cols_w - 20.0) / 2.0;
    let mut y = body.top() + 2.0;
    let mut ya = y;
    let mut yb = y;
    for (l, v) in &facts_a {
        if ya + 16.0 > body.bottom() {
            break;
        }
        ya = fact(&painter, &p, body.left(), ya, col_w, l, v);
    }
    for (l, v) in &facts_b {
        if yb + 16.0 > body.bottom() {
            break;
        }
        yb = fact(&painter, &p, body.left() + col_w + 20.0, yb, col_w, l, v);
    }
    y = ya.max(yb) + 4.0;
    // Path and command line span both columns.
    let lf = font::ui(11.5);
    for (label, value) in [
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::Path),
            raw.exe.as_deref().map(|e| e.display().to_string()),
        ),
        (
            labels::column_title(soyuz_core::config::monitor::ProcColumn::Command),
            raw.cmdline.as_deref().map(str::to_string),
        ),
    ] {
        if y + 16.0 > body.bottom() {
            break;
        }
        painter.text(pos2(body.left(), y), Align2::LEFT_TOP, label, lf.clone(), p.text_tertiary);
        let text = value.unwrap_or_else(|| dash.to_string());
        let g = trunc_galley(&painter, &text, font::mono(11.0), p.text_primary, cols_w - 90.0);
        painter.galley(pos2(body.left() + 90.0, y), g, p.text_primary);
        y += font::grow(18.0);
    }
    if wide {
        let trail = trail(ui, c.pane.0, raw.key, c.generation);
        let x = body.right() - chart_w;
        let h = ((body.height() - 24.0) / 2.0).max(30.0);
        let span_from = trail.0.first().map_or(now_ms - 60_000, |pt| pt.t.min(now_ms - 30_000));
        let span = (span_from, now_ms);
        let fmt = c.fmt();
        for (i, (pts, label, h_, unit, ymax)) in [
            (
                &trail.0,
                labels::column_title(soyuz_core::config::monitor::ProcColumn::Cpu),
                Hue::Cpu,
                Unit::Percent,
                Some(100.0 * ncpu as f32),
            ),
            (
                &trail.1,
                labels::column_title(soyuz_core::config::monitor::ProcColumn::Memory),
                Hue::Memory,
                Unit::Bytes,
                None,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let r = Rect::from_min_size(pos2(x, body.top() + i as f32 * (h + 12.0)), vec2(chart_w, h));
            let latest = pts.last().map_or(dash.to_string(), |pt| super::charts::fmt_value(unit, pt.avg, fmt));
            painter.text(pos2(r.left(), r.top() + 6.0), Align2::LEFT_CENTER, label, font::ui(10.5), p.text_tertiary);
            painter.text(
                pos2(r.right(), r.top() + 6.0),
                Align2::RIGHT_CENTER,
                latest,
                font::numbers(10.5),
                p.text_secondary,
            );
            let plot = Rect::from_min_max(pos2(r.left(), r.top() + 16.0), r.max);
            let mut chart = Chart::mini(span, unit, fmt);
            chart.y_max = ymax;
            chart.crosshair = true;
            let s = [Series::new(label, hue(&p, h_), pts)];
            chart.show(ui, plot, Id::unique(("soyuz.monitor.details.spark", c.pane.0, i)), &p, &s);
        }
    }
}

fn sockets(ui: &mut Ui, body: Rect, c: &mut PageCtx<'_>, frame: &Arc<Frame>, pr: &ProcRow) {
    let p = c.p.clone();
    let pid = pr.raw.key.pid;
    let (rows, loading): (Vec<soyuz_sysmon::Connection>, bool) = {
        let v = c.view();
        (v.connections.rows.iter().filter(|x| x.pid == Some(pid)).cloned().collect(), v.connections.loading())
    };
    if rows.is_empty() {
        let msg = if loading { tr!("common-loading") } else { tr!("monitor-gui-no-sockets") };
        if loading {
            spinner(ui, body.center() - vec2(0.0, 14.0), 7.0, p.accent);
        }
        ui.painter().text(body.center() + vec2(0.0, 8.0), Align2::CENTER_CENTER, msg, font::ui(12.0), p.text_tertiary);
        return;
    }
    let cols = [
        Col::new(tr!("monitor-gui-col-proto"), 64.0),
        Col::new(tr!("monitor-gui-col-local"), 200.0),
        Col::new(tr!("monitor-gui-col-remote"), 200.0),
        Col::new(tr!("monitor-gui-col-state"), 120.0),
    ];
    let row_h = font::height(24.0, 12.0);
    let table = Table::new(Id::unique(("soyuz.monitor.details.sockets", c.pane.0)), &cols, rows.len(), row_h);
    table.show(ui, body, &p, |rc| {
        let x = &rows[rc.index];
        row_bg(rc.painter, &p, rc.rect, false, rc.hovered, false);
        let f = font::ui(12.0);
        rc.text(0, &format!("{:?}", x.proto).to_uppercase(), false, font::mono(11.0), p.text_secondary);
        rc.text(1, &endpoint(&x.local_addr, x.local_port), false, font::mono(11.5), p.text_primary);
        rc.text(2, &endpoint(&x.remote_addr, x.remote_port), false, font::mono(11.5), p.text_primary);
        rc.text(3, &x.state, false, f, p.text_secondary);
        format!("{:?} {} {}", x.proto, endpoint(&x.local_addr, x.local_port), x.state)
    });
    let _ = frame;
}

fn list_tab(ui: &mut Ui, body: Rect, c: &mut PageCtx<'_>, tab: DetailsTab, pr: &ProcRow) {
    let p = c.p.clone();
    let style = c.style();
    let (data, loading) = {
        let v = c.view();
        (v.details.data.clone(), v.details.loading())
    };
    let Some(data) = data else {
        if loading {
            spinner(ui, body.center() - vec2(0.0, 14.0), 7.0, p.accent);
            ui.painter().text(
                body.center() + vec2(0.0, 8.0),
                Align2::CENTER_CENTER,
                tr!("common-loading"),
                font::ui(12.0),
                p.text_tertiary,
            );
        }
        return;
    };
    let d = match data {
        Ok(d) => d,
        Err(e) => {
            ui.painter().text(
                body.center(),
                Align2::CENTER_CENTER,
                labels::control_error(&e),
                font::ui(12.0),
                p.text_tertiary,
            );
            return;
        }
    };
    let row_h = font::height(24.0, 12.0);
    let id = Id::unique(("soyuz.monitor.details.list", c.pane.0, tab as u8));
    let limited = d.limited;
    let footer = if limited { font::grow(18.0) } else { 0.0 };
    let tr_ = Rect::from_min_max(body.min, pos2(body.right(), body.bottom() - footer));
    match tab {
        DetailsTab::Threads => {
            let cols = [
                Col::new(tr!("monitor-gui-col-tid"), 80.0).numeric(),
                Col::new(tr!("monitor-gui-col-thread-name"), 240.0),
                Col::new(tr!("monitor-gui-col-state"), 110.0),
                Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::CpuTime), 110.0).numeric(),
                Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::Priority), 80.0).numeric(),
            ];
            let mut rows = d.threads.clone();
            rows.sort_by_key(|a| std::cmp::Reverse(a.cpu_ms));
            if rows.is_empty() {
                nothing(ui, tr_, &p);
            }
            Table::new(id, &cols, rows.len(), row_h).show(ui, tr_, &p, |rc| {
                let t = &rows[rc.index];
                row_bg(rc.painter, &p, rc.rect, false, rc.hovered, false);
                rc.text(0, &t.tid.to_string(), true, font::numbers(12.0), p.text_primary);
                rc.text(
                    1,
                    if t.name.is_empty() { PageCtx::dash() } else { &t.name },
                    false,
                    font::ui(12.0),
                    p.text_primary,
                );
                rc.text(2, labels::state(t.state), false, font::ui(12.0), p.text_secondary);
                rc.text(3, &labels::cpu_time(t.cpu_ms), true, font::numbers(12.0), p.text_primary);
                rc.text(
                    4,
                    &t.priority.map_or(PageCtx::dash().to_string(), |x| x.to_string()),
                    true,
                    font::numbers(12.0),
                    p.text_secondary,
                );
                format!("{} {}", t.tid, t.name)
            });
        }
        DetailsTab::Modules => {
            let cols = [
                Col::new(tr!("monitor-gui-col-module"), 420.0),
                Col::new(tr!("monitor-gui-col-size"), 100.0).numeric(),
                Col::new(tr!("monitor-gui-col-base"), 150.0).numeric(),
            ];
            let rows = &d.modules;
            if rows.is_empty() {
                nothing(ui, tr_, &p);
            }
            Table::new(id, &cols, rows.len(), row_h).show(ui, tr_, &p, |rc| {
                let m = &rows[rc.index];
                row_bg(rc.painter, &p, rc.rect, false, rc.hovered, false);
                rc.text(0, &m.path, false, font::mono(11.0), p.text_primary);
                rc.text(1, &soyuz_core::format_size(m.size, style), true, font::numbers(12.0), p.text_primary);
                rc.text(2, &format!("{:#x}", m.base), true, font::mono(11.0), p.text_secondary);
                m.path.clone()
            });
        }
        DetailsTab::Files => {
            let cols =
                [Col::new(tr!("monitor-gui-col-fd"), 60.0).numeric(), Col::new(tr!("monitor-gui-col-path"), 560.0)];
            let rows = &d.open_files;
            if rows.is_empty() {
                nothing(ui, tr_, &p);
            }
            Table::new(id, &cols, rows.len(), row_h).show(ui, tr_, &p, |rc| {
                let f = &rows[rc.index];
                row_bg(rc.painter, &p, rc.rect, false, rc.hovered, false);
                rc.text(
                    0,
                    &f.fd.map_or(PageCtx::dash().to_string(), |x| x.to_string()),
                    true,
                    font::numbers(12.0),
                    p.text_secondary,
                );
                rc.text(1, &f.path, false, font::mono(11.0), p.text_primary);
                f.path.clone()
            });
        }
        DetailsTab::Environment => {
            let cols =
                [Col::new(tr!("monitor-gui-col-variable"), 220.0), Col::new(tr!("monitor-gui-col-value"), 520.0)];
            let rows = &d.environment;
            if rows.is_empty() {
                nothing(ui, tr_, &p);
            }
            Table::new(id, &cols, rows.len(), row_h).show(ui, tr_, &p, |rc| {
                let (k, v) = &rows[rc.index];
                row_bg(rc.painter, &p, rc.rect, false, rc.hovered, false);
                rc.text(0, k, false, font::mono(11.0), p.text_secondary);
                rc.text(1, v, false, font::mono(11.0), p.text_primary);
                k.clone()
            });
        }
        DetailsTab::General | DetailsTab::Sockets => {}
    }
    if limited {
        let y = body.bottom() - footer / 2.0;
        ui.painter().text(
            pos2(body.left() + 6.0, y),
            Align2::CENTER_CENTER,
            ph::WARNING,
            crate::theme::icons::font(11.0),
            p.warning,
        );
        ui.painter().text(
            pos2(body.left() + 16.0, y),
            Align2::LEFT_CENTER,
            tr!("monitor-gui-limited"),
            font::ui(11.0),
            p.warning,
        );
    }
    let _ = pr;
}

fn nothing(ui: &Ui, rect: Rect, p: &crate::theme::Palette) {
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        tr!("monitor-gui-nothing-here"),
        font::ui(12.0),
        p.text_tertiary,
    );
}

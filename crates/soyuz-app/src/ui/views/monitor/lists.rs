//! The Connections, Services and Startup pages: a filter box, a sortable table, and the
//! actions of each (a connection links to its process; a service can be started, stopped
//! or restarted after a confirmation).

use egui::{Align2, Id, Rect, Ui, pos2, vec2};
use soyuz_core::config::monitor::MonitorPage;
use soyuz_sysmon::{RowId, ServiceOp, ServiceState};

use super::table::{Col, HeaderEvent, Sort, Table, row_bg};
use super::{PageCtx, filter_box};
use crate::models::monitor::lists::{endpoint, process_key, process_name};
use crate::models::monitor::{MonitorCmd, labels};
use crate::theme::{font, icons::ph};
use crate::ui::widgets::{menu_item, menu_separator, spinner};

fn toolbar_h() -> f32 {
    font::grow(40.0)
}

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>, page: MonitorPage) {
    let p = c.p.clone();
    let pane = c.pane.0;
    let frame = c.frame.clone();
    let mut pending: Vec<MonitorCmd> = Vec::new();
    let tb = Rect::from_min_size(rect.min, vec2(rect.width(), toolbar_h()));
    let inner = Rect::from_min_max(pos2(tb.left() + 14.0, tb.top() + 6.0), pos2(tb.right() - 12.0, tb.bottom() - 6.0));
    let (filter_text, focus, loading, count, total, sort, descending, cursor) = {
        let v = c.view();
        let (l, text, focus, count, total, sort, desc, cursor) = match page {
            MonitorPage::Connections => {
                let f = std::mem::take(&mut v.connections.focus_filter);
                (
                    v.connections.loading(),
                    v.connections.filter_text.clone(),
                    f,
                    v.connections.visible(frame.as_deref()).len(),
                    v.connections.rows.len(),
                    v.connections.sort,
                    v.connections.descending,
                    v.connections.cursor,
                )
            }
            MonitorPage::Services => {
                let f = std::mem::take(&mut v.services.focus_filter);
                (
                    v.services.loading(),
                    v.services.filter_text.clone(),
                    f,
                    v.services.visible(None).len(),
                    v.services.rows.len(),
                    v.services.sort,
                    v.services.descending,
                    v.services.cursor,
                )
            }
            _ => {
                let f = std::mem::take(&mut v.startup.focus_filter);
                (
                    v.startup.loading(),
                    v.startup.filter_text.clone(),
                    f,
                    v.startup.visible(None).len(),
                    v.startup.rows.len(),
                    v.startup.sort,
                    v.startup.descending,
                    v.startup.cursor,
                )
            }
        };
        (text, focus, l, count, total, sort, desc, cursor)
    };
    // Toolbar: filter, count.
    let fw = inner.width().min(font::grow(420.0));
    let fr = Rect::from_min_size(pos2(inner.left(), inner.top() + 1.0), vec2(fw, inner.height() - 2.0));
    let hint = tr!("monitor-gui-list-filter-hint");
    let (changed, fresp) =
        filter_box(ui, fr, Id::unique(("soyuz.monitor.list.filter", pane, page as u8)), &filter_text, hint, focus);
    if let Some(t) = changed {
        pending.push(match page {
            MonitorPage::Connections => MonitorCmd::ConnectionsFilter(t),
            MonitorPage::Services => MonitorCmd::ServicesFilter(t),
            _ => MonitorCmd::StartupFilter(t),
        });
    }
    if fresp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        if filter_text.is_empty() {
            fresp.surrender_focus();
        } else {
            pending.push(match page {
                MonitorPage::Connections => MonitorCmd::ConnectionsFilter(String::new()),
                MonitorPage::Services => MonitorCmd::ServicesFilter(String::new()),
                _ => MonitorCmd::StartupFilter(String::new()),
            });
        }
    }
    let count_text = if count == total {
        labels::count(count as u64)
    } else {
        tr!(
            "monitor-gui-count-of",
            shown = labels::count(count as u64).as_str(),
            total = labels::count(total as u64).as_str()
        )
    };
    ui.painter().text(
        pos2(fr.right() + 10.0, inner.center().y),
        Align2::LEFT_CENTER,
        count_text,
        font::numbers(11.5),
        p.text_tertiary,
    );
    if loading {
        spinner(ui, pos2(inner.right() - 10.0, inner.center().y), 6.0, p.accent);
    }
    let body = Rect::from_min_max(pos2(rect.left(), tb.bottom()), rect.max);
    let row_h = font::height(c.config.appearance.density.row_height(), 13.0);
    let focused = c.focused && c.list_focus;
    let sort_s = Some(Sort { col: sort as usize, descending });
    let id = Id::unique(("soyuz.monitor.list.table", pane, page as u8));
    match page {
        MonitorPage::Connections => {
            let (rows, idx): (Vec<soyuz_sysmon::Connection>, Vec<usize>) = {
                let v = c.view();
                (v.connections.rows.clone(), v.connections.visible(frame.as_deref()))
            };
            let cols = [
                Col::new(tr!("monitor-gui-col-proto"), 86.0),
                Col::new(tr!("monitor-gui-col-local"), 210.0),
                Col::new(tr!("monitor-gui-col-remote"), 210.0),
                Col::new(tr!("monitor-gui-col-state"), 120.0),
                Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::Pid), 70.0).numeric(),
                Col::new(tr!("monitor-gui-col-process"), 220.0),
            ];
            let fr_ = frame.as_deref();
            if idx.is_empty() && !loading {
                empty_line(
                    ui,
                    body,
                    &p,
                    if total == 0 { tr!("monitor-gui-no-connections") } else { tr!("monitor-gui-no-matches") },
                );
            }
            let out = Table {
                sort: sort_s,
                scroll_to: Some(cursor),
                name: labels::page_title(page).to_string(),
                ..Table::new(id, &cols, idx.len(), row_h)
            }
            .show(ui, body, &p, |rc| {
                let x = &rows[idx[rc.index]];
                row_bg(rc.painter, &p, rc.rect, rc.index == cursor, rc.hovered, focused);
                rc.text(0, &format!("{:?}", x.proto).to_uppercase(), false, font::mono(11.0), p.text_secondary);
                rc.text(1, &endpoint(&x.local_addr, x.local_port), false, font::mono(11.5), p.text_primary);
                rc.text(2, &endpoint(&x.remote_addr, x.remote_port), false, font::mono(11.5), p.text_primary);
                let sc = match x.state.as_str() {
                    "ESTABLISHED" => p.success,
                    "LISTEN" | "LISTENING" => p.text_secondary,
                    _ => p.text_tertiary,
                };
                rc.text(3, &x.state, false, font::ui(12.0), sc);
                rc.text(
                    4,
                    &x.pid.map_or(PageCtx::dash().to_string(), |v| v.to_string()),
                    true,
                    font::numbers(12.0),
                    p.text_primary,
                );
                let name = process_name(fr_, x.pid);
                rc.text(
                    5,
                    if name.is_empty() { PageCtx::dash() } else { &name },
                    false,
                    font::ui(12.5),
                    p.text_primary,
                );
                format!("{:?} {} {} {}", x.proto, endpoint(&x.local_addr, x.local_port), x.state, name)
            });
            header_events(&out.events, &mut pending, page);
            if let Some((i, _)) = out.clicked {
                c.cmds.push(crate::cmd::Cmd::FocusPane(c.pane));
                c.view().connections.cursor = i;
            }
            let goto = |i: usize| -> Option<MonitorCmd> {
                let x = &rows[*idx.get(i)?];
                let key = process_key(fr_, x.pid)?;
                Some(MonitorCmd::Select(Some(RowId::Proc(key))))
            };
            if let Some(i) = out.double_clicked
                && let Some(sel) = goto(i)
            {
                pending.push(MonitorCmd::SetPage(MonitorPage::Processes));
                pending.push(sel);
            }
            if let Some(i) = out.secondary {
                c.view().connections.cursor = i;
            }
            let cur = c.view().connections.cursor;
            let cmds_out = &mut pending;
            super::context_menu(&out.bg, |ui| {
                ui.set_width(220.0);
                let has = goto(cur).is_some();
                if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("monitor-gui-go-to-process"), None, has).clicked() {
                    if let Some(sel) = goto(cur) {
                        cmds_out.push(MonitorCmd::SetPage(MonitorPage::Processes));
                        cmds_out.push(sel);
                    }
                    ui.close();
                }
                if let Some(x) = idx.get(cur).map(|k| &rows[*k]) {
                    menu_separator(ui);
                    if menu_item(ui, Some(ph::COPY), tr!("monitor-gui-copy-remote"), None, !x.remote_addr.is_empty())
                        .clicked()
                    {
                        cmds_out.push(MonitorCmd::CopyText(endpoint(&x.remote_addr, x.remote_port)));
                        ui.close();
                    }
                    if menu_item(ui, Some(ph::COPY), tr!("monitor-gui-copy-local"), None, true).clicked() {
                        cmds_out.push(MonitorCmd::CopyText(endpoint(&x.local_addr, x.local_port)));
                        ui.close();
                    }
                }
            });
        }
        MonitorPage::Services => {
            let (rows, idx): (Vec<soyuz_sysmon::ServiceInfo>, Vec<usize>) = {
                let v = c.view();
                (v.services.rows.clone(), v.services.visible(None))
            };
            let cols = [
                Col::new(tr!("monitor-gui-col-service"), 260.0),
                Col::new(tr!("monitor-gui-col-state"), 110.0),
                Col::new(tr!("monitor-gui-col-start-mode"), 110.0),
                Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::Pid), 70.0).numeric(),
                Col::new(tr!("monitor-gui-col-service-name"), 200.0),
                Col::new(tr!("monitor-gui-col-description"), 420.0),
            ];
            if idx.is_empty() && !loading {
                empty_line(
                    ui,
                    body,
                    &p,
                    if total == 0 { tr!("monitor-gui-no-services") } else { tr!("monitor-gui-no-matches") },
                );
            }
            let out = Table {
                sort: sort_s,
                scroll_to: Some(cursor),
                name: labels::page_title(page).to_string(),
                ..Table::new(id, &cols, idx.len(), row_h)
            }
            .show(ui, body, &p, |rc| {
                let s = &rows[idx[rc.index]];
                row_bg(rc.painter, &p, rc.rect, rc.index == cursor, rc.hovered, focused);
                rc.text(
                    0,
                    if s.display_name.is_empty() { &s.name } else { &s.display_name },
                    false,
                    font::ui(12.5),
                    p.text_primary,
                );
                let (dot, color) = match s.state {
                    ServiceState::Running => (true, p.success),
                    ServiceState::Starting | ServiceState::Stopping => (true, p.warning),
                    ServiceState::Failed => (true, p.danger),
                    ServiceState::Paused => (true, p.warning),
                    ServiceState::Stopped | ServiceState::Unknown => (false, p.text_tertiary),
                };
                let cell = rc.cell(1);
                if dot {
                    rc.painter.circle_filled(pos2(cell.left() + 4.0, cell.center().y), 3.5, color);
                } else {
                    rc.painter.circle_stroke(
                        pos2(cell.left() + 4.0, cell.center().y),
                        3.0,
                        egui::Stroke::new(1.0, color),
                    );
                }
                super::table::text_in(
                    rc.painter,
                    Rect::from_min_max(pos2(cell.left() + 14.0, cell.top()), cell.max),
                    labels::service_state(s.state),
                    false,
                    font::ui(12.0),
                    p.text_secondary,
                );
                rc.text(2, &s.start_mode, false, font::ui(12.0), p.text_secondary);
                rc.text(
                    3,
                    &s.pid.map_or(PageCtx::dash().to_string(), |v| v.to_string()),
                    true,
                    font::numbers(12.0),
                    p.text_primary,
                );
                rc.text(4, &s.name, false, font::mono(11.0), p.text_secondary);
                rc.text(5, &s.description, false, font::ui(12.0), p.text_tertiary);
                format!("{} {}", s.display_name, labels::service_state(s.state))
            });
            header_events(&out.events, &mut pending, page);
            if let Some((i, _)) = out.clicked {
                c.cmds.push(crate::cmd::Cmd::FocusPane(c.pane));
                c.view().services.cursor = i;
            }
            if let Some(i) = out.secondary {
                c.view().services.cursor = i;
            }
            let cur = c.view().services.cursor;
            let cmds_out = &mut pending;
            let fr_ = frame.as_deref();
            super::context_menu(&out.bg, |ui| {
                ui.set_width(220.0);
                let Some(s) = idx.get(cur).map(|k| &rows[*k]) else { return };
                crate::ui::widgets::menu_caption(ui, if s.display_name.is_empty() { &s.name } else { &s.display_name });
                let running = matches!(s.state, ServiceState::Running | ServiceState::Starting | ServiceState::Paused);
                if menu_item(ui, Some(ph::PLAY), labels::service_op(ServiceOp::Start), None, !running).clicked() {
                    cmds_out.push(MonitorCmd::Service(s.name.clone(), ServiceOp::Start));
                    ui.close();
                }
                if menu_item(ui, Some(ph::STOP), labels::service_op(ServiceOp::Stop), None, running).clicked() {
                    cmds_out.push(MonitorCmd::Service(s.name.clone(), ServiceOp::Stop));
                    ui.close();
                }
                if menu_item(ui, Some(ph::ARROW_CLOCKWISE), labels::service_op(ServiceOp::Restart), None, running)
                    .clicked()
                {
                    cmds_out.push(MonitorCmd::Service(s.name.clone(), ServiceOp::Restart));
                    ui.close();
                }
                if let Some(key) = process_key(fr_, s.pid) {
                    menu_separator(ui);
                    if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("monitor-gui-go-to-process"), None, true).clicked()
                    {
                        cmds_out.push(MonitorCmd::SetPage(MonitorPage::Processes));
                        cmds_out.push(MonitorCmd::Select(Some(RowId::Proc(key))));
                        ui.close();
                    }
                }
            });
        }
        _ => {
            let (rows, idx): (Vec<soyuz_sysmon::StartupItem>, Vec<usize>) = {
                let v = c.view();
                (v.startup.rows.clone(), v.startup.visible(None))
            };
            let cols = [
                Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::Name), 240.0),
                Col::new(tr!("monitor-gui-col-enabled"), 100.0),
                Col::new(tr!("monitor-gui-col-source"), 260.0),
                Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::Command), 520.0),
            ];
            if idx.is_empty() && !loading {
                empty_line(
                    ui,
                    body,
                    &p,
                    if total == 0 { tr!("monitor-gui-no-startup") } else { tr!("monitor-gui-no-matches") },
                );
            }
            let out = Table {
                sort: sort_s,
                scroll_to: Some(cursor),
                name: labels::page_title(page).to_string(),
                ..Table::new(id, &cols, idx.len(), row_h)
            }
            .show(ui, body, &p, |rc| {
                let s = &rows[idx[rc.index]];
                row_bg(rc.painter, &p, rc.rect, rc.index == cursor, rc.hovered, focused);
                rc.text(0, &s.name, false, font::ui(12.5), p.text_primary);
                let cell = rc.cell(1);
                let (glyph, color, word) = if s.enabled {
                    (ph::CHECK_CIRCLE, p.success, tr!("monitor-gui-enabled"))
                } else {
                    (ph::MINUS_CIRCLE, p.text_tertiary, tr!("monitor-gui-disabled"))
                };
                rc.painter.text(
                    pos2(cell.left() + 6.0, cell.center().y),
                    Align2::CENTER_CENTER,
                    glyph,
                    crate::theme::icons::font(12.0),
                    color,
                );
                super::table::text_in(
                    rc.painter,
                    Rect::from_min_max(pos2(cell.left() + 16.0, cell.top()), cell.max),
                    word,
                    false,
                    font::ui(12.0),
                    p.text_secondary,
                );
                rc.text(2, &s.source, false, font::ui(11.5), p.text_tertiary);
                rc.text(3, &s.command, false, font::mono(11.0), p.text_secondary);
                format!("{} {}", s.name, word)
            });
            header_events(&out.events, &mut pending, page);
            if let Some((i, _)) = out.clicked {
                c.cmds.push(crate::cmd::Cmd::FocusPane(c.pane));
                c.view().startup.cursor = i;
            }
            if let Some(i) = out.secondary {
                c.view().startup.cursor = i;
            }
            let cur = c.view().startup.cursor;
            let cmds_out = &mut pending;
            super::context_menu(&out.bg, |ui| {
                ui.set_width(220.0);
                let Some(s) = idx.get(cur).map(|k| &rows[*k]) else { return };
                if menu_item(ui, Some(ph::COPY), tr!("monitor-gui-copy-command"), None, !s.command.is_empty()).clicked()
                {
                    cmds_out.push(MonitorCmd::CopyText(s.command.clone()));
                    ui.close();
                }
                if menu_item(ui, Some(ph::COPY), tr!("monitor-gui-copy-source"), None, !s.source.is_empty()).clicked() {
                    cmds_out.push(MonitorCmd::CopyText(s.source.clone()));
                    ui.close();
                }
            });
        }
    }
    for cmd in pending {
        c.cmd(cmd);
    }
}

fn header_events(events: &[HeaderEvent], pending: &mut Vec<MonitorCmd>, page: MonitorPage) {
    for e in events {
        if let HeaderEvent::Sort(i) = e {
            pending.push(match page {
                MonitorPage::Connections => MonitorCmd::ConnectionsSort(*i as u8),
                MonitorPage::Services => MonitorCmd::ServicesSort(*i as u8),
                _ => MonitorCmd::StartupSort(*i as u8),
            });
        }
    }
}

fn empty_line(ui: &Ui, body: Rect, p: &crate::theme::Palette, text: &str) {
    ui.painter().text(
        pos2(body.center().x, body.top() + font::grow(80.0)),
        Align2::CENTER_CENTER,
        text,
        font::ui(12.5),
        p.text_tertiary,
    );
}

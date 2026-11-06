//! Keyboard handling of a focused monitor pane. Runs before the key map dispatch, so the
//! keys the page uses (arrows, Space, Delete, Enter, `/`) never reach the file actions;
//! chords it does not take still fall through to the global key map.

use egui::{Event, Key, Modifiers};
use soyuz_core::config::monitor::MonitorPage;
use soyuz_sysmon::{ProcSignal, RowId};

use super::processes;
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::core::Focus;
use crate::models::monitor::{HistoryState, MonitorCmd, SignalScope};

/// Consume the keys the focused monitor page uses. `true` when the page took the keyboard.
pub fn handle(app: &mut SoyuzApp, ctx: &egui::Context) -> bool {
    // An open menu or popup has the keyboard (Esc closes it, arrows walk it).
    if !crate::models::monitor::focused(&app.core) || app.core.ui.focus != Focus::List || egui::Popup::is_any_open(ctx)
    {
        return false;
    }
    let pane = app.core.ws.pane().id;
    let cfg = app.core.config.monitor.clone();
    let page = app.core.ui.monitor.view(pane, &cfg).page;
    let mut cmds: Vec<MonitorCmd> = Vec::new();
    let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
    let any = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k) || i.consume_key(Modifiers::SHIFT, k));
    // `/` and Ctrl+F put the keyboard into the filter box of the pages that have one.
    let has_filter = matches!(
        page,
        MonitorPage::Processes | MonitorPage::Connections | MonitorPage::Services | MonitorPage::Startup
    );
    if has_filter
        && (pressed(Modifiers::NONE, Key::Slash)
            || pressed(Modifiers::COMMAND, Key::F)
            || pressed(Modifiers::CTRL, Key::F))
    {
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, Event::Text(t) if t == "/")));
        let v = app.core.ui.monitor.view(pane, &cfg);
        match page {
            MonitorPage::Processes => v.focus_filter = true,
            MonitorPage::Connections => v.connections.focus_filter = true,
            MonitorPage::Services => v.services.focus_filter = true,
            _ => v.startup.focus_filter = true,
        }
        return true;
    }
    let mut took = false;
    match page {
        MonitorPage::Processes => took = processes_keys(app, ctx, pane, &cfg, &mut cmds),
        MonitorPage::Connections | MonitorPage::Services | MonitorPage::Startup => {
            let frame = app.core.ui.monitor.frame.clone();
            let v = app.core.ui.monitor.view(pane, &cfg);
            let page_rows = 20;
            let visible = match page {
                MonitorPage::Connections => v.connections.visible(frame.as_deref()),
                MonitorPage::Services => v.services.visible(None),
                _ => v.startup.visible(None),
            };
            let cursor = match page {
                MonitorPage::Connections => &mut v.connections.cursor,
                MonitorPage::Services => &mut v.services.cursor,
                _ => &mut v.startup.cursor,
            };
            if let Some(n) = nav(ctx, *cursor, visible.len(), page_rows) {
                *cursor = n;
                took = true;
            }
            let cur = *cursor;
            if page == MonitorPage::Connections && any(Key::Enter) {
                if let Some(x) = visible.get(cur).map(|k| &v.connections.rows[*k])
                    && let Some(key) = crate::models::monitor::lists::process_key(frame.as_deref(), x.pid)
                {
                    cmds.push(MonitorCmd::SetPage(MonitorPage::Processes));
                    cmds.push(MonitorCmd::Select(Some(RowId::Proc(key))));
                }
                took = true;
            }
        }
        MonitorPage::History => {
            let range = app.core.ui.monitor.view(pane, &cfg).history.range_secs;
            if pressed(Modifiers::NONE, Key::R) {
                cmds.push(if soyuz_sysmon::Hub::global().recording() == soyuz_sysmon::Recording::Off {
                    MonitorCmd::RecordStart
                } else {
                    MonitorCmd::RecordStop
                });
                took = true;
            }
            if pressed(Modifiers::NONE, Key::Space)
                && soyuz_sysmon::Hub::global().recording() != soyuz_sysmon::Recording::Off
            {
                cmds.push(MonitorCmd::RecordPause);
                took = true;
            }
            if any(Key::ArrowLeft) {
                cmds.push(MonitorCmd::HistoryPan(i64::from(range / 10)));
                took = true;
            }
            if any(Key::ArrowRight) {
                cmds.push(MonitorCmd::HistoryPan(-i64::from(range / 10)));
                took = true;
            }
            if any(Key::Home) || any(Key::End) {
                cmds.push(MonitorCmd::HistoryLive);
                took = true;
            }
            let ranges = HistoryState::RANGES;
            let i = ranges.iter().position(|r| *r == range).unwrap_or(2);
            if any(Key::Plus) || any(Key::Equals) {
                cmds.push(MonitorCmd::HistoryRange(ranges[(i + 1).min(ranges.len() - 1)]));
                took = true;
            }
            if any(Key::Minus) {
                cmds.push(MonitorCmd::HistoryRange(ranges[i.saturating_sub(1)]));
                took = true;
            }
        }
        MonitorPage::Snapshots => {
            let s = &mut app.core.ui.monitor.view(pane, &cfg).snapshots;
            if s.renaming.is_none() {
                if let Some(n) = nav(ctx, s.cursor, s.metas.len(), 10) {
                    cmds.push(MonitorCmd::SnapshotCursor(n));
                    took = true;
                }
                if let Some(m) = s.metas.get(s.cursor) {
                    let id = m.id.clone();
                    if pressed(Modifiers::NONE, Key::Delete) {
                        cmds.push(MonitorCmd::DeleteSnapshot(id.clone()));
                        took = true;
                    }
                    if pressed(Modifiers::NONE, Key::F2) {
                        cmds.push(MonitorCmd::StartRename(id.clone()));
                        took = true;
                    }
                    if pressed(Modifiers::NONE, Key::Enter) {
                        cmds.push(MonitorCmd::CompareFrom(id));
                        cmds.push(MonitorCmd::RunCompare);
                        took = true;
                    }
                }
            }
        }
        _ => {}
    }
    // Pages of the rail: Ctrl+PageUp / PageDown walk them in both directions.
    if pressed(Modifiers::CTRL | Modifiers::SHIFT, Key::ArrowDown)
        || pressed(Modifiers::CTRL | Modifiers::SHIFT, Key::ArrowUp)
    {
        let all = MonitorPage::ALL;
        let i = all.iter().position(|p| *p == page).unwrap_or(0);
        let down = ctx.input(|i| i.key_down(Key::ArrowDown));
        let n = if down { (i + 1) % all.len() } else { (i + all.len() - 1) % all.len() };
        cmds.push(MonitorCmd::SetPage(all[n]));
        took = true;
    }
    for c in cmds {
        app.core.cmds.push(Cmd::Monitor(pane, c));
    }
    took
}

/// The new cursor after the list keys, if one was pressed.
fn nav(ctx: &egui::Context, cursor: usize, len: usize, page_rows: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    let k = |key: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key) || i.consume_key(Modifiers::SHIFT, key));
    if k(Key::ArrowDown) {
        Some((cursor + 1).min(last))
    } else if k(Key::ArrowUp) {
        Some(cursor.saturating_sub(1).min(last))
    } else if k(Key::PageDown) {
        Some((cursor + page_rows).min(last))
    } else if k(Key::PageUp) {
        Some(cursor.saturating_sub(page_rows).min(last))
    } else if k(Key::Home) {
        Some(0)
    } else if k(Key::End) {
        Some(last)
    } else {
        None
    }
}

fn processes_keys(
    app: &mut SoyuzApp,
    ctx: &egui::Context,
    pane: crate::state::pane::PaneId,
    cfg: &soyuz_core::config::monitor::Monitor,
    cmds: &mut Vec<MonitorCmd>,
) -> bool {
    let Some(frame) = app.core.ui.monitor.frame.clone() else { return false };
    let generation = app.core.ui.monitor.generation;
    let page_rows = processes::page_rows(ctx, pane.0);
    let view = app.core.ui.monitor.view(pane, cfg);
    let selected = view.selected.clone();
    let marked = view.marked.clone();
    let marked_any = !marked.is_empty();
    let filter_any = !view.filter_text.is_empty();
    let filter_text = view.filter_text.clone();
    let rows = view.rows(&frame, generation, cfg);
    let n = rows.len();
    let pos = selected.as_ref().and_then(|id| soyuz_sysmon::position_of(rows, &frame, id));
    let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
    let mut took = false;
    // Selection movement.
    let cur = pos.unwrap_or(0);
    let target = if n == 0 {
        None
    } else if pressed(Modifiers::NONE, Key::ArrowDown) {
        Some(if pos.is_some() { (cur + 1).min(n - 1) } else { 0 })
    } else if pressed(Modifiers::NONE, Key::ArrowUp) {
        Some(if pos.is_some() { cur.saturating_sub(1) } else { 0 })
    } else if pressed(Modifiers::NONE, Key::PageDown) {
        Some((cur + page_rows).min(n - 1))
    } else if pressed(Modifiers::NONE, Key::PageUp) {
        Some(cur.saturating_sub(page_rows))
    } else if pressed(Modifiers::NONE, Key::Home) {
        Some(0)
    } else if pressed(Modifiers::NONE, Key::End) {
        Some(n - 1)
    } else {
        None
    };
    if let Some(t) = target {
        cmds.push(MonitorCmd::Select(Some(rows[t].id(&frame))));
        processes::request_scroll(ctx, pane.0);
        took = true;
    }
    if let Some(i) = pos {
        let row = &rows[i];
        let id = row.id(&frame);
        if pressed(Modifiers::NONE, Key::ArrowLeft) {
            if row.has_children && !row.collapsed {
                cmds.push(MonitorCmd::ToggleCollapsed(id.clone()));
            } else if row.depth > 0 {
                // The parent: the nearest row above with a smaller depth.
                if let Some(parent) = rows[..i].iter().rev().find(|r| r.depth < row.depth) {
                    cmds.push(MonitorCmd::Select(Some(parent.id(&frame))));
                    processes::request_scroll(ctx, pane.0);
                }
            }
            took = true;
        }
        if pressed(Modifiers::NONE, Key::ArrowRight) {
            if row.has_children && row.collapsed {
                cmds.push(MonitorCmd::ToggleCollapsed(id.clone()));
            } else if row.has_children && i + 1 < n {
                cmds.push(MonitorCmd::Select(Some(rows[i + 1].id(&frame))));
                processes::request_scroll(ctx, pane.0);
            }
            took = true;
        }
        if pressed(Modifiers::NONE, Key::Space) {
            if let RowId::Proc(k) = id {
                cmds.push(MonitorCmd::ToggleMark(k));
            } else {
                cmds.push(MonitorCmd::ToggleCollapsed(id.clone()));
            }
            ctx.input_mut(|inp| inp.events.retain(|e| !matches!(e, Event::Text(t) if t == " ")));
            took = true;
        }
        if pressed(Modifiers::NONE, Key::Enter) {
            if row.has_children {
                cmds.push(MonitorCmd::ToggleCollapsed(id));
            } else if !cfg.details_panel {
                cmds.push(MonitorCmd::ToggleDetails);
            }
            took = true;
        }
    }
    if pressed(Modifiers::SHIFT, Key::Delete) {
        cmds.push(MonitorCmd::Signal { scope: SignalScope::Selection, sig: ProcSignal::Kill });
        took = true;
    } else if pressed(Modifiers::NONE, Key::Delete) {
        cmds.push(MonitorCmd::Signal { scope: SignalScope::Selection, sig: ProcSignal::Terminate });
        took = true;
    }
    if pressed(Modifiers::NONE, Key::Escape) {
        if marked_any {
            cmds.push(MonitorCmd::ClearMarks);
            took = true;
        } else if filter_any {
            cmds.push(MonitorCmd::SetFilter(String::new()));
            took = true;
        } else if selected.is_some() {
            cmds.push(MonitorCmd::Select(None));
            took = true;
        } else {
            // Nothing to clear here: give the key back to the window (toasts, overlays).
            ctx.input_mut(|i| {
                i.events.push(Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                });
            });
        }
    }
    if pressed(Modifiers::CTRL, Key::A) || pressed(Modifiers::COMMAND, Key::A) {
        // Mark every visible process.
        for r in rows {
            if let RowId::Proc(k) = r.id(&frame)
                && !marked.contains(&k)
            {
                cmds.push(MonitorCmd::ToggleMark(k));
            }
        }
        took = true;
    }
    // Type-ahead: a plain letter goes into the filter box.
    let typed: Option<String> = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Text(t) if t.chars().all(|c| c.is_alphanumeric()) && !t.is_empty() => Some(t.clone()),
            _ => None,
        })
    });
    if let Some(t) = typed
        && !ctx.input(|i| i.modifiers.any())
    {
        let mut text = filter_text;
        text.push_str(&t);
        app.core.ui.monitor.view(pane, cfg).focus_filter = true;
        cmds.push(MonitorCmd::SetFilter(text));
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, Event::Text(_))));
        took = true;
    }
    took
}

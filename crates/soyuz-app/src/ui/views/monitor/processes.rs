//! The Processes page: toolbar (view, filter, toggles), the virtualized process table
//! (list, tree with guides, groups with totals, heat tints, program icons, flashes for new
//! rows), its context menu, the details panel under a splitter, and the affinity editor.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align2, CursorIcon, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use soyuz_core::config::monitor::{CpuScale, ProcColumn, ProcView};
use soyuz_sysmon::{
    CellValue, Frame, ProcKey, ProcPriority, ProcSignal, ProcState, RowId, RowKind, TableRow, cell, cell_total,
};

use super::table::{Col, HeaderEvent, Sort, Table, row_bg};
use super::{PageCtx, details, filter_box, segmented_text, segmented_width};
use crate::models::monitor::cmd::CopyWhat;
use crate::models::monitor::{MonitorCmd, SignalScope, labels};
use crate::services::thumbs::{self, ThumbKey};
use crate::theme::font::semibold;
use crate::theme::tokens::radius;
use crate::theme::{Palette, font, icons as ticons, icons::ph};
use crate::ui::widgets::{Btn, icon_toggle, pill_button, rail_button, trunc_galley};

const ROW_ICON: f32 = 16.0;

fn toolbar_h() -> f32 {
    font::grow(40.0)
}

/// The flash of a row that appeared recently: `(key → time it appeared)`.
#[derive(Clone, Default)]
struct Flashes {
    generation: u64,
    added: HashMap<ProcKey, f64>,
}

/// Per-pane GUI state that is not worth a command.
#[derive(Clone, Default)]
struct Local {
    /// A column edge being dragged: `(column, width)`.
    live_width: Option<(ProcColumn, f32)>,
    /// The details splitter being dragged: the live height.
    live_details: Option<f32>,
    /// The row index the selection had last frame (follow on reorder).
    last_pos: Option<usize>,
    /// Keyboard moved the selection: scroll to it once.
    scroll_once: bool,
    /// Rows on one page (for PageUp / PageDown in `keys`).
    page_rows: usize,
    /// The row a context menu opened on.
    menu_key: Option<RowId>,
    /// The header a context menu opened on.
    menu_col: Option<usize>,
}

fn local_id(pane: u64) -> Id {
    Id::unique(("soyuz.monitor.procs.local", pane))
}

/// How many rows fit in the table (read by the keyboard handler).
pub fn page_rows(ctx: &egui::Context, pane: u64) -> usize {
    ctx.data(|d| d.get_temp::<Local>(local_id(pane))).map_or(20, |l| l.page_rows.max(1))
}

/// The keyboard moved the selection: scroll to it on the next frame.
pub fn request_scroll(ctx: &egui::Context, pane: u64) {
    let mut l: Local = ctx.data(|d| d.get_temp(local_id(pane))).unwrap_or_default();
    l.scroll_once = true;
    ctx.data_mut(|d| d.insert_temp(local_id(pane), l));
}

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>) {
    let p = c.p.clone();
    let pane = c.pane.0;
    let mut local: Local = ui.ctx().data(|d| d.get_temp(local_id(pane))).unwrap_or_default();
    let mut pending: Vec<MonitorCmd> = Vec::new();
    let Some(frame) = c.frame.clone() else {
        toolbar(ui, Rect::from_min_size(rect.min, vec2(rect.width(), toolbar_h())), c, &mut pending, None);
        super::empty(ui, rect, ph::LIST_BULLETS, tr!("monitor-no-data"), tr!("monitor-gui-waiting"));
        flush(c, pending);
        return;
    };
    let cfg = c.cfg().clone();
    let config = c.config;
    let generation = c.generation;
    let tb = Rect::from_min_size(rect.min, vec2(rect.width(), toolbar_h()));
    let error_h = if c.view().filter_error.is_some() { font::grow(18.0) } else { 0.0 };
    toolbar(ui, tb, c, &mut pending, Some(&frame));
    if let Some(e) = c.view().filter_error.clone() {
        let er = Rect::from_min_size(pos2(rect.left() + 14.0, tb.bottom()), vec2(rect.width() - 28.0, error_h));
        ui.painter().text(
            pos2(er.left() + 2.0, er.center().y),
            Align2::LEFT_CENTER,
            tr!("monitor-gui-filter-error", message = e.message.as_str()),
            font::ui(11.0),
            p.danger,
        );
    }
    let details_on = cfg.details_panel;
    let details_h = if details_on {
        local.live_details.unwrap_or(cfg.details_height).clamp(80.0, (rect.height() * 0.7).max(80.0))
    } else {
        0.0
    };
    let splitter_h = if details_on { 7.0 } else { 0.0 };
    let table_rect = Rect::from_min_max(
        pos2(rect.left(), tb.bottom() + error_h),
        pos2(rect.right(), rect.bottom() - details_h - splitter_h),
    );
    let details_rect = Rect::from_min_max(pos2(rect.left(), rect.bottom() - details_h), rect.max);

    // ---- rows ----
    let row_h = font::height(config.appearance.density.row_height(), 13.0);
    let (selected, marked, collapsed_n, me) = {
        let v = c.view();
        (v.selected.clone(), v.marked.clone(), v.collapsed.len(), v.me().map(str::to_string))
    };
    let ncpu = frame.ncpu();
    let now_ms = frame.at_ms;
    let total_mem = frame.memory.raw.total;
    let scale = cfg.cpu_scale;
    let view_kind = cfg.view;
    let heat_on = cfg.heat_cells;
    let icons_on = cfg.process_icons;
    let ppp = ui.ctx().pixels_per_point();
    let icon_px = thumbs::icon_px(ROW_ICON, ppp);
    let flashes = update_flashes(ui, pane, &frame, generation);
    let now = ui.input(|i| i.time);
    let cols: Vec<Col> = cfg
        .columns
        .iter()
        .map(|cc| {
            let w = match local.live_width {
                Some((col, w)) if col == cc.id => w,
                _ if cc.width > 0.0 => cc.width,
                _ => cc.id.default_width(),
            };
            let mut col = Col::new(labels::column_title(cc.id), font::grow(w)).tip(labels::column_desc(cc.id));
            if cc.id.numeric() {
                col = col.numeric();
            }
            col
        })
        .collect();
    let sort_i = cfg
        .columns
        .iter()
        .position(|cc| cc.id == cfg.sort_by)
        .map(|i| Sort { col: i, descending: cfg.sort_descending });
    let _ = collapsed_n;

    let (rows_len, sel_pos, out) = {
        let PageCtx { monitor, pane: pid, config, thumbs, focused, list_focus, .. } = c;
        let view = monitor.view(*pid, &config.monitor);
        let rows: &[TableRow] = view.rows(&frame, generation, &config.monitor);
        let sel_pos = selected.as_ref().and_then(|id| soyuz_sysmon::position_of(rows, &frame, id));
        let scroll_to = match (sel_pos, local.last_pos, local.scroll_once, cfg.follow_selected) {
            (Some(i), _, true, _) => Some(i),
            (Some(i), Some(last), _, true) if last != i => Some(i),
            _ => None,
        };
        let focused = *focused && *list_focus;
        let name_col = cfg.columns.iter().position(|cc| cc.id == ProcColumn::Name);
        let table = Table {
            sort: sort_i,
            menu_col: local.menu_col,
            reorder: true,
            scroll_to,
            name: tr!("monitor-page-processes").to_string(),
            ..Table::new(Id::unique(("soyuz.monitor.procs.table", pane)), &cols, rows.len(), row_h)
        };
        let out = table.show(ui, table_rect, &p, |rc| {
            let row = &rows[rc.index];
            let id = row.id(&frame);
            let is_sel = selected.as_ref() == Some(&id);
            row_bg(rc.painter, &p, rc.rect, is_sel, rc.hovered, focused);
            let dim = if row.matched { 1.0 } else { 0.45 };
            let a11y_name;
            match row.kind {
                RowKind::Group { ref label, count } => {
                    let name =
                        if label.is_empty() { tr!("monitor-gui-unknown-user").to_string() } else { label.to_string() };
                    a11y_name = format!("{name} ({count})");
                    if let Some(nc) = name_col {
                        let cell_r = rc.cell(nc);
                        let chev = if row.collapsed { ph::CARET_RIGHT } else { ph::CARET_DOWN };
                        rc.painter.text(
                            pos2(cell_r.left() + 6.0, cell_r.center().y),
                            Align2::CENTER_CENTER,
                            chev,
                            ticons::font(10.0),
                            p.text_secondary,
                        );
                        let g = trunc_galley(rc.painter, &name, semibold(12.5), p.text_primary, cell_r.width() - 60.0);
                        let gw = g.size().x;
                        rc.painter.galley(
                            pos2(cell_r.left() + 18.0, cell_r.center().y - g.size().y / 2.0),
                            g,
                            p.text_primary,
                        );
                        let count_s = labels::count(u64::from(count));
                        let cg = rc.painter.layout_no_wrap(count_s, font::numbers(10.5), p.text_tertiary);
                        let br = Rect::from_min_size(
                            pos2(cell_r.left() + 24.0 + gw, cell_r.center().y - 8.0),
                            vec2(cg.size().x + 10.0, 16.0),
                        );
                        rc.painter.rect_filled(br, crate::theme::tokens::radius::same(radius::PILL), p.bg_surface2);
                        rc.painter.galley(
                            pos2(br.left() + 5.0, br.center().y - cg.size().y / 2.0),
                            cg,
                            p.text_tertiary,
                        );
                    }
                    if let Some(t) = &row.totals {
                        for (ci, cc) in cfg.columns.iter().enumerate() {
                            if cc.id == ProcColumn::Name {
                                continue;
                            }
                            let v = cell_total(cc.id, t, ncpu, scale);
                            if matches!(v, CellValue::Missing) {
                                continue;
                            }
                            paint_value(rc, ci, cc.id, &v, &p, config, heat_on, total_mem, 0.85);
                        }
                    }
                }
                RowKind::Proc(_) => {
                    let Some(pr) = row.proc(&frame) else { return String::new() };
                    let raw = &pr.raw;
                    a11y_name = format!("{} {}", raw.name, raw.key.pid);
                    let flash = flashes
                        .added
                        .get(&raw.key)
                        .map(|t0| (1.0 - ((now - t0) / 1.2) as f32).clamp(0.0, 1.0))
                        .unwrap_or(0.0);
                    if flash > 0.0 {
                        rc.painter.rect_filled(
                            rc.rect.shrink2(vec2(4.0, 0.0)),
                            crate::theme::tokens::radius::same(radius::ROW),
                            p.success.gamma_multiply(0.18 * flash),
                        );
                    }
                    if marked.contains(&raw.key) {
                        let mr = Rect::from_min_size(
                            pos2(rc.rect.left() + 4.0, rc.rect.top() + 4.0),
                            vec2(3.0, rc.rect.height() - 8.0),
                        );
                        rc.painter.rect_filled(mr, crate::theme::tokens::radius::same(2), p.accent);
                    }
                    for (ci, cc) in cfg.columns.iter().enumerate() {
                        if cc.id == ProcColumn::Name {
                            paint_name(rc, ci, row, pr, &p, view_kind, dim, icons_on, icon_px, thumbs, &frame);
                            continue;
                        }
                        let v = match (&row.totals, row.collapsed) {
                            (Some(t), true) => {
                                let tv = cell_total(cc.id, t, ncpu, scale);
                                if matches!(tv, CellValue::Missing) { cell(cc.id, pr, now_ms, ncpu, scale) } else { tv }
                            }
                            _ => cell(cc.id, pr, now_ms, ncpu, scale),
                        };
                        paint_value(rc, ci, cc.id, &v, &p, config, heat_on, total_mem, dim);
                    }
                    if raw.state == ProcState::Suspended {
                        let cr = rc.cell(name_col.unwrap_or(0));
                        rc.painter.text(
                            pos2(cr.right() - 6.0, cr.center().y),
                            Align2::RIGHT_CENTER,
                            ph::PAUSE,
                            ticons::font(10.0),
                            p.warning,
                        );
                    }
                }
            }
            a11y_name
        });
        (rows.len(), sel_pos, out)
    };
    local.last_pos = sel_pos;
    local.scroll_once = false;
    local.page_rows = out.page_rows;
    let _ = rows_len;

    // ---- header events ----
    for e in &out.events {
        match *e {
            HeaderEvent::Sort(i) => {
                if let Some(cc) = cfg.columns.get(i) {
                    pending.push(MonitorCmd::SortBy(cc.id));
                }
            }
            HeaderEvent::Resize { col, width } => {
                if let Some(cc) = cfg.columns.get(col) {
                    local.live_width = Some((cc.id, width / font::growth()));
                }
            }
            HeaderEvent::ResizeDone { col, width } => {
                if let Some(cc) = cfg.columns.get(col) {
                    pending.push(MonitorCmd::SetColumnWidth(cc.id, (width / font::growth()).round()));
                }
                local.live_width = None;
            }
            HeaderEvent::Move { from, to } => {
                if let Some(cc) = cfg.columns.get(from) {
                    pending.push(MonitorCmd::MoveColumn(cc.id, to));
                }
            }
            HeaderEvent::Menu(ci) => local.menu_col = Some(ci),
        }
    }
    if let Some(hresp) = &out.menu {
        let shown: Vec<ProcColumn> = cfg.columns.iter().map(|x| x.id).collect();
        let sort_by = cfg.sort_by;
        let cmds_out = &mut pending;
        let opened = super::context_menu(hresp, |ui| {
            ui.set_width(240.0);
            crate::ui::widgets::menu_caption(ui, tr!("monitor-gui-columns"));
            egui::ScrollArea::vertical().max_height(font::grow(380.0)).show(ui, |ui| {
                for col in ProcColumn::ALL {
                    let on = shown.contains(&col);
                    let r = crate::ui::widgets::menu_check(ui, on, labels::column_title(col), None);
                    let r = r.on_hover_text(labels::column_desc(col));
                    if r.clicked() && col != ProcColumn::Name {
                        cmds_out.push(MonitorCmd::ToggleColumn(col));
                    }
                }
            });
            crate::ui::widgets::menu_separator(ui);
            if crate::ui::widgets::menu_item(
                ui,
                Some(ph::SLIDERS_HORIZONTAL),
                tr!("monitor-gui-choose-columns"),
                None,
                true,
            )
            .clicked()
            {
                cmds_out.push(MonitorCmd::ChooseColumns);
                ui.close();
            }
            if crate::ui::widgets::menu_item(
                ui,
                Some(ph::ARROW_COUNTER_CLOCKWISE),
                tr!("monitor-gui-reset-columns"),
                None,
                true,
            )
            .clicked()
            {
                cmds_out.push(MonitorCmd::ResetColumns);
                ui.close();
            }
            let _ = sort_by;
        });
        if opened.is_none() && !hresp.secondary_clicked() {
            local.menu_col = None;
        }
    }

    // ---- row clicks ----
    if let Some((i, mods)) = out.clicked {
        c.cmds.push(crate::cmd::Cmd::FocusPane(c.pane));
        if let Some((id, has_children, depth)) = row_at(c, &frame, generation, i) {
            let px = out.bg.interact_pointer_pos().map_or(f32::MAX, |q| q.x);
            let name_x = cfg.columns.iter().position(|cc| cc.id == ProcColumn::Name).map_or(f32::MAX, |nc| {
                // The chevron sits at the start of the name cell, after the indent.
                let x0 = out.bg.rect.left()
                    + 4.0
                    + cfg.columns[..nc]
                        .iter()
                        .map(|cc| if cc.width > 0.0 { cc.width } else { cc.id.default_width() })
                        .sum::<f32>();
                x0 + 8.0 + f32::from(depth) * 14.0
            });
            let on_chevron = has_children && (px - name_x).abs() < 12.0;
            if on_chevron {
                pending.push(MonitorCmd::ToggleCollapsed(id));
            } else if mods.command || mods.ctrl {
                if let RowId::Proc(k) = id {
                    pending.push(MonitorCmd::ToggleMark(k));
                }
                pending.push(MonitorCmd::Select(Some(id)));
            } else {
                pending.push(MonitorCmd::Select(Some(id)));
            }
        }
    }
    if let Some(i) = out.double_clicked
        && let Some((id, has_children, _)) = row_at(c, &frame, generation, i)
    {
        if has_children {
            pending.push(MonitorCmd::ToggleCollapsed(id));
        } else if !cfg.details_panel {
            pending.push(MonitorCmd::ToggleDetails);
        }
    }
    if let Some(i) = out.secondary {
        c.cmds.push(crate::cmd::Cmd::FocusPane(c.pane));
        if let Some((id, _, _)) = row_at(c, &frame, generation, i) {
            if selected.as_ref() != Some(&id) {
                pending.push(MonitorCmd::Select(Some(id.clone())));
            }
            local.menu_key = Some(id);
        }
    }
    if let Some(RowId::Proc(key)) = local.menu_key.clone()
        && let Some(pr) = frame.proc(key)
    {
        let marked_n = marked.len();
        let in_tree = view_kind == ProcView::Tree;
        let has_kids = frame.procs.iter().any(|q| q.raw.parent == Some(key.pid) && !q.raw.idle);
        let cmds_out = &mut pending;
        let details_on = cfg.details_panel;
        let me = me.clone();
        let opened = super::context_menu(&out.bg, |ui| {
            row_menu(ui, pr, marked_n, in_tree, has_kids, details_on, me.as_deref(), cmds_out);
        });
        if opened.is_none() && !out.bg.secondary_clicked() {
            local.menu_key = None;
        }
    } else if let Some(RowId::Group(label)) = local.menu_key.clone() {
        let cmds_out = &mut pending;
        let opened = super::context_menu(&out.bg, |ui| {
            ui.set_width(220.0);
            if crate::ui::widgets::menu_item(
                ui,
                Some(ph::ARROWS_IN_LINE_VERTICAL),
                tr!("monitor-gui-toggle-group"),
                None,
                true,
            )
            .clicked()
            {
                cmds_out.push(MonitorCmd::ToggleCollapsed(RowId::Group(label.clone())));
                ui.close();
            }
            if crate::ui::widgets::menu_item(
                ui,
                Some(ph::ARROWS_OUT_LINE_VERTICAL),
                tr!("monitor-gui-expand-all"),
                None,
                true,
            )
            .clicked()
            {
                cmds_out.push(MonitorCmd::ExpandAll);
                ui.close();
            }
            if crate::ui::widgets::menu_item(
                ui,
                Some(ph::ARROWS_IN_LINE_VERTICAL),
                tr!("monitor-gui-collapse-all"),
                None,
                true,
            )
            .clicked()
            {
                cmds_out.push(MonitorCmd::CollapseAll);
                ui.close();
            }
        });
        if opened.is_none() && !out.bg.secondary_clicked() {
            local.menu_key = None;
        }
    }

    // ---- details ----
    if details_on {
        let sr = Rect::from_min_max(pos2(rect.left(), table_rect.bottom()), pos2(rect.right(), details_rect.top()));
        let sresp = ui.interact(sr, Id::unique(("soyuz.monitor.details.split", pane)), Sense::click_and_drag());
        crate::ui::a11y::splitter(&sresp, tr!("monitor-gui-resize-details"));
        ui.painter().hline(sr.x_range(), sr.center().y.round() + 0.5, Stroke::new(1.0, p.border_subtle));
        if sresp.hovered() || sresp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
            let bar = Rect::from_center_size(sr.center(), vec2(40.0, 2.0));
            ui.painter().rect_filled(bar, crate::theme::tokens::radius::same(1), p.accent.gamma_multiply(0.8));
        }
        if sresp.dragged() {
            let h = local.live_details.unwrap_or(cfg.details_height) - sresp.drag_delta().y;
            local.live_details = Some(h.clamp(80.0, 900.0));
        }
        if sresp.drag_stopped()
            && let Some(h) = local.live_details.take()
        {
            pending.push(MonitorCmd::SetDetailsHeight(h));
        }
        if sresp.double_clicked() {
            pending.push(MonitorCmd::ToggleDetails);
        }
        details::show(ui, details_rect, c, &frame, &mut pending);
    }

    // ---- affinity editor ----
    if let Some(edit) = c.view().affinity.clone() {
        affinity_modal(ui, pane, &edit, &mut pending);
    }

    ui.ctx().data_mut(|d| d.insert_temp(local_id(pane), local));
    flush(c, pending);
}

/// Identity, children flag and depth of row `i` of the table as built for `frame`.
fn row_at(c: &mut PageCtx<'_>, frame: &Frame, generation: u64, i: usize) -> Option<(RowId, bool, u16)> {
    let PageCtx { monitor, pane, config, .. } = c;
    let view = monitor.view(*pane, &config.monitor);
    let rows = view.rows(frame, generation, &config.monitor);
    rows.get(i).map(|r| (r.id(frame), r.has_children, r.depth))
}

fn flush(c: &mut PageCtx<'_>, pending: Vec<MonitorCmd>) {
    for cmd in pending {
        c.cmd(cmd);
    }
}

fn update_flashes(ui: &Ui, pane: u64, frame: &Frame, generation: u64) -> Flashes {
    let id = Id::unique(("soyuz.monitor.procs.flash", pane));
    let now = ui.input(|i| i.time);
    let mut f: Flashes = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    if f.generation != generation {
        // The first frame shown lists every process as new: no flash storm.
        if f.generation != 0 {
            for k in &frame.added {
                f.added.insert(*k, now);
            }
        }
        f.generation = generation;
    }
    f.added.retain(|_, t| now - *t < 1.3);
    if !f.added.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, f.clone()));
    f
}

#[allow(clippy::too_many_arguments)]
fn paint_name(
    rc: &super::table::RowCtx<'_>,
    ci: usize,
    row: &TableRow,
    pr: &soyuz_sysmon::ProcRow,
    p: &Palette,
    view: ProcView,
    dim: f32,
    icons_on: bool,
    icon_px: u32,
    thumbs: &crate::services::thumbs::Thumbs,
    frame: &Frame,
) {
    let cell_r = rc.cell(ci);
    let cy = cell_r.center().y;
    let indent = 14.0;
    let mut x = cell_r.left();
    if view == ProcView::Tree {
        // Guide lines from the ancestors that still have siblings below.
        let guide = Stroke::new(1.0, p.border_subtle);
        for d in 0..row.depth {
            let gx = (cell_r.left() + 6.0 + f32::from(d) * indent).round() + 0.5;
            if row.ancestors_last & (1u64 << d.min(63)) == 0 {
                rc.painter.vline(gx, rc.rect.y_range(), guide);
            }
        }
        if row.depth > 0 {
            let gx = (cell_r.left() + 6.0 + f32::from(row.depth - 1) * indent).round() + 0.5;
            let y_end = if row.last_sibling { cy.round() + 0.5 } else { rc.rect.bottom() };
            rc.painter.vline(gx, rc.rect.top()..=y_end, guide);
            rc.painter.hline(gx..=(gx + 8.0), cy.round() + 0.5, guide);
        }
        x += f32::from(row.depth) * indent;
        if row.has_children {
            let chev = if row.collapsed { ph::CARET_RIGHT } else { ph::CARET_DOWN };
            rc.painter.text(
                pos2(x + 6.0, cy),
                Align2::CENTER_CENTER,
                chev,
                ticons::font(10.0),
                p.text_secondary.gamma_multiply(dim),
            );
        }
        x += 14.0;
    } else if view != ProcView::List {
        x += 18.0;
    }
    // Icon.
    let icon_c = pos2(x + ROW_ICON / 2.0, cy);
    let drew_icon = icons_on
        && pr.raw.exe.as_deref().is_some_and(thumbs::has_program_icon)
        && pr
            .raw
            .exe
            .as_deref()
            .and_then(|exe| {
                thumbs.get_program_icon(&ThumbKey { path: exe.to_path_buf(), mtime: None, size: 0, px: icon_px })
            })
            .is_some_and(|tex| {
                crate::ui::views::flow::paint_program_icon(rc.painter, icon_c, ROW_ICON, &tex, dim);
                true
            });
    if !drew_icon {
        let (glyph, color) = if pr.raw.kernel {
            (ph::CIRCUITRY, p.text_tertiary)
        } else if pr.raw.elevated == Some(true) {
            (ph::SHIELD_CHECK, p.glyph_exe)
        } else {
            (ph::CUBE, p.text_secondary)
        };
        rc.painter.text(icon_c, Align2::CENTER_CENTER, glyph, ticons::font(13.0), color.gamma_multiply(dim));
    }
    x += ROW_ICON + 7.0;
    let color = p.text_primary.gamma_multiply(dim);
    let max_w = cell_r.right() - x - 2.0;
    let name = pr.raw.name.as_ref();
    let g = trunc_galley(rc.painter, name, font::ui(12.5), color, max_w);
    let gw = g.size().x;
    rc.painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
    // Collapsed subtree: how many are folded away.
    if row.collapsed
        && let Some(t) = &row.totals
        && t.count > 1
        && max_w - gw > 40.0
    {
        let s = format!("+{}", t.count.saturating_sub(1));
        let cg = rc.painter.layout_no_wrap(s, font::numbers(10.0), p.text_tertiary);
        rc.painter.galley(pos2(x + gw + 6.0, cy - cg.size().y / 2.0), cg, p.text_tertiary);
    }
    let _ = frame;
}

#[allow(clippy::too_many_arguments)]
fn paint_value(
    rc: &super::table::RowCtx<'_>,
    ci: usize,
    col: ProcColumn,
    v: &CellValue,
    p: &Palette,
    config: &soyuz_core::config::Config,
    heat_on: bool,
    total_mem: u64,
    dim: f32,
) {
    let cell_r = rc.cell(ci);
    if cell_r.width() < 6.0 {
        return;
    }
    let heat = if heat_on { labels::cell_heat(col, v, total_mem) } else { None };
    if let Some(h) = heat {
        let full = Rect::from_min_max(
            pos2(cell_r.left() - 6.0, rc.rect.top() + 1.0),
            pos2(cell_r.right() + 6.0, rc.rect.bottom() - 1.0),
        );
        rc.painter.rect_filled(
            full,
            crate::theme::tokens::radius::same(3),
            super::charts::heat(p, h).gamma_multiply(dim),
        );
        // A tiny bar when the column is wide: the share read at a glance, not only by tint.
        if cell_r.width() >= 96.0
            && matches!(col, ProcColumn::Cpu | ProcColumn::Gpu | ProcColumn::MemoryPercent | ProcColumn::Memory)
        {
            let bar = Rect::from_min_size(
                pos2(cell_r.left(), cell_r.bottom() - 4.0),
                vec2((cell_r.width() * 0.45 * h).max(1.5), 2.0),
            );
            rc.painter.rect_filled(bar, crate::theme::tokens::radius::same(1), p.accent.gamma_multiply(0.7 * dim));
        }
    }
    let text = labels::cell_text(v, config);
    let (font, color) = match v {
        CellValue::Missing => (font::numbers(12.0), p.text_tertiary.gamma_multiply(dim)),
        CellValue::Text(_) | CellValue::State(_) | CellValue::Priority(_) | CellValue::Bool(_) => {
            (font::ui(12.0), p.text_secondary.gamma_multiply(dim))
        }
        _ => (font::numbers(12.0), p.text_primary.gamma_multiply(dim)),
    };
    let color = if matches!(v, CellValue::State(ProcState::Suspended | ProcState::Stopped)) {
        p.warning.gamma_multiply(dim)
    } else {
        color
    };
    rc.text(ci, &text, col.numeric(), font, color);
}

#[allow(clippy::too_many_arguments)]
fn row_menu(
    ui: &mut Ui,
    pr: &soyuz_sysmon::ProcRow,
    marked_n: usize,
    in_tree: bool,
    has_kids: bool,
    details_on: bool,
    me: Option<&str>,
    out: &mut Vec<MonitorCmd>,
) {
    use crate::ui::widgets::{menu_caption, menu_check, menu_item, menu_separator, submenu};
    ui.set_width(250.0);
    let key = pr.raw.key;
    let risk = soyuz_sysmon::risk(&pr.raw, me);
    let title = if marked_n > 1 {
        tr!("monitor-gui-marked-count", count = marked_n)
    } else {
        format!("{}  ·  {}", pr.raw.name, key.pid)
    };
    menu_caption(ui, &title);
    let sel = SignalScope::Selection;
    let sig = |ui: &mut Ui, out: &mut Vec<MonitorCmd>, glyph, label, scope, s: ProcSignal, enabled: bool| {
        if menu_item(ui, Some(glyph), label, None, enabled).clicked() {
            out.push(MonitorCmd::Signal { scope, sig: s });
            ui.close();
        }
    };
    sig(ui, out, ph::X_CIRCLE, labels::signal(ProcSignal::Terminate), sel, ProcSignal::Terminate, true);
    sig(ui, out, ph::SKULL, labels::signal(ProcSignal::Kill), sel, ProcSignal::Kill, true);
    if has_kids || in_tree {
        sig(
            ui,
            out,
            ph::TREE_STRUCTURE,
            tr!("monitor-gui-end-tree"),
            SignalScope::Tree,
            ProcSignal::Terminate,
            has_kids,
        );
        sig(ui, out, ph::TREE_STRUCTURE, tr!("monitor-gui-kill-tree"), SignalScope::Tree, ProcSignal::Kill, has_kids);
    }
    if pr.raw.state == ProcState::Suspended || pr.raw.state == ProcState::Stopped {
        sig(ui, out, ph::PLAY, labels::signal(ProcSignal::Resume), sel, ProcSignal::Resume, true);
    } else {
        let can = !soyuz_sysmon::refuses(&pr.raw, ProcSignal::Suspend);
        let r = menu_item(ui, Some(ph::PAUSE), labels::signal(ProcSignal::Suspend), None, can);
        let r = if can { r } else { r.on_hover_text(tr!("monitor-risk-critical")) };
        if r.clicked() {
            out.push(MonitorCmd::Signal { scope: sel, sig: ProcSignal::Suspend });
            ui.close();
        }
    }
    if cfg!(unix) {
        sig(ui, out, ph::ARROW_U_UP_LEFT, labels::signal(ProcSignal::Hangup), sel, ProcSignal::Hangup, true);
        sig(ui, out, ph::LIGHTNING, labels::signal(ProcSignal::Interrupt), sel, ProcSignal::Interrupt, true);
    }
    menu_separator(ui);
    submenu(ui, Some(ph::GAUGE), tr!("monitor-gui-priority"), |ui| {
        ui.set_width(200.0);
        for pri in [
            ProcPriority::Realtime,
            ProcPriority::High,
            ProcPriority::AboveNormal,
            ProcPriority::Normal,
            ProcPriority::BelowNormal,
            ProcPriority::Idle,
        ] {
            let on = pr.raw.priority == Some(pri);
            if menu_check(ui, on, labels::priority(pri), None).clicked() {
                out.push(MonitorCmd::SetPriority(key, pri));
                ui.close();
            }
        }
    });
    if menu_item(ui, Some(ph::CPU), tr!("monitor-gui-affinity"), None, !cfg!(target_os = "macos")).clicked() {
        out.push(MonitorCmd::EditAffinity(key));
        ui.close();
    }
    menu_separator(ui);
    if menu_item(ui, Some(ph::FOLDER_OPEN), tr!("monitor-gui-open-location"), None, pr.raw.exe.is_some()).clicked() {
        out.push(MonitorCmd::OpenLocation(key));
        ui.close();
    }
    submenu(ui, Some(ph::COPY), tr!("common-copy"), |ui| {
        ui.set_width(200.0);
        for (what, label, on) in [
            (CopyWhat::Pid, tr!("monitor-gui-copy-pid"), true),
            (CopyWhat::Name, tr!("monitor-gui-copy-name"), true),
            (CopyWhat::Path, tr!("monitor-gui-copy-path"), pr.raw.exe.is_some()),
            (CopyWhat::Command, tr!("monitor-gui-copy-command"), pr.raw.cmdline.is_some()),
        ] {
            if menu_item(ui, None, label, None, on).clicked() {
                out.push(MonitorCmd::CopyProcess(key, what));
                ui.close();
            }
        }
    });
    menu_separator(ui);
    if menu_check(ui, details_on, tr!("monitor-gui-details"), None).clicked() {
        out.push(MonitorCmd::ToggleDetails);
        ui.close();
    }
    if risk != soyuz_sysmon::ProcRisk::Normal {
        menu_separator(ui);
        let note = if risk == soyuz_sysmon::ProcRisk::Critical {
            tr!("monitor-risk-critical")
        } else {
            tr!("monitor-risk-other-user")
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(note).font(font::ui(11.0)).color(crate::theme::palette(ui.ctx()).warning),
            )
            .wrap(),
        );
    }
}

fn toolbar(ui: &mut Ui, tb: Rect, c: &mut PageCtx<'_>, pending: &mut Vec<MonitorCmd>, frame: Option<&Arc<Frame>>) {
    let p = c.p.clone();
    let cfg = c.cfg().clone();
    let pane = c.pane.0;
    let inner = Rect::from_min_max(pos2(tb.left() + 14.0, tb.top() + 6.0), pos2(tb.right() - 12.0, tb.bottom() - 6.0));
    let wide = inner.width() >= font::grow(900.0);
    let (filter_text, filter_regex, focus_filter) = {
        let v = c.view();
        let f = v.focus_filter;
        v.focus_filter = false;
        (v.filter_text.clone(), v.filter_regex, f)
    };
    // Right side: toggles and tools.
    let right = Rect::from_min_max(pos2(inner.left(), inner.top()), inner.max);
    let mut bar = super::tool_row(ui, right);
    let mut right_w = 0.0;
    let measure = |r: &egui::Response, right_w: &mut f32| {
        *right_w = right.right() - r.rect.left();
    };
    let r = rail_button(&mut bar, ph::SLIDERS_HORIZONTAL, tr!("monitor-gui-choose-columns"), true, false);
    if r.clicked() {
        pending.push(MonitorCmd::ChooseColumns);
    }
    measure(&r, &mut right_w);
    let r = rail_button(&mut bar, ph::EXPORT, tr!("monitor-gui-export-csv"), frame.is_some(), false);
    if r.clicked() {
        pending.push(MonitorCmd::ExportProcesses);
    }
    measure(&r, &mut right_w);
    let r = rail_button(&mut bar, ph::INFO, tr!("monitor-gui-details"), true, cfg.details_panel);
    if r.clicked() {
        pending.push(MonitorCmd::ToggleDetails);
    }
    measure(&r, &mut right_w);
    crate::ui::widgets::vdivider(&mut bar, 18.0);
    let r = icon_toggle(&mut bar, ph::FIRE, tr!("monitor-gui-heat"), true, cfg.heat_cells);
    if r.clicked() {
        pending.push(MonitorCmd::ToggleHeat);
    }
    let scale_tip = if cfg.cpu_scale == CpuScale::PerCore {
        tr!("monitor-gui-scale-per-core")
    } else {
        tr!("monitor-gui-scale-machine")
    };
    let r = icon_toggle(&mut bar, ph::PERCENT, scale_tip, true, cfg.cpu_scale == CpuScale::PerCore);
    if r.clicked() {
        pending.push(MonitorCmd::SetCpuScale(if cfg.cpu_scale == CpuScale::PerCore {
            CpuScale::Machine
        } else {
            CpuScale::PerCore
        }));
    }
    let r = icon_toggle(&mut bar, ph::USERS, tr!("monitor-gui-other-users"), true, cfg.show_other_users);
    if r.clicked() {
        pending.push(MonitorCmd::ToggleOtherUsers);
    }
    let r = icon_toggle(&mut bar, ph::CIRCUITRY, tr!("monitor-gui-kernel"), true, cfg.show_kernel);
    if r.clicked() {
        pending.push(MonitorCmd::ToggleKernel);
    }
    measure(&r, &mut right_w);
    // Left side: the view switch.
    let views = [ProcView::List, ProcView::Tree, ProcView::ByName, ProcView::ByUser];
    let labels_v: Vec<&str> = views.iter().map(|v| labels::view_title(*v)).collect();
    let seg_w = segmented_width(ui, &labels_v);
    let sel = views.iter().position(|v| *v == cfg.view).unwrap_or(0);
    let seg_r = Rect::from_min_size(inner.min, vec2(seg_w, inner.height()));
    let seg_fits = inner.width() - right_w - seg_w > 160.0;
    let x = if seg_fits {
        if let Some(i) = segmented_text(ui, seg_r, Id::unique(("soyuz.monitor.procs.view", pane)), &labels_v, sel) {
            pending.push(MonitorCmd::SetView(views[i]));
        }
        seg_r.right() + 10.0
    } else {
        // Icon segmented control in narrow panes.
        let items = [
            (ph::LIST_BULLETS, labels_v[0]),
            (ph::TREE_STRUCTURE, labels_v[1]),
            (ph::APP_WINDOW, labels_v[2]),
            (ph::USERS, labels_v[3]),
        ];
        let mut child =
            ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(inner.min, vec2(130.0, inner.height()))));
        if let Some(i) = crate::ui::widgets::segmented(&mut child, &items, sel, vec2(28.0, inner.height() - 4.0)) {
            pending.push(MonitorCmd::SetView(views[i]));
        }
        inner.left() + 4.0 * 28.0 + 14.0
    };
    // Filter with the regex toggle and the match count.
    let count_text = match frame {
        Some(f) => {
            let n = {
                let PageCtx { monitor, pane: pid, config, generation, .. } = &mut *c;
                monitor.view(*pid, &config.monitor).rows(f, *generation, &config.monitor).len()
            };
            labels::count(n as u64)
        }
        None => String::new(),
    };
    let count_g = ui.painter().layout_no_wrap(count_text.clone(), font::numbers(11.5), p.text_tertiary);
    let count_w = if count_text.is_empty() { 0.0 } else { count_g.size().x + 10.0 };
    let regex_w = 30.0;
    let filter_right = inner.right() - right_w - 10.0;
    let fw = (filter_right - x - regex_w - count_w - 8.0).max(80.0);
    let fr = Rect::from_min_size(
        pos2(x, inner.top() + 1.0),
        vec2(fw.min(if wide { 520.0 } else { 380.0 }), inner.height() - 2.0),
    );
    let hint = if filter_regex { tr!("monitor-gui-filter-regex-hint") } else { tr!("monitor-gui-filter-hint") };
    let (changed, fresp) =
        filter_box(ui, fr, Id::unique(("soyuz.monitor.procs.filter", pane)), &filter_text, hint, focus_filter);
    if let Some(t) = changed {
        pending.push(MonitorCmd::SetFilter(t));
    }
    if fresp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        if filter_text.is_empty() {
            fresp.surrender_focus();
        } else {
            pending.push(MonitorCmd::SetFilter(String::new()));
        }
    }
    let fresp = fresp.on_hover_text(tr!("monitor-gui-filter-help"));
    let _ = fresp;
    // `.*` toggle.
    let rr = Rect::from_min_size(pos2(fr.right() + 4.0, fr.top()), vec2(regex_w - 4.0, fr.height()));
    let rresp = ui.interact(rr, Id::unique(("soyuz.monitor.procs.regex", pane)), Sense::click());
    crate::ui::a11y::toggle(&rresp, tr!("monitor-gui-regex"), filter_regex);
    let cr = crate::theme::tokens::radius::same(radius::ROW);
    if filter_regex {
        ui.painter().rect_filled(rr, cr, p.accent_subtle);
    } else if rresp.hovered() {
        ui.painter().rect_filled(rr, cr, p.bg_surface2);
    }
    let rc = if filter_regex { p.accent } else { p.text_tertiary };
    ui.painter().text(rr.center(), Align2::CENTER_CENTER, ".*", font::mono(12.0), rc);
    if rresp.on_hover_text(tr!("monitor-gui-regex")).clicked() {
        pending.push(MonitorCmd::ToggleFilterRegex);
    }
    if count_w > 0.0 {
        ui.painter().galley(
            pos2(rr.right() + 8.0, inner.center().y - count_g.size().y / 2.0),
            count_g,
            p.text_tertiary,
        );
    }
    let _ = wide;
}

/// The affinity editor: a grid of logical CPUs.
fn affinity_modal(ui: &mut Ui, pane: u64, edit: &crate::models::monitor::AffinityEdit, pending: &mut Vec<MonitorCmd>) {
    let ctx = ui.ctx().clone();
    let p = crate::theme::palette(&ctx);
    let frame = egui::Frame::new()
        .fill(p.bg_surface3)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(crate::theme::tokens::radius::same(radius::MODAL))
        .inner_margin(egui::Margin::same(20))
        .shadow(p.shadow(crate::theme::tokens::shadow::WINDOW));
    let mut close = false;
    let resp = egui::Modal::new(Id::unique(("soyuz.monitor.affinity", pane))).frame(frame).show(&ctx, |ui| {
        ui.set_width(380.0);
        crate::ui::a11y::dialog(ui.ctx(), ui.unique_id(), tr!("monitor-gui-affinity-title"), false, true);
        ui.horizontal(|ui| {
            crate::ui::widgets::icon_label(ui, ticons::icon(ph::CPU, 18.0).color(p.accent));
            ui.add_space(2.0);
            ui.label(egui::RichText::new(tr!("monitor-gui-affinity-title")).font(semibold(16.0)).color(p.text_primary));
        });
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(tr!("monitor-gui-affinity-for", name = edit.name.as_str(), pid = edit.key.pid))
                .color(p.text_secondary),
        );
        ui.add_space(12.0);
        let n = edit.cpus.len();
        let per_row = if n > 32 { 8 } else { 4 };
        egui::Grid::new(("soyuz.monitor.affinity.grid", pane)).num_columns(per_row).spacing([10.0, 6.0]).show(
            ui,
            |ui| {
                for (i, on) in edit.cpus.iter().enumerate() {
                    let mut v = *on;
                    if ui.checkbox(&mut v, format!("CPU {i}")).changed() {
                        pending.push(MonitorCmd::ToggleAffinityCpu(i));
                    }
                    if (i + 1) % per_row == 0 {
                        ui.end_row();
                    }
                }
            },
        );
        ui.add_space(8.0);
        let on_n = edit.cpus.iter().filter(|x| **x).count();
        ui.label(
            egui::RichText::new(tr!("monitor-gui-affinity-count", on = on_n, total = n))
                .font(font::ui(11.5))
                .color(p.text_tertiary),
        );
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if pill_button(ui, None, tr!("monitor-gui-apply"), Btn::Primary).clicked() {
                pending.push(MonitorCmd::ApplyAffinity);
                close = true;
            }
            ui.add_space(6.0);
            if pill_button(ui, None, tr!("common-cancel"), Btn::Secondary).clicked() {
                close = true;
                pending.push(MonitorCmd::CancelAffinity);
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
            pending.push(MonitorCmd::CancelAffinity);
        }
    });
    if resp.should_close() && !close {
        pending.push(MonitorCmd::CancelAffinity);
    }
}

//! The Snapshots page: the saved snapshots (take, rename, delete, export), and comparing
//! two of them, or one with now: totals diff cards and a sortable table of added, removed
//! and changed processes, each change marked by an icon and a word, never by colour alone.

use std::sync::Arc;

use egui::{Align2, Id, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use soyuz_sysmon::snapshot::SnapshotMeta;
use soyuz_sysmon::{Change, Comparison, DiffSort, MatchBy};

use super::table::{Col, HeaderEvent, Sort, Table, row_bg};
use super::{PageCtx, badge, caption, card, filter_box, segmented_text};
use crate::models::monitor::{CompareTarget, MonitorCmd, labels};
use crate::theme::font::semibold;
use crate::theme::tokens::radius;
use crate::theme::{Palette, font, icons as ticons, icons::ph};
use crate::ui::widgets::dropdown::{Dropdown, Item};
use crate::ui::widgets::{Btn, chip, menu_item, menu_separator, pill_button, pill_button_on, spinner};

const GAP: f32 = 12.0;

fn label_id(pane: u64) -> Id {
    Id::unique(("soyuz.monitor.snap.label", pane))
}

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>) {
    let p = c.p.clone();
    let pane = c.pane.0;
    let config = c.config;
    let style = c.style();
    let mut pending: Vec<MonitorCmd> = Vec::new();
    let (
        metas,
        cursor,
        a,
        b,
        match_by,
        sort,
        descending,
        show_same,
        filter,
        comparison,
        shown,
        comparing,
        error,
        renaming,
    ) = {
        let s = &c.view().snapshots;
        (
            s.metas.clone(),
            s.cursor,
            s.a.clone(),
            s.b.clone(),
            s.match_by,
            s.sort,
            s.descending,
            s.show_same,
            s.filter.clone(),
            s.comparison.clone(),
            s.shown.clone(),
            s.comparing,
            s.error.clone(),
            s.renaming.clone(),
        )
    };
    let wide = rect.width() >= font::grow(960.0);
    let list_w = if wide { font::grow(320.0) } else { rect.width() };
    let list_rect = if wide {
        Rect::from_min_max(rect.min, pos2(rect.left() + list_w, rect.bottom()))
    } else {
        Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + font::grow(230.0)))
    };
    let cmp_rect = if wide {
        Rect::from_min_max(pos2(list_rect.right(), rect.top()), rect.max)
    } else {
        Rect::from_min_max(pos2(rect.left(), list_rect.bottom()), rect.max)
    };
    snapshot_list(
        ui,
        list_rect,
        &p,
        pane,
        &metas,
        cursor,
        a.as_deref(),
        &b,
        renaming.as_ref(),
        config,
        c.frame.is_some(),
        &mut pending,
    );
    if wide {
        ui.painter().vline(list_rect.right() - 0.5, list_rect.y_range(), Stroke::new(1.0, p.border_subtle));
    } else {
        ui.painter().hline(rect.x_range(), list_rect.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
    }
    compare_panel(
        ui,
        cmp_rect,
        &p,
        pane,
        &metas,
        a.as_deref(),
        &b,
        match_by,
        sort,
        descending,
        show_same,
        &filter,
        comparison.as_deref(),
        &shown,
        comparing,
        error.as_deref(),
        style,
        config,
        c.focused && c.list_focus,
        &mut pending,
    );
    for cmd in pending {
        c.cmd(cmd);
    }
}

#[allow(clippy::too_many_arguments)]
fn snapshot_list(
    ui: &mut Ui,
    rect: Rect,
    p: &Palette,
    pane: u64,
    metas: &[SnapshotMeta],
    cursor: usize,
    a: Option<&str>,
    b: &CompareTarget,
    renaming: Option<&(String, String)>,
    config: &soyuz_core::config::Config,
    have_frame: bool,
    pending: &mut Vec<MonitorCmd>,
) {
    let inner = rect.shrink2(vec2(14.0, 0.0));
    // Take a snapshot: label field and button.
    let tb = Rect::from_min_size(pos2(inner.left(), inner.top() + 6.0), vec2(inner.width(), font::grow(28.0)));
    let mut label: String = ui.ctx().data(|d| d.get_temp(label_id(pane))).unwrap_or_default();
    let btn_w = font::grow(96.0);
    let field = Rect::from_min_max(tb.min, pos2(tb.right() - btn_w - 8.0, tb.bottom()));
    let (changed, fresp) = filter_box(
        ui,
        field,
        Id::unique(("soyuz.monitor.snap.labelbox", pane)),
        &label,
        tr!("monitor-gui-snapshot-label-hint"),
        false,
    );
    if let Some(t) = changed {
        label = t;
    }
    let enter = fresp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(field.right() + 8.0, tb.top()), tb.max))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    if pill_button_on(&mut bar, Some(ph::CAMERA), tr!("monitor-gui-take"), Btn::Primary, have_frame).clicked()
        || (enter && have_frame)
    {
        pending.push(MonitorCmd::TakeSnapshot(label.clone()));
        label.clear();
    }
    ui.ctx().data_mut(|d| d.insert_temp(label_id(pane), label));
    // The list.
    let body = Rect::from_min_max(pos2(rect.left(), tb.bottom() + 8.0), rect.max);
    if metas.is_empty() {
        super::empty(ui, body, ph::CAMERA, tr!("monitor-gui-no-snapshots"), tr!("monitor-gui-no-snapshots-sub"));
        return;
    }
    let row_h = font::grow(54.0);
    let mut child = ui.new_child(UiBuilder::new().max_rect(body));
    let mut clicked: Option<usize> = None;
    let mut menu_on: Option<(usize, egui::Response)> = None;
    ScrollArea::vertical().id_salt(("soyuz.monitor.snap.list", pane)).auto_shrink([false, false]).show_rows(
        &mut child,
        row_h,
        metas.len(),
        |ui, range| {
            for i in range {
                let m = &metas[i];
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                let r = Rect::from_min_max(pos2(r.left() + 8.0, r.top()), pos2(r.right() - 8.0, r.bottom() - 4.0));
                let painter = ui.painter_at(r.expand(2.0));
                let is_cur = i == cursor;
                let cr = crate::theme::tokens::radius::same(radius::CARD);
                painter.rect_filled(
                    r,
                    cr,
                    if is_cur {
                        p.bg_surface2
                    } else if resp.hovered() {
                        p.bg_surface2.gamma_multiply(0.6)
                    } else {
                        p.bg_window
                    },
                );
                painter.rect_stroke(
                    r,
                    cr,
                    Stroke::new(1.0, if is_cur { p.accent.gamma_multiply(0.6) } else { p.border_subtle }),
                    egui::StrokeKind::Inside,
                );
                let x = r.left() + 12.0;
                let rename_here = renaming.filter(|(id, _)| *id == m.id);
                if let Some((_, text)) = rename_here {
                    let mut t = text.clone();
                    let field = Rect::from_min_size(pos2(x - 4.0, r.top() + 6.0), vec2(r.width() - 16.0, 22.0));
                    let mut fchild = ui.new_child(
                        UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let rid = Id::unique(("soyuz.monitor.snap.rename", pane));
                    let fr = fchild.add(
                        egui::TextEdit::singleline(&mut t)
                            .id(rid)
                            .desired_width(field.width())
                            .font(semibold(12.5))
                            .margin(egui::Margin::symmetric(4, 2)),
                    );
                    if !fr.has_focus() && !ui.memory(|mm| mm.had_focus_last_frame(rid)) {
                        fr.request_focus();
                    }
                    if fr.changed() {
                        pending.push(MonitorCmd::RenameText(t.clone()));
                    }
                    if ui.input(|inp| inp.key_pressed(egui::Key::Enter)) {
                        pending.push(MonitorCmd::CommitRename);
                    } else if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                        pending.push(MonitorCmd::CancelRename);
                    }
                } else {
                    let g = crate::ui::widgets::trunc_galley(
                        &painter,
                        &m.label,
                        semibold(12.5),
                        p.text_primary,
                        r.width() - 24.0 - if m.auto { 44.0 } else { 0.0 },
                    );
                    painter.galley(pos2(x, r.top() + 8.0), g, p.text_primary);
                    let mut bx = r.right() - 12.0;
                    if m.auto {
                        let w = badge(&painter, p, pos2(bx - 44.0, r.top() + 8.0), tr!("monitor-gui-auto"), p.warning);
                        bx -= w + 6.0;
                    }
                    if a == Some(m.id.as_str()) {
                        let w = badge(&painter, p, pos2(bx - 20.0, r.top() + 8.0), "A", p.accent);
                        bx -= w + 6.0;
                    }
                    if *b == CompareTarget::Snapshot(m.id.clone()) {
                        badge(&painter, p, pos2(bx - 20.0, r.top() + 8.0), "B", p.accent);
                    }
                }
                let line = format!(
                    "{}  ·  {}  ·  {} {}  ·  {} {}",
                    labels::time(m.taken_ms, config),
                    m.hostname,
                    labels::count(u64::from(m.procs)),
                    tr!("monitor-gui-procs-short"),
                    labels::percent(m.cpu_percent),
                    tr!("monitor-gui-cpu-short")
                );
                let lg = crate::ui::widgets::trunc_galley(
                    &painter,
                    &line,
                    font::ui(10.5),
                    p.text_tertiary,
                    r.width() - 24.0,
                );
                painter.galley(pos2(x, r.bottom() - 20.0), lg, p.text_tertiary);
                crate::ui::a11y::list_item(
                    ui,
                    Id::unique(("soyuz.monitor.snap.row", pane, i)),
                    r,
                    &format!("{} {}", m.label, labels::time(m.taken_ms, config)),
                    is_cur,
                    i,
                    metas.len(),
                );
                if resp.clicked() {
                    clicked = Some(i);
                }
                if resp.secondary_clicked() {
                    clicked = Some(i);
                }
                if resp.secondary_clicked() || resp.context_menu_opened() {
                    menu_on = Some((i, resp.clone()));
                }
            }
        },
    );
    if let Some(i) = clicked {
        pending.push(MonitorCmd::SnapshotCursor(i));
    }
    if let Some((i, resp)) = menu_on
        && let Some(m) = metas.get(i)
    {
        let id = m.id.clone();
        super::context_menu(&resp, |ui| {
            ui.set_width(230.0);
            if menu_item(ui, Some(ph::ARROW_LINE_LEFT), tr!("monitor-gui-compare-from"), None, true).clicked() {
                pending.push(MonitorCmd::CompareFrom(id.clone()));
                ui.close();
            }
            if menu_item(ui, Some(ph::ARROW_LINE_RIGHT), tr!("monitor-gui-compare-with"), None, true).clicked() {
                pending.push(MonitorCmd::CompareWith(CompareTarget::Snapshot(id.clone())));
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("monitor-gui-rename"), None, true).clicked() {
                pending.push(MonitorCmd::StartRename(id.clone()));
                ui.close();
            }
            if menu_item(ui, Some(ph::FILE_CSV), tr!("monitor-gui-export-csv"), None, true).clicked() {
                pending.push(MonitorCmd::ExportSnapshot { id: id.clone(), csv: true });
                ui.close();
            }
            if menu_item(ui, Some(ph::BRACKETS_CURLY), tr!("monitor-gui-export-json"), None, true).clicked() {
                pending.push(MonitorCmd::ExportSnapshot { id: id.clone(), csv: false });
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::TRASH), tr!("common-delete"), None, true).clicked() {
                pending.push(MonitorCmd::DeleteSnapshot(id.clone()));
                ui.close();
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_panel(
    ui: &mut Ui,
    rect: Rect,
    p: &Palette,
    pane: u64,
    metas: &[SnapshotMeta],
    a: Option<&str>,
    b: &CompareTarget,
    match_by: MatchBy,
    sort: DiffSort,
    descending: bool,
    show_same: bool,
    filter: &str,
    comparison: Option<&Comparison>,
    shown: &[usize],
    comparing: bool,
    error: Option<&str>,
    style: soyuz_core::SizeStyle,
    config: &soyuz_core::config::Config,
    focused: bool,
    pending: &mut Vec<MonitorCmd>,
) {
    let inner = rect.shrink2(vec2(14.0, 0.0));
    // Controls row.
    let cr = Rect::from_min_size(pos2(inner.left(), inner.top() + 6.0), vec2(inner.width(), font::grow(28.0)));
    let mut bar = ui.new_child(UiBuilder::new().max_rect(cr).layout(egui::Layout::left_to_right(egui::Align::Center)));
    bar.spacing_mut().item_spacing.x = 8.0;
    let items_a: Vec<Item<String>> = metas
        .iter()
        .map(|m| Item::new(m.id.clone(), m.label.clone()).secondary(labels::time(m.taken_ms, config)))
        .collect();
    let mut a_val = a.map(str::to_string).unwrap_or_default();
    let dd_w = font::grow(if inner.width() > 700.0 { 170.0 } else { 130.0 });
    let out = Dropdown::new(format!("monitor.snap.a.{pane}"))
        .width(dd_w)
        .name(tr!("monitor-gui-compare-a"))
        .placeholder(tr!("monitor-gui-pick-snapshot"))
        .show(&mut bar, &mut a_val, &items_a);
    if out.changed {
        pending.push(MonitorCmd::CompareFrom(a_val.clone()));
    }
    bar.label(egui::RichText::new(ph::ARROW_RIGHT).font(ticons::font(13.0)).color(p.text_tertiary));
    let mut items_b: Vec<Item<String>> = vec![Item::new(String::new(), tr!("monitor-gui-now")).icon(ph::RECORD)];
    items_b.extend(
        metas.iter().map(|m| Item::new(m.id.clone(), m.label.clone()).secondary(labels::time(m.taken_ms, config))),
    );
    let mut b_val = match b {
        CompareTarget::Live => String::new(),
        CompareTarget::Snapshot(id) => id.clone(),
    };
    let out = Dropdown::new(format!("monitor.snap.b.{pane}"))
        .width(dd_w)
        .name(tr!("monitor-gui-compare-b"))
        .show(&mut bar, &mut b_val, &items_b);
    if out.changed {
        pending.push(MonitorCmd::CompareWith(if b_val.is_empty() {
            CompareTarget::Live
        } else {
            CompareTarget::Snapshot(b_val.clone())
        }));
    }
    let titles = [tr!("monitor-gui-by-program"), tr!("monitor-gui-by-process")];
    let seg_w = super::segmented_width(&bar, &titles);
    let (seg_r, _) = bar.allocate_exact_size(vec2(seg_w, cr.height()), Sense::hover());
    let sel = if match_by == MatchBy::Program { 0 } else { 1 };
    if let Some(i) = segmented_text(&mut bar, seg_r, Id::unique(("soyuz.monitor.snap.matchby", pane)), &titles, sel) {
        pending.push(MonitorCmd::SetMatchBy(if i == 0 { MatchBy::Program } else { MatchBy::Process }));
    }
    if pill_button_on(&mut bar, Some(ph::GIT_DIFF), tr!("monitor-gui-compare"), Btn::Primary, a.is_some() && !comparing)
        .clicked()
    {
        pending.push(MonitorCmd::RunCompare);
    }
    if comparing {
        let (r, _) = bar.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        spinner(&bar, r.center(), 6.0, p.accent);
    }
    if comparison.is_some() && pill_button(&mut bar, Some(ph::X), tr!("common-close"), Btn::Ghost).clicked() {
        pending.push(MonitorCmd::CloseCompare);
    }
    let body = Rect::from_min_max(pos2(rect.left(), cr.bottom() + 8.0), rect.max);
    if let Some(e) = error {
        ui.painter().text(pos2(inner.left(), body.top() + 10.0), Align2::LEFT_TOP, e, font::ui(12.0), p.danger);
        return;
    }
    let Some(cmp) = comparison else {
        super::empty(ui, body, ph::GIT_DIFF, tr!("monitor-gui-compare-title"), tr!("monitor-gui-compare-sub"));
        return;
    };

    // ---- totals ----
    let t = &cmp.totals;
    let dash = PageCtx::dash().to_string();
    let pct = |v: f32| labels::percent(v);
    let size = |v: u64| soyuz_core::format_size(v, style);
    let rate = |v: f64| labels::rate(v, style);
    let delta_pct = |a: f32, b: f32| format!("{:+.1}", b - a);
    let delta_bytes = |a: u64, b: u64| if b >= a { format!("+{}", size(b - a)) } else { format!("-{}", size(a - b)) };
    let delta_count = |a: u64, b: u64| {
        if b >= a { format!("+{}", labels::count(b - a)) } else { format!("-{}", labels::count(a - b)) }
    };
    let delta_rate = |a: f64, b: f64| if b >= a { format!("+{}", rate(b - a)) } else { format!("-{}", rate(a - b)) };
    let cards: Vec<(&str, String, String, String)> = vec![
        (
            labels::page_title(soyuz_core::config::monitor::MonitorPage::Cpu),
            pct(t.cpu.0),
            pct(t.cpu.1),
            delta_pct(t.cpu.0, t.cpu.1),
        ),
        (
            labels::page_title(soyuz_core::config::monitor::MonitorPage::Memory),
            size(t.mem_used.0),
            size(t.mem_used.1),
            delta_bytes(t.mem_used.0, t.mem_used.1),
        ),
        (tr!("monitor-gui-swap"), size(t.swap_used.0), size(t.swap_used.1), delta_bytes(t.swap_used.0, t.swap_used.1)),
        (
            tr!("monitor-gui-processes"),
            labels::count(u64::from(t.processes.0)),
            labels::count(u64::from(t.processes.1)),
            delta_count(u64::from(t.processes.0), u64::from(t.processes.1)),
        ),
        (
            tr!("monitor-gui-threads"),
            labels::count(t.threads.0),
            labels::count(t.threads.1),
            delta_count(t.threads.0, t.threads.1),
        ),
        (
            tr!("monitor-gui-handles"),
            t.handles.0.map_or(dash.clone(), labels::count),
            t.handles.1.map_or(dash.clone(), labels::count),
            match (t.handles.0, t.handles.1) {
                (Some(x), Some(y)) => delta_count(x, y),
                _ => dash.clone(),
            },
        ),
        (
            tr!("monitor-gui-disk-read"),
            rate(t.disk_read.0),
            rate(t.disk_read.1),
            delta_rate(t.disk_read.0, t.disk_read.1),
        ),
        (
            tr!("monitor-gui-disk-write"),
            rate(t.disk_write.0),
            rate(t.disk_write.1),
            delta_rate(t.disk_write.0, t.disk_write.1),
        ),
        (tr!("monitor-gui-net-received"), rate(t.net_rx.0), rate(t.net_rx.1), delta_rate(t.net_rx.0, t.net_rx.1)),
        (tr!("monitor-gui-net-sent"), rate(t.net_tx.0), rate(t.net_tx.1), delta_rate(t.net_tx.0, t.net_tx.1)),
    ];
    let (cols, cw) = super::grid_cols(inner.width(), font::grow(150.0), GAP);
    let rows = cards.len().div_ceil(cols);
    let card_h = font::grow(62.0);
    let totals_h = rows as f32 * (card_h + GAP);
    let painter = ui.painter_at(body);
    for (i, (title, va, vb, d)) in cards.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(inner.left() + (i % cols) as f32 * (cw + GAP), body.top() + (i / cols) as f32 * (card_h + GAP)),
            vec2(cw, card_h),
        );
        card(ui, p, r, Id::unique(("soyuz.monitor.snap.total", pane, i)), false);
        caption(&painter, p, pos2(r.left() + 10.0, r.top() + 12.0), title);
        let line = format!("{va}  →  {vb}");
        let g = crate::ui::widgets::trunc_galley(&painter, &line, font::numbers(12.0), p.text_primary, cw - 20.0);
        painter.galley(pos2(r.left() + 10.0, r.top() + 24.0), g, p.text_primary);
        let dg = crate::ui::widgets::trunc_galley(&painter, d, font::numbers(11.0), p.text_secondary, cw - 20.0);
        painter.galley(pos2(r.left() + 10.0, r.top() + 42.0), dg, p.text_secondary);
    }
    // ---- filter row ----
    let fr_y = body.top() + totals_h + 2.0;
    let frow = Rect::from_min_size(pos2(inner.left(), fr_y), vec2(inner.width(), font::grow(28.0)));
    let summary = tr!("monitor-gui-diff-summary", added = cmp.added, removed = cmp.removed, changed = cmp.changed);
    let sg = painter.layout_no_wrap(summary, font::ui(11.5), p.text_secondary);
    painter.galley(pos2(frow.left(), frow.center().y - sg.size().y / 2.0), sg.clone(), p.text_secondary);
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(frow.left() + sg.size().x + 12.0, frow.top()), frow.max))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 8.0;
    if chip(&mut bar, Some(ph::EQUALS), tr!("monitor-gui-show-unchanged"), show_same).clicked() {
        pending.push(MonitorCmd::DiffShowSame(!show_same));
    }
    let (fb, _) = bar.allocate_exact_size(
        vec2(font::grow(200.0).min(bar.available_width() - 4.0), frow.height() - 2.0),
        Sense::hover(),
    );
    let (changed, _) = filter_box(
        ui,
        fb,
        Id::unique(("soyuz.monitor.snap.filter", pane)),
        filter,
        tr!("monitor-gui-list-filter-hint"),
        false,
    );
    if let Some(t) = changed {
        pending.push(MonitorCmd::DiffFilter(t));
    }
    // ---- the table ----
    let table_rect = Rect::from_min_max(pos2(rect.left(), frow.bottom() + 6.0), rect.max);
    let by_program = match_by == MatchBy::Program;
    let sorts = [
        DiffSort::Change,
        DiffSort::Name,
        DiffSort::User,
        DiffSort::CpuDelta,
        DiffSort::RssDelta,
        DiffSort::ThreadsDelta,
        DiffSort::HandlesDelta,
        DiffSort::ReadDelta,
        DiffSort::WriteDelta,
        DiffSort::GpuDelta,
        DiffSort::CountDelta,
    ];
    let mut cols_v = vec![
        Col::new(tr!("monitor-gui-col-change"), 100.0),
        Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::Name), 220.0),
        Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::User), 120.0),
        Col::new(labels::page_title(soyuz_core::config::monitor::MonitorPage::Cpu), 150.0).numeric(),
        Col::new(labels::page_title(soyuz_core::config::monitor::MonitorPage::Memory), 190.0).numeric(),
        Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::Threads), 110.0).numeric(),
        Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::Handles), 110.0).numeric(),
        Col::new(tr!("monitor-gui-read-total"), 150.0).numeric(),
        Col::new(tr!("monitor-gui-write-total"), 150.0).numeric(),
        Col::new(labels::column_title(soyuz_core::config::monitor::ProcColumn::Gpu), 110.0).numeric(),
    ];
    if by_program {
        cols_v.push(Col::new(tr!("monitor-gui-col-count"), 90.0).numeric());
    }
    let sort_i = sorts.iter().position(|s| *s == sort).map(|i| Sort { col: i, descending });
    let row_h = font::height(config.appearance.density.row_height(), 13.0);
    let cursor = {
        // The cursor lives in the model; the GUI reads it through the shown list.
        usize::MAX
    };
    let out = Table {
        sort: sort_i,
        name: tr!("monitor-gui-compare-title").to_string(),
        ..Table::new(Id::unique(("soyuz.monitor.snap.table", pane)), &cols_v, shown.len(), row_h)
    }
    .show(ui, table_rect, p, |rc| {
        let d = &cmp.procs[shown[rc.index]];
        row_bg(rc.painter, p, rc.rect, rc.index == cursor, rc.hovered, focused);
        let (glyph, word, color) = match d.change {
            Change::Added => (ph::PLUS_CIRCLE, tr!("monitor-gui-added"), p.success),
            Change::Removed => (ph::MINUS_CIRCLE, tr!("monitor-gui-removed"), p.danger),
            Change::Changed => (ph::ARROWS_LEFT_RIGHT, tr!("monitor-gui-changed"), p.warning),
            Change::Same => (ph::EQUALS, tr!("monitor-gui-same"), p.text_tertiary),
        };
        let cell = rc.cell(0);
        rc.painter.text(
            pos2(cell.left() + 6.0, cell.center().y),
            Align2::CENTER_CENTER,
            glyph,
            ticons::font(12.0),
            color,
        );
        super::table::text_in(
            rc.painter,
            Rect::from_min_max(pos2(cell.left() + 16.0, cell.top()), cell.max),
            word,
            false,
            font::ui(12.0),
            color,
        );
        rc.text(1, &d.name, false, font::ui(12.5), p.text_primary);
        rc.text(2, d.user.as_deref().unwrap_or(PageCtx::dash()), false, font::ui(12.0), p.text_secondary);
        let pair = |f: &dyn Fn(&soyuz_sysmon::DiffValues) -> Option<String>| -> String {
            match (d.a.as_ref().and_then(f), d.b.as_ref().and_then(f)) {
                (Some(x), Some(y)) if x == y => x,
                (Some(x), Some(y)) => format!("{x} → {y}"),
                (Some(x), None) => format!("{x} → –"),
                (None, Some(y)) => format!("– → {y}"),
                (None, None) => PageCtx::dash().to_string(),
            }
        };
        rc.text(3, &pair(&|v| Some(labels::percent(v.cpu_percent))), true, font::numbers(12.0), p.text_primary);
        rc.text(4, &pair(&|v| Some(soyuz_core::format_size(v.rss, style))), true, font::numbers(12.0), p.text_primary);
        rc.text(5, &pair(&|v| Some(labels::count(v.threads))), true, font::numbers(12.0), p.text_primary);
        rc.text(6, &pair(&|v| v.handles.map(labels::count)), true, font::numbers(12.0), p.text_primary);
        rc.text(
            7,
            &pair(&|v| v.read_bytes.map(|b| soyuz_core::format_size(b, style))),
            true,
            font::numbers(12.0),
            p.text_primary,
        );
        rc.text(
            8,
            &pair(&|v| v.write_bytes.map(|b| soyuz_core::format_size(b, style))),
            true,
            font::numbers(12.0),
            p.text_primary,
        );
        rc.text(9, &pair(&|v| v.gpu_percent.map(labels::percent)), true, font::numbers(12.0), p.text_primary);
        if by_program {
            rc.text(10, &format!("{} → {}", d.count_a, d.count_b), true, font::numbers(12.0), p.text_secondary);
        }
        format!("{word} {}", d.name)
    });
    for e in &out.events {
        if let HeaderEvent::Sort(i) = e
            && let Some(s) = sorts.get(*i)
        {
            pending.push(MonitorCmd::DiffSortBy(*s));
        }
    }
    if out.clicked.is_some() {
        // Selection is visual only here; nothing to keep.
    }
    if shown.is_empty() {
        ui.painter().text(
            pos2(table_rect.center().x, table_rect.top() + font::grow(70.0)),
            Align2::CENTER_CENTER,
            if cmp.procs.is_empty() { tr!("monitor-gui-nothing-here") } else { tr!("monitor-gui-no-matches") },
            font::ui(12.5),
            p.text_tertiary,
        );
    }
    let _ = Arc::new(0);
}

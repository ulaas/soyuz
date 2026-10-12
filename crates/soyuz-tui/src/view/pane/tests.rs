//! Tests of the pane views: layout and scrolling per mode, the model, and what each mode
//! draws (TestBackend).

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use soyuz_appcore::state::layout::NavDir;
use soyuz_appcore::state::view_memory::ViewState;
use soyuz_core::{EntryInfo, Location, SortDir};

use super::columns::{ColumnVm, NAME_MIN, Placed, fit};
use super::*;

fn pane_in(n: usize, mode: ViewMode) -> Pane {
    let mut p = Pane::new(Location::Dir("/data".into()), ViewState::default());
    p.view.mode = mode;
    let entries: Vec<EntryInfo> = (0..n)
        .map(|i| {
            let mut e = EntryInfo::minimal(format!("/data/f{i:03}.txt").into(), EntryKind::File);
            e.size = 1024 * (i as u64 + 1);
            e
        })
        .collect();
    p.set_entries_ordered(entries);
    p.status = ListingStatus::Ready;
    p.refresh_visible(true, true);
    p
}

fn pane(n: usize) -> Pane {
    pane_in(n, ViewMode::Details)
}

fn build(p: &Pane, config: &Config) -> PaneVm {
    let cx = BuildCx { config, focused: true, checkboxes: false, icons: IconSet::Ascii, footer: None };
    PaneVm::build(p, 0, cx)
}

/// Draw `m` into a `w` × `h` terminal: the lines (right-trimmed), the buffer and the hits.
fn draw(m: &PaneVm, w: u16, h: u16) -> (Vec<String>, Buffer, Hits) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut hits = Hits::default();
    t.draw(|f| draw_pane(f, Rect::new(0, 0, w, h), m, &TuiTheme::plain(), &mut hits)).unwrap();
    let buf = t.backend().buffer().clone();
    let lines = (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
        .collect();
    (lines, buf, hits)
}

#[test]
fn layout_is_cached_until_the_listing_or_width_changes() {
    let mut p = pane(50);
    prepare_pane(&mut p, 40, 10);
    let sig = p.layout_sig;
    assert_eq!(p.layout.as_ref().map(|l| (l.n, l.cols)), Some((50, 1)));
    // The details header takes a row.
    assert_eq!((p.viewport_h, p.page_rows), (9.0, 9));
    prepare_pane(&mut p, 40, 12);
    assert_eq!(p.layout_sig, sig, "a height change keeps the layout");
    prepare_pane(&mut p, 41, 12);
    assert_ne!(p.layout_sig, sig);
    // A view mode change lays out again.
    let sig = p.layout_sig;
    p.view.mode = ViewMode::List;
    prepare_pane(&mut p, 41, 12);
    assert_ne!(p.layout_sig, sig);
    assert_eq!(p.viewport_h, 12.0);
}

#[test]
fn scrolling_follows_the_cursor_only_when_asked() {
    let mut p = pane(100);
    prepare_pane(&mut p, 40, 11);
    assert_eq!(p.scroll_y, 0.0);
    p.move_cursor(25, false, false);
    assert!(p.scroll_to_cursor);
    prepare_pane(&mut p, 40, 11);
    assert_eq!(p.scroll_y, 16.0, "cursor on the last row");
    assert!(!p.scroll_to_cursor);
    // A wheel scroll (T2.9) moves the view without the cursor and stays.
    p.scroll_y = 60.0;
    prepare_pane(&mut p, 40, 11);
    assert_eq!(p.scroll_y, 60.0);
    // Clamped to the end of the list.
    p.scroll_y = 500.0;
    prepare_pane(&mut p, 40, 11);
    assert_eq!(p.scroll_y, 90.0);
    p.move_cursor(3, false, false);
    prepare_pane(&mut p, 40, 11);
    assert_eq!(p.scroll_y, 3.0, "cursor on the first row");
}

#[test]
fn restore_clamps_and_keeps_the_cursor_visible() {
    let mut p = pane(100);
    p.restore_scroll = Some(40.0);
    prepare_pane(&mut p, 40, 11);
    assert_eq!((p.scroll_y, p.restore_scroll), (40.0, None));
    // A GUI offset in pixels: clamped, then the cursor is brought into view.
    p.move_cursor(5, false, false);
    p.scroll_to_cursor = false;
    p.restore_scroll = Some(2400.0);
    prepare_pane(&mut p, 40, 11);
    assert_eq!(p.scroll_y, 5.0);
    // While loading, an offset the listing cannot reach yet stays pending.
    let mut p = pane(5);
    p.status = ListingStatus::Loading { since: std::time::Instant::now() };
    p.restore_scroll = Some(30.0);
    prepare_pane(&mut p, 40, 11);
    assert_eq!((p.scroll_y, p.restore_scroll), (0.0, Some(30.0)));
}

/// The list's cells as drawn are the cells keyboard navigation moves through.
#[test]
fn list_geometry_matches_the_flow_layout() {
    for w in 0..=200u16 {
        let g = geometry(PaneMode::List, Rect::new(3, 2, w, 10));
        let cols = g.cols();
        assert!(cols >= 1 && (g.cell_w >= LIST_CELL_MIN || cols == 1), "{w}: {g:?}");
        assert!(cols * g.cell_w + (cols - 1) * g.gap <= w.max(1), "{w}: {g:?}");
        let mut p = pane_in(50, ViewMode::List);
        prepare_pane(&mut p, w, 10);
        assert_eq!(p.layout.as_ref().map(|l| l.cols), Some(usize::from(cols)), "width {w}");
    }
    // Details: a header line; Miller: three columns, or only the middle one when narrow.
    let g = geometry(PaneMode::Details, Rect::new(0, 1, 50, 10));
    assert_eq!((g.header, g.list), (Rect::new(0, 1, 50, 1), Rect::new(0, 2, 50, 9)));
    assert!(geometry(PaneMode::Details, Rect::new(0, 1, 50, 1)).header.is_empty());
    let g = geometry(PaneMode::Miller, Rect::new(0, 0, 82, 10));
    assert_eq!((g.parent.width, g.list.x, g.list.width, g.child.x, g.child.width), (20, 21, 40, 62, 20));
    assert_eq!(g.child.right(), 82);
    let g = geometry(PaneMode::Miller, Rect::new(0, 0, MILLER_MIN_W - 1, 10));
    assert!(g.parent.is_empty() && g.child.is_empty() && g.list.width == MILLER_MIN_W - 1);
}

#[test]
fn columns_fit_the_width() {
    let cols: Vec<ColumnVm> =
        [Column::Name, Column::Size, Column::Kind, Column::Modified].into_iter().map(ColumnVm::new).collect();
    let (size, kind, modified) = (cols[1].width, cols[2].width, cols[3].width);
    assert_eq!((size, kind, modified), (11, 15, 19));
    // Wide: everything, the name fills.
    let f = fit(&cols, 100, 2);
    assert_eq!(f.len(), 4);
    assert_eq!(f[0], Placed { index: Some(0), column: Column::Name, x: 0, w: 100 - 48 });
    assert_eq!(f[3].x + f[3].w, 100);
    // Narrower: columns leave from the right, the name keeps its minimum.
    let f = fit(&cols, 2 + NAME_MIN + 12 + 16, 2);
    assert_eq!(f.iter().map(|p| p.column).collect::<Vec<_>>(), [Column::Name, Column::Size, Column::Kind]);
    let f = fit(&cols, 20, 2);
    assert_eq!(f, [Placed { index: Some(0), column: Column::Name, x: 0, w: 20 }]);
    // The name in the middle; no name column listed: it comes first.
    let mid: Vec<ColumnVm> = [Column::Size, Column::Name].into_iter().map(ColumnVm::new).collect();
    let f = fit(&mid, 40, 2);
    assert_eq!((f[0].x, f[0].w, f[1].x, f[1].w), (0, 11, 12, 28));
    let f = fit(&cols[1..2], 40, 2);
    assert_eq!((f[0].column, f[0].w, f[1].x), (Column::Name, 28, 29));
}

#[test]
fn details_view_draws_columns_and_the_sort_mark() {
    let config = Config::default();
    let mut p = pane(30);
    p.view.sort.dir = SortDir::Desc;
    p.move_cursor(2, false, false);
    p.selection.insert("f004.txt".into());
    prepare_pane(&mut p, 70, 8);
    let m = build(&p, &config);
    assert_eq!(m.mode, PaneMode::Details);
    assert_eq!(
        m.columns.iter().map(|c| c.column).collect::<Vec<_>>(),
        [Column::Name, Column::Size, Column::Kind, Column::Modified]
    );
    assert_eq!(m.columns[0].sort, Some(SortDir::Desc));
    let (lines, buf, hits) = draw(&m, 70, 8);
    // Header: the name sorted descending (`v` in the ASCII icon set), sizes right-aligned.
    assert!(lines[0].starts_with("  Name v"), "{:?}", lines[0]);
    assert!(lines[0].contains("Size") && lines[0].contains("Kind"), "{:?}", lines[0]);
    assert!(buf[(2, 0)].modifier.contains(Modifier::BOLD), "the sort column stands out");
    // 70 cells: 2 + name 20 + 11 + 1 + 15 + 1 + 19 + 1 → the name gets 20 cells.
    assert!(lines[1].starts_with("  f000.txt"), "{:?}", lines[1]);
    let size_end = 2 + 20 + 1 + 11;
    let size: String = (size_end - 11..size_end).map(|x| buf[(x as u16, 1)].symbol().to_string()).collect();
    assert_eq!(size.trim_start(), soyuz_core::size::format_size(1024, config.general.size_style));
    // The cursor row (index 2, line 3) is reversed, the selected one bold with a mark.
    assert!(buf[(0, 3)].modifier.contains(Modifier::REVERSED) && buf[(69, 3)].modifier.contains(Modifier::REVERSED));
    assert!(lines[5].starts_with("* f004.txt") && buf[(3, 5)].modifier.contains(Modifier::BOLD));
    // Click targets: headers per column, rows per item.
    assert_eq!(hits.at(5, 0), Some(HitTarget::PaneColumn { pane: p.id, column: Column::Name }));
    assert_eq!(hits.at(30, 0), Some(HitTarget::PaneColumn { pane: p.id, column: Column::Size }));
    assert_eq!(hits.at(40, 3), Some(HitTarget::PaneRow { pane: p.id, vis_version: p.vis_version, index: 2 }));
}

#[test]
fn list_view_cells_match_navigation() {
    let config = Config::default();
    let mut p = pane_in(10, ViewMode::List);
    // 64 cells: three cells of 20 with a blank column between.
    prepare_pane(&mut p, 64, 5);
    let layout = p.layout.clone().unwrap();
    assert_eq!(layout.cols, 3);
    p.move_cursor(4, false, false);
    prepare_pane(&mut p, 64, 5);
    let m = build(&p, &config);
    assert_eq!(m.mode, PaneMode::List);
    assert_eq!(m.rows().count(), 10);
    let (lines, buf, hits) = draw(&m, 64, 5);
    assert!(lines[0].starts_with("  f000.txt"), "{:?}", lines[0]);
    assert_eq!(&lines[0][21..31], "  f001.txt");
    assert_eq!(&lines[1][21..31], "* f004.txt", "the cursor item is selected");
    // Only the cursor cell is reversed.
    assert!(buf[(21, 1)].modifier.contains(Modifier::REVERSED) && buf[(40, 1)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(20, 1)].modifier.contains(Modifier::REVERSED) && !buf[(42, 1)].modifier.contains(Modifier::REVERSED));
    // What navigation does, the drawing shows: Right from 4 is 5, Down is 7.
    assert_eq!(layout.nav(Some(4), NavDir::Right, 5.0), Some(5));
    assert_eq!(layout.nav(Some(4), NavDir::Down, 5.0), Some(7));
    let at = |x, y| match hits.at(x, y) {
        Some(HitTarget::PaneRow { index, .. }) => Some(index),
        _ => None,
    };
    assert_eq!((at(25, 1), at(45, 1), at(25, 2)), (Some(4), Some(5), Some(7)));
    assert_eq!(at(20, 1), None, "the gap between cells");
}

#[test]
fn grid_and_gallery_modes() {
    assert_eq!(PaneMode::of(&pane_in(1, ViewMode::Grid)), PaneMode::Grid);
    assert_eq!(PaneMode::of(&pane_in(1, ViewMode::Gallery)), PaneMode::Gallery);
    // Miller outside a folder: details.
    let mut p = pane_in(1, ViewMode::Miller);
    assert_eq!(PaneMode::of(&p), PaneMode::Miller);
    p.location = Location::Trash;
    assert_eq!(PaneMode::of(&p), PaneMode::Details);
}

#[test]
fn checkboxes_icons_groups_and_footer() {
    let config = Config::default();
    let mut p = pane_in(4, ViewMode::List);
    p.view.group = soyuz_core::GroupBy::Extension;
    p.dirty = true;
    p.refresh_visible(true, true);
    p.selection.insert("f001.txt".into());
    prepare_pane(&mut p, 30, 6);
    let cx = BuildCx {
        config: &config,
        focused: true,
        checkboxes: true,
        icons: IconSet::Unicode,
        footer: Some("g …".into()),
    };
    let m = PaneVm::build(&p, 0, cx);
    let (lines, buf, hits) = draw(&m, 30, 6);
    assert!(lines[0].ends_with("(4)"), "{:?}", lines[0]);
    assert!(matches!(hits.at(1, 0), Some(HitTarget::PaneHeader { group: 0, .. })));
    // `[ ] ` then the icon and its space, then the name.
    assert!(lines[1].starts_with("[ ] "), "{:?}", lines[1]);
    assert!(lines[2].starts_with("[x] ") && lines[2].contains("f001.txt"), "{:?}", lines[2]);
    assert_eq!(buf[(7, 1)].symbol(), "f", "the name after the mark and the icon");
    // The footer overlay sits on the last line, right-aligned.
    assert!(lines[5].ends_with(" g …"), "{:?}", lines[5]);
    assert!(buf[(29, 5)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn miller_draws_three_columns() {
    let m = PaneVm {
        id: PaneId(9),
        vis_version: 1,
        focused: true,
        mode: PaneMode::Miller,
        lines: vec![
            PaneLine::Item(Row { index: 0, name: "sub".into(), kind: RowKind::Dir, cursor: true, ..Row::default() }),
            PaneLine::Item(Row { index: 1, name: "a.txt".into(), ..Row::default() }),
        ],
        total: 2,
        miller: Some(MillerVm {
            parent: miller::MillerCol {
                rows: vec![
                    aux(0, "alpha", RowKind::Dir, false),
                    aux(1, "data", RowKind::Dir, true),
                    aux(2, "zeta.txt", RowKind::File, false),
                ],
                message: None,
            },
            child: miller::MillerCol { rows: vec![aux(0, "inner.txt", RowKind::File, false)], message: None },
        }),
        ..PaneVm::default()
    };
    let (lines, buf, hits) = draw(&m, 82, 4);
    // parent 0..20 | current 21..61 | child 62..82
    assert!(lines[0].starts_with(" alpha/"), "{:?}", lines[0]);
    assert_eq!(span(&lines[0], 20, 30), "│  sub/   ");
    assert!(lines[0].contains("│ inner.txt"), "{:?}", lines[0]);
    assert!(lines[1].starts_with(" data/") && span(&lines[1], 21, 28) == "  a.txt", "{:?}", lines[1]);
    // The current folder is marked in the parent column, the cursor in the middle one.
    assert!(buf[(5, 1)].modifier.contains(Modifier::UNDERLINED));
    assert!(buf[(25, 0)].modifier.contains(Modifier::REVERSED) && !buf[(5, 0)].modifier.contains(Modifier::REVERSED));
    assert_eq!(hits.at(3, 1), Some(HitTarget::MillerRow { pane: PaneId(9), parent: true, index: 1 }));
    assert_eq!(hits.at(70, 0), Some(HitTarget::MillerRow { pane: PaneId(9), parent: false, index: 0 }));
    assert_eq!(hits.at(30, 1), Some(HitTarget::PaneRow { pane: PaneId(9), vis_version: 1, index: 1 }));
    // A side column still loading says so.
    let mut loading = m.clone();
    loading.miller.as_mut().unwrap().child = miller::MillerCol { rows: Vec::new(), message: Some("Loading…".into()) };
    let (lines, _, _) = draw(&loading, 82, 4);
    assert!(lines[0].ends_with("Loading…"), "{:?}", lines[0]);
}

fn aux(index: usize, name: &str, kind: RowKind, on_path: bool) -> miller::AuxRow {
    miller::AuxRow {
        index,
        name: name.into(),
        kind,
        class: if kind == RowKind::Dir { FileClass::Folder } else { FileClass::Text },
        icon: IconId::File,
        hidden: false,
        on_path,
    }
}

/// 100 000 entries: the model holds only the rows in view, in every mode, wherever the
/// view is scrolled to (no per-frame work proportional to the listing).
#[test]
fn large_listings_build_only_the_rows_in_view() {
    let config = Config::default();
    for (mode, w, per_line) in [(ViewMode::Details, 80, 1), (ViewMode::List, 80, 3), (ViewMode::Miller, 30, 1)] {
        let mut p = pane_in(100_000, mode);
        for cursor in [0, 50_000, 99_999] {
            p.move_cursor(cursor, false, false);
            prepare_pane(&mut p, w, 30);
            let m = build(&p, &config);
            let rows = p.viewport_h as usize;
            assert!(m.lines.len() <= rows * per_line, "{mode:?}: {} lines", m.lines.len());
            assert!(m.rows().any(|r| r.cursor && r.index == cursor), "{mode:?}: cursor {cursor} in view");
            let (_, _, hits) = draw(&m, w, 30);
            assert!(hits.len() <= 1 + 4 + rows * per_line, "{mode:?}: {} hits", hits.len());
        }
    }
}

/// Tiny rects never panic, in any mode.
#[test]
fn tiny_rects() {
    let config = Config::default();
    for mode in [ViewMode::Details, ViewMode::List, ViewMode::Miller] {
        let mut p = pane_in(5, mode);
        p.move_cursor(1, false, false);
        for w in 0..=45u16 {
            for h in 0..=4u16 {
                prepare_pane(&mut p, w, h);
                let mut m = build(&p, &config);
                m.footer = Some("g …".into());
                let mut t = Terminal::new(TestBackend::new(w.max(1), h.max(1))).unwrap();
                let mut hits = Hits::default();
                t.draw(|f| draw_pane(f, Rect::new(0, 0, w, h), &m, &TuiTheme::plain(), &mut hits)).unwrap();
            }
        }
    }
}

/// Cells `a..b` of a drawn line (every character here is one cell wide).
fn span(s: &str, a: usize, b: usize) -> String {
    s.chars().skip(a).take(b - a).collect()
}

#[test]
fn grid_layout_scrolls_by_lines_of_cells() {
    let mut p = pane_in(30, ViewMode::Grid);
    p.view.zoom = 96;
    // 40 wide: 3 cells of 12 (+ 1 gap); 13 rows: two lines of 6-row cells.
    prepare_pane_px(&mut p, 40, 13, (10, 20));
    let layout = p.layout.as_ref().unwrap();
    assert_eq!((layout.cols, layout.cell.y, layout.lines().len()), (3, 6.0, 10));
    assert_eq!(layout.nav(Some(1), NavDir::Down, p.viewport_h), Some(4));
    assert_eq!(layout.nav(Some(4), NavDir::Right, p.viewport_h), Some(5));
    assert_eq!(p.page_rows, 2);
    // The cursor on the last item: its line is the last one shown.
    p.move_cursor(29, false, false);
    prepare_pane_px(&mut p, 40, 13, (10, 20));
    assert_eq!(p.scroll_y, 8.0);
    let m = build(&p, &Config::default());
    assert_eq!(m.mode, PaneMode::Grid);
    assert_eq!(m.rows().map(|r| r.index).collect::<Vec<_>>(), (24..30).collect::<Vec<_>>());
    // Wheel past the end clamps; back to the top line.
    p.scroll_y = 50.0;
    prepare_pane_px(&mut p, 40, 13, (10, 20));
    assert_eq!(p.scroll_y, 8.0);
    p.move_cursor(0, false, false);
    prepare_pane_px(&mut p, 40, 13, (10, 20));
    assert_eq!(p.scroll_y, 0.0);
    // Smaller icons: compact one-row cells.
    p.view.zoom = 16;
    prepare_pane_px(&mut p, 40, 13, (10, 20));
    assert_eq!(p.layout.as_ref().unwrap().cell.y, 1.0);
}

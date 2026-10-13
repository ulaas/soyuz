//! Screen-reader mode (Phase T8, `crate::a11y`): the look, the caret per focus kind, the
//! one-pane layout, the status sentence and the toasts in the status row. Hand-built
//! models like `view::tests`; assertion-style (the snapshots come with P6).

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use soyuz_appcore::icon::IconId;
use soyuz_appcore::models::jobs_rows::Tone;
use soyuz_appcore::models::toasts::ToastKind;
use soyuz_core::config::TuiSection;

use super::tests::model;
use super::*;
use crate::a11y::is_box_char;
use crate::app::NullIo;
use crate::input::TuiFocus;
use crate::input::menu::MenuCmd;
use crate::layout::{Facts, HitTarget};
use crate::widgets::{Button, Field, Form, Menu, MenuItem};

/// A frame drawn in screen-reader mode (`facts.screen_reader` is set here): the lines,
/// the buffer, the hits and the terminal's cursor when it is shown.
struct Shot {
    lines: Vec<String>,
    buf: Buffer,
    hits: Hits,
    regions: Regions,
    cursor: Option<Position>,
}

fn draw_with(m: &ViewModel, facts: &Facts, w: u16, h: u16, theme: &TuiTheme) -> Shot {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let regions = Regions::compute(Rect::new(0, 0, w, h), &TuiSection::default(), facts);
    let mut hits = Hits::default();
    t.draw(|f| draw(f, m, &regions, theme, &mut hits)).unwrap();
    let buf = t.backend().buffer().clone();
    let lines = (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
        .collect();
    let b = t.backend_mut();
    let cursor = b.cursor_visible().then(|| b.cursor_position());
    Shot { lines, buf, hits, regions, cursor }
}

/// The plain theme (Unicode icons, rounded boxes) in screen-reader mode.
fn sr_theme() -> TuiTheme {
    TuiTheme::plain().with_screen_reader(true)
}

fn sr(m: &ViewModel, facts: &Facts, w: u16, h: u16) -> Shot {
    draw_with(m, &Facts { screen_reader: true, ..facts.clone() }, w, h, &sr_theme())
}

fn hit(s: &Shot, target: HitTarget) -> Position {
    let (r, _) = s.hits.iter().filter(|(_, t)| *t == target).last().unwrap_or_else(|| panic!("no {target:?}"));
    Position { x: r.x, y: r.y }
}

fn toast(kind: ToastKind, title: &str, body: Option<&str>) -> ToastLine {
    ToastLine { id: 1, kind, title: title.into(), body: body.map(Into::into), action: None, sticky: false }
}

fn dialog(focus: usize) -> DialogVm {
    let mut form = Form::new(
        vec![Field::text("Name", "a.txt"), Field::check("Apply to all", true)],
        vec![Button::new("Rename"), Button::new("Cancel")],
    );
    form.focus = focus;
    DialogVm {
        body: vec!["Rename “a.txt”".into()],
        hints: "Enter rename · Esc cancel".into(),
        ..DialogVm::from_form("Rename", &form)
    }
}

fn palette() -> PaletteVm {
    let row = |index: usize, title: &str, selected: bool| {
        palette::PaletteLine::Item(palette::PaletteRow {
            index,
            icon: "★",
            tint: crate::theme::Role::Accent,
            title: vec![(title.into(), false)],
            detail: "detail".into(),
            tag: Some("View".into()),
            chord: Some("Ctrl+H".into()),
            selected,
        })
    };
    PaletteVm {
        open: true,
        glyph: "❯",
        input: "hid".into(),
        cursor: 3,
        lines: vec![
            palette::PaletteLine::Header("VIEW".into()),
            row(0, "Show hidden files", false),
            row(1, "Hide sidebar", true),
        ],
        hints: "↑↓ move · Enter run".into(),
        ..PaletteVm::default()
    }
}

fn menu() -> MenuVm {
    let open = |s: &str| MenuItem::run(s, MenuCmd::Tool(0));
    let mut menu = Menu::new(vec![open("Open"), MenuItem::Separator, open("Rename"), open("Delete")], (5, 3));
    menu.cursor = Some(2);
    MenuVm { menu: Some(menu) }
}

fn jobs() -> JobsVm {
    let item = jobs::JobItem {
        icon: IconId::Copy,
        icon_tone: Tone::Accent,
        title: "Copying 3 files".into(),
        dim: false,
        tag: None,
        line: "45 %".into(),
        line_tone: Tone::Muted,
        bar: Some(jobs::Bar { permille: Some(450), tone: Tone::Accent }),
        stats: "1 of 3 files".into(),
        stats_tone: Tone::Muted,
        right: "2 min left".into(),
        buttons: Vec::new(),
        log: Vec::new(),
    };
    JobsVm {
        visible: true,
        focused: true,
        tabs: vec!["Jobs".into(), "History".into()],
        items: vec![item],
        len: 1,
        ..JobsVm::default()
    }
}

/// Every screen-reader screen worth drawing, with the facts it needs.
fn screens() -> Vec<(&'static str, ViewModel, Facts)> {
    let base = Facts::default();
    let mut out = vec![("browse", model(), base.clone())];
    let mut m = model();
    m.panes[0].index = 1;
    m.other_pane = Some("Other pane: /left".into());
    out.push(("dual", m, Facts { shown: 2, focused: 1, ..base.clone() }));
    let mut m = model();
    m.toasts.lines = vec![toast(ToastKind::Error, "Copy failed", Some("disk full"))];
    out.push(("toast", m, base.clone()));
    let mut m = model();
    m.dialog = Some(dialog(0));
    out.push(("dialog", m, Facts { focus: TuiFocus::Dialog, ..base.clone() }));
    let mut m = model();
    m.palette = palette();
    out.push(("palette", m, Facts { focus: TuiFocus::Palette, ..base.clone() }));
    let mut m = model();
    m.menu = menu();
    out.push(("menu", m, Facts { focus: TuiFocus::Menu, ..base.clone() }));
    let mut m = model();
    m.jobs = jobs();
    out.push(("jobs", m, Facts { focus: TuiFocus::Jobs, bottom_rows: 6, ..base.clone() }));
    let mut m = model();
    m.sidebar = sidebar::SidebarVm {
        lines: vec![
            sidebar::SidebarLine {
                index: 0,
                kind: sidebar::LineKind::Header { open: true },
                label: "Favorites".into(),
                depth: 0,
                active: false,
                cursor: false,
                dim: false,
                mark: sidebar::Mark::None,
                right: String::new(),
                eject: false,
            },
            sidebar::SidebarLine {
                index: 1,
                kind: sidebar::LineKind::Place,
                label: "Downloads".into(),
                depth: 0,
                active: true,
                cursor: true,
                dim: false,
                mark: sidebar::Mark::Dot(Tone::Success),
                right: String::new(),
                eject: true,
            },
        ],
        focused: true,
    };
    out.push(("sidebar", m, Facts { sidebar: true, focus: TuiFocus::Sidebar, ..base.clone() }));
    out
}

#[test]
fn screen_reader_screens_have_no_box_drawing_and_show_the_cursor() {
    for (name, m, facts) in screens() {
        let s = sr(&m, &facts, 80, 24);
        for (y, l) in s.lines.iter().enumerate() {
            assert!(!l.chars().any(is_box_char), "{name}: box drawing on row {y}: {l:?}\n{:#?}", s.lines);
            // No icon glyphs, no `>` markers, no arrows a reader calls "greater than".
            assert!(!l.contains('★') && !l.contains('❯') && !l.contains('>'), "{name}: row {y}: {l:?}");
        }
        assert!(s.cursor.is_some(), "{name}: the cursor is hidden");
    }
}

#[test]
fn the_default_look_is_unchanged_and_keeps_the_cursor_hidden() {
    for (name, m, facts) in screens() {
        let plain = draw_with(&m, &facts, 80, 24, &TuiTheme::plain());
        // Screen-reader mode off by the flag: the very same frame as without it.
        let off = draw_with(&m, &facts, 80, 24, &TuiTheme::plain().with_screen_reader(false));
        assert_eq!(plain.buf, off.buf, "{name}");
        if matches!(name, "browse" | "dual" | "toast" | "menu" | "jobs" | "sidebar") {
            assert_eq!(plain.cursor, None, "{name}: the default look shows the cursor");
        }
    }
    // The hardware cursor only shows in screen-reader mode.
    let s = sr(&model(), &Facts::default(), 40, 10);
    assert_eq!(s.cursor, Some(Position { x: 0, y: 3 }));
}

#[test]
fn the_caret_follows_the_focus() {
    // The list: the cursor row (header on row 1, `sub/` on row 2, `a.txt` on row 3).
    let s = sr(&model(), &Facts::default(), 40, 10);
    assert_eq!(s.cursor, Some(Position { x: 0, y: 3 }));
    assert!(s.lines[3].starts_with("  a.txt"), "{:?}", s.lines);
    assert!(s.lines[4].starts_with("* b.txt") && s.lines[2] == "  sub/", "{:?}", s.lines);
    // Every screen puts it on its focused element.
    for (name, m, facts) in screens() {
        let s = sr(&m, &facts, 80, 24);
        let want = match name {
            "browse" | "toast" => Some(hit(&s, HitTarget::PaneRow { pane: m.panes[0].id, vis_version: 3, index: 1 })),
            "dual" => Some(hit(&s, HitTarget::PaneRow { pane: m.panes[0].id, vis_version: 3, index: 1 })),
            "palette" => Some(hit(&s, HitTarget::PaletteRow(1))),
            "menu" => Some(hit(&s, HitTarget::MenuItem { depth: 0, index: 2 })),
            "jobs" => Some(hit(&s, HitTarget::JobRow(0))),
            "sidebar" => Some(hit(&s, HitTarget::SidebarRow(1))),
            // The name field's text cursor, after "a.txt".
            "dialog" => {
                let f = hit(&s, HitTarget::DialogField(0));
                assert_eq!(s.cursor.map(|c| c.y), Some(f.y), "{:#?}", s.lines);
                None
            }
            _ => unreachable!(),
        };
        if want.is_some() {
            assert_eq!(s.cursor, want, "{name}: {:#?}", s.lines);
        }
    }
    // A dialog's focused button.
    let mut m = model();
    m.dialog = Some(dialog(3));
    let s = sr(&m, &Facts { focus: TuiFocus::Dialog, ..Facts::default() }, 80, 24);
    assert_eq!(s.cursor, Some(hit(&s, HitTarget::DialogButton(1))));
    // The address field: its text cursor (drawn before the panes, recorded after them).
    let mut m = model();
    m.crumbs.edit = Some(crumbs::AddressVm { text: "/data/x".into(), cursor: 3, focused: true, ..Default::default() });
    let s = sr(&m, &Facts { focus: TuiFocus::Address, ..Facts::default() }, 40, 10);
    assert_eq!(s.cursor, Some(Position { x: 3, y: 0 }));
    // The filter field in the status row.
    let mut m = model();
    m.status.filter = Some(status::FilterVm { label: "Filter".into(), text: "ab".into(), cursor: Some(2) });
    let s = sr(&m, &Facts { focus: TuiFocus::Filter, ..Facts::default() }, 40, 10);
    assert!(s.lines[9].starts_with(" Filter ab"), "{:?}", s.lines[9]);
    assert_eq!(s.cursor, Some(Position { x: 10, y: 9 }));
    // A panel that places the cursor itself (the editor): the list's caret is not used.
    let s = sr(&model(), &Facts { focus: TuiFocus::Editor, ..Facts::default() }, 40, 10);
    assert_eq!(s.cursor, None);
    // The preview: its first row.
    let s = sr(&model(), &Facts { focus: TuiFocus::Preview, preview: true, ..Facts::default() }, 40, 20);
    assert_eq!(s.cursor, Some(Position { x: s.regions.preview.x, y: s.regions.preview.y }));
}

#[test]
fn dual_panes_draw_only_the_focused_one() {
    let (_, mut m, facts) = screens().into_iter().find(|(n, ..)| *n == "dual").unwrap();
    m.panes[0].id = soyuz_appcore::state::pane::PaneId(8);
    let s = sr(&m, &facts, 80, 12);
    // One pane at full width: no title lines, no splitter; its header on row 1.
    assert_eq!(s.regions.panes.len(), 1);
    assert_eq!(s.regions.panes[0].index, 1);
    assert_eq!(s.regions.panes[0].rect.width, 80);
    assert!(s.regions.titles.is_empty() && s.regions.splitters.is_empty());
    assert!(s.lines[1].starts_with("  Name"), "{:#?}", s.lines);
    // The other pane's folder above the status line.
    assert_eq!(s.lines[10], "Other pane: /left");
    assert!(s.lines[11].starts_with("a.txt, file"), "{:?}", s.lines[11]);
}

#[test]
fn the_status_line_is_one_sentence_starting_with_the_focused_item() {
    let mut m = model();
    m.status = StatusVm {
        visible: true,
        counts: "3 items".into(),
        hidden: "· 2 hidden".into(),
        selected: "1 selected  (1 KiB)".into(),
        free: "12 GB free".into(),
        mode: "Details".into(),
        sort: "Name ↑".into(),
        hint: "Ctrl+Q quit".into(),
        ..StatusVm::default()
    };
    let want = "a.txt, file, 1 KiB, 2 of 3; 3 items, 2 hidden, 1 selected (1 KiB); 12 GB free; Details, sorted by Name, ascending";
    assert_eq!(crate::a11y::status_sentence(&m, Some(TuiFocus::List)), want);
    let s = sr(&m, &Facts::default(), 140, 10);
    assert_eq!(s.lines[9], want);
    // No key hint, no right-aligned parts; drawn in the terminal's own colours.
    let c = &s.buf[(0, 9)];
    assert!(c.fg == ratatui::style::Color::Reset && c.bg == ratatui::style::Color::Reset && c.modifier.is_empty());
    // A selected folder.
    for l in &mut m.panes[0].lines {
        if let PaneLine::Item(r) = l {
            r.cursor = r.index == 0;
            r.selected = r.index == 0;
        }
    }
    assert!(crate::a11y::status_sentence(&m, None).starts_with("sub, folder, 1 of 3, selected; 3 items"));
    // Other focus kinds lead with their own element.
    let mut m = model();
    m.palette = palette();
    assert!(crate::a11y::status_sentence(&m, Some(TuiFocus::Palette)).starts_with("Hide sidebar; "));
    m.menu = menu();
    assert!(crate::a11y::status_sentence(&m, Some(TuiFocus::Menu)).starts_with("Rename; "));
    m.dialog = Some(dialog(1));
    assert!(crate::a11y::status_sentence(&m, Some(TuiFocus::Dialog)).starts_with("Rename: Apply to all, checked; "));
    m.dialog = Some(dialog(3));
    assert!(crate::a11y::status_sentence(&m, Some(TuiFocus::Dialog)).starts_with("Rename: Cancel; "));
    // Text fields add nothing (typing must not make the line change with every key).
    assert_eq!(crate::a11y::status_sentence(&m, Some(TuiFocus::Address)), "3 items · 1 selected");
}

#[test]
fn toasts_go_to_the_status_row() {
    let mut m = model();
    m.toasts.lines = vec![
        toast(ToastKind::Info, "Copied", None),
        ToastLine {
            action: Some("Undo (Ctrl+Z)".into()),
            ..toast(ToastKind::Warning, "Moved 3 items", Some("2 skipped"))
        },
    ];
    let s = sr(&m, &Facts::default(), 80, 10);
    // The newest one, read top to bottom; no boxes, no toast targets.
    assert_eq!(s.lines[9], "Warning: Moved 3 items. 2 skipped. Undo (Ctrl+Z)");
    assert!(s.lines[..9].iter().all(|l| !l.contains("Copied") && !l.contains("Moved")), "{:#?}", s.lines);
    assert!(!s.hits.iter().any(|(_, t)| matches!(t, HitTarget::Toast(_))));
    // The default look keeps its floating boxes.
    let plain = draw_with(&m, &Facts::default(), 80, 10, &TuiTheme::plain());
    assert!(plain.lines.iter().any(|l| l.contains("[!] Moved 3 items") || l.contains("Moved 3 items")));
    assert!(plain.hits.iter().any(|(_, t)| matches!(t, HitTarget::Toast(_))));
}

#[test]
fn the_status_line_is_rewritten_when_it_changes() {
    let mut m = model();
    let regions = Regions { status: Rect::new(0, 9, 40, 1), focus: Some(TuiFocus::List), ..Regions::default() };
    let (mut st, hits, mut io) = (crate::a11y::State::default(), Hits::default(), NullIo::default());
    // Off: nothing.
    crate::a11y::after_frame(&mut st, false, &m, &regions, &hits, &mut io).unwrap();
    assert!(io.written.is_empty());
    crate::a11y::after_frame(&mut st, true, &m, &regions, &hits, &mut io).unwrap();
    let first = String::from_utf8(io.written.clone()).unwrap();
    assert!(first.starts_with("\x1b7\x1b[10;1H\x1b[0ma.txt, file, 1 KiB, 2 of 3; 3 items"), "{first:?}");
    assert!(first.ends_with("…\x1b[K\x1b8"), "fitted to the row: {first:?}");
    // The same line again: nothing more.
    crate::a11y::after_frame(&mut st, true, &m, &regions, &hits, &mut io).unwrap();
    assert_eq!(io.written.len(), first.len());
    // The cursor moved: the whole line again.
    for l in &mut m.panes[0].lines {
        if let PaneLine::Item(r) = l {
            r.cursor = r.index == 2;
        }
    }
    crate::a11y::after_frame(&mut st, true, &m, &regions, &hits, &mut io).unwrap();
    let all = String::from_utf8(io.written.clone()).unwrap();
    assert!(all[first.len()..].starts_with("\x1b7\x1b[10;1H\x1b[0mb.txt, file, 1 KiB, 3 of 3, selected"), "{all:?}");
    // An open filter field owns the row: nothing is written.
    m.status.filter = Some(status::FilterVm { label: "Filter".into(), text: "a".into(), cursor: Some(1) });
    let n = io.written.len();
    crate::a11y::after_frame(&mut st, true, &m, &regions, &hits, &mut io).unwrap();
    assert_eq!(io.written.len(), n);
}

#[test]
fn screen_reader_mode_survives_tiny_sizes() {
    let panels = [
        Facts::default(),
        Facts { shown: 2, focused: 1, tab_strip: true, ..Facts::default() },
        Facts { shown: 4, focused: 3, ..Facts::default() },
        Facts { sidebar: true, focus: TuiFocus::Sidebar, ..Facts::default() },
        Facts { ai: true, focus: TuiFocus::Ai, ..Facts::default() },
        Facts { editor: true, editor_maximized: true, focus: TuiFocus::Editor, ..Facts::default() },
        Facts { editor: true, preview: true, bottom_rows: 6, search_rows: 2, ..Facts::default() },
    ];
    for (name, m, facts) in screens() {
        for p in panels.iter().chain([&facts]) {
            for w in 0..=30 {
                for h in 0..=8 {
                    let s = sr(&m, p, w, h);
                    if let Some(c) = s.cursor {
                        assert!(c.x < w.max(1) && c.y < h.max(1), "{name} {w}x{h}: {c:?}");
                    }
                }
            }
        }
    }
}

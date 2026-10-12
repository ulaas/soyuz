//! Headless tests of T4: grid and gallery views. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
use soyuz_core::config::{TuiColorMode, TuiImages};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::layout::HitTarget;
use soyuz_tui::view::{PaneMode, PaneVm};

use crate::common::{self, Run, key, shift};

/// Ctrl+Shift+`digit` (the view-mode chords), as the kitty keyboard protocol sends it.
fn ctrl_shift(digit: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(digit), KeyModifiers::CONTROL | KeyModifiers::SHIFT))
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE })
}

fn pane(a: &App) -> Option<&PaneVm> {
    a.view()?.focused_pane()
}

fn mode(a: &App) -> Option<PaneMode> {
    pane(a).map(|p| p.mode)
}

/// The focused pane's cursor position.
fn cur(a: &App) -> Option<usize> {
    a.core.ws.pane().cursor_pos()
}

/// The cell drawn for item `index` in the last frame.
fn cell(a: &App, index: usize) -> Option<Rect> {
    a.hits().iter().find_map(|(r, t)| match t {
        HitTarget::PaneRow { index: i, .. } if *i == index => Some(*r),
        _ => None,
    })
}

/// Frames (with `events` in the first) until `cond` holds.
fn until(
    app: &mut App,
    t: &mut Terminal<TestBackend>,
    io: &mut NullIo,
    events: &[Event],
    what: &str,
    cond: impl Fn(&mut App, &Terminal<TestBackend>) -> bool,
) {
    app.frame(t, events, io).unwrap();
    let start = Instant::now();
    while !cond(app, t) {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what} (cursor {:?}, {} selected)",
            cur(app),
            app.core.ws.pane().selection.len()
        );
        std::thread::sleep(Duration::from_millis(10));
        app.frame(t, &[], io).unwrap();
    }
}

fn setup(data: &Path) {
    for c in 'a'..='i' {
        std::fs::write(data.join(format!("{c}.txt")), b"x").unwrap();
    }
    image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 0, 0, 255])).save(data.join("pic.png")).unwrap();
}

/// Grid view: cells of several rows, Left / Right by item and Up / Down by line, the PNG
/// drawn as a half-block thumbnail, clicks select, the gallery's bigger cells.
#[test]
fn tui_grid_navigates_and_draws_thumbnails() {
    match common::relaunch(module_path!(), "tui_grid_navigates_and_draws_thumbnails", setup) {
        Run::Child(data) => grid(data),
        Run::Parent(_) => {}
    }
}

fn grid(data: PathBuf) {
    let mut c = common::test_config();
    c.tui.images = TuiImages::Halfblocks;
    c.tui.color_mode = TuiColorMode::Truecolor;
    c.appearance.show_thumbnails = true;
    let mut app = common::new_app(c, &data);
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut io = NullIo::default();
    until(&mut app, &mut t, &mut io, &[], "the listing", |a, _| pane(a).is_some_and(|p| p.total == 10));

    // The view-mode action (Ctrl+Shift+2).
    until(&mut app, &mut t, &mut io, &[ctrl_shift('2')], "the grid", |a, _| {
        mode(a) == Some(PaneMode::Grid) && a.core.ws.pane().layout.as_ref().is_some_and(|l| l.cols >= 2)
    });
    let cols = app.core.ws.pane().layout.as_ref().map(|l| l.cols).unwrap();
    assert!(cols < 10, "two lines of cells: {cols}");
    let first = cell(&app, 0).expect("the first cell");
    assert!(first.height > 1 && first.width >= 12, "{first:?}");

    // Keyboard: Home, Right by item, Down / Up by line, End, Left.
    until(&mut app, &mut t, &mut io, &[key(KeyCode::Home)], "Home", |a, _| cur(a) == Some(0));
    until(&mut app, &mut t, &mut io, &[key(KeyCode::Right)], "Right", |a, _| cur(a) == Some(1));
    until(&mut app, &mut t, &mut io, &[key(KeyCode::Down)], "Down", |a, _| cur(a) == Some(1 + cols));
    let below = cell(&app, 1 + cols).unwrap();
    assert_eq!((below.x, below.y), (cell(&app, 1).unwrap().x, first.y + first.height), "the next line of cells");
    until(&mut app, &mut t, &mut io, &[key(KeyCode::Up)], "Up", |a, _| cur(a) == Some(1));
    until(&mut app, &mut t, &mut io, &[shift(KeyCode::Right)], "Shift+Right", |a, _| {
        let p = a.core.ws.pane();
        cur(a) == Some(2) && [1, 2].iter().all(|&i| p.key_at(i).is_some_and(|k| p.selection.contains(k)))
    });
    until(&mut app, &mut t, &mut io, &[key(KeyCode::End)], "End", |a, _| cur(a) == Some(9));
    until(&mut app, &mut t, &mut io, &[key(KeyCode::Left)], "Left", |a, _| cur(a) == Some(8));

    // pic.png (the last item) gets a half-block thumbnail in its cell.
    until(&mut app, &mut t, &mut io, &[], "the thumbnail", |a, t| {
        let Some(r) = cell(a, 9) else { return false };
        let has_thumb = pane(a).and_then(|p| p.grid.as_ref()).is_some_and(|g| g.thumb(9).is_some());
        let buf = t.backend().buffer();
        has_thumb
            && (r.y..r.bottom() - 1).any(|y| {
                (r.x..r.right())
                    .any(|x| buf[(x, y)].fg == Color::Rgb(255, 0, 0) || buf[(x, y)].bg == Color::Rgb(255, 0, 0))
            })
    });
    // A text file has no thumbnail.
    assert!(pane(&app).and_then(|p| p.grid.as_ref()).is_some_and(|g| g.thumb(0).is_none()));

    // A click selects the cell's item.
    let r = cell(&app, 3).unwrap();
    let (x, y) = (r.x + r.width / 2, r.y + 1);
    let press =
        [mouse(MouseEventKind::Down(MouseButton::Left), x, y), mouse(MouseEventKind::Up(MouseButton::Left), x, y)];
    until(&mut app, &mut t, &mut io, &press, "the click", |a, _| {
        let p = a.core.ws.pane();
        cur(a) == Some(3) && p.selection.len() == 1 && p.key_at(3).is_some_and(|k| p.selection.contains(k))
    });

    // The gallery: bigger cells.
    until(&mut app, &mut t, &mut io, &[ctrl_shift('5')], "the gallery", |a, _| {
        mode(a) == Some(PaneMode::Gallery) && cell(a, 3).is_some_and(|c| c.height > first.height)
    });
    // Down in the gallery moves by its own line length.
    let gcols = app.core.ws.pane().layout.as_ref().map(|l| l.cols).unwrap();
    until(&mut app, &mut t, &mut io, &[key(KeyCode::Home)], "Home", |a, _| cur(a) == Some(0));
    until(&mut app, &mut t, &mut io, &[key(KeyCode::Down)], "Down", |a, _| cur(a) == Some(gcols.min(9)));
    app.finish();
    common::ok();
}

//! Headless tests of step T2.3: pane views (details / list / grid / Miller, columns, scrolling). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use soyuz_core::config::{Action, KeyChord, keymap};
use soyuz_tui::app::App;
use soyuz_tui::layout::HitTarget;
use soyuz_tui::view::{PaneMode, PaneVm};

use crate::common::{self, Run, Step, at, ctrl, cursor, key, labels};

/// Ctrl+Shift+`digit`, the way a terminal with the kitty keyboard protocol sends it.
fn ctrl_shift(digit: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(digit), KeyModifiers::CONTROL | KeyModifiers::SHIFT))
}

fn pane(a: &App) -> Option<&PaneVm> {
    a.view()?.focused_pane()
}

fn mode(a: &App) -> Option<PaneMode> {
    pane(a).map(|p| p.mode)
}

/// The focused pane's row `index` is drawn in a cell right of the pane's left edge.
fn drawn_in_second_cell(a: &App, index: usize) -> bool {
    let Some(left) = a.regions().pane(a.core.ws.tab().focused).map(|r| r.x) else { return false };
    a.hits().iter().any(|(r, t)| matches!(t, HitTarget::PaneRow { index: i, .. } if *i == index) && r.x > left)
}

/// Switches details → list → Miller → details with the default chords: the list draws
/// several cells per line and Left / Right move through them, Miller shows the parent
/// folder (this one marked) and the folder under the cursor.
#[test]
fn tui_switches_view_modes() {
    let run = common::relaunch(module_path!(), "tui_switches_view_modes", |data| {
        std::fs::create_dir_all(data.join("sub")).unwrap();
        std::fs::write(data.join("sub").join("inner.txt"), b"i").unwrap();
        for f in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(data.join(f), b"x").unwrap();
        }
    });
    if let Run::Child(data) = run {
        run_tui(data);
    }
}

fn run_tui(data: PathBuf) {
    let config = common::test_config();
    let (km, _) = keymap(&config);
    for (action, chord) in [(Action::ViewDetails, "Ctrl+Shift+1"), (Action::ViewList, "Ctrl+Shift+3")] {
        let chord: KeyChord = chord.parse().unwrap();
        assert!(km.chords_for(&action).contains(&chord), "{action:?} is not on {chord}");
    }
    assert!(km.chords_for(&Action::ViewMiller).contains(&"Ctrl+Shift+4".parse().unwrap()));
    let mut app = common::new_app(config, &data);
    let d1 = data.clone();
    let steps = vec![
        Step::new(
            "the details view with its columns",
            move |a| {
                at(a, &d1)
                    && labels(a) == ["sub/", "a.txt", "b.txt", "c.txt"]
                    && mode(a) == Some(PaneMode::Details)
                    && pane(a).is_some_and(|p| p.columns.len() >= 2)
                    && a.hits().iter().any(|(_, t)| matches!(t, HitTarget::PaneColumn { .. }))
            },
            ctrl_shift('3'),
        ),
        Step::new(
            "the list view with several cells per line",
            |a| mode(a) == Some(PaneMode::List) && a.core.ws.pane().layout.as_ref().is_some_and(|l| l.cols >= 2),
            key(KeyCode::Right),
        ),
        Step::new("the cursor on sub/", |a| cursor(a).as_deref() == Some("sub/"), key(KeyCode::Right)),
        Step::new(
            "the cursor on a.txt, in the second cell",
            |a| cursor(a).as_deref() == Some("a.txt") && drawn_in_second_cell(a, 1),
            key(KeyCode::Left),
        ),
        Step::new("the cursor back on sub/", |a| cursor(a).as_deref() == Some("sub/"), ctrl_shift('4')),
        Step::new(
            "Miller columns: the parent with this folder marked, the subfolder's contents",
            |a| {
                let Some(m) = pane(a).filter(|p| p.mode == PaneMode::Miller).and_then(|p| p.miller.as_ref()) else {
                    return false;
                };
                m.parent.rows.iter().any(|r| r.on_path && r.name == "data")
                    && m.child.rows.iter().map(|r| r.name.as_str()).eq(["inner.txt"])
                    && a.hits().iter().any(|(_, t)| matches!(t, HitTarget::MillerRow { parent: false, .. }))
            },
            ctrl_shift('1'),
        ),
        Step::new("the details view again", |a| mode(a) == Some(PaneMode::Details), ctrl('q')),
    ];
    let (terminal, _) = common::run_script(&mut app, steps, 80, 24);
    let screen = common::screen(&terminal);
    let header = screen.lines().nth(1).unwrap_or_default();
    assert!(header.contains("Name") && header.contains("Size"), "{screen}");
    assert!(screen.lines().nth(2).is_some_and(|l| l.starts_with("* sub/")), "the cursor item is selected: {screen}");
    app.finish();
    common::ok();
}

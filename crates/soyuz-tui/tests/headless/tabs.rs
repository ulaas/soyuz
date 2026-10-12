//! Headless tests of step T2.4: tabs, dual and quad panes. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! New tabs are opened on folders of the sandbox (Ctrl+Enter, "open in new tab"): the
//! New Tab action opens the home folder, which tests never list.

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use soyuz_core::config::KeyPreset;
use soyuz_tui::app::App;
use soyuz_tui::layout::HitTarget;

use crate::common::{self, Run, Step, at, ctrl, cursor, key, labels};

fn chord(code: KeyCode, m: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, m))
}

fn fixture(data: &Path) {
    for (dir, file) in [("alpha", "a1.txt"), ("beta", "b1.txt")] {
        std::fs::create_dir_all(data.join(dir)).unwrap();
        std::fs::write(data.join(dir).join(file), b"x").unwrap();
    }
    std::fs::write(data.join("top.txt"), b"t").unwrap();
}

/// The active tab's pane `i` is at `dir`.
fn pane_at(app: &App, i: usize, dir: &Path) -> bool {
    app.core.ws.tab().panes.get(i).is_some_and(|p| p.location == soyuz_core::Location::Dir(dir.to_path_buf()))
}

/// The last frame drew `n` panes with `n` title lines (`n > 1`) and the focus on `focused`.
fn panes_drawn(app: &App, n: usize, focused: usize) -> bool {
    let r = app.regions();
    let titles = if n > 1 { n } else { 0 };
    app.core.ws.tab().shown() == n
        && app.core.ws.tab().focused == focused
        && r.panes.len() == n
        && r.titles.len() == titles
        && app.view().is_some_and(|v| {
            v.tabs.panes.iter().filter(|p| p.focused).map(|p| p.index).collect::<Vec<_>>()
                == if n > 1 { vec![focused] } else { vec![] }
        })
}

/// `n` tabs, tab `active` active, and the strip drawn when there is more than one.
fn tabs_drawn(app: &App, n: usize, active: usize) -> bool {
    let strip = app.hits().iter().filter(|(_, t)| matches!(t, HitTarget::Tab(_))).count();
    app.core.ws.tabs.len() == n
        && app.core.ws.active == active
        && app.view().is_some_and(|v| v.tabs.active == active && v.tabs.tabs().count() == n)
        && (if n > 1 { strip >= n && !app.regions().tabs.is_empty() } else { app.regions().tabs.is_empty() })
}

#[test]
fn tui_tabs_dual_and_quad() {
    match common::relaunch(module_path!(), "tui_tabs_dual_and_quad", fixture) {
        Run::Child(data) => run_tabs(data),
        Run::Parent(_) => {}
    }
}

fn run_tabs(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let (alpha, beta) = (data.join("alpha"), data.join("beta"));
    let d = data.clone();
    let (a1, a2) = (alpha.clone(), alpha.clone());
    let b1 = beta.clone();
    let (d1, d2, d3) = (data.clone(), data.clone(), data.clone());
    let ctrl_shift_tab = chord(KeyCode::BackTab, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    let steps = vec![
        Step::new(
            "the listing, one tab, no strip",
            move |a| at(a, &d) && labels(a) == ["alpha/", "beta/", "top.txt"] && tabs_drawn(a, 1, 0),
            key(KeyCode::Down),
        ),
        Step::new("the cursor on alpha/", |a| cursor(a).as_deref() == Some("alpha/"), ctrl_enter()),
        // Opened in the background: two tabs, the first still active, the strip shown.
        Step::new(
            "a second tab with the strip",
            |a| tabs_drawn(a, 2, 0) && a.view().is_some_and(|v| v.tabs.tabs().any(|t| t.title == "alpha")),
            chord(KeyCode::Tab, KeyModifiers::CONTROL),
        ),
        Step::new(
            "the second tab (Ctrl+Tab) in alpha",
            move |a| tabs_drawn(a, 2, 1) && at(a, &a1) && labels(a) == ["a1.txt"],
            key(KeyCode::Backspace),
        ),
        Step::new(
            "up to the data folder in tab 2",
            move |a| at(a, &d1) && labels(a).len() == 3 && cursor(a).as_deref() == Some("alpha/"),
            key(KeyCode::Down),
        ),
        Step::new("the cursor on beta/", |a| cursor(a).as_deref() == Some("beta/"), key(KeyCode::Enter)),
        Step::new("tab 2 in beta", move |a| at(a, &b1) && labels(a) == ["b1.txt"], ctrl_shift_tab),
        Step::new(
            "back in tab 1 (Ctrl+Shift+Tab), still in the data folder",
            move |a| tabs_drawn(a, 2, 0) && at(a, &d2) && labels(a).len() == 3,
            ctrl('2'),
        ),
        Step::new("tab 2 (Ctrl+2)", |a| tabs_drawn(a, 2, 1), ctrl('1')),
        Step::new("tab 1 (Ctrl+1)", |a| tabs_drawn(a, 2, 0), key(KeyCode::F(9))),
        Step::new("dual panes (F9)", |a| panes_drawn(a, 2, 0), key(KeyCode::Tab)),
        Step::new(
            "the second pane focused (Tab) with its listing",
            |a| panes_drawn(a, 2, 1) && labels(a).len() == 3,
            key(KeyCode::Down),
        ),
        Step::new("the cursor in the second pane", |a| cursor(a).as_deref() == Some("alpha/"), key(KeyCode::Enter)),
        Step::new(
            "the second pane in alpha, the first unchanged",
            move |a| pane_at(a, 1, &a2) && pane_at(a, 0, &d3) && labels(a) == ["a1.txt"],
            key(KeyCode::Tab),
        ),
        Step::new("the first pane focused again", |a| panes_drawn(a, 2, 0), shift_f9()),
        Step::new("four panes (Shift+F9)", |a| panes_drawn(a, 4, 0), key(KeyCode::Tab)),
        Step::new("Tab cycles through the quad", |a| panes_drawn(a, 4, 1), key(KeyCode::Tab)),
        Step::new("the third pane", |a| panes_drawn(a, 4, 2), shift_f9()),
        Step::new("back to dual (Shift+F9)", |a| a.core.ws.tab().shown() == 2, key(KeyCode::F(9))),
        Step::new("a single pane again (F9)", |a| panes_drawn(a, 1, a.core.ws.tab().focused), ctrl('2')),
        Step::new("tab 2 to close", |a| tabs_drawn(a, 2, 1), ctrl('w')),
        Step::new("one tab left, no strip (Ctrl+W)", |a| tabs_drawn(a, 1, 0), ctrl('q')),
    ];
    let (terminal, _) = common::run_script(&mut app, steps, 80, 24);
    let screen = common::screen(&terminal);
    assert!(screen.contains("top.txt"), "{screen}");
    common::ok();
}

fn ctrl_enter() -> Event {
    chord(KeyCode::Enter, KeyModifiers::CONTROL)
}

/// Commander key map: Tab is "focus other pane", Ctrl+U swaps the panes.
#[test]
fn tui_swap_panes() {
    match common::relaunch(module_path!(), "tui_swap_panes", fixture) {
        Run::Child(data) => run_swap(data),
        Run::Parent(_) => {}
    }
}

fn run_swap(data: PathBuf) {
    let mut c = common::test_config();
    c.keys.preset = KeyPreset::Commander;
    let mut app = common::new_app(c, &data);
    let alpha = data.join("alpha");
    let (d1, d2) = (data.clone(), data.clone());
    let (a1, a2) = (alpha.clone(), alpha.clone());
    let steps = vec![
        // F9 is not bound in this preset: Shift+F9 twice (quad, then back to dual).
        Step::new("the listing", move |a| at(a, &d1) && labels(a).len() == 3, shift_f9()),
        Step::new("quad", |a| panes_drawn(a, 4, 0), shift_f9()),
        Step::new("dual panes", |a| panes_drawn(a, 2, 0), key(KeyCode::Tab)),
        Step::new(
            "the second pane with its listing",
            |a| panes_drawn(a, 2, 1) && labels(a).len() == 3,
            key(KeyCode::Down),
        ),
        Step::new("the cursor on alpha/", |a| cursor(a).as_deref() == Some("alpha/"), key(KeyCode::Enter)),
        Step::new("the second pane in alpha", move |a| pane_at(a, 1, &a1) && labels(a) == ["a1.txt"], ctrl('u')),
        Step::new("swapped (Ctrl+U): alpha on the left", move |a| pane_at(a, 0, &a2) && pane_at(a, 1, &d2), ctrl('q')),
    ];
    let (terminal, _) = common::run_script(&mut app, steps, 100, 20);
    // Both pane titles are on screen.
    let screen = common::screen(&terminal);
    assert!(screen.contains("alpha"), "{screen}");
    common::ok();
}

fn shift_f9() -> Event {
    chord(KeyCode::F(9), KeyModifiers::SHIFT)
}

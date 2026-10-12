//! Headless tests of T3: the context menu. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! The menus are driven by [`Driver`]: a state machine that looks at the last frame and
//! sends the next event (the number of rows above "Copy path" depends on the platform and
//! the settings, so the arrows are pressed until the cursor is on it). The host never
//! touches the system clipboard (`set_local_clipboard(false)`); copies are read back from
//! `TuiHost::last_copy` and the OSC 52 bytes written to the terminal.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::state::pane::ContextTarget;
use soyuz_appcore::tr;
use soyuz_core::config::{Config, TuiOsc52};
use soyuz_tui::app::{App, EventSource, LoopEnd, NullIo, TuiEvent, run_loop};
use soyuz_tui::input::TuiFocus;
use soyuz_tui::input::menu::MenuCmd;
use soyuz_tui::layout::HitTarget;
use soyuz_tui::widgets::{Menu, MenuItem};

use crate::common::{self, Run, at, ctrl, cursor, key, key_mods, labels};

/// Sends what `step` returns for the last frame (nothing: wait); 60 s at most.
pub struct Driver<F: FnMut(&App) -> Option<Event>> {
    pub step: F,
    pub since: Instant,
}

impl<F: FnMut(&App) -> Option<Event>> EventSource for Driver<F> {
    fn next(&mut self, app: &App, redraw: &Receiver<()>, timeout: Duration) -> Vec<TuiEvent> {
        assert!(self.since.elapsed() < Duration::from_secs(60), "the menu script timed out");
        if let Some(e) = (self.step)(app) {
            return vec![TuiEvent::Term(e)];
        }
        let _ = redraw.recv_timeout(timeout.min(Duration::from_millis(10)));
        Vec::new()
    }
}

/// Run the loop on an 80 × 24 test terminal with `step` until it sends Ctrl+Q.
pub fn drive(app: &mut App, step: impl FnMut(&App) -> Option<Event>) -> NullIo {
    let mut source = Driver { step, since: Instant::now() };
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
    let mut io = NullIo::default();
    let end = run_loop(app, &mut terminal, &mut source, &mut io).expect("the loop runs");
    assert_eq!(end, LoopEnd::Quit);
    io
}

/// Settings for the menu tests: mouse on, OSC 52 as given.
pub fn config(osc52: TuiOsc52) -> Config {
    let mut c = common::test_config();
    c.tui.mouse = true;
    c.tui.osc52 = osc52;
    c
}

/// A terminal UI on `data` whose copies never reach the system clipboard.
pub fn app(c: Config, data: &Path) -> App {
    let app = common::new_app(c, data);
    app.host.set_local_clipboard(false);
    app.host.set_clipboard_env(soyuz_tui::clipboard::Env::default());
    app
}

/// The open menu in the last frame.
pub fn open_menu(a: &App) -> Option<&Menu<MenuCmd>> {
    a.view()?.menu.menu.as_ref()
}

/// Label of the highlighted row of the open menu (top level).
pub fn menu_cursor(a: &App) -> Option<String> {
    let m = open_menu(a)?;
    match m.items.get(m.cursor?)? {
        MenuItem::Entry(e) => Some(e.label.clone()),
        MenuItem::Separator => None,
    }
}

/// Labels of the open menu's rows (top level; separators are "-").
pub fn menu_labels(a: &App) -> Vec<String> {
    open_menu(a)
        .map(|m| {
            m.items
                .iter()
                .map(|i| match i {
                    MenuItem::Entry(e) => e.label.clone(),
                    MenuItem::Separator => "-".into(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The menu is open, drawn and owns the keyboard.
pub fn menu_open(a: &App) -> bool {
    open_menu(a).is_some() && a.focus() == TuiFocus::Menu
}

/// Press Down until `label` is highlighted, then Enter. `None` while it is not open.
pub fn pick(a: &App, label: &str) -> Option<Event> {
    if !menu_open(a) {
        return None;
    }
    assert!(menu_labels(a).iter().any(|l| l == label), "no {label:?} in {:?}", menu_labels(a));
    Some(if menu_cursor(a).as_deref() == Some(label) { key(KeyCode::Enter) } else { key(KeyCode::Down) })
}

fn two_files(data: &Path) {
    std::fs::write(data.join("a.txt"), b"a").unwrap();
    std::fs::write(data.join("b.txt"), b"bb").unwrap();
}

fn same_file(copied: &str, want: &Path) -> bool {
    Path::new(copied).canonicalize().ok() == want.canonicalize().ok()
}

fn right_click(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// Visible positions of the selected items of the focused pane.
fn selected(a: &App) -> Vec<usize> {
    let p = a.core.ws.pane();
    (0..p.visible.len()).filter(|&i| p.key_at(i).is_some_and(|k| p.selection.contains(k))).collect()
}

#[test]
fn tui_shift_f10_copy_path() {
    if let Run::Child(data) = common::relaunch(module_path!(), "tui_shift_f10_copy_path", two_files) {
        shift_f10_copy_path(data);
    }
}

fn shift_f10_copy_path(data: PathBuf) {
    let mut app = app(config(TuiOsc52::Always), &data);
    let want = data.join("a.txt");
    let copy_path = tr!("app-ctx-copy-path");
    let mut phase = 0;
    let io = drive(&mut app, |a| match phase {
        0 if at(a, &data) && labels(a) == ["a.txt", "b.txt"] => {
            phase = 1;
            Some(key(KeyCode::Down))
        }
        1 if cursor(a).as_deref() == Some("a.txt") => {
            phase = 2;
            Some(key_mods(KeyCode::F(10), KeyModifiers::SHIFT))
        }
        2 => {
            let e = pick(a, copy_path)?;
            if e == key(KeyCode::Enter) {
                phase = 3;
            }
            Some(e)
        }
        3 if !menu_open(a) && a.host.last_copy().is_some_and(|c| same_file(&c, &want)) => {
            phase = 4;
            Some(ctrl('q'))
        }
        _ => None,
    });
    assert_eq!(selected(&app), vec![0], "Shift+F10 selected the cursor's item");
    let out = String::from_utf8_lossy(&io.written);
    assert_eq!(out.matches("\x1b]52;c;").count(), 1, "one OSC 52 copy: {out:?}");
    common::ok();
}

#[test]
fn tui_right_click_menus_and_esc() {
    if let Run::Child(data) = common::relaunch(module_path!(), "tui_right_click_menus_and_esc", two_files) {
        right_click_menus(data);
    }
}

fn right_click_menus(data: PathBuf) {
    let mut app = app(config(TuiOsc52::Never), &data);
    let view = tr!("app-ctx-view");
    let open = tr!("app-ctx-open");
    let mut phase = 0;
    let mut background_row = 0;
    drive(&mut app, |a| {
        match phase {
            // Row 1 (b.txt) is on screen row 3.
            0 if at(a, &data)
                && labels(a) == ["a.txt", "b.txt"]
                && matches!(a.hits().at(4, 3), Some(HitTarget::PaneRow { index: 1, .. })) =>
            {
                phase = 1;
                Some(right_click(4, 3))
            }
            1 if menu_open(a) => {
                assert_eq!(selected(a), vec![1], "the clicked row is selected first");
                assert_eq!(a.core.ws.pane().context, ContextTarget::Items);
                assert_eq!(menu_labels(a).first().map(String::as_str), Some(open));
                phase = 2;
                Some(key(KeyCode::Esc))
            }
            2 if !menu_open(a) && a.focus() == TuiFocus::List => {
                background_row = (4..20)
                    .find(|&y| matches!(a.hits().at(4, y), Some(HitTarget::PaneBackground { .. })))
                    .expect("empty area below the rows");
                phase = 3;
                Some(right_click(4, background_row))
            }
            3 if menu_open(a) => {
                assert!(selected(a).is_empty(), "a right click on the empty area clears the selection");
                assert_eq!(menu_labels(a).first().map(String::as_str), Some(view), "the background menu");
                phase = 4;
                // A press outside the menu closes it.
                Some(right_click(79, 23))
            }
            4 if !menu_open(a) => {
                phase = 5;
                Some(ctrl('q'))
            }
            _ => None,
        }
    });
    assert!(background_row > 3);
    assert!(app.host.last_copy().is_none(), "nothing ran");
    common::ok();
}

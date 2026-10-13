//! Headless tests of Phase T7 (agent A): workspaces (save as, manage, hotkeys), tab
//! history, the close-window path and About. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`, like the other
//! files here.

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::models::frontend::FrontendCmd;
use soyuz_appcore::models::workspaces::{self, WsCmd};
use soyuz_appcore::ops::{self, NavOpts};
use soyuz_appcore::palette::Exec;
use soyuz_appcore::state::workspace_windows::WindowId;
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::Action;
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;
use soyuz_tui::widgets::form::FieldVmKind;

use crate::common::{self, Run, Step, alt, at, ctrl, dialog_title, key, key_mods, toasts, type_text};

fn folders(data: &Path) {
    for d in ["a", "a/b", "a/c", "sub"] {
        std::fs::create_dir_all(data.join(d)).unwrap();
    }
    std::fs::write(data.join("f.txt"), b"x").unwrap();
}

/// The rows of the dialog drawn in the last frame.
pub fn rows(a: &App) -> Vec<String> {
    a.view()
        .and_then(|v| v.dialog.as_ref())
        .map(|d| d.list.iter().map(|r| r.text.clone()).collect())
        .unwrap_or_default()
}

/// The list cursor of the dialog drawn in the last frame.
pub fn row_cursor(a: &App) -> Option<usize> {
    Some(a.view()?.dialog.as_ref()?.list_state.cursor)
}

/// The text of field `i` of the dialog drawn in the last frame.
pub fn field(a: &App, i: usize) -> Option<String> {
    match &a.view()?.dialog.as_ref()?.fields.get(i)?.kind {
        FieldVmKind::Text { text, .. } => Some(text.clone()),
        _ => None,
    }
}

/// The validation message of the dialog drawn in the last frame.
pub fn error(a: &App) -> Option<String> {
    a.view()?.dialog.as_ref()?.error.clone()
}

pub fn no_dialog(a: &App) -> bool {
    dialog_title(a).is_none() && a.focus() == TuiFocus::List
}

fn toast_has(a: &App, s: &str) -> bool {
    toasts(a).iter().any(|t| t.contains(s))
}

/// The sandbox's `workspaces.toml`.
fn store(data: &Path) -> String {
    let dir = data.parent().unwrap().join(soyuz_core::config::PORTABLE_DATA_DIR);
    std::fs::read_to_string(dir.join("workspaces.toml")).unwrap_or_default()
}

/// The names in `workspaces.toml`, in order.
fn names(data: &Path) -> Vec<String> {
    store(data)
        .lines()
        .filter_map(|l| l.trim().strip_prefix("name = \""))
        .map(|n| n.trim_end_matches('"').to_string())
        .collect()
}

/// Steps that run `action` from the palette (Ctrl+K, its words, Enter once it is the
/// first result), starting when `ready` holds.
pub fn palette(steps: &mut Vec<Step>, ready: impl Fn(&App) -> bool + 'static, query: &'static str, action: Action) {
    steps.push(Step::new("ready for the palette", ready, ctrl('k')));
    for (i, c) in query.char_indices() {
        let typed = &query[..i];
        steps.push(Step::new(
            "typing in the palette",
            move |a| a.view().is_some_and(|v| v.palette.open && v.palette.input == typed),
            key(KeyCode::Char(c)),
        ));
    }
    steps.push(Step::new(
        "the action ranked first",
        move |a| {
            let first = a.core.palette.results.get(a.core.palette.selected).map(|i| &i.exec);
            a.view().is_some_and(|v| v.palette.open && v.palette.input == query)
                && first == Some(&Exec::Action(action.clone()))
        },
        key(KeyCode::Enter),
    ));
}

fn backspaces(steps: &mut Vec<Step>, n: usize) {
    for _ in 0..n {
        steps.push(Step::new("erasing", |_| true, key(KeyCode::Backspace)));
    }
}

#[test]
fn save_as_then_update() {
    match common::relaunch(module_path!(), "save_as_then_update", folders) {
        Run::Child(data) => save_as(data),
        Run::Parent(r) => assert_eq!(names(&r.data), ["data", "data2"], "{}", store(&r.data)),
    }
}

fn save_as(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    app.core.cmds.push(Cmd::Action(Action::SaveWorkspace));
    let save_title = || Some(tr!("workspaces-save-title").to_string());
    let saved = tr!("workspaces-saved", name = "data");
    let updated = tr!("workspaces-updated", name = "data");
    let mut steps = vec![Step::new(
        "the dialog with the folder's name",
        move |a| dialog_title(a) == save_title() && field(a, 0).as_deref() == Some("data"),
        key(KeyCode::Enter),
    )];
    let d = data.clone();
    palette(
        &mut steps,
        move |a| no_dialog(a) && toast_has(a, &saved) && names(&d) == ["data"],
        "save workspace",
        Action::SaveWorkspace,
    );
    // The suggestion is now the workspace last saved: saving again updates it.
    steps.push(Step::new(
        "the dialog with the workspace's name",
        move |a| dialog_title(a) == save_title() && field(a, 0).as_deref() == Some("data"),
        key(KeyCode::Enter),
    ));
    palette(&mut steps, move |a| no_dialog(a) && toast_has(a, &updated), "save workspace", Action::SaveWorkspace);
    steps.push(Step::new("the dialog again", move |a| dialog_title(a) == save_title(), key(KeyCode::Backspace)));
    backspaces(&mut steps, 3);
    steps.push(Step::new("an empty name", |a| field(a, 0).as_deref() == Some(""), key(KeyCode::Enter)));
    steps.push(Step::new(
        "refused",
        |a| error(a).as_deref() == Some(tr!("tui-workspaces-name-empty")) && dialog_title(a).is_some(),
        key(KeyCode::Char('d')),
    ));
    type_text(&mut steps, "ata2");
    steps.push(Step::new("typed", |a| field(a, 0).as_deref() == Some("data2"), key(KeyCode::Enter)));
    let d = data.clone();
    steps.push(Step::new("a second workspace", move |a| no_dialog(a) && names(&d) == ["data", "data2"], ctrl('q')));
    // A small screen: the box keeps the field, the buttons and the hints.
    common::run_script(&mut app, steps, 48, 12);
    common::ok();
}

#[test]
fn manage_edits_moves_deletes_and_opens() {
    match common::relaunch(module_path!(), "manage_edits_moves_deletes_and_opens", folders) {
        Run::Child(data) => manage(data),
        Run::Parent(r) => {
            assert_eq!(names(&r.data), ["deux", "one"], "{}", store(&r.data));
            assert!(store(&r.data).contains("hotkey = \"Ctrl+F12\""), "{}", store(&r.data));
        }
    }
}

fn manage(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    // Three workspaces on three folders, saved through the core; the pane ends on `data`.
    for (name, dir) in [("one", data.join("a")), ("two", data.join("sub")), ("three", data.clone())] {
        let id = app.core.ws.pane().id;
        ops::navigate(&mut app.core, id, Location::Dir(dir), NavOpts::user());
        workspaces::apply(&mut app.core, WsCmd::Save(name.into()));
    }
    let title = || Some(tr!("workspaces-manage-title").to_string());
    let d = data.clone();
    // The palette opens the dialog (`Action::ManageWorkspaces`).
    let mut steps = Vec::new();
    palette(&mut steps, |a| a.view().is_some() && no_dialog(a), "manage workspaces", Action::ManageWorkspaces);
    steps.extend([
        Step::new(
            "the list",
            move |a| {
                dialog_title(a) == title()
                    && rows(a).len() == 3
                    && rows(a)[0].starts_with("one")
                    && names(&d) == ["one", "two", "three"]
            },
            key(KeyCode::Down),
        ),
        Step::new("on two", |a| row_cursor(a) == Some(1), key_mods(KeyCode::Up, KeyModifiers::ALT)),
    ]);
    let d = data.clone();
    steps.push(Step::new(
        "moved up, the cursor with it",
        move |a| names(&d) == ["two", "one", "three"] && row_cursor(a) == Some(0) && rows(a)[0].starts_with("two"),
        key(KeyCode::F(2)),
    ));
    steps.push(Step::new(
        "editing",
        |a| field(a, 0).as_deref() == Some("two") && field(a, 1).as_deref() == Some(""),
        key(KeyCode::Tab),
    ));
    type_text(&mut steps, "Nope+");
    steps.push(Step::new("typed a bad hotkey", |a| field(a, 1).as_deref() == Some("Nope+"), key(KeyCode::Enter)));
    steps.push(Step::new(
        "refused",
        |a| error(a).is_some_and(|e| e.contains("Nope+")) && field(a, 1).is_some(),
        key(KeyCode::Backspace),
    ));
    backspaces(&mut steps, 4);
    type_text(&mut steps, "Ctrl+F12");
    steps.push(Step::new(
        "a good hotkey",
        |a| field(a, 1).as_deref() == Some("Ctrl+F12"),
        key_mods(KeyCode::BackTab, KeyModifiers::SHIFT),
    ));
    backspaces(&mut steps, 3);
    type_text(&mut steps, "deux");
    steps.push(Step::new("renamed", |a| field(a, 0).as_deref() == Some("deux"), key(KeyCode::Enter)));
    let d = data.clone();
    steps.push(Step::new(
        "saved, back to the list",
        move |a| {
            names(&d) == ["deux", "one", "three"]
                && store(&d).contains("Ctrl+F12")
                && field(a, 0).is_none()
                && rows(a).first().is_some_and(|r| r.starts_with("deux"))
        },
        key(KeyCode::End),
    ));
    steps.push(Step::new("on three", |a| row_cursor(a) == Some(2), key(KeyCode::Delete)));
    let d = data.clone();
    let deleted = tr!("workspaces-deleted", name = "three");
    steps.push(Step::new(
        "deleted at once",
        move |a| {
            names(&d) == ["deux", "one"] && rows(a).len() == 2 && toast_has(a, &deleted) && row_cursor(a) == Some(1)
        },
        key(KeyCode::Enter),
    ));
    let (d, a_dir) = (data.clone(), data.join("a"));
    let opened = tr!("workspaces-opened", name = "one");
    steps.push(Step::new(
        "opened one",
        move |a| no_dialog(a) && at(a, &a_dir) && toast_has(a, &opened),
        key_mods(KeyCode::F(12), KeyModifiers::CONTROL),
    ));
    let sub = d.join("sub");
    steps.push(Step::new("the hotkey opened deux", move |a| at(a, &sub), ctrl('q')));
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

#[test]
fn tab_history_goes_back_to_a_branch() {
    match common::relaunch(module_path!(), "tab_history_goes_back_to_a_branch", folders) {
        Run::Child(data) => history(data),
        Run::Parent(_) => {}
    }
}

fn history(data: PathBuf) {
    let mut c = common::test_config();
    c.tui.mouse = true;
    let mut app = common::new_app(c, &data);
    // data → a → b, back to a, then c: a branch at a.
    let pane = app.core.ws.pane().id;
    for loc in [data.join("a"), data.join("a").join("b")] {
        app.core.cmds.push(Cmd::Navigate { pane, location: Location::Dir(loc), select: None });
    }
    app.core.cmds.push(Cmd::Action(Action::Back));
    app.core.cmds.push(Cmd::Navigate { pane, location: Location::Dir(data.join("a").join("c")), select: None });
    app.core.cmds.push(Cmd::Action(Action::TabHistory));
    let title = tr!("workspaces-history-title").to_string();
    let b_dir = data.join("a").join("b");
    let mut phase = 0;
    super::menu::drive(&mut app, |a| match phase {
        // The tree with ASCII guides (the test config's icons), the cursor on c.
        0 if dialog_title(a).as_deref() == Some(title.as_str())
            && rows(a).len() == 4
            && rows(a)[1].starts_with("`-a")
            && rows(a)[2].starts_with("  |-b")
            && rows(a)[3].starts_with("  `-c")
            && row_cursor(a) == Some(3) =>
        {
            phase = 1;
            Some(key(KeyCode::Up))
        }
        // A click on the row of b, then a second click goes there.
        1 if row_cursor(a) == Some(2) => {
            let (x, y) = row_pos(a, 1)?;
            phase = 2;
            Some(click(x, y))
        }
        2 if row_cursor(a) == Some(1) => {
            let (x, y) = row_pos(a, 2)?;
            phase = 3;
            Some(click(x, y))
        }
        3 if row_cursor(a) == Some(2) => {
            let (x, y) = row_pos(a, 2)?;
            phase = 4;
            Some(click(x, y))
        }
        4 if no_dialog(a) && at(a, &b_dir) => {
            phase = 5;
            Some(ctrl('q'))
        }
        _ => None,
    });
    common::ok();
}

/// The screen cell of dialog row `i` in the last frame.
pub fn row_pos(a: &App, i: usize) -> Option<(u16, u16)> {
    a.hits().iter().find(|(_, t)| *t == HitTarget::DialogRow(i)).map(|(r, _)| (r.x + 2, r.y))
}

pub fn click(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn about_copies_info_and_window_requests_stay_quiet() {
    match common::relaunch(module_path!(), "about_copies_info_and_window_requests_stay_quiet", folders) {
        Run::Child(data) => about(data),
        Run::Parent(_) => {}
    }
}

fn about(data: PathBuf) {
    let mut app = super::menu::app(super::menu::config(soyuz_core::config::TuiOsc52::Never), &data);
    // Unreachable in the terminal UI (no other windows): logged, no dialog, no toast.
    app.core.frontend.push(FrontendCmd::CloseWindowPrompt { id: WindowId(7), tabs: 3 });
    // Merging windows says it is not available here.
    app.core.cmds.push(Cmd::Action(Action::MergeWindows));
    app.core.cmds.push(Cmd::Action(Action::About));
    let title = tr!("dialogs-about-title").to_string();
    let not_here = tr!("tui-not-available").to_string();
    let commit = tr!("dialogs-about-commit").to_string();
    let mut phase = 0;
    super::menu::drive(&mut app, |a| match phase {
        0 if dialog_title(a).as_deref() == Some(title.as_str())
            && rows(a).iter().any(|r| r.starts_with(commit.as_str()))
            && toast_has(a, &not_here) =>
        {
            phase = 1;
            Some(alt('c'))
        }
        1 if no_dialog(a) && a.host.last_copy().is_some_and(|t| t.starts_with("Soyuz\nVersion: ")) => {
            phase = 2;
            Some(ctrl('q'))
        }
        _ => None,
    });
    common::ok();
}

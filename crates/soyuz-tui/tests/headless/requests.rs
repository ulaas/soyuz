//! Headless tests of T3.2: the dialogs the core requests (`FrontendCmd`): confirm, select
//! by pattern, columns and open with. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;

use crossterm::event::KeyCode;
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::models::frontend::FrontendCmd;
use soyuz_appcore::models::requests::columns;
use soyuz_appcore::models::toasts::Toast;
use soyuz_core::Column;
use soyuz_core::config::Action;
use soyuz_tui::input::TuiFocus;

use crate::common::{self, Run, Step, ctrl, dialog_title, key, labels, toasts};

fn files(data: &std::path::Path) {
    for n in ["a.txt", "b.txt", "c.rs"] {
        std::fs::write(data.join(n), n).unwrap();
    }
}

fn no_dialog(a: &soyuz_tui::app::App) -> bool {
    dialog_title(a).is_none() && a.focus() == TuiFocus::List
}

#[test]
fn select_by_pattern_selects_matching_rows() {
    match common::relaunch(module_path!(), "select_by_pattern_selects_matching_rows", files) {
        Run::Child(data) => select_by_pattern(data),
        Run::Parent(_) => {}
    }
}

fn select_by_pattern(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    app.core.cmds.push(Cmd::Action(Action::SelectByPattern));
    let mut steps = vec![Step::new(
        "the dialog over the listed folder",
        |a| dialog_title(a).as_deref() == Some("Select by pattern") && labels(a).iter().any(|l| l.contains("c.rs")),
        key(KeyCode::Char('t')),
    )];
    // `*.` + `txt`.
    common::type_text(&mut steps, "xt");
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
    steps.push(Step::new(
        "the .txt files selected",
        |a| {
            let sel = &a.core.ws.pane().selection;
            no_dialog(a)
                && sel.len() == 2
                && sel.contains("a.txt")
                && sel.contains("b.txt")
                && toasts(a).iter().any(|t| t.contains('2'))
        },
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

#[test]
fn column_chooser_toggles_a_column() {
    match common::relaunch(module_path!(), "column_chooser_toggles_a_column", files) {
        Run::Child(data) => column_chooser(data),
        Run::Parent(_) => {}
    }
}

fn column_chooser(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let was = columns::is_visible(app.core.ws.pane(), Column::Created);
    let row = columns::choosable().iter().position(|&c| c == Column::Created).expect("Created is offered");
    app.core.cmds.push(Cmd::Action(Action::ChooseColumns));
    let mut steps =
        vec![Step::new("the chooser", |a| dialog_title(a).as_deref() == Some("Columns"), key(KeyCode::Home))];
    for _ in 0..row {
        steps.push(Step::new("down", |_| true, key(KeyCode::Down)));
    }
    steps.push(Step::new(
        "the cursor on Created",
        |a| {
            let d = a.view().and_then(|v| v.dialog.as_ref());
            d.is_some_and(|d| d.list.get(d.list_state.cursor).is_some_and(|r| r.text.contains("Created")))
        },
        key(KeyCode::Char(' ')),
    ));
    steps.push(Step::new(
        "Created toggled, its tick redrawn",
        move |a| {
            let d = a.view().and_then(|v| v.dialog.as_ref());
            let tick = if was { "[ ]" } else { "[x]" };
            columns::is_visible(a.core.ws.pane(), Column::Created) != was
                && d.is_some_and(|d| d.list.iter().any(|r| r.text.starts_with(tick) && r.text.contains("Created")))
        },
        key(KeyCode::Enter),
    ));
    steps.push(Step::new("closed", no_dialog, ctrl('q')));
    common::run_script(&mut app, steps, 80, 30);
    common::ok();
}

#[test]
fn open_with_lists_apps_or_says_none_without_launching() {
    match common::relaunch(module_path!(), "open_with_lists_apps_or_says_none_without_launching", files) {
        Run::Child(data) => open_with(data),
        Run::Parent(_) => {}
    }
}

fn open_with(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    app.core.cmds.push(Cmd::ChooseOpenWith(data.join("a.txt")));
    let steps = vec![
        Step::new(
            "the applications or the empty state",
            |a| {
                let Some(d) = a.view().and_then(|v| v.dialog.as_ref()) else { return false };
                d.title.contains("a.txt")
                    && (!d.list.is_empty() || d.body.iter().any(|b| b.contains("No registered applications")))
            },
            // Open with nothing typed and no row: a message, nothing launched.
            key(KeyCode::Char('z')),
        ),
        Step::new("typed a relative program", |a| dialog_title(a).is_some(), key(KeyCode::Enter)),
        Step::new(
            "not a program",
            |a| a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.error.is_some()),
            key(KeyCode::Esc),
        ),
        Step::new("closed", no_dialog, ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

#[test]
fn confirm_runs_the_command_only_when_confirmed() {
    match common::relaunch(module_path!(), "confirm_runs_the_command_only_when_confirmed", files) {
        Run::Child(data) => confirm(data),
        Run::Parent(_) => {}
    }
}

fn confirm(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let ask = |what: &str| FrontendCmd::Confirm {
        title: format!("Really {what}?"),
        message: "It cannot be undone".into(),
        confirm: "Go".into(),
        on_confirm: Cmd::Toast(Toast::info(format!("ran {what}"))),
    };
    app.core.frontend.push(ask("one"));
    app.core.frontend.push(ask("two"));
    let steps = vec![
        // The newest is on top: cancel it.
        Step::new("two on top", |a| dialog_title(a).as_deref() == Some("Really two?"), key(KeyCode::Esc)),
        Step::new("one uncovered", |a| dialog_title(a).as_deref() == Some("Really one?"), key(KeyCode::Enter)),
        Step::new(
            "only one ran",
            |a| {
                let t = toasts(a);
                no_dialog(a) && t.iter().any(|s| s == "ran one") && !t.iter().any(|s| s == "ran two")
            },
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

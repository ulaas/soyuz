//! Headless tests of Phase T7 (agent B): previous versions (also from Properties). Each
//! test runs in its own sandbox: `crate::common::relaunch(module_path!(), "<test name>",
//! |data| ...)`, like the other files here.
//!
//! The scan is the real one (`soyuz_fs::snapshots`, read-only) on a new temp file: it
//! finds no earlier version, and the dialog explains why (no copies, or copies only an
//! administrator may read: a message, never a restart button). Nothing is opened,
//! restored or elevated.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::models::versions::looking_text;
use soyuz_appcore::tr;
use soyuz_core::config::Action;
use soyuz_tui::app::App;

use crate::common::{self, Run, Step, alt, at, ctrl, cursor, dialog_title, key, key_mods, labels, toasts};

/// The buttons of the dialog drawn in the last frame.
fn buttons(a: &App) -> Vec<String> {
    a.view()
        .and_then(|v| v.dialog.as_ref())
        .map(|d| d.buttons.iter().map(|b| b.label.clone()).collect())
        .unwrap_or_default()
}

/// The versions dialog of `name` shows, its scan done (a note or the list).
fn settled(a: &App, name: &str) -> bool {
    let Some(d) = a.view().and_then(|v| v.dialog.as_ref()) else { return false };
    d.title == tr!("tui-versions-title", name = name) && d.body.len() >= 2 && d.body[1] != looking_text()
}

#[test]
fn versions_open_from_properties() {
    let run = common::relaunch(module_path!(), "versions_open_from_properties", |data| {
        std::fs::write(data.join("a.txt"), b"now").unwrap();
    });
    if let Run::Child(data) = run {
        from_properties(data);
    }
}

fn from_properties(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let props = tr!("dialogs-props-title-one", name = "a.txt");
    let p = props.clone();
    let p2 = props.clone();
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d) && labels(a) == ["a.txt"], key(KeyCode::Down)),
        Step::new(
            "the cursor on a.txt",
            |a| cursor(a).as_deref() == Some("a.txt"),
            key_mods(KeyCode::Enter, KeyModifiers::ALT),
        ),
        Step::new(
            "Properties with Previous versions…",
            move |a| {
                dialog_title(a).as_deref() == Some(p.as_str())
                    && buttons(a).contains(&tr!("tui-versions-button").into())
            },
            alt('v'),
        ),
        Step::new(
            "the versions dialog settled, no administrator button",
            |a| settled(a, "a.txt") && !buttons(a).contains(&tr!("tools-versions-elevate").into()),
            key(KeyCode::F(5)),
        ),
        Step::new("looked again", |a| settled(a, "a.txt"), key(KeyCode::Esc)),
        Step::new("Properties again", move |a| dialog_title(a).as_deref() == Some(p2.as_str()), key(KeyCode::Esc)),
        Step::new("closed", |a| dialog_title(a).is_none(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

#[test]
fn versions_turned_off_toast() {
    let run = common::relaunch(module_path!(), "versions_turned_off_toast", |data| {
        std::fs::write(data.join("a.txt"), b"now").unwrap();
    });
    if let Run::Child(data) = run {
        let mut c = common::test_config();
        c.sync.previous_versions = false;
        let mut app = common::new_app(c, &data);
        let d = data.clone();
        let props = tr!("dialogs-props-title-one", name = "a.txt");
        let steps = vec![
            Step::new("the listing", move |a| at(a, &d) && labels(a) == ["a.txt"], key(KeyCode::Down)),
            Step::new(
                "the cursor on a.txt",
                |a| cursor(a).as_deref() == Some("a.txt"),
                key_mods(KeyCode::Enter, KeyModifiers::ALT),
            ),
            Step::new(
                "Properties without Previous versions…",
                move |a| {
                    dialog_title(a).as_deref() == Some(props.as_str())
                        && !buttons(a).contains(&tr!("tui-versions-button").into())
                },
                key(KeyCode::Esc),
            ),
            Step::new("closed", |a| dialog_title(a).is_none(), ctrl('q')),
        ];
        // The action itself says previous versions are off (`models::tools`), no dialog.
        app.core.cmds.push(Cmd::Action(Action::PreviousVersions));
        let mut first = steps;
        first.insert(
            0,
            Step::new(
                "the turned-off toast",
                |a| toasts(a).iter().any(|t| t == tr!("tools-toast-versions-off")) && dialog_title(a).is_none(),
                key(KeyCode::Home),
            ),
        );
        common::run_script(&mut app, first, 100, 30);
        common::ok();
    }
}

//! Headless tests of T5: remote locations (W2-locations). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! Nothing here reaches the network or the OS credential store: the connect form is only
//! validated and cancelled, host-key questions are built in the test, and the saved
//! connection of the sidebar test has no secret.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_appcore::models::frontend::FrontendCmd;
use soyuz_appcore::models::remote::{HostKeyAsk, RemotePrompt};
use soyuz_appcore::services::remote::{self as svc, AuthKind, SavedConn};
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::{KeyContext, SidebarSection};
use soyuz_tui::app::App;

use super::menu::{app, drive, menu_labels, menu_open, pick};
use crate::common::{self, Run, Step, ctrl, dialog_title, key, key_mods};

fn sidebar_cursor(a: &App) -> Option<String> {
    a.view()?.sidebar.lines.iter().find(|l| l.cursor).map(|l| l.label.clone())
}

/// The validation message of the open dialog.
fn dialog_error(a: &App) -> Option<String> {
    a.view()?.dialog.as_ref()?.error.clone()
}

#[test]
fn connect_form_needs_a_server_and_cancels() {
    let run = common::relaunch(module_path!(), "connect_form_needs_a_server_and_cancels", |_| {});
    if let Run::Child(data) = run {
        connect_form(data);
    }
}

fn connect_form(data: PathBuf) {
    let mut c = common::test_config();
    c.keys.bindings.insert("connect_to_server".into(), vec!["F9".into()]);
    let mut app = common::new_app(c, &data);
    let d = data.clone();
    let steps = vec![
        Step::new("the listing", move |a| common::at(a, &d), key(KeyCode::F(9))),
        Step::new(
            "the connect form, on the server field",
            |a| {
                let Some(v) = a.view().and_then(|v| v.dialog.clone()) else { return false };
                v.title == tr!("remote-dialog-connect-title")
                    && v.fields.iter().any(|f| f.label == tr!("remote-protocol"))
                    && v.fields.iter().any(|f| f.focused && f.label == tr!("remote-server"))
            },
            key(KeyCode::Enter),
        ),
        Step::new(
            "Connect refused without a server",
            |a| {
                dialog_title(a).as_deref() == Some(tr!("remote-dialog-connect-title"))
                    && dialog_error(a).as_deref() == Some(tr!("remote-error-server"))
            },
            key(KeyCode::Esc),
        ),
        Step::new("the form closed", |a| dialog_title(a).is_none(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    assert!(common::at(&app, &data), "nothing was opened");
    assert!(svc::connections().is_empty(), "nothing was saved");
    common::ok();
}

#[test]
fn host_key_trust_answers_yes_and_escape_no() {
    let run = common::relaunch(module_path!(), "host_key_trust_answers_yes_and_escape_no", |_| {});
    if let Run::Child(data) = run {
        host_keys(data);
    }
}

fn host_keys(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let (first_tx, first) = crossbeam_channel::bounded(2);
    let (changed_tx, changed) = crossbeam_channel::bounded(2);
    let ask = HostKeyAsk::from_parts("first.example", 22, "ssh-ed25519", "SHA256:new", None, first_tx);
    app.core.frontend.push(FrontendCmd::Remote(RemotePrompt::HostKey(ask)));
    let ask = HostKeyAsk::from_parts(
        "moved.example",
        2222,
        "ssh-ed25519",
        "SHA256:other",
        Some("SHA256:old".into()),
        changed_tx,
    );
    app.core.frontend.push(FrontendCmd::Remote(RemotePrompt::HostKey(ask)));
    let steps = vec![
        // The newest question is on top: a changed key, drawn as an alert, Cancel focused.
        Step::new(
            "the changed key",
            |a| {
                let Some(v) = a.view().and_then(|v| v.dialog.clone()) else { return false };
                v.title == tr!("remote-hostkey-changed-title")
                    && v.alert
                    && v.body.iter().any(|l| l.contains("SHA256:old"))
                    && v.body.iter().any(|l| l.contains("moved.example:2222"))
            },
            key(KeyCode::Esc),
        ),
        Step::new(
            "the first-time question",
            |a| dialog_title(a).as_deref() == Some(tr!("remote-hostkey-trust-title")),
            key(KeyCode::Enter),
        ),
        Step::new("both answered", |a| dialog_title(a).is_none(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    assert_eq!(changed.try_iter().collect::<Vec<_>>(), vec![false], "Esc: don't trust");
    assert_eq!(first.try_iter().collect::<Vec<_>>(), vec![true], "Trust and connect");
    common::ok();
}

#[test]
fn network_row_menu_and_connect_row() {
    let run = common::relaunch(module_path!(), "network_row_menu_and_connect_row", |_| {});
    if let Run::Child(data) = run {
        network(data);
    }
}

fn network(data: PathBuf) {
    // A saved connection without a secret (no credential store call), in the sandbox.
    let conn = SavedConn {
        id: String::new(),
        name: "Lab".into(),
        uri: "sftp://me@lab.invalid".into(),
        auth: AuthKind::Password,
        key_file: None,
        has_secret: false,
    };
    assert_eq!(svc::save_connection(conn, None), None);
    let address = svc::connections()[0].parsed().unwrap().display();
    let mut c = common::test_config();
    c.layout.sidebar = true;
    c.sidebar.sections = vec![SidebarSection::Network];
    let mut app = app(c, &data);
    let mut stage = 0;
    let mut labels = Vec::new();
    let shift_f10 = key_mods(KeyCode::F(10), KeyModifiers::SHIFT);
    drive(&mut app, |a| match stage {
        0 if a.view().is_some_and(|v| v.sidebar.lines.iter().any(|l| l.label == tr!("remote-connect"))) => {
            stage = 1;
            Some(key(KeyCode::F(12)))
        }
        1 if a.key_context() == KeyContext::Sidebar && sidebar_cursor(a).as_deref() == Some("Lab") => {
            stage = 2;
            Some(shift_f10.clone())
        }
        2 if menu_open(a) => {
            labels = menu_labels(a);
            let e = pick(a, tr!("remote-copy-address"));
            if e.as_ref().is_some_and(|e| *e == key(KeyCode::Enter)) {
                stage = 3;
            }
            e
        }
        3 if !menu_open(a) && a.host.last_copy().is_some() => {
            stage = 4;
            Some(key(KeyCode::F(12)))
        }
        4 if a.key_context() == KeyContext::Sidebar => {
            if sidebar_cursor(a).as_deref() == Some(tr!("remote-connect")) {
                stage = 5;
                Some(key(KeyCode::Enter))
            } else {
                Some(key(KeyCode::Down))
            }
        }
        5 if dialog_title(a).as_deref() == Some(tr!("remote-dialog-connect-title")) => {
            stage = 6;
            Some(key(KeyCode::Esc))
        }
        6 if dialog_title(a).is_none() => {
            stage = 7;
            Some(ctrl('q'))
        }
        _ => None,
    });
    assert_eq!(stage, 7);
    for want in
        [tr!("remote-open-new-tab"), tr!("remote-edit-connection"), tr!("remote-disconnect"), tr!("remote-remove")]
    {
        assert!(labels.iter().any(|l| l == want), "{want:?} in {labels:?}");
    }
    assert_eq!(app.host.last_copy().as_deref(), Some(address.as_str()));
    assert!(!matches!(app.core.ws.pane().location, Location::Remote(_)), "nothing was opened");
    common::ok();
}

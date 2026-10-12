//! Browses a temporary folder (down twice, up, into a subfolder and back), opens the delete
//! confirmation and cancels it, then quits with Ctrl+Q and checks that the terminal UI's
//! own session file was written.

use std::path::PathBuf;

use crossterm::event::KeyCode;
use soyuz_core::config::{self, KeyContext};

use crate::common::{self, Run, Step, at, ctrl, cursor, key, labels};

#[test]
fn tui_browses_confirms_and_quits() {
    let run = common::relaunch(module_path!(), "tui_browses_confirms_and_quits", |data| {
        std::fs::create_dir_all(data.join("sub")).unwrap();
        std::fs::write(data.join("sub").join("inner.txt"), b"i").unwrap();
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join("b.txt"), b"bb").unwrap();
    });
    match run {
        Run::Child(data) => run_tui(data),
        Run::Parent(p) => {
            assert!(p.data.join("a.txt").is_file(), "the cancelled delete kept a.txt");
            assert!(p.config_dir().join("session-tui.toml").is_file(), "the TUI saved its own session");
            assert!(!p.config_dir().join("session.toml").exists(), "the GUI session is untouched");
        }
    }
}

fn run_tui(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let sub = data.join("sub");
    let d1 = data.clone();
    let d2 = data.clone();
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d1) && labels(a) == ["sub/", "a.txt", "b.txt"], key(KeyCode::Down)),
        // Like the GUI, the first Down puts the cursor on the first item.
        Step::new("the cursor on sub/ first", |a| cursor(a).as_deref() == Some("sub/"), key(KeyCode::Down)),
        Step::new("the cursor on a.txt", |a| cursor(a).as_deref() == Some("a.txt"), key(KeyCode::Up)),
        Step::new("the cursor on sub/", |a| cursor(a).as_deref() == Some("sub/"), key(KeyCode::Enter)),
        Step::new("the subfolder", move |a| at(a, &sub) && labels(a) == ["inner.txt"], key(KeyCode::Backspace)),
        Step::new(
            "back in the folder with the cursor on sub/",
            move |a| at(a, &d2) && labels(a).len() == 3 && cursor(a).as_deref() == Some("sub/"),
            key(KeyCode::Down),
        ),
        Step::new("the cursor on a.txt again", |a| cursor(a).as_deref() == Some("a.txt"), key(KeyCode::Delete)),
        Step::new(
            "the delete confirmation",
            |a| {
                a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.title.contains("a.txt"))
                    && a.key_context() == KeyContext::Dialog
            },
            key(KeyCode::Esc),
        ),
        Step::new(
            "the confirmation to close",
            |a| a.view().is_some_and(|v| v.dialog.is_none()) && !a.core.dialogs.is_open(),
            ctrl('q'),
        ),
    ];
    let (terminal, io) = common::run_script(&mut app, steps, 80, 24);
    assert!(data.join("a.txt").is_file(), "Esc cancelled the delete");
    // The window title went out as OSC 2 after a frame.
    let written = String::from_utf8_lossy(&io.written);
    assert!(written.contains("\x1b]2;data"), "{written:?}");
    // The last frame shows the listing and the status line.
    let screen = common::screen(&terminal);
    assert!(screen.contains("a.txt") && screen.contains("3 items"), "{screen}");
    // The list rows were click targets with the pane's layout in cell units.
    assert!(app.hits().iter().any(|(_, t)| matches!(t, soyuz_tui::layout::HitTarget::PaneRow { .. })));
    let pane = app.core.ws.pane();
    assert_eq!(pane.layout.as_ref().map(|l| (l.n, l.cols)), Some((3, 1)));
    assert_eq!(pane.viewport_h, 22.0);
    app.finish();
    assert!(config::config_dir().join("session-tui.toml").is_file(), "session-tui.toml written");
    common::ok();
}

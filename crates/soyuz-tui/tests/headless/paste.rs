//! A terminal paste (`Event::Paste`) in the file list runs Soyuz's Paste instead of typing
//! into type-ahead; in a text field it stays text. `App::list_paste` is replaced by a
//! counter, so the test never reads the system clipboard.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossterm::event::{Event, KeyCode};
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;

use crate::common::{self, Run, Step, at, ctrl, cursor, key};

static PASTES: AtomicUsize = AtomicUsize::new(0);

fn pastes() -> usize {
    PASTES.load(Ordering::SeqCst)
}

fn address(a: &App) -> Option<String> {
    (a.focus() == TuiFocus::Address && !a.core.ui.address.request_focus)
        .then(|| a.view().and_then(|v| v.crumbs.edit.as_ref()).map(|e| e.text.clone()))
        .flatten()
}

#[test]
fn tui_paste_in_the_list_runs_paste() {
    let run = common::relaunch(module_path!(), "tui_paste_in_the_list_runs_paste", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join("b.txt"), b"b").unwrap();
    });
    if let Run::Child(data) = run {
        run_paste(data);
    }
}

fn run_paste(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    app.list_paste = |_| {
        PASTES.fetch_add(1, Ordering::SeqCst);
    };
    let d1 = data.clone();
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d1) && common::labels(a).len() == 2, key(KeyCode::Down)),
        // Typed, `b` would move the cursor to b.txt (type-ahead); pasted, it pastes.
        Step::new("the cursor on a.txt", |a| cursor(a).as_deref() == Some("a.txt"), Event::Paste("b".into())),
        Step::new("one paste", |a| pastes() == 1 && cursor(a).as_deref() == Some("a.txt"), ctrl('l')),
        Step::new("the address field", |a| address(a).is_some(), Event::Paste("zz".into())),
        Step::new(
            "the text pasted into the field",
            |a| address(a).is_some_and(|t| t.contains("zz")),
            key(KeyCode::Esc),
        ),
        Step::new("the list again, still one paste", |a| a.focus() == TuiFocus::List && pastes() == 1, ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    assert_eq!(pastes(), 1);
    assert_eq!(cursor(&app).as_deref(), Some("a.txt"), "the paste did not reach type-ahead");
    common::ok();
}

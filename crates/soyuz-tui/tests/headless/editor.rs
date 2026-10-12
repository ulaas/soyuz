//! Headless tests of the editors (Phase T4) and of quitting with unsaved changes. Each
//! test runs in its own sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use soyuz_appcore::editors::{self, DocKind};

use crate::common::{self, Run, Step, ctrl};

/// A document with unsaved changes never hangs a quit: without an editor view the
/// terminal UI saves it (`editors::pump` runs every frame), then quits.
#[test]
fn tui_quit_saves_a_dirty_document() {
    let run = common::relaunch(module_path!(), "tui_quit_saves_a_dirty_document", |data| {
        std::fs::write(data.join("note.txt"), b"world\n").unwrap();
    });
    match run {
        Run::Child(data) => run_tui(data),
        Run::Parent(p) => {
            let text = std::fs::read_to_string(p.data.join("note.txt")).unwrap();
            assert!(text.starts_with("hello"), "the edit was saved: {text:?}");
        }
    }
}

fn run_tui(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let file = data.join("note.txt");
    editors::devscript(&mut app.core, &app.host, "edit", &file.display().to_string());
    let start = Instant::now();
    while !app.core.editors.docs.first().is_some_and(|d| matches!(d.kind, DocKind::Text(_))) {
        assert!(start.elapsed() < Duration::from_secs(30), "the document never loaded");
        editors::pump(&mut app.core, &app.host);
        std::thread::sleep(Duration::from_millis(10));
    }
    editors::devscript(&mut app.core, &app.host, "editor", "type=hello");
    assert_eq!(app.core.editors.dirty_count(), 1, "typing made the document dirty");
    let steps = vec![Step::new("the first frame", |a| a.view().is_some(), ctrl('q'))];
    common::run_script(&mut app, steps, 80, 24);
    assert_eq!(app.core.editors.dirty_count(), 0, "saved before quitting");
    common::ok();
}

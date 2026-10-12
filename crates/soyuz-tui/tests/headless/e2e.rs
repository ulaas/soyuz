//! One run through the T3 / T4 terminal UI end to end, in its own sandbox
//! (`crate::common::relaunch`): copy a file with Ctrl+C / Ctrl+V into a folder that has
//! one of the same name (Keep both), see the copy in the jobs panel's history (Alt+J,
//! Tab), rename a file inline (F2), show it in the preview pane (Alt+P) and in Quick Look
//! (Space, Space closes), then edit it (F4), type, save (Ctrl+S) and close it (Ctrl+W).
//!
//! The system clipboard is off in the child process
//! (`os_clipboard::set_system_clipboard(false)`): Ctrl+C / Ctrl+V go through the
//! internal clipboard only and never touch the user's.

use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use soyuz_appcore::editors::DocKind;
use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::view::editor::BodyVm;

use crate::common::{self, Run, Step, alt, at, ctrl, cursor, key, labels, type_text, wait_jobs_idle};

/// Names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// The first line of `p` (empty when it cannot be read).
fn first_line(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default().lines().next().unwrap_or_default().to_string()
}

/// The inline rename field shows `text`.
fn rename_text(a: &App, text: &str) -> bool {
    a.view()
        .and_then(|v| v.focused_pane())
        .is_some_and(|p| p.rows().any(|r| r.edit.as_ref().is_some_and(|e| e.text == text)))
}

#[test]
fn tui_copy_paste_jobs_rename_preview_quicklook_edit() {
    let run = common::relaunch(module_path!(), "tui_copy_paste_jobs_rename_preview_quicklook_edit", |data| {
        for d in ["src", "dst"] {
            std::fs::create_dir_all(data.join(d)).unwrap();
        }
        std::fs::write(data.join("src").join("a.txt"), "new copy\n").unwrap();
        std::fs::write(data.join("dst").join("a.txt"), "old file\n").unwrap();
    });
    match run {
        Run::Child(data) => run_e2e(data),
        Run::Parent(p) => {
            let dst = p.data.join("dst");
            let got = names(&dst);
            assert_eq!(got.len(), 2, "the copy kept both: {got:?}");
            assert!(got.iter().any(|n| n == "renamed.txt"), "{got:?}");
            let edited = std::fs::read_to_string(dst.join("renamed.txt")).unwrap();
            assert!(edited.starts_with("e2e:"), "the edit was saved: {edited:?}");
            let other = got.iter().find(|n| *n != "renamed.txt").unwrap();
            // One file came from src, the other was there: both contents survive.
            let mut contents =
                [edited.trim_start_matches("e2e:").to_string(), std::fs::read_to_string(dst.join(other)).unwrap()];
            contents.sort();
            assert_eq!(contents, ["new copy\n".to_string(), "old file\n".to_string()]);
            assert_eq!(std::fs::read_to_string(p.data.join("src").join("a.txt")).unwrap(), "new copy\n");
        }
    }
}

fn run_e2e(data: PathBuf) {
    soyuz_appcore::services::os_clipboard::set_system_clipboard(false);
    let mut c = common::test_config();
    c.tui.preview_ratio = 0.5;
    let src = data.join("src");
    let dst = data.join("dst");
    let mut app = common::new_app(c, &src);
    app.host.set_local_clipboard(false);
    let (d1, d2, d3) = (dst.clone(), dst.clone(), dst.clone());
    let renamed = dst.join("renamed.txt");
    let (r1, r2) = (renamed.clone(), renamed.clone());
    let preview_has = move |a: &App| {
        let want = first_line(&r1);
        !want.is_empty()
            && a.view().is_some_and(|v| v.preview.visible && v.preview.body_text().iter().any(|l| l.contains(&want)))
    };
    let quicklook_on = |a: &App| {
        a.focus() == TuiFocus::QuickLook
            && a.view().is_some_and(|v| v.quicklook.visible && v.quicklook.title == "renamed.txt")
    };
    let text_ready = |a: &App| a.view().is_some_and(|v| v.editor.focused && matches!(v.editor.body, BodyVm::Text(_)));
    let doc_text = |a: &App| match a.core.editors.active_doc().map(|d| &d.kind) {
        Some(DocKind::Text(t)) => t.buf.text(),
        _ => String::new(),
    };
    let saved = move |a: &App| {
        a.core.editors.dirty_count() == 0
            && !a.core.editors.docs.iter().any(|d| d.saving)
            && std::fs::read_to_string(&r2).is_ok_and(|s| s.starts_with("e2e:"))
    };
    let history_row = |a: &App| {
        a.view().is_some_and(|v| v.jobs.tab == 1 && v.jobs.items.first().is_some_and(|i| i.title.starts_with("Copied")))
    };
    let mut steps = vec![
        // Copy a.txt (the internal clipboard), go up and into dst.
        Step::new("the listing", move |a| at(a, &src) && labels(a) == ["a.txt"], key(KeyCode::Down)),
        Step::new("the cursor on a.txt", |a| cursor(a).as_deref() == Some("a.txt"), ctrl('c')),
        Step::new("a.txt on the clipboard", |a| !a.core.fileops.clipboard.is_empty(), key(KeyCode::Backspace)),
        Step::new("the parent folder", |a| labels(a) == ["dst/", "src/"] && cursor(a).is_some(), key(KeyCode::Home)),
        Step::new("the cursor on dst", |a| cursor(a).as_deref() == Some("dst/"), key(KeyCode::Enter)),
        Step::new("dst with its a.txt", move |a| at(a, &d1) && labels(a) == ["a.txt"], ctrl('v')),
        // The name is taken: Keep both (K).
        Step::new(
            "the conflict prompt",
            |a| matches!(a.core.dialogs.top(), Some(DialogModel::Prompt(_))) && a.focus() == TuiFocus::Dialog,
            key(KeyCode::Char('k')),
        ),
        Step::new(
            "both files in dst",
            move |a| wait_jobs_idle(a) && !a.core.dialogs.is_open() && names(&d2).len() == 2 && labels(a).len() == 2,
            alt('j'),
        ),
        // The jobs panel: its history lists the copy.
        Step::new("the jobs panel has the keyboard", |a| a.focus() == TuiFocus::Jobs, key(KeyCode::Tab)),
        Step::new("the history lists the copy", history_row, key(KeyCode::Esc)),
        // Rename the first file inline.
        Step::new("the list again", |a| a.focus() == TuiFocus::List, key(KeyCode::Home)),
        Step::new(
            "the cursor on the first file",
            |a| cursor(a).is_some_and(|c| labels(a).first() == Some(&c)),
            key(KeyCode::F(2)),
        ),
        Step::new("the rename field", |a| a.focus() == TuiFocus::Rename, ctrl('a')),
    ];
    type_text(&mut steps, "renamed.txt");
    steps.extend([
        Step::new("the new name typed", |a| rename_text(a, "renamed.txt"), key(KeyCode::Enter)),
        Step::new(
            "renamed on disk, the cursor on it",
            move |a| {
                d3.join("renamed.txt").is_file()
                    && a.core.fileops.rename.is_none()
                    && wait_jobs_idle(a)
                    && cursor(a).as_deref() == Some("renamed.txt")
            },
            alt('p'),
        ),
        // The preview pane shows it, then Quick Look.
        Step::new("the preview shows the file", preview_has, key(KeyCode::Char(' '))),
        Step::new("Quick Look on renamed.txt", quicklook_on, key(KeyCode::Char(' '))),
        Step::new(
            "Quick Look closed",
            |a| !a.core.ui.quicklook.open && a.focus() == TuiFocus::List && cursor(a).as_deref() == Some("renamed.txt"),
            key(KeyCode::F(4)),
        ),
        // Edit: type at the start, save, close.
        Step::new("the text editor has the keyboard", text_ready, key(KeyCode::Char('e'))),
    ]);
    type_text(&mut steps, "2e:");
    steps.extend([
        Step::new("typed", move |a| doc_text(a).starts_with("e2e:"), ctrl('s')),
        Step::new("saved", saved, ctrl('w')),
        Step::new("closed", |a| a.core.editors.docs.is_empty(), ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 100, 30);
    let got = names(&dst);
    assert!(got.iter().any(|n| n == "renamed.txt") && got.len() == 2, "{got:?}");
    common::ok();
}

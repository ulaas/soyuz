//! Headless tests of T5: smart folders (W2-locations). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! A smart folder planted in the sandbox's `smart_folders.toml` shows in the sidebar,
//! opens from it (the status line names it), is edited through its menu (Shift+F10 →
//! Edit → the editor → Save rewrites the file and renames the row) and deleted (Delete →
//! the confirmation → gone from the file and the sidebar).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers};
use soyuz_appcore::models::smart::{folders, shown_name};
use soyuz_appcore::tr;
use soyuz_core::config::{KeyContext, SidebarSection, config_dir};
use soyuz_tui::app::App;

use super::menu::{app, drive, menu_open, pick};
use crate::common::{self, Run, ctrl, dialog_title, key, key_mods};

fn sidebar_cursor(a: &App) -> Option<String> {
    a.view()?.sidebar.lines.iter().find(|l| l.cursor).map(|l| l.label.clone())
}

fn sidebar_has(a: &App, label: &str) -> bool {
    a.view().is_some_and(|v| v.sidebar.lines.iter().any(|l| l.label == label))
}

fn smart_file() -> PathBuf {
    config_dir().join("smart_folders.toml")
}

fn file_has(needle: &str) -> bool {
    std::fs::read_to_string(smart_file()).is_ok_and(|t| t.contains(needle))
}

/// The query of the planted folder: `.txt` files of the sandbox's data folder.
fn query(data: &Path) -> String {
    format!("ext:txt in:\"{}\"", data.display())
}

#[test]
fn smart_folder_opens_edits_and_deletes_from_the_sidebar() {
    let run = common::relaunch(module_path!(), "smart_folder_opens_edits_and_deletes_from_the_sidebar", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join("b.md"), b"b").unwrap();
        let cfg = data.parent().unwrap().join(soyuz_core::config::PORTABLE_DATA_DIR);
        std::fs::create_dir_all(&cfg).unwrap();
        let toml = format!("[[folder]]\nname = \"Docs\"\nquery = '{}'\n", query(data));
        std::fs::write(cfg.join("smart_folders.toml"), toml).unwrap();
    });
    if let Run::Child(data) = run {
        smart_flow(data);
    }
}

fn smart_flow(data: PathBuf) {
    let mut c = common::test_config();
    c.layout.sidebar = true;
    c.sidebar.sections = vec![SidebarSection::Smart];
    let mut app = app(c, &data);
    let mut stage = 0;
    let mut typed: VecDeque<Event> = VecDeque::new();
    let shift_f10 = key_mods(KeyCode::F(10), KeyModifiers::SHIFT);
    let mut saw_status = false;
    let mut saw_cheat_sheet = false;
    drive(&mut app, |a| {
        if let Some(e) = typed.pop_front() {
            return Some(e);
        }
        let shown = shown_name(&a.core.ws.pane().location);
        match stage {
            // The planted folder and the "New smart folder" row are listed.
            0 if sidebar_has(a, "Docs") && sidebar_has(a, tr!("views-smart-new")) => {
                stage = 1;
                Some(key(KeyCode::F(12)))
            }
            1 if a.key_context() == KeyContext::Sidebar && sidebar_cursor(a).as_deref() == Some("Docs") => {
                stage = 2;
                Some(key(KeyCode::Enter))
            }
            // Open: the plain search (the index is off) lists the text file only.
            2 if shown.as_deref() == Some("Docs") && common::labels(a) == ["a.txt"] => {
                saw_status = a.view().is_some_and(|v| v.status.smart.starts_with("Docs"));
                stage = 3;
                Some(key(KeyCode::F(12)))
            }
            3 if a.key_context() == KeyContext::Sidebar && sidebar_cursor(a).as_deref() == Some("Docs") => {
                stage = 4;
                Some(shift_f10.clone())
            }
            4 => {
                let e = pick(a, tr!("views-smart-edit"));
                if e.as_ref().is_some_and(|e| *e == key(KeyCode::Enter)) {
                    stage = 5;
                }
                e
            }
            // The editor, focused on the name: " 2" makes it "Docs 2", Enter saves.
            5 if dialog_title(a).as_deref() == Some(tr!("views-smart-dialog-edit")) => {
                let d = a.view().and_then(|v| v.dialog.clone()).unwrap_or_default();
                saw_cheat_sheet = d.body.iter().any(|l| l.starts_with("kind:image"));
                assert_eq!(d.fields.len(), 3, "{d:?}");
                stage = 6;
                typed.extend([key(KeyCode::Char(' ')), key(KeyCode::Char('2'))]);
                Some(key(KeyCode::End))
            }
            6 => {
                stage = 7;
                Some(key(KeyCode::Enter))
            }
            7 if dialog_title(a).is_none()
                && file_has("Docs 2")
                && sidebar_has(a, "Docs 2")
                && shown.as_deref() == Some("Docs 2") =>
            {
                stage = 8;
                Some(key(KeyCode::F(12)))
            }
            8 if a.key_context() == KeyContext::Sidebar && sidebar_cursor(a).as_deref() == Some("Docs 2") => {
                stage = 9;
                Some(shift_f10.clone())
            }
            9 if menu_open(a) => {
                let e = pick(a, tr!("views-smart-delete"));
                if e.as_ref().is_some_and(|e| *e == key(KeyCode::Enter)) {
                    stage = 10;
                }
                e
            }
            10 if dialog_title(a).as_deref() == Some(tr!("views-smart-delete-title")) => {
                stage = 11;
                Some(key(KeyCode::Enter))
            }
            11 if folders().is_empty() && !file_has("Docs") && !sidebar_has(a, "Docs 2") => {
                stage = 12;
                Some(ctrl('q'))
            }
            _ => None,
        }
    });
    assert_eq!(stage, 12);
    assert!(saw_status, "the status line names the smart folder");
    assert!(saw_cheat_sheet, "the editor shows the query examples");
    common::ok();
}

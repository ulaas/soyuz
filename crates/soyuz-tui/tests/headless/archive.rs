//! Headless tests of T5: archive dialogs (W2-locations). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! Compress… and Extract to… have no default keys: the tests bind `create_archive` to F9
//! and `extract_to` to Shift+F9.
//! - a folder is compressed (the Compress dialog → a zip job), the zip is browsed (Enter),
//!   Extract to… extracts it into another folder (a job), the files are there;
//! - a 7z with a password (the names are encrypted too) asks for the password when it is
//!   browsed; the password lists it (the archives hold the compressed folder itself).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers};
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::Config;
use soyuz_tui::app::App;
use soyuz_tui::widgets::form::FieldVmKind;

use super::menu::drive;
use crate::common::{self, Run, ctrl, cursor, dialog_title, key, key_mods, labels, wait_jobs_idle};

fn config() -> Config {
    let mut c = common::test_config();
    c.keys.bindings.insert("create_archive".into(), vec!["F9".into()]);
    c.keys.bindings.insert("extract_to".into(), vec!["Shift+F9".into()]);
    c
}

fn in_archive(a: &App, archive: &Path) -> bool {
    matches!(&a.core.ws.pane().location, Location::Archive { archive: x, .. } if x == archive)
}

/// Type `text` (one key per frame) through `typed`.
fn type_into(typed: &mut VecDeque<Event>, text: &str) {
    typed.extend(text.chars().map(|c| key(KeyCode::Char(c))));
}

/// The first field's text of the open dialog.
fn dialog_name(a: &App) -> Option<String> {
    let d = a.view()?.dialog.as_ref()?;
    match &d.fields.first()?.kind {
        FieldVmKind::Text { text, .. } => Some(text.clone()),
        _ => None,
    }
}

/// Move the cursor to `name` (Down, wrapping with Home), then send `then`.
fn on(a: &App, name: &str, then: Event) -> Option<Event> {
    if cursor(a).as_deref() == Some(name) {
        Some(then)
    } else if labels(a).iter().any(|l| l == name) {
        Some(if labels(a).last().map(String::as_str) == cursor(a).as_deref() {
            key(KeyCode::Home)
        } else {
            key(KeyCode::Down)
        })
    } else {
        None
    }
}

#[test]
fn compress_then_extract_to_a_folder() {
    let run = common::relaunch(module_path!(), "compress_then_extract_to_a_folder", |data| {
        std::fs::create_dir_all(data.join("src")).unwrap();
        std::fs::create_dir_all(data.join("out")).unwrap();
        std::fs::write(data.join("src").join("one.txt"), b"1").unwrap();
        std::fs::write(data.join("src").join("two.txt"), b"22").unwrap();
    });
    if let Run::Child(data) = run {
        compress_extract(data);
    }
}

fn compress_extract(data: PathBuf) {
    let mut app = common::new_app(config(), &data);
    let zip = data.join("src.zip");
    let out = data.join("out");
    let mut stage = 0;
    let mut typed = VecDeque::new();
    let shift_f9 = key_mods(KeyCode::F(9), KeyModifiers::SHIFT);
    drive(&mut app, |a| {
        if let Some(e) = typed.pop_front() {
            return Some(e);
        }
        match stage {
            0 => {
                let e = on(a, "src/", key(KeyCode::F(9)));
                if e.as_ref().is_some_and(|e| *e == key(KeyCode::F(9))) {
                    stage = 1;
                }
                e
            }
            // The dialog names the archive after the folder; Enter compresses.
            1 if dialog_title(a).as_deref() == Some(tr!("views-archive-compress")) => {
                assert_eq!(dialog_name(a).as_deref(), Some("src.zip"));
                stage = 2;
                Some(key(KeyCode::Enter))
            }
            2 if dialog_title(a).is_none() && wait_jobs_idle(a) && zip.is_file() => {
                let e = on(a, "src.zip", key(KeyCode::Enter));
                if e.as_ref().is_some_and(|e| *e == key(KeyCode::Enter)) {
                    stage = 3;
                }
                e
            }
            3 if in_archive(a, &zip) && labels(a) == ["src/"] => {
                stage = 4;
                Some(shift_f9.clone())
            }
            // Extract to…: the destination is the archive's folder; "/out" goes into `out`.
            4 if dialog_title(a).as_deref() == Some(tr!("views-archive-extract")) => {
                stage = 5;
                type_into(&mut typed, &format!("{}out", std::path::MAIN_SEPARATOR));
                Some(key(KeyCode::End))
            }
            5 => {
                stage = 6;
                Some(key(KeyCode::Enter))
            }
            6 if dialog_title(a).is_none() && wait_jobs_idle(a) && out.join("src").join("two.txt").is_file() => {
                stage = 7;
                Some(ctrl('q'))
            }
            _ => None,
        }
    });
    assert_eq!(stage, 7);
    // The archive holds the folder itself.
    assert_eq!(std::fs::read(out.join("src").join("one.txt")).unwrap(), b"1");
    assert_eq!(std::fs::read(out.join("src").join("two.txt")).unwrap(), b"22");
    common::ok();
}

#[test]
fn a_password_7z_asks_before_listing() {
    let run = common::relaunch(module_path!(), "a_password_7z_asks_before_listing", |data| {
        std::fs::create_dir_all(data.join("secret")).unwrap();
        std::fs::write(data.join("secret").join("s.txt"), b"hidden").unwrap();
    });
    if let Run::Child(data) = run {
        password(data);
    }
}

fn password(data: PathBuf) {
    let mut app = common::new_app(config(), &data);
    let archive = data.join("secret.7z");
    let mut stage = 0;
    let mut typed = VecDeque::new();
    let mut asked = false;
    drive(&mut app, |a| {
        if let Some(e) = typed.pop_front() {
            return Some(e);
        }
        match stage {
            0 => {
                let e = on(a, "secret/", key(KeyCode::F(9)));
                if e.as_ref().is_some_and(|e| *e == key(KeyCode::F(9))) {
                    stage = 1;
                }
                e
            }
            // Format: 7z (the next one after ZIP); the form keeps the focus on the format.
            1 if dialog_title(a).as_deref() == Some(tr!("views-archive-compress")) => {
                stage = 2;
                typed.push_back(key(KeyCode::Right));
                Some(key(KeyCode::Tab))
            }
            // The name follows the format; Tab, Tab: level, password; Enter compresses.
            2 if dialog_name(a).is_some_and(|n| n == "secret.7z") => {
                stage = 3;
                typed.extend([key(KeyCode::Tab), key(KeyCode::Tab)]);
                type_into(&mut typed, "pw");
                None
            }
            3 => {
                stage = 4;
                Some(key(KeyCode::Enter))
            }
            4 if dialog_title(a).is_none() && wait_jobs_idle(a) && archive.is_file() => {
                let e = on(a, "secret.7z", key(KeyCode::Enter));
                if e.as_ref().is_some_and(|e| *e == key(KeyCode::Enter)) {
                    stage = 5;
                }
                e
            }
            5 if dialog_title(a).as_deref() == Some(tr!("views-archive-password-title")) => {
                asked = true;
                stage = 6;
                type_into(&mut typed, "pw");
                None
            }
            6 => {
                stage = 7;
                Some(key(KeyCode::Enter))
            }
            7 if dialog_title(a).is_none() && in_archive(a, &archive) && labels(a) == ["secret/"] => {
                stage = 8;
                Some(ctrl('q'))
            }
            _ => None,
        }
    });
    assert_eq!(stage, 8);
    assert!(asked, "the listing asked for the password");
    common::ok();
}

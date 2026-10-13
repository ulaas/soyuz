//! Headless tests of Phase T7 (agent A): split and join files. Each test runs in its own
//! sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`, like the
//! other files here.
//!
//! The dialogs open from the tool queue (`ToolCmd::Split` / `Join`, what the actions and
//! menus queue); the script ([`super::menu::drive`]) presses keys and clicks as the last
//! frame shows the dialog's state.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers};
use soyuz_appcore::models::split::{checksum_path, part_path};
use soyuz_appcore::models::tools::{ToolCmd, queue};
use soyuz_appcore::tr;

use super::workspaces::{click, field, no_dialog, row_cursor};
use crate::common::{self, Run, ctrl, dialog_title, key, key_mods, toasts};

const MIB: usize = 1024 * 1024;

fn data(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

fn big_file(dir: &Path) {
    std::fs::write(dir.join("big.bin"), data(3 * MIB)).unwrap();
    std::fs::create_dir_all(dir.join("out")).unwrap();
}

/// Start button of the dialog drawn in the last frame is enabled (the probe is done).
fn can_start(a: &soyuz_tui::app::App) -> bool {
    a.view().and_then(|v| v.dialog.as_ref()).and_then(|d| d.buttons.first()).is_some_and(|b| b.enabled)
}

fn dialog_row_pos(a: &soyuz_tui::app::App, i: usize) -> Option<(u16, u16)> {
    super::workspaces::row_pos(a, i)
}

#[test]
fn split_then_join_into_a_folder() {
    match common::relaunch(module_path!(), "split_then_join_into_a_folder", big_file) {
        Run::Child(data) => split_join(data),
        Run::Parent(r) => {
            let src = r.data.join("big.bin");
            assert!((1..=3).all(|n| part_path(&src, n).is_file()), "three parts");
            assert!(!part_path(&src, 4).exists());
            assert!(checksum_path(&src).is_file());
            assert_eq!(std::fs::read(r.data.join("out").join("big.bin")).unwrap(), std::fs::read(&src).unwrap());
        }
    }
}

fn split_join(data: PathBuf) {
    let mut c = common::test_config();
    c.tui.mouse = true;
    let mut app = common::new_app(c, &data);
    let src = data.join("big.bin");
    queue(ToolCmd::Split(src.clone()));
    let split_title = format!("{} — big.bin", tr!("tools-split-title"));
    let join_title = format!("{} — big.bin.001", tr!("tools-join-title"));
    let done = tr!("tools-split-done").to_string();
    let joined = tr!("tools-join-done").to_string();
    let sep = std::path::MAIN_SEPARATOR;
    let mut typing: VecDeque<Event> = VecDeque::new();
    let mut phase = 0;
    super::menu::drive(&mut app, |a| {
        if let Some(e) = typing.pop_front() {
            return Some(e);
        }
        match phase {
            // The size arrived from the probe: a click on "1 GB" picks it.
            0 if dialog_title(a).as_deref() == Some(split_title.as_str()) && can_start(a) => {
                let (x, y) = dialog_row_pos(a, 3)?;
                phase = 1;
                Some(click(x, y))
            }
            // End picks Custom, which enables its field; Tab twice reaches it.
            1 if row_cursor(a) == Some(3) => {
                phase = 2;
                typing.extend([key(KeyCode::Tab), key(KeyCode::Tab)]);
                Some(key(KeyCode::End))
            }
            2 if row_cursor(a) == Some(5) && a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.focus == 0) => {
                phase = 3;
                typing.extend((0..5).map(|_| key(KeyCode::Backspace)));
                typing.extend("1 MB".chars().map(|c| key(KeyCode::Char(c))));
                Some(key(KeyCode::Backspace))
            }
            3 if field(a, 0).as_deref() == Some("1 MB") => {
                phase = 4;
                Some(key(KeyCode::Enter))
            }
            // Done: the toast, then Enter (Done) closes; the join of a part opens next.
            4 if toasts(a).iter().any(|t| t.contains(&done)) && checksum_path(&src).is_file() => {
                phase = 5;
                Some(key(KeyCode::Enter))
            }
            5 if no_dialog(a) => {
                phase = 6;
                queue(ToolCmd::Join(part_path(&src, 1)));
                None
            }
            // Shift+Tab reaches the output folder: into `out`.
            6 if dialog_title(a).as_deref() == Some(join_title.as_str()) && can_start(a) => {
                phase = 7;
                typing.extend(format!("{sep}out").chars().map(|c| key(KeyCode::Char(c))));
                Some(key_mods(KeyCode::BackTab, KeyModifiers::SHIFT))
            }
            7 if field(a, 0).is_some_and(|f| f.ends_with("out")) => {
                phase = 8;
                Some(key(KeyCode::Enter))
            }
            8 if toasts(a).iter().any(|t| t.contains(&joined)) => {
                phase = 9;
                Some(key(KeyCode::Esc))
            }
            9 if no_dialog(a) => {
                phase = 10;
                Some(ctrl('q'))
            }
            _ => None,
        }
    });
    common::ok();
}

fn huge_file(dir: &Path) {
    std::fs::write(dir.join("huge.bin"), vec![7u8; 96 * MIB]).unwrap();
}

#[test]
fn closing_while_splitting_leaves_no_partial_output() {
    match common::relaunch(module_path!(), "closing_while_splitting_leaves_no_partial_output", huge_file) {
        Run::Child(data) => close_while_running(data),
        Run::Parent(r) => {
            // Either the run finished before Esc arrived, or it was cancelled and cleaned up.
            let src = r.data.join("huge.bin");
            let parts = (1..=96).filter(|&n| part_path(&src, n).exists()).count();
            let ck = checksum_path(&src).exists();
            assert!((parts == 0 && !ck) || (parts == 96 && ck), "{parts} parts, checksum {ck}");
        }
    }
}

fn close_while_running(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let src = data.join("huge.bin");
    queue(ToolCmd::Split(src.clone()));
    let title = format!("{} — huge.bin", tr!("tools-split-title"));
    let running = tr!("tui-split-running-hints").to_string();
    let done = tr!("tools-split-done").to_string();
    let mut typing: VecDeque<Event> = VecDeque::new();
    let mut phase = 0;
    super::menu::drive(&mut app, |a| {
        if let Some(e) = typing.pop_front() {
            return Some(e);
        }
        match phase {
            // Custom, then 1 MB parts typed into its field (Tab twice from Start).
            0 if dialog_title(a).as_deref() == Some(title.as_str()) && can_start(a) => {
                phase = 1;
                typing.extend([key(KeyCode::Tab), key(KeyCode::Tab)]);
                Some(key(KeyCode::End))
            }
            1 if row_cursor(a) == Some(5) && a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.focus == 0) => {
                phase = 2;
                typing.extend((0..5).map(|_| key(KeyCode::Backspace)));
                typing.extend("1 MB".chars().map(|c| key(KeyCode::Char(c))));
                Some(key(KeyCode::Backspace))
            }
            2 if field(a, 0).as_deref() == Some("1 MB") => {
                phase = 3;
                Some(key(KeyCode::Enter))
            }
            // Running (or already done): Esc stops it, a second Esc closes the dialog.
            3 if a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.hints == running)
                || toasts(a).iter().any(|t| t.contains(&done)) =>
            {
                phase = 4;
                Some(key(KeyCode::Esc))
            }
            4 if no_dialog(a) => {
                phase = 5;
                Some(ctrl('q'))
            }
            // Stopped (the run reported back): Esc closes.
            4 if a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.hints != running) => {
                Some(key(KeyCode::Esc))
            }
            _ => None,
        }
    });
    // The worker was told to stop by the closed dialog; give it time to clean up.
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(20) {
        let n = (1..=96).filter(|&n| part_path(&src, n).exists()).count();
        if n == 0 || (n == 96 && checksum_path(&src).exists()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    common::ok();
}

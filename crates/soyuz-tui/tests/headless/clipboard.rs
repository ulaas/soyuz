//! Headless tests of T3: the clipboard (OSC 52). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! "Copy folder path" from the background menu copies the folder; `[tui] osc52` decides
//! whether the copy is also written to the terminal as OSC 52 (and how, inside tmux). The
//! host never touches the system clipboard (`set_local_clipboard(false)`), so `auto`
//! behaves as on a machine without a local text clipboard. The encoder itself is
//! unit-tested in `soyuz_tui::clipboard`.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_appcore::tr;
use soyuz_core::config::TuiOsc52;
use soyuz_tui::clipboard::Env;

use super::menu::{app, config, drive, menu_open, pick};
use crate::common::{self, Run, at, ctrl, key_mods, labels};

fn one_file(data: &Path) {
    std::fs::write(data.join("a.txt"), b"a").unwrap();
}

/// Copy the folder path through the background menu; returns what reached the terminal.
fn copy_folder_path(data: &Path, osc52: TuiOsc52, env: Env) -> (String, Option<String>) {
    let mut app = app(config(osc52), data);
    app.host.set_clipboard_env(env);
    let label = tr!("app-ctx-copy-folder-path");
    let mut phase = 0;
    let io = drive(&mut app, |a| match phase {
        // No cursor yet: the background menu.
        0 if at(a, data) && labels(a) == ["a.txt"] && a.core.ws.pane().cursor.is_none() => {
            phase = 1;
            Some(key_mods(KeyCode::F(10), KeyModifiers::SHIFT))
        }
        1 => {
            let e = pick(a, label)?;
            if e == crate::common::key(KeyCode::Enter) {
                phase = 2;
            }
            Some(e)
        }
        2 if !menu_open(a) && a.host.last_copy().is_some() => {
            phase = 3;
            Some(ctrl('q'))
        }
        _ => None,
    });
    (String::from_utf8_lossy(&io.written).into_owned(), app.host.last_copy())
}

fn same_dir(copied: Option<&str>, want: &Path) -> bool {
    copied.is_some_and(|c| Path::new(c).canonicalize().ok() == want.canonicalize().ok())
}

#[test]
fn tui_osc52_never_writes_nothing() {
    if let Run::Child(data) = common::relaunch(module_path!(), "tui_osc52_never_writes_nothing", one_file) {
        osc52_never(data);
    }
}

fn osc52_never(data: PathBuf) {
    let (out, copied) = copy_folder_path(&data, TuiOsc52::Never, Env::default());
    assert!(same_dir(copied.as_deref(), &data), "{copied:?}");
    assert!(!out.contains("]52;"), "no OSC 52 with `never`: {out:?}");
    common::ok();
}

#[test]
fn tui_osc52_auto_and_tmux() {
    if let Run::Child(data) = common::relaunch(module_path!(), "tui_osc52_auto_and_tmux", one_file) {
        osc52_auto_tmux(data);
    }
}

fn osc52_auto_tmux(data: PathBuf) {
    // `auto` without a local clipboard (and over SSH) sends OSC 52; inside tmux wrapped in
    // its passthrough.
    let env = Env { ssh: true, tmux: true, screen: false };
    let (out, copied) = copy_folder_path(&data, TuiOsc52::Auto, env);
    assert!(same_dir(copied.as_deref(), &data), "{copied:?}");
    let want = soyuz_tui::clipboard::osc52(copied.as_deref().unwrap_or_default(), &env).unwrap();
    assert!(out.contains(std::str::from_utf8(&want).unwrap()), "{out:?}");
    assert!(out.contains("\x1bPtmux;\x1b\x1b]52;c;"), "{out:?}");
    common::ok();
}

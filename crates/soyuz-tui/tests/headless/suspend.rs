//! Headless tests of T5 / T6: suspend to a shell or a text editor (W1-suspend). Each test runs in
//! its own sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! No real shell or editor ever starts: `App::child_runner` is replaced by fake runners
//! that record the [`ChildSpec`] and act like the program would (rewrite the file, exit
//! with a code, fail to start). `suspend::set_env_source` hides `$VISUAL`, `$EDITOR` and
//! `PATH` from the editor resolution, so the machine's own editors never count.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::models::frontend::{EditPurpose, SuspendRequest};
use soyuz_appcore::tr;
use soyuz_core::config::{self, Action, Config, TuiShellMode};
use soyuz_tui::app::App;
use soyuz_tui::suspend::{self, ChildSpec, Env};

use crate::common::{self, Run, Step, ctrl, key, key_mods};

/// Every spec a fake runner was handed, in order.
static SEEN: Mutex<Vec<ChildSpec>> = Mutex::new(Vec::new());

fn seen() -> Vec<ChildSpec> {
    SEEN.lock().unwrap().clone()
}

/// No `$VISUAL` / `$EDITOR`, an empty `PATH`; only an explicit path can be found.
fn isolated_env() -> Env {
    Env {
        visual: None,
        editor: None,
        path_dirs: Vec::new(),
        is_file: Box::new(Path::is_file),
        is_windows: cfg!(windows),
    }
}

/// This test binary: a file that exists, used as the "editor" / "shell" program.
fn exe() -> String {
    std::env::current_exe().unwrap().to_string_lossy().into_owned()
}

/// A fake editor: turns `general.show_hidden` on in the file it is given (its last
/// argument), exits 0.
fn edit_config(spec: &ChildSpec) -> io::Result<Option<i32>> {
    SEEN.lock().unwrap().push(spec.clone());
    let path = PathBuf::from(spec.args.last().expect("the file to edit"));
    let mut c = config::load(&path).expect("config.toml written before the editor starts").config;
    c.general.show_hidden = true;
    config::save_pretty(&path, &c).unwrap();
    Ok(Some(0))
}

/// A fake editor for a scratch file: writes `{"edited": true}`, exits 0.
fn edit_scratch(spec: &ChildSpec) -> io::Result<Option<i32>> {
    SEEN.lock().unwrap().push(spec.clone());
    std::fs::write(spec.args.last().expect("the file to edit"), b"{\"edited\": true}\n")?;
    Ok(Some(0))
}

/// A fake shell: a command line exits 3, an interactive shell cannot start.
fn shell_fails(spec: &ChildSpec) -> io::Result<Option<i32>> {
    SEEN.lock().unwrap().push(spec.clone());
    if spec.wait_key { Ok(Some(3)) } else { Err(io::Error::new(io::ErrorKind::NotFound, "no such shell")) }
}

/// A fake interactive shell: exits 0 at once.
fn shell_ok(spec: &ChildSpec) -> io::Result<Option<i32>> {
    SEEN.lock().unwrap().push(spec.clone());
    Ok(Some(0))
}

/// The runner must never be called.
fn never(spec: &ChildSpec) -> io::Result<Option<i32>> {
    panic!("no child may start here: {spec:?}");
}

fn has_toast(a: &App, text: &str) -> bool {
    common::toasts(a).iter().any(|t| t.contains(text))
}

/// The terminal UI's own key for ToggleTerminal (Ctrl+`).
fn toggle_terminal() -> crossterm::event::Event {
    key_mods(KeyCode::Char('`'), KeyModifiers::CONTROL)
}

/// "Edit config file" runs the configured editor on config.toml: the terminal is handed
/// over once and taken back once, the edit is reloaded ("Configuration reloaded") and the
/// app goes on.
#[test]
fn tui_suspend_edit_config_reloads() {
    let run = common::relaunch(module_path!(), "tui_suspend_edit_config_reloads", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join(".hidden"), b"h").unwrap();
    });
    match run {
        Run::Child(data) => {
            suspend::set_env_source(isolated_env);
            let mut c = common::test_config();
            c.tui.editor = format!("\"{}\" --wait", exe());
            let mut app = common::new_app(c, &data);
            app.child_runner = edit_config;
            app.core.cmds.push(Cmd::Action(Action::EditConfigFile));
            let reloaded = tr!("services-config-reloaded").to_string();
            let steps = vec![Step::new(
                "the config reloaded with hidden files shown",
                move |a: &App| {
                    a.core.config.general.show_hidden
                        && has_toast(a, &reloaded)
                        && common::labels(a).iter().any(|l| l.contains(".hidden"))
                },
                ctrl('q'),
            )];
            let (_, io) = common::run_script(&mut app, steps, 100, 24);
            assert_eq!(io.suspended, 1, "the terminal was handed over once");
            assert_eq!(io.resumed, 1, "and taken back once");
            let specs = seen();
            assert_eq!(specs.len(), 1);
            let path = config::default_path();
            assert_eq!(specs[0].program, PathBuf::from(exe()));
            assert_eq!(specs[0].args, vec![std::ffi::OsString::from("--wait"), path.clone().into_os_string()]);
            assert_eq!(specs[0].cwd.as_deref(), path.parent());
            assert!(!specs[0].wait_key);
            common::ok();
        }
        Run::Parent(p) => {
            let c = config::load(&p.config_dir().join("config.toml")).unwrap().config;
            assert!(c.general.show_hidden, "the editor's change stays on disk");
        }
    }
}

/// An editor run for a scratch file hands the file back to its owner through
/// `App::take_finished_edit` (once).
#[test]
fn tui_suspend_scratch_edit_is_handed_back() {
    let run = common::relaunch(module_path!(), "tui_suspend_scratch_edit_is_handed_back", |data| {
        std::fs::write(data.join("plan.json"), b"{}\n").unwrap();
    });
    let Run::Child(data) = run else { return };
    suspend::set_env_source(isolated_env);
    let mut c = common::test_config();
    c.tui.editor = exe();
    let mut app = common::new_app(c, &data);
    app.child_runner = edit_scratch;
    let file = data.join("plan.json");
    app.request_suspend(SuspendRequest::Editor { path: file.clone(), purpose: EditPurpose::Scratch(7) });
    let steps = vec![Step::new("the editor ran", |_: &App| seen().len() == 1, ctrl('q'))];
    let (_, io) = common::run_script(&mut app, steps, 100, 24);
    assert_eq!((io.suspended, io.resumed), (1, 1));
    assert!(app.take_finished_edit(8).is_none(), "another token");
    let done = app.take_finished_edit(7).expect("the finished edit");
    assert_eq!(done.path, file);
    assert_eq!(done.exit, Some(0));
    assert!(app.take_finished_edit(7).is_none(), "handed back once");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"edited\": true}\n");
    common::ok();
}

/// `[tui] shell_mode = "suspend"`: ToggleTerminal runs the shell in the focused pane's
/// folder in place of the terminal UI, the loop goes on afterwards and Ctrl+Q quits.
#[test]
fn tui_suspend_shell_mode_runs_the_shell_in_the_pane_folder() {
    let run = common::relaunch(module_path!(), "tui_suspend_shell_mode_runs_the_shell_in_the_pane_folder", |data| {
        std::fs::create_dir(data.join("sub")).unwrap();
        std::fs::write(data.join("sub").join("x.txt"), b"x").unwrap();
    });
    let Run::Child(data) = run else { return };
    suspend::set_env_source(isolated_env);
    let mut c = common::test_config();
    c.tui.shell_mode = TuiShellMode::Suspend;
    c.terminal.shell = exe();
    c.terminal.args = vec!["--flag".into()];
    let mut app = common::new_app(c, &data);
    app.child_runner = shell_ok;
    let sub = data.join("sub");
    let in_sub = sub.clone();
    let steps = vec![
        Step::new("the listing", |a: &App| common::labels(a).iter().any(|l| l.contains("sub")), key(KeyCode::Down)),
        Step::new("on sub", |a: &App| common::cursor(a).is_some_and(|c| c.contains("sub")), key(KeyCode::Enter)),
        Step::new("in sub", move |a: &App| common::at(a, &in_sub), toggle_terminal()),
        Step::new("the shell ran", |_: &App| seen().len() == 1, key(KeyCode::Up)),
        // The loop goes on: keys still work after the suspend.
        Step::new("listing again", |a: &App| common::labels(a).iter().any(|l| l.contains("x.txt")), ctrl('q')),
    ];
    let (_, io) = common::run_script(&mut app, steps, 100, 24);
    assert_eq!((io.suspended, io.resumed), (1, 1));
    let spec = &seen()[0];
    assert_eq!(spec.program, PathBuf::from(exe()));
    assert_eq!(spec.args, vec![std::ffi::OsString::from("--flag")]);
    assert_eq!(spec.cwd.as_deref(), Some(sub.as_path()));
    assert!(!spec.wait_key, "an interactive shell does not wait for Enter");
    assert!(!app.core.ui.terminal.focused, "no terminal pane in suspend mode");
    common::ok();
}

/// A command line that exits non-zero and a shell that cannot start both say so in a
/// toast; the terminal is taken back after each.
#[test]
fn tui_suspend_exit_code_and_start_failure_toasts() {
    let run = common::relaunch(module_path!(), "tui_suspend_exit_code_and_start_failure_toasts", |_| {});
    let Run::Child(data) = run else { return };
    suspend::set_env_source(isolated_env);
    let mut c = common::test_config();
    c.tui.shell_mode = TuiShellMode::Suspend;
    c.terminal.shell = exe();
    let mut app = common::new_app(c, &data);
    app.child_runner = shell_fails;
    app.request_suspend(SuspendRequest::Command { dir: data.clone(), line: "exit 3".into() });
    let name = Path::new(&exe()).file_name().unwrap().to_string_lossy().into_owned();
    let exited = tr!("tui-suspend-exit", program = name.as_str(), code = "3");
    let failed = tr!("tui-suspend-failed", program = name.as_str());
    let steps = vec![
        Step::new("the exit code toast", move |a: &App| has_toast(a, &exited), toggle_terminal()),
        Step::new("the start failure toast", move |a: &App| has_toast(a, &failed), ctrl('q')),
    ];
    let (_, io) = common::run_script(&mut app, steps, 100, 24);
    assert_eq!((io.suspended, io.resumed), (2, 2));
    let specs = seen();
    assert_eq!(specs.len(), 2);
    assert!(specs[0].wait_key, "a command line waits for Enter");
    assert_eq!(specs[0].args.last(), Some(&std::ffi::OsString::from("exit 3")));
    assert_eq!(specs[0].cwd.as_deref(), Some(data.as_path()));
    common::ok();
}

/// No console editor anywhere: a toast names what was tried, the terminal is never handed
/// over, and the config file opens in the built-in settings editor instead.
#[test]
fn tui_suspend_without_an_editor() {
    let run = common::relaunch(module_path!(), "tui_suspend_without_an_editor", |_| {});
    let Run::Child(data) = run else { return };
    suspend::set_env_source(isolated_env);
    let mut c: Config = common::test_config();
    c.tui.editor = "no-such-editor-here".into();
    let mut app = common::new_app(c, &data);
    app.child_runner = never;
    app.core.cmds.push(Cmd::Action(Action::EditConfigFile));
    let title = tr!("tui-suspend-no-editor").to_string();
    let names = ["no-such-editor-here"]
        .into_iter()
        .chain(suspend::EDITOR_PROBES)
        .chain(if cfg!(windows) { Some("edit") } else { None })
        .collect::<Vec<_>>()
        .join(", ");
    let body = tr!("tui-suspend-no-editor-tried", names = names.as_str());
    let steps =
        vec![Step::new("the no-editor toast", move |a: &App| has_toast(a, &title) && has_toast(a, &body), ctrl('q'))];
    let (_, io) = common::run_script(&mut app, steps, 100, 24);
    assert_eq!((io.suspended, io.resumed), (0, 0), "nothing was handed over");
    assert!(seen().is_empty());
    common::ok();
}

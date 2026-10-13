//! Headless tests of T6: the settings editor (W2-settings-editor). Each test runs in its own
//! sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! The editor is driven by keys only (search, move, change, edit in place, reset); the
//! checks read the live config, the drawn model (`ViewModel::settings`) and
//! `config.toml` in the sandbox's portable config folder. `$EDITOR` runs are faked
//! (`App::child_runner`), as in `headless::suspend`.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::models::frontend::FrontendCmd;
use soyuz_appcore::models::settings::Section;
use soyuz_appcore::models::smart::SmartCmd;
use soyuz_appcore::tr;
use soyuz_core::config::{self, Action, Config, TuiBorders};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::suspend::{self, ChildSpec, Env};
use soyuz_tui::theme::TuiTheme;
use soyuz_tui::view::settings::{LineVm, SettingsVm};

use crate::common::{self, Run, Step, ctrl, key, key_mods, type_text};

/// The comment the user keeps at the top of `config.toml`.
const COMMENT: &str = "# my own note, keep me";

/// `config.toml` of the sandbox (portable mode: next to the test binary's copy).
fn config_file(data: &Path) -> PathBuf {
    let name = config::default_path().file_name().expect("config file name").to_owned();
    data.parent().expect("sandbox").join(config::PORTABLE_DATA_DIR).join(name)
}

/// Write the test config with the user's comment on top (what the app starts with, so a
/// hot reload at start changes nothing).
fn write_config(data: &Path, c: &Config) {
    let path = config_file(data);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    config::save_pretty(&path, c).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{COMMENT}\n{text}")).unwrap();
}

fn settings(a: &App) -> Option<&SettingsVm> {
    a.view().map(|v| &v.settings).filter(|s| s.visible)
}

fn cursor_line(a: &App) -> Option<&LineVm> {
    settings(a)?.lines.iter().find(|l| l.cursor)
}

/// The cursor is on the key named `name`.
fn on(a: &App, name: &str) -> bool {
    cursor_line(a).is_some_and(|l| l.label == name)
}

fn ctrl_comma() -> crossterm::event::Event {
    key_mods(KeyCode::Char(','), KeyModifiers::CONTROL)
}

/// Steps: search for `query` and go to the first result.
fn search(steps: &mut Vec<Step>, query: &str) {
    steps.push(Step::new("the editor is open", |a: &App| settings(a).is_some(), key(KeyCode::Char('/'))));
    type_text(steps, query);
    steps.push(Step::new("search typed", |_| true, key(KeyCode::Enter)));
}

/// Ctrl+, opens the editor; Right on `tui.borders` cycles it to `plain` (live: the theme
/// is rebuilt); Space on `general.show_hidden` shows hidden files at once; `config.toml`
/// gets both keys shortly after, with the user's comment kept.
#[test]
fn tui_settings_change_apply_and_save() {
    let run = common::relaunch(module_path!(), "tui_settings_change_apply_and_save", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join(".hidden"), b"h").unwrap();
        write_config(data, &common::test_config());
    });
    match run {
        Run::Child(data) => {
            let mut app = common::new_app(common::test_config(), &data);
            let file = config_file(&data);
            let mut steps = vec![Step::new("the list is shown", |a: &App| !common::labels(a).is_empty(), ctrl_comma())];
            search(&mut steps, "tui.borders");
            steps.push(Step::new("on borders", |a: &App| on(a, "borders"), key(KeyCode::Right)));
            steps.push(Step::new(
                "borders are plain and the theme follows",
                |a: &App| {
                    a.core.config.tui.borders == TuiBorders::Plain
                        && TuiTheme::cached(&a.core, false).borders() == TuiBorders::Plain
                        && cursor_line(a).is_some_and(|l| l.value == "plain" && l.modified)
                },
                key(KeyCode::Esc),
            ));
            search(&mut steps, "general.show_hidden");
            steps.push(Step::new("on show_hidden", |a: &App| on(a, "show_hidden"), key(KeyCode::Char(' '))));
            let file2 = file.clone();
            steps.push(Step::new(
                "hidden files show and config.toml has both keys, comment kept",
                move |a: &App| {
                    let text = std::fs::read_to_string(&file2).unwrap_or_default();
                    a.core.config.general.show_hidden
                        && common::labels(a).iter().any(|l| l.contains(".hidden"))
                        && text.contains(COMMENT)
                        && text.contains("show_hidden = true")
                        && text.contains("borders = \"plain\"")
                },
                key(KeyCode::Esc),
            ));
            steps.push(Step::new("search cleared", |a: &App| settings(a).is_some(), key(KeyCode::Esc)));
            steps.push(Step::new("closed", |a: &App| settings(a).is_none() && !a.core.ui.settings_open, ctrl('q')));
            common::run_script(&mut app, steps, 100, 30);
            common::ok();
        }
        Run::Parent(r) => {
            let text = std::fs::read_to_string(config_file(&r.data)).unwrap();
            assert!(text.starts_with(COMMENT), "{text}");
        }
    }
}

/// A number that does not parse is refused with a message and nothing changes; a valid
/// one applies (clamped to its range); Ctrl+R asks, then resets the whole section.
#[test]
fn tui_settings_invalid_number_and_section_reset() {
    let run = common::relaunch(module_path!(), "tui_settings_invalid_number_and_section_reset", |_| {});
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let rows_before = app.core.config.tui.jobs_rows;
    app.core.frontend.push(FrontendCmd::OpenSettings(Some(Section::TerminalUi)));
    let mut steps = Vec::new();
    search(&mut steps, "jobs_rows");
    steps.push(Step::new("on jobs_rows", |a: &App| on(a, "jobs_rows"), key(KeyCode::Enter)));
    steps.push(Step::new(
        "editing in place",
        |a: &App| cursor_line(a).is_some_and(|l| l.edit_cursor.is_some()),
        key_mods(KeyCode::Backspace, KeyModifiers::NONE),
    ));
    for _ in 0..3 {
        steps.push(Step::new("clearing", |_| true, key(KeyCode::Backspace)));
    }
    type_text(&mut steps, "abc");
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
    steps.push(Step::new(
        "refused with a message",
        move |a: &App| {
            settings(a).is_some_and(|s| s.error.as_deref().is_some_and(|e| e.contains("abc")))
                && cursor_line(a).is_some_and(|l| l.error && l.edit_cursor.is_some())
                && a.core.config.tui.jobs_rows == rows_before
        },
        key(KeyCode::Esc),
    ));
    steps.push(Step::new(
        "edit cancelled",
        |a: &App| cursor_line(a).is_some_and(|l| l.edit_cursor.is_none()),
        key(KeyCode::Enter),
    ));
    for _ in 0..4 {
        steps.push(Step::new("clearing", |_| true, key(KeyCode::Backspace)));
    }
    type_text(&mut steps, "999");
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
    steps.push(Step::new(
        "clamped to the range and applied",
        |a: &App| a.core.config.tui.jobs_rows == soyuz_core::config::TuiSection::JOBS_ROWS_MAX,
        ctrl('r'),
    ));
    steps.push(Step::new(
        "asked first",
        |a: &App| settings(a).is_some_and(|s| s.popup.as_ref().is_some_and(|p| p.alert)),
        key(KeyCode::Left),
    ));
    steps.push(Step::new("Reset focused", |_| true, key(KeyCode::Enter)));
    let reset = tr!("tui-settings-section-reset", section = Section::TerminalUi.title());
    steps.push(Step::new(
        "the section is back to its defaults",
        move |a: &App| {
            a.core.config.tui == Config::default().tui
                && common::toasts(a).contains(&reset)
                && settings(a).is_some_and(|s| s.popup.is_none())
        },
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

/// No `$VISUAL` / `$EDITOR`, an empty `PATH`.
fn isolated_env() -> Env {
    Env {
        visual: None,
        editor: None,
        path_dirs: Vec::new(),
        is_file: Box::new(Path::is_file),
        is_windows: cfg!(windows),
    }
}

/// A fake editor: turns `general.show_hidden` on in the file it is given.
fn edit_config(spec: &ChildSpec) -> io::Result<Option<i32>> {
    let path = PathBuf::from(spec.args.last().expect("the file to edit"));
    let mut c = config::load(&path).expect("config.toml").config;
    c.general.show_hidden = true;
    config::save_pretty(&path, &c).unwrap();
    Ok(Some(0))
}

static WROTE: AtomicBool = AtomicBool::new(false);

/// Ctrl+E in the editor runs `$EDITOR` on config.toml; afterwards the editor shows the
/// file's values. A change written to the file from outside (hot reload) shows too.
#[test]
fn tui_settings_editor_and_hot_reload_refresh_the_draft() {
    let run = common::relaunch(module_path!(), "tui_settings_editor_and_hot_reload_refresh_the_draft", |data| {
        write_config(data, &common::test_config());
    });
    let Run::Child(data) = run else { return };
    suspend::set_env_source(isolated_env);
    let mut c = common::test_config();
    let exe = std::env::current_exe().unwrap().to_string_lossy().into_owned();
    c.tui.editor = format!("\"{exe}\" --wait");
    write_config(&data, &c);
    let mut app = common::new_app(c, &data);
    app.child_runner = edit_config;
    let file = config_file(&data);
    let mut steps = vec![Step::new("started", |a: &App| a.view().is_some(), ctrl_comma())];
    search(&mut steps, "general.show_hidden");
    steps.push(Step::new(
        "on show_hidden, off",
        |a: &App| on(a, "show_hidden") && cursor_line(a).is_some_and(|l| l.value == "[ ]"),
        ctrl('e'),
    ));
    steps.push(Step::new(
        "back from the editor: the row shows the file's value, still open",
        move |a: &App| {
            let shown = on(a, "show_hidden") && cursor_line(a).is_some_and(|l| l.value == "[x]");
            if shown && !WROTE.swap(true, Ordering::SeqCst) {
                // Now change the file from outside.
                let mut c = config::load(&file).unwrap().config;
                c.general.show_hidden = false;
                config::save_pretty(&file, &c).unwrap();
            }
            shown
        },
        key(KeyCode::F(1)),
    ));
    steps.push(Step::new(
        "the hot reload shows in the editor",
        |a: &App| !a.core.config.general.show_hidden && cursor_line(a).is_some_and(|l| l.value == "[ ]"),
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

/// The core's requests open the editor on their section (smart folders → Preview, the AI
/// panel → AI, Show keybindings → Keyboard); Ctrl+, toggles it.
#[test]
fn tui_settings_open_on_a_section() {
    let run = common::relaunch(module_path!(), "tui_settings_open_on_a_section", |_| {});
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    app.core.cmds.push(Cmd::Smart(SmartCmd::OpenSettings));
    let title = |s: Section| move |a: &App| settings(a).is_some_and(|v| v.title.ends_with(s.title()));
    let steps = vec![
        Step::new("smart folders open Preview", title(Section::Preview), ctrl_comma()),
        Step::new("Ctrl+, closed it", |a: &App| settings(a).is_none(), ctrl_comma()),
        Step::new("Ctrl+, opens it again", |a: &App| settings(a).is_some(), key(KeyCode::Esc)),
        Step::new("Esc closed it", |a: &App| settings(a).is_none(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    // The AI panel's and the palette's ways in.
    let mut app = common::new_app(common::test_config(), &data);
    soyuz_appcore::ai::panel::apply(&mut app.core, &app.host, vec![soyuz_appcore::ai::panel::PanelAct::OpenSettings]);
    let steps = vec![
        Step::new("the AI panel opens AI", title(Section::Ai), key(KeyCode::Esc)),
        Step::new("closed", |a: &App| settings(a).is_none(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    let mut app = common::new_app(common::test_config(), &data);
    app.core.cmds.push(Cmd::Action(Action::ShowKeybindings));
    let steps = vec![Step::new("Show keybindings opens Keyboard", title(Section::Keyboard), ctrl('q'))];
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

/// Every size from nothing to tiny, with the editor open on each kind of page and with a
/// box over it: nothing panics.
#[test]
fn tui_settings_tiny_sizes() {
    let run = common::relaunch(module_path!(), "tui_settings_tiny_sizes", |_| {});
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let mut io = NullIo::default();
    let mut frame = |app: &mut App, w: u16, h: u16, events: &[crossterm::event::Event]| {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        app.frame(&mut t, events, &mut io).unwrap();
    };
    for section in [Section::TerminalUi, Section::Keyboard, Section::Preview] {
        for extra in [None, Some(key(KeyCode::Enter)), Some(ctrl('r')), Some(key(KeyCode::Char('/')))] {
            app.core.frontend.push(FrontendCmd::OpenSettings(Some(section)));
            frame(&mut app, 80, 24, &[]);
            assert!(app.view().is_some_and(|v| v.settings.visible), "{section:?} open");
            if let Some(e) = &extra {
                frame(&mut app, 80, 24, std::slice::from_ref(e));
            }
            for w in 0..=24 {
                for h in 0..=8 {
                    frame(&mut app, w, h, &[]);
                }
            }
            frame(&mut app, 80, 24, &[key(KeyCode::Esc)]);
            frame(&mut app, 80, 24, &[key(KeyCode::Esc)]);
        }
    }
    common::ok();
}

/// Enter on a list key opens the list editor: Insert starts an item, Enter adds it, Alt+S
/// saves the list (live); a list of tables opens the table editor.
#[test]
fn tui_settings_list_editor() {
    let run = common::relaunch(module_path!(), "tui_settings_list_editor", |_| {});
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    app.core.frontend.push(FrontendCmd::OpenSettings(Some(Section::Terminal)));
    let before = app.core.config.terminal.args.len();
    let mut steps = Vec::new();
    search(&mut steps, "terminal.args");
    steps.push(Step::new("on args", |a: &App| on(a, "args"), key(KeyCode::Enter)));
    steps.push(Step::new(
        "the list editor",
        |a: &App| common::dialog_title(a).is_some_and(|t| t == "terminal.args"),
        key(KeyCode::Insert),
    ));
    type_text(&mut steps, "-x");
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
    steps.push(Step::new(
        "added to the list",
        move |a: &App| a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.list.len() == before + 1),
        key_mods(KeyCode::Char('s'), KeyModifiers::ALT),
    ));
    steps.push(Step::new(
        "saved and live",
        |a: &App| {
            common::dialog_title(a).is_none()
                && a.core.config.terminal.args.iter().any(|x| x == "-x")
                && cursor_line(a).is_some_and(|l| l.value.contains("-x") && l.modified)
        },
        key(KeyCode::Esc),
    ));
    search(&mut steps, "custom_commands");
    steps.push(Step::new(
        "a list of tables reads as a summary",
        |a: &App| on(a, "custom_commands") && cursor_line(a).is_some_and(|l| l.value.contains('0')),
        key(KeyCode::Enter),
    ));
    steps.push(Step::new("the table editor opened", |a: &App| common::dialog_title(a).is_some(), key(KeyCode::Esc)));
    steps.push(Step::new("closed again", |a: &App| common::dialog_title(a).is_none(), ctrl('q')));
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

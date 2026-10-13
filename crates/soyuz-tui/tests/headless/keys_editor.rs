//! Headless tests of T6: the key bindings page (W2-settings-editor). Each test runs in its own
//! sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! Show keybindings opens the Keyboard section; the filter, the recorder dialog (raw key
//! capture), the problems lines, chip colours, reset and the preset switch are driven by
//! keys; `config.toml` and the live key map are checked.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_appcore::cmd::Cmd;
use soyuz_core::config::{self, Action, KeyContext, KeyPreset, KeySequence, parse_sequence};
use soyuz_tui::app::App;
use soyuz_tui::input::settings_keys::ChipKind;
use soyuz_tui::view::settings::{LineKind, LineVm, SettingsVm};

use crate::common::{self, Run, Step, ctrl, key, key_mods, type_text};

fn config_file(data: &Path) -> PathBuf {
    let name = config::default_path().file_name().expect("config file name").to_owned();
    data.parent().expect("sandbox").join(config::PORTABLE_DATA_DIR).join(name)
}

fn settings(a: &App) -> Option<&SettingsVm> {
    a.view().map(|v| &v.settings).filter(|s| s.visible)
}

fn cursor_line(a: &App) -> Option<&LineVm> {
    settings(a)?.lines.iter().find(|l| l.cursor)
}

fn refresh_line(a: &App) -> Option<&LineVm> {
    settings(a)?
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Action && l.label == soyuz_appcore::palette::catalog::title(&Action::Refresh))
}

fn problems(a: &App) -> bool {
    settings(a).is_some_and(|s| s.lines.iter().any(|l| l.kind == LineKind::Note && l.label.contains("Ctrl+T G")))
}

fn sequence() -> KeySequence {
    parse_sequence("Ctrl+T G").unwrap()
}

/// Record "Ctrl+T G" for Refresh: a sequence behind New tab's own chord is a problem
/// (listed on top, its chip in the danger colour), written to `config.toml` and live in
/// the key map; Backspace puts the preset back. Then the preset switches to Vim.
#[test]
fn tui_keys_record_conflict_reset_and_preset() {
    let run = common::relaunch(module_path!(), "tui_keys_record_conflict_reset_and_preset", |_| {});
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let file = config_file(&data);
    app.core.cmds.push(Cmd::Action(Action::ShowKeybindings));
    let keyboard = soyuz_appcore::models::settings::Section::Keyboard.title();
    let mut steps = vec![Step::new(
        "the Keyboard page",
        move |a: &App| settings(a).is_some_and(|s| s.keys_page && s.title.ends_with(keyboard)),
        key(KeyCode::Char('r')),
    )];
    // Typing filters the actions.
    type_text(&mut steps, "efresh");
    steps.push(Step::new("filtered", |a: &App| refresh_line(a).is_some(), key(KeyCode::Enter)));
    for _ in 0..2 {
        steps.push(Step::new("moving", |_| true, key(KeyCode::Down)));
    }
    steps.push(Step::new(
        "on Refresh",
        |a: &App| cursor_line(a).is_some_and(|l| l.kind == LineKind::Action && Some(l) == refresh_line(a)),
        key(KeyCode::Enter),
    ));
    steps.push(Step::new(
        "the recorder is open",
        |a: &App| {
            common::dialog_title(a)
                .is_some_and(|t| t.contains(&soyuz_appcore::palette::catalog::title(&Action::Refresh)))
        },
        ctrl('t'),
    ));
    steps.push(Step::new(
        "it says what Ctrl+T does now",
        |a: &App| a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.error.is_some()),
        key(KeyCode::Char('g')),
    ));
    steps.push(Step::new("recorded", |_| true, key(KeyCode::Enter)));
    let file2 = file.clone();
    steps.push(Step::new(
        "a problem, a red chip, saved and live",
        move |a: &App| {
            let text = std::fs::read_to_string(&file2).unwrap_or_default();
            common::dialog_title(a).is_none()
                && problems(a)
                && refresh_line(a).is_some_and(|l| {
                    l.modified && l.chips.iter().any(|c| c.text == "Ctrl+T G" && c.kind == ChipKind::Conflict)
                })
                && text.contains("Ctrl+T G")
                && soyuz_appcore::keys::build_keymap(&a.core.config)
                    .0
                    .sequences_for(&Action::Refresh)
                    .contains(&sequence())
                && a.core.keymap.sequences_for(&Action::Refresh).contains(&sequence())
        },
        key(KeyCode::Backspace),
    ));
    steps.push(Step::new(
        "back to the preset",
        |a: &App| {
            !problems(a)
                && refresh_line(a).is_some_and(|l| !l.modified && !l.chips.iter().any(|c| c.text == "Ctrl+T G"))
                && a.core.config.keys.bindings.is_empty()
        },
        key(KeyCode::Esc),
    ));
    steps.push(Step::new("filter cleared", |a: &App| settings(a).is_some_and(|s| s.placeholder), key(KeyCode::Home)));
    steps.push(Step::new(
        "on the preset",
        |a: &App| cursor_line(a).is_some_and(|l| l.label == "preset"),
        key(KeyCode::Left),
    ));
    steps.push(Step::new(
        "the Vim preset is live and saved",
        move |a: &App| {
            let j = "J".parse().unwrap();
            a.core.config.keys.preset == KeyPreset::Vim
                && a.core.keymap.lookup(KeyContext::FileList, j) == Some(&Action::CursorDown)
                && cursor_line(a).is_some_and(|l| l.value == "vim")
                && std::fs::read_to_string(&file).is_ok_and(|t| t.contains("preset = \"vim\""))
        },
        key_mods(KeyCode::Char('q'), KeyModifiers::CONTROL),
    ));
    common::run_script(&mut app, steps, 120, 40);
    common::ok();
}

/// Delete removes the chosen binding (Right chooses the next chip); the filter switches
/// narrow the list (Alt+M: only changed actions).
#[test]
fn tui_keys_remove_a_binding_and_filter() {
    let run = common::relaunch(module_path!(), "tui_keys_remove_a_binding_and_filter", |_| {});
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    app.core.cmds.push(Cmd::Action(Action::ShowKeybindings));
    let mut steps =
        vec![Step::new("open", |a: &App| settings(a).is_some_and(|s| s.keys_page), key(KeyCode::Char('r')))];
    type_text(&mut steps, "efresh");
    steps.push(Step::new("filtered", |a: &App| refresh_line(a).is_some(), key(KeyCode::Enter)));
    for _ in 0..2 {
        steps.push(Step::new("moving", |_| true, key(KeyCode::Down)));
    }
    steps.push(Step::new(
        "on Refresh, F5 is a binding",
        |a: &App| {
            cursor_line(a).is_some_and(|l| Some(l) == refresh_line(a))
                && refresh_line(a).is_some_and(|l| l.chips.iter().any(|c| c.text == "F5"))
        },
        key(KeyCode::Right),
    ));
    steps.push(Step::new(
        "the second chip is chosen",
        |a: &App| refresh_line(a).is_some_and(|l| l.chips.get(1).is_some_and(|c| c.chosen)),
        key(KeyCode::Delete),
    ));
    steps.push(Step::new(
        "one binding fewer, the action is changed",
        |a: &App| {
            !a.core.config.keys.bindings.is_empty()
                && refresh_line(a).is_some_and(|l| l.modified && !l.chips.iter().any(|c| c.text == "F5"))
        },
        key_mods(KeyCode::Char('m'), KeyModifiers::ALT),
    ));
    steps.push(Step::new(
        "only changed actions",
        |a: &App| {
            settings(a).is_some_and(|s| {
                s.lines.iter().filter(|l| l.kind == LineKind::Action).count() == 1
                    && s.lines.iter().any(|l| l.kind == LineKind::Toggle && l.label.starts_with("[x]"))
            })
        },
        ctrl('q'),
    ));
    common::run_script(&mut app, steps, 120, 40);
    common::ok();
}

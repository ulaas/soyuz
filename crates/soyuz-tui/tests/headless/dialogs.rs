//! Headless tests of the dialog stacks (Phase T3 skeleton): core prompts and terminal UI
//! dialogs are ordered by when they opened, the top-most one owns the keyboard. Each test
//! runs in its own sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crossterm::event::KeyCode;
use soyuz_appcore::models::dialogs::QuitModel;
use soyuz_core::config::KeyContext;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::input::dialogs::{TuiDialog, pattern};
use soyuz_tui::widgets::form::FieldVmKind;
use soyuz_tui::widgets::{Button, Field, Form};

use crate::common::{self, Run, Step, ctrl, dialog_title, key};

const PATTERN: &str = "Select by pattern";

#[test]
fn tui_dialogs_stack_with_core_prompts() {
    let run = common::relaunch(module_path!(), "tui_dialogs_stack_with_core_prompts", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
    });
    match run {
        Run::Child(data) => run_tui(data),
        Run::Parent(p) => assert!(p.data.join("a.txt").is_file()),
    }
}

fn field_text(a: &soyuz_tui::app::App) -> Option<String> {
    match &a.view()?.dialog.as_ref()?.fields.first()?.kind {
        FieldVmKind::Text { text, .. } => Some(text.clone()),
        _ => None,
    }
}

fn run_tui(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    // A terminal UI dialog first, then a core prompt: the prompt is on top.
    let form = Form::new(vec![Field::text("Pattern", "*.txt")], vec![Button::new("OK")]);
    app.push_dialog(TuiDialog::Pattern(pattern::State { title: PATTERN.into(), form, ..Default::default() }));
    app.core.dialogs.push(QuitModel::new(1, Arc::new(AtomicBool::new(true))));
    let steps = vec![
        Step::new(
            "the core prompt on top",
            |a| {
                dialog_title(a).is_some_and(|t| t != PATTERN)
                    && a.focus() == TuiFocus::Dialog
                    && a.key_context() == KeyContext::Dialog
            },
            // The quit prompt's fallback: Esc keeps the app open.
            key(KeyCode::Esc),
        ),
        Step::new(
            "the terminal UI dialog uncovered",
            |a| dialog_title(a).as_deref() == Some(PATTERN),
            key(KeyCode::Char('x')),
        ),
        Step::new("the field edited", |a| field_text(a).as_deref() == Some("*.txtx"), key(KeyCode::Esc)),
        Step::new(
            "every dialog closed",
            |a| dialog_title(a).is_none() && a.focus() == TuiFocus::List && !a.core.dialogs.is_open(),
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

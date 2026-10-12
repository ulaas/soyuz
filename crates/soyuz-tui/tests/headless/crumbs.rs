//! Headless tests of step T2.5: breadcrumbs and the address field. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;

use crossterm::event::{Event, KeyCode};
use soyuz_core::config::KeyContext;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;

use crate::common::{self, Run, Step, at, ctrl, key};

fn typed(c: char) -> Event {
    key(KeyCode::Char(c))
}

/// The address field is open with `pred` holding for its text.
fn editing(a: &soyuz_tui::app::App, pred: impl Fn(&str) -> bool) -> bool {
    a.focus() == TuiFocus::Address
        && !a.core.ui.address.request_focus
        && a.view().and_then(|v| v.crumbs.edit.as_ref()).is_some_and(|e| pred(&e.text))
}

fn completions(a: &soyuz_tui::app::App) -> Vec<String> {
    a.view().and_then(|v| v.crumbs.edit.as_ref()).map(|e| e.completions.clone()).unwrap_or_default()
}

#[test]
fn tui_address_completes_and_navigates() {
    let run = common::relaunch(module_path!(), "tui_address_completes_and_navigates", |data| {
        std::fs::create_dir_all(data.join("sub").join("deep")).unwrap();
        std::fs::create_dir_all(data.join("other")).unwrap();
        std::fs::write(data.join("summer.txt"), b"s").unwrap();
    });
    if let Run::Child(data) = run {
        run_complete(data);
    }
}

fn run_complete(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let sub = data.join("sub");
    let d1 = data.clone();
    let sep = std::path::MAIN_SEPARATOR;
    let steps = vec![
        Step::new(
            "the listing with breadcrumbs",
            move |a| {
                at(a, &d1)
                    && a.view().is_some_and(|v| v.crumbs.segments.last().is_some_and(|s| s == "data"))
                    && a.hits().iter().any(|(_, t)| matches!(t, HitTarget::Crumb(_)))
            },
            ctrl('l'),
        ),
        // Opened on the folder with a separator after it, the cursor at the end.
        Step::new("the address field", move |a| editing(a, |t| t.ends_with(sep)), typed('s')),
        Step::new("s typed", |a| editing(a, |t| t.ends_with('s')), typed('u')),
        Step::new(
            "the completion of su",
            |a| {
                editing(a, |t| t.ends_with("su")) && completions(a) == ["sub"] && a.key_context() == KeyContext::Omnibar
            },
            key(KeyCode::Tab),
        ),
        Step::new(
            "su completed to sub",
            move |a| editing(a, |t| t.ends_with(&format!("sub{sep}"))),
            key(KeyCode::Enter),
        ),
        Step::new(
            "in sub",
            move |a| at(a, &sub) && a.focus() == TuiFocus::List && !a.core.ui.address.editing,
            ctrl('q'),
        ),
    ];
    let (terminal, _) = common::run_script(&mut app, steps, 80, 24);
    let screen = common::screen(&terminal);
    assert!(screen.lines().next().is_some_and(|l| l.trim_end().ends_with("> sub")), "{screen}");
    app.finish();
    common::ok();
}

#[test]
fn tui_address_esc_cancels() {
    let run = common::relaunch(module_path!(), "tui_address_esc_cancels", |data| {
        std::fs::create_dir_all(data.join("sub")).unwrap();
    });
    if let Run::Child(data) = run {
        run_cancel(data);
    }
}

fn run_cancel(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d1 = data.clone();
    let d2 = data.clone();
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d1) && a.view().is_some(), ctrl('l')),
        Step::new("the address field", |a| editing(a, |_| true), typed('s')),
        Step::new(
            "the completion of s",
            |a| editing(a, |t| t.ends_with('s')) && completions(a) == ["sub"],
            key(KeyCode::Down),
        ),
        // Down highlights the first completion.
        Step::new(
            "the highlighted completion",
            |a| a.view().and_then(|v| v.crumbs.edit.as_ref()).is_some_and(|e| e.selected == Some(0)),
            key(KeyCode::Esc),
        ),
        Step::new(
            "the breadcrumbs again, still in the folder",
            move |a| {
                at(a, &d2)
                    && !a.core.ui.address.editing
                    && a.focus() == TuiFocus::List
                    && a.view().is_some_and(|v| v.crumbs.edit.is_none())
            },
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 80, 24);
    assert_eq!(app.core.ws.pane().location, soyuz_core::Location::Dir(data.clone()));
    app.finish();
    common::ok();
}

//! Headless tests of T3: copy, move, trash, new items and links through the jobs engine. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! The core prompts drive real jobs here: "Copy to" into a folder with conflicting files
//! (Keep both, for all), a permanent delete after its confirmation (never the real
//! trash), quitting while a job waits for an answer, and a new hard link.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_appcore::models::dialogs::{DialogModel, LinkChoice, LinkModel, TransferModel};
use soyuz_appcore::tr;
use soyuz_tui::app::App;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::widgets::form::FieldVmKind;

use crate::common::{self, Run, Step, at, ctrl, cursor, dialog_title, key, key_mods, labels};

fn write(p: &Path, s: &str) {
    std::fs::write(p, s).unwrap();
}

/// Names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// A job prompt is on top.
fn prompt_open(a: &App) -> bool {
    matches!(a.core.dialogs.top(), Some(DialogModel::Prompt(_))) && a.focus() == TuiFocus::Dialog
}

/// The first field of the drawn dialog is a ticked checkbox.
fn first_box_ticked(a: &App) -> bool {
    a.view()
        .and_then(|v| v.dialog.as_ref())
        .and_then(|d| d.fields.first())
        .is_some_and(|f| f.kind == FieldVmKind::Check(true))
}

/// No dialog of either stack is open.
fn no_dialog(a: &App) -> bool {
    !a.core.dialogs.is_open() && dialog_title(a).is_none()
}

#[test]
fn tui_copy_to_conflicts_keep_both_for_all() {
    let run = common::relaunch(module_path!(), "tui_copy_to_conflicts_keep_both_for_all", |data| {
        for d in ["src", "dst"] {
            std::fs::create_dir_all(data.join(d)).unwrap();
        }
        for n in ["a.txt", "b.txt"] {
            write(&data.join("src").join(n), "new");
            write(&data.join("dst").join(n), "old");
        }
    });
    match run {
        Run::Child(data) => run_copy_to(data),
        Run::Parent(p) => {
            let dst = p.data.join("dst");
            let got = names(&dst);
            assert_eq!(got.len(), 4, "both kept: {got:?}");
            for n in ["a.txt", "b.txt"] {
                assert_eq!(std::fs::read_to_string(dst.join(n)).unwrap(), "old", "{n} was not replaced");
            }
            let copies: Vec<&String> = got.iter().filter(|n| !["a.txt", "b.txt"].contains(&n.as_str())).collect();
            for c in copies {
                assert_eq!(std::fs::read_to_string(dst.join(c)).unwrap(), "new", "{c} is a copy of the source");
            }
        }
    }
}

fn run_copy_to(data: PathBuf) {
    let src = data.join("src");
    let dst = data.join("dst");
    let mut app = common::new_app(common::test_config(), &src);
    let sources = vec![src.join("a.txt"), src.join("b.txt")];
    app.core.dialogs.push(TransferModel::new(sources, dst.clone(), false));
    let s1 = src.clone();
    let steps = vec![
        Step::new(
            "the Copy to dialog with the destination",
            move |a| {
                at(a, &s1)
                    && dialog_title(a).as_deref() == Some(tr!("dialogs-transfer-copy-title"))
                    && a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.body[0].contains('2'))
            },
            key(KeyCode::Enter),
        ),
        // The focus starts on Replace (the default answer); Tab wraps to "apply to all".
        Step::new(
            "the conflict prompt",
            |a| prompt_open(a) && dialog_title(a).as_deref() == Some(tr!("dialogs-conflict-title-file")),
            key(KeyCode::Tab),
        ),
        Step::new(
            "the focus on apply to all",
            |a| a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.focus == 0),
            key(KeyCode::Char(' ')),
        ),
        Step::new("apply to all ticked", first_box_ticked, key(KeyCode::Char('k'))),
        // The second conflict is answered by "apply to all": no prompt shows again.
        Step::new(
            "the copy to finish without another prompt",
            move |a| common::wait_jobs_idle(a) && no_dialog(a) && names(&dst).len() == 4,
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

#[test]
fn tui_permanent_delete_after_confirmation() {
    let run = common::relaunch(module_path!(), "tui_permanent_delete_after_confirmation", |data| {
        write(&data.join("a.txt"), "a");
        write(&data.join("b.txt"), "b");
    });
    match run {
        Run::Child(data) => run_delete(data),
        Run::Parent(p) => {
            assert!(!p.data.join("a.txt").exists(), "a.txt deleted");
            assert!(p.data.join("b.txt").is_file(), "b.txt kept (the first confirmation was cancelled)");
        }
    }
}

fn run_delete(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d1 = data.clone();
    let d2 = data.clone();
    let shift_del = key_mods(KeyCode::Delete, KeyModifiers::SHIFT);
    let confirm = |name: &'static str| {
        move |a: &App| {
            matches!(a.core.dialogs.top(), Some(DialogModel::DeleteConfirm(_)))
                && dialog_title(a).is_some_and(|t| t.contains(name))
                && a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.alert && d.buttons.len() == 2)
        }
    };
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d1) && labels(a) == ["a.txt", "b.txt"], key(KeyCode::Down)),
        Step::new("the cursor on a.txt", |a| cursor(a).as_deref() == Some("a.txt"), key(KeyCode::Down)),
        // b.txt: Shift+Delete asks; Left focuses Cancel, Enter presses it.
        Step::new("the cursor on b.txt", |a| cursor(a).as_deref() == Some("b.txt"), shift_del.clone()),
        Step::new("the confirmation for b.txt", confirm("b.txt"), key(KeyCode::Left)),
        Step::new(
            "Cancel focused",
            |a| a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.buttons[0].focused),
            key(KeyCode::Enter),
        ),
        Step::new("the confirmation closed", |a| no_dialog(a) && a.focus() == TuiFocus::List, key(KeyCode::Up)),
        Step::new("the cursor on a.txt again", |a| cursor(a).as_deref() == Some("a.txt"), shift_del),
        Step::new("the confirmation for a.txt", confirm("a.txt"), key(KeyCode::Enter)),
        Step::new(
            "a.txt deleted",
            move |a| common::wait_jobs_idle(a) && no_dialog(a) && !d2.join("a.txt").exists(),
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 80, 24);
    assert!(data.join("b.txt").is_file());
    common::ok();
}

#[test]
fn tui_quit_while_a_job_waits() {
    let run = common::relaunch(module_path!(), "tui_quit_while_a_job_waits", |data| {
        for d in ["src", "dst"] {
            std::fs::create_dir_all(data.join(d)).unwrap();
        }
        write(&data.join("src").join("a.txt"), "new");
        write(&data.join("dst").join("a.txt"), "old");
    });
    match run {
        Run::Child(data) => run_quit(data),
        Run::Parent(p) => {
            assert_eq!(names(&p.data.join("dst")), ["a.txt"], "the cancelled copy left nothing behind");
            assert_eq!(std::fs::read_to_string(p.data.join("dst").join("a.txt")).unwrap(), "old");
        }
    }
}

fn run_quit(data: PathBuf) {
    let src = data.join("src");
    let mut app = common::new_app(common::test_config(), &src);
    app.core.dialogs.push(TransferModel::new(vec![src.join("a.txt")], data.join("dst"), false));
    let quit_open =
        |a: &App| matches!(a.core.dialogs.top(), Some(DialogModel::Quit(_))) && a.focus() == TuiFocus::Dialog;
    let steps = vec![
        Step::new(
            "the Copy to dialog",
            |a| dialog_title(a).as_deref() == Some(tr!("dialogs-transfer-copy-title")),
            key(KeyCode::Enter),
        ),
        // Esc decides later: the job keeps waiting.
        Step::new("the conflict prompt", prompt_open, key(KeyCode::Esc)),
        Step::new(
            "the prompt deferred",
            |a| no_dialog(a) && !common::wait_jobs_idle(a) && a.focus() == TuiFocus::List,
            ctrl('q'),
        ),
        // A second Ctrl+Q while the question is open does nothing; K keeps Soyuz open.
        Step::new("the quit question", quit_open, ctrl('q')),
        Step::new("still asking", quit_open, key(KeyCode::Char('k'))),
        Step::new("kept open", |a| no_dialog(a) && !common::wait_jobs_idle(a), ctrl('q')),
        // C: cancel the job and quit.
        Step::new("the quit question again", quit_open, key(KeyCode::Char('c'))),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

#[test]
fn tui_new_hard_link() {
    let run = common::relaunch(module_path!(), "tui_new_hard_link", |data| {
        write(&data.join("a.txt"), "data");
    });
    match run {
        Run::Child(data) => run_link(data),
        Run::Parent(p) => {
            let got = names(&p.data);
            assert_eq!(got.len(), 2, "{got:?}");
            for n in got {
                assert_eq!(std::fs::read_to_string(p.data.join(n)).unwrap(), "data");
            }
        }
    }
}

fn run_link(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let model = LinkModel::new(data.clone(), Some(data.join("a.txt")), LinkChoice::Hardlink, vec!["a.txt".into()]);
    app.core.dialogs.push(model);
    let d = data.clone();
    let steps = vec![
        Step::new(
            "the new link prompt with a name",
            |a| {
                dialog_title(a).as_deref() == Some(tr!("dialogs-link-title"))
                    && a.view()
                        .and_then(|v| v.dialog.as_ref())
                        .is_some_and(|d| d.fields.len() == 3 && d.error.is_none())
            },
            key(KeyCode::Enter),
        ),
        Step::new("the link", move |a| common::wait_jobs_idle(a) && no_dialog(a) && names(&d).len() == 2, ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

#[test]
fn tui_link_dialog_picks_a_target() {
    let run = common::relaunch(module_path!(), "tui_link_dialog_picks_a_target", |data| {
        write(&data.join("a.txt"), "data");
    });
    match run {
        Run::Child(data) => run_link_dialog(data),
        Run::Parent(p) => {
            let got = names(&p.data);
            assert_eq!(got.len(), 2, "{got:?}");
            for n in got {
                assert_eq!(std::fs::read_to_string(p.data.join(n)).unwrap(), "data", "a hard link to a.txt");
            }
        }
    }
}

fn run_link_dialog(data: PathBuf) {
    use soyuz_appcore::models::frontend::FrontendCmd;
    use soyuz_appcore::models::properties::{LinkKind, MetaDialog};
    let mut app = common::new_app(common::test_config(), &data);
    // `NewHardlink` with nothing selected: no target yet.
    app.core.frontend.push(FrontendCmd::MetaDialog(MetaDialog::Link(LinkKind::Hardlink)));
    let d = data.clone();
    let dialog = |a: &App| a.view().and_then(|v| v.dialog.clone());
    let target = |a: &App| {
        a.view()
            .and_then(|v| v.dialog.as_ref())
            .and_then(|d| d.fields.get(1))
            .and_then(|f| match &f.kind {
                FieldVmKind::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    };
    let steps = vec![
        // The picker lists the folder: "..", then a.txt; Create waits for a target.
        Step::new(
            "the link dialog with the picker",
            move |a| {
                dialog(a).is_some_and(|d| {
                    d.title == tr!("dialogs-link-title")
                        && d.list.iter().any(|r| r.text == "a.txt")
                        && d.buttons.get(1).is_some_and(|b| !b.enabled)
                })
            },
            key(KeyCode::Down),
        ),
        Step::new("the picker armed on ..", |_| true, key(KeyCode::Down)),
        Step::new(
            "the cursor on a.txt",
            move |a| dialog(a).is_some_and(|d| d.list.get(d.list_state.cursor).is_some_and(|r| r.text == "a.txt")),
            key(KeyCode::Tab),
        ),
        // Tab took it: the target and the name follow, the probe says "File".
        Step::new(
            "a.txt is the target",
            move |a| {
                target(a).ends_with("a.txt")
                    && dialog(a).is_some_and(|d| {
                        d.body.iter().any(|l| l.ends_with(tr!("dialogs-link-file")))
                            && d.buttons.get(1).is_some_and(|b| b.enabled)
                    })
            },
            key(KeyCode::Enter),
        ),
        Step::new("the link", move |a| common::wait_jobs_idle(a) && no_dialog(a) && names(&d).len() == 2, ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

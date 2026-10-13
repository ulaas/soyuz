//! One run through the T3 / T4 terminal UI end to end, in its own sandbox
//! (`crate::common::relaunch`): copy a file with Ctrl+C / Ctrl+V into a folder that has
//! one of the same name (Keep both), see the copy in the jobs panel's history (Alt+J,
//! Tab), rename a file inline (F2), show it in the preview pane (Alt+P) and in Quick Look
//! (Space, Space closes), then edit it (F4), type, save (Ctrl+S) and close it (Ctrl+W).
//!
//! The system clipboard is off in the child process
//! (`os_clipboard::set_system_clipboard(false)`): Ctrl+C / Ctrl+V go through the
//! internal clipboard only and never touch the user's.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::ai::state::{CardState, ChatItem};
use soyuz_appcore::editors::DocKind;
use soyuz_appcore::models::compare::RowAction;
use soyuz_appcore::models::devices::{self, DeskBackend};
use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::models::frontend::FrontendCmd;
use soyuz_appcore::models::split::{checksum_path, part_path};
use soyuz_appcore::models::tools::{ToolCmd, queue, shelf_open};
use soyuz_appcore::models::versions::looking_text;
use soyuz_appcore::models::workspaces::{self, WsCmd};
use soyuz_appcore::ops::{self, NavOpts};
use soyuz_appcore::palette::Exec;
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::{
    self, Action, Config, KeyContext, SidebarSection, TuiBorders, TuiColorMode, TuiIcons, TuiOsc52, UnmountedDevices,
};
use soyuz_fs::udisks::{BlockDevice, MountOptions};
use soyuz_tui::app::{App, EventSource, LoopEnd, NullIo, TuiEvent, run_loop};
use soyuz_tui::input::TuiFocus;
use soyuz_tui::suspend::{self, ChildSpec, Env};
use soyuz_tui::view::compare::{BodyVm as CompareBodyVm, TableVm as CompareTableVm};
use soyuz_tui::view::editor::BodyVm;
use soyuz_tui::view::screens::dupes::{DupesVm, LineKind, StatusVm as DupesStatus};
use soyuz_tui::view::screens::treemap::{StatusVm as TreemapStatus, TreemapVm};
use soyuz_tui::view::screens::{ScreenVm, ScreensVm};
use soyuz_tui::view::search::{Progress, StatusVm as SearchStatusVm};

use super::menu::{menu_cursor, menu_labels};
use super::workspaces::{field, no_dialog, row_cursor, rows};
use crate::common::{
    self, Run, Step, alt, at, ctrl, cursor, dialog_title, key, key_mods, labels, toasts, type_text, wait_jobs_idle,
};

/// Names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

/// The first line of `p` (empty when it cannot be read).
fn first_line(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default().lines().next().unwrap_or_default().to_string()
}

/// The inline rename field shows `text`.
fn rename_text(a: &App, text: &str) -> bool {
    a.view()
        .and_then(|v| v.focused_pane())
        .is_some_and(|p| p.rows().any(|r| r.edit.as_ref().is_some_and(|e| e.text == text)))
}

#[test]
fn tui_copy_paste_jobs_rename_preview_quicklook_edit() {
    let run = common::relaunch(module_path!(), "tui_copy_paste_jobs_rename_preview_quicklook_edit", |data| {
        for d in ["src", "dst"] {
            std::fs::create_dir_all(data.join(d)).unwrap();
        }
        std::fs::write(data.join("src").join("a.txt"), "new copy\n").unwrap();
        std::fs::write(data.join("dst").join("a.txt"), "old file\n").unwrap();
    });
    match run {
        Run::Child(data) => run_e2e(data),
        Run::Parent(p) => {
            let dst = p.data.join("dst");
            let got = names(&dst);
            assert_eq!(got.len(), 2, "the copy kept both: {got:?}");
            assert!(got.iter().any(|n| n == "renamed.txt"), "{got:?}");
            let edited = std::fs::read_to_string(dst.join("renamed.txt")).unwrap();
            assert!(edited.starts_with("e2e:"), "the edit was saved: {edited:?}");
            let other = got.iter().find(|n| *n != "renamed.txt").unwrap();
            // One file came from src, the other was there: both contents survive.
            let mut contents =
                [edited.trim_start_matches("e2e:").to_string(), std::fs::read_to_string(dst.join(other)).unwrap()];
            contents.sort();
            assert_eq!(contents, ["new copy\n".to_string(), "old file\n".to_string()]);
            assert_eq!(std::fs::read_to_string(p.data.join("src").join("a.txt")).unwrap(), "new copy\n");
        }
    }
}

fn run_e2e(data: PathBuf) {
    soyuz_appcore::services::os_clipboard::set_system_clipboard(false);
    let mut c = common::test_config();
    c.tui.preview_ratio = 0.5;
    let src = data.join("src");
    let dst = data.join("dst");
    let mut app = common::new_app(c, &src);
    app.host.set_local_clipboard(false);
    let (d1, d2, d3) = (dst.clone(), dst.clone(), dst.clone());
    let renamed = dst.join("renamed.txt");
    let (r1, r2) = (renamed.clone(), renamed.clone());
    let preview_has = move |a: &App| {
        let want = first_line(&r1);
        !want.is_empty()
            && a.view().is_some_and(|v| v.preview.visible && v.preview.body_text().iter().any(|l| l.contains(&want)))
    };
    let quicklook_on = |a: &App| {
        a.focus() == TuiFocus::QuickLook
            && a.view().is_some_and(|v| v.quicklook.visible && v.quicklook.title == "renamed.txt")
    };
    let text_ready = |a: &App| a.view().is_some_and(|v| v.editor.focused && matches!(v.editor.body, BodyVm::Text(_)));
    let doc_text = |a: &App| match a.core.editors.active_doc().map(|d| &d.kind) {
        Some(DocKind::Text(t)) => t.buf.text(),
        _ => String::new(),
    };
    let saved = move |a: &App| {
        a.core.editors.dirty_count() == 0
            && !a.core.editors.docs.iter().any(|d| d.saving)
            && std::fs::read_to_string(&r2).is_ok_and(|s| s.starts_with("e2e:"))
    };
    let history_row = |a: &App| {
        a.view().is_some_and(|v| v.jobs.tab == 1 && v.jobs.items.first().is_some_and(|i| i.title.starts_with("Copied")))
    };
    let mut steps = vec![
        // Copy a.txt (the internal clipboard), go up and into dst.
        Step::new("the listing", move |a| at(a, &src) && labels(a) == ["a.txt"], key(KeyCode::Down)),
        Step::new("the cursor on a.txt", |a| cursor(a).as_deref() == Some("a.txt"), ctrl('c')),
        Step::new("a.txt on the clipboard", |a| !a.core.fileops.clipboard.is_empty(), key(KeyCode::Backspace)),
        Step::new("the parent folder", |a| labels(a) == ["dst/", "src/"] && cursor(a).is_some(), key(KeyCode::Home)),
        Step::new("the cursor on dst", |a| cursor(a).as_deref() == Some("dst/"), key(KeyCode::Enter)),
        Step::new("dst with its a.txt", move |a| at(a, &d1) && labels(a) == ["a.txt"], ctrl('v')),
        // The name is taken: Keep both (K).
        Step::new(
            "the conflict prompt",
            |a| matches!(a.core.dialogs.top(), Some(DialogModel::Prompt(_))) && a.focus() == TuiFocus::Dialog,
            key(KeyCode::Char('k')),
        ),
        Step::new(
            "both files in dst",
            move |a| wait_jobs_idle(a) && !a.core.dialogs.is_open() && names(&d2).len() == 2 && labels(a).len() == 2,
            alt('j'),
        ),
        // The jobs panel: its history lists the copy.
        Step::new("the jobs panel has the keyboard", |a| a.focus() == TuiFocus::Jobs, key(KeyCode::Tab)),
        Step::new("the history lists the copy", history_row, key(KeyCode::Esc)),
        // Rename the first file inline.
        Step::new("the list again", |a| a.focus() == TuiFocus::List, key(KeyCode::Home)),
        Step::new(
            "the cursor on the first file",
            |a| cursor(a).is_some_and(|c| labels(a).first() == Some(&c)),
            key(KeyCode::F(2)),
        ),
        Step::new("the rename field", |a| a.focus() == TuiFocus::Rename, ctrl('a')),
    ];
    type_text(&mut steps, "renamed.txt");
    steps.extend([
        Step::new("the new name typed", |a| rename_text(a, "renamed.txt"), key(KeyCode::Enter)),
        Step::new(
            "renamed on disk, the cursor on it",
            move |a| {
                d3.join("renamed.txt").is_file()
                    && a.core.fileops.rename.is_none()
                    && wait_jobs_idle(a)
                    && cursor(a).as_deref() == Some("renamed.txt")
            },
            alt('p'),
        ),
        // The preview pane shows it, then Quick Look.
        Step::new("the preview shows the file", preview_has, key(KeyCode::Char(' '))),
        Step::new("Quick Look on renamed.txt", quicklook_on, key(KeyCode::Char(' '))),
        Step::new(
            "Quick Look closed",
            |a| !a.core.ui.quicklook.open && a.focus() == TuiFocus::List && cursor(a).as_deref() == Some("renamed.txt"),
            key(KeyCode::F(4)),
        ),
        // Edit: type at the start, save, close.
        Step::new("the text editor has the keyboard", text_ready, key(KeyCode::Char('e'))),
    ]);
    type_text(&mut steps, "2e:");
    steps.extend([
        Step::new("typed", move |a| doc_text(a).starts_with("e2e:"), ctrl('s')),
        Step::new("saved", saved, ctrl('w')),
        Step::new("closed", |a| a.core.editors.docs.is_empty(), ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 100, 30);
    let got = names(&dst);
    assert!(got.iter().any(|n| n == "renamed.txt") && got.len() == 2, "{got:?}");
    common::ok();
}

// ---- Phase T5 + T6: one run through every new part ----

/// What a [`Beat`] does with the last frame.
enum Act {
    /// Not yet: wait for the next frame.
    Wait,
    /// Send this event and go on to the next beat.
    Send(Event),
    /// Send this event and look again after the next frame (e.g. move the cursor).
    Repeat(Event),
}

/// One beat of the T5 / T6 script.
struct Beat {
    what: &'static str,
    act: Box<dyn FnMut(&App) -> Act>,
}

/// Wait until `when` holds, then send `ev`.
fn beat(what: &'static str, when: impl Fn(&App) -> bool + 'static, ev: Event) -> Beat {
    let mut ev = Some(ev);
    Beat { what, act: Box::new(move |a| if when(a) { Act::Send(ev.take().expect("sent once")) } else { Act::Wait }) }
}

/// Move the focused pane's cursor onto `name` (Down, wrapping with Home), then send `then`.
fn seek(what: &'static str, name: &'static str, then: Event) -> Beat {
    let mut then = Some(then);
    Beat {
        what,
        act: Box::new(move |a| {
            if cursor(a).as_deref() == Some(name) {
                Act::Send(then.take().expect("sent once"))
            } else if labels(a).iter().any(|l| l == name) {
                let last = labels(a).last().cloned();
                Act::Repeat(key(if last.is_some() && last == cursor(a) { KeyCode::Home } else { KeyCode::Down }))
            } else {
                Act::Wait
            }
        }),
    }
}

/// Repeat `ev` while `again` holds, then send `then` once `done` holds.
fn until(
    what: &'static str,
    done: impl Fn(&App) -> bool + 'static,
    again: impl Fn(&App) -> bool + 'static,
    ev: Event,
    then: Event,
) -> Beat {
    let mut then = Some(then);
    Beat {
        what,
        act: Box::new(move |a| {
            if done(a) {
                Act::Send(then.take().expect("sent once"))
            } else if again(a) {
                Act::Repeat(ev.clone())
            } else {
                Act::Wait
            }
        }),
    }
}

/// Beats that type `text`, one character per frame.
fn typing(beats: &mut Vec<Beat>, text: &str) {
    for c in text.chars() {
        beats.push(beat("typing", |_| true, key(KeyCode::Char(c))));
    }
}

/// The event source playing the beats (60 s per beat; the app must quit after the last).
struct Beats {
    beats: VecDeque<Beat>,
    since: Instant,
}

impl EventSource for Beats {
    fn next(&mut self, app: &App, redraw: &Receiver<()>, timeout: Duration) -> Vec<TuiEvent> {
        let wait = timeout.min(Duration::from_millis(10));
        let Some(b) = self.beats.front_mut() else {
            assert!(self.since.elapsed() < Duration::from_secs(60), "the app did not quit after the last beat");
            let _ = redraw.recv_timeout(wait);
            return Vec::new();
        };
        match (b.act)(app) {
            Act::Send(e) => {
                self.beats.pop_front();
                self.since = Instant::now();
                vec![TuiEvent::Term(e)]
            }
            Act::Repeat(e) => {
                assert!(self.since.elapsed() < Duration::from_secs(60), "timed out at {}", b.what);
                vec![TuiEvent::Term(e)]
            }
            Act::Wait => {
                assert!(
                    self.since.elapsed() < Duration::from_secs(60),
                    "timed out waiting for {}; focus {:?}, rows {:?}, dialog {:?}, toasts {:?}",
                    b.what,
                    app.focus(),
                    labels(app),
                    dialog_title(app),
                    common::toasts(app)
                );
                let _ = redraw.recv_timeout(wait);
                Vec::new()
            }
        }
    }
}

/// `config.toml` of the sandbox (portable mode: next to the test binary's copy).
fn config_file(data: &Path) -> PathBuf {
    let name = config::default_path().file_name().expect("config file name").to_owned();
    data.parent().expect("sandbox").join(config::PORTABLE_DATA_DIR).join(name)
}

/// The settings of the T5 / T6 run: the assistant may work in the sandbox (fake model), a
/// predictable shell, `$EDITOR` is this test binary (its runs are faked), and the commands
/// without a default chord get one.
fn t56_config(data: &Path) -> Config {
    let mut c = common::test_config();
    c.ai.enabled = true;
    c.ai.scope.allowed_roots = vec![data.to_path_buf()];
    c.terminal.shell = if cfg!(windows) { "cmd.exe".into() } else { "/bin/sh".into() };
    c.terminal.args = Vec::new();
    c.terminal.shell_integration = false;
    c.terminal.follow_navigation = false;
    c.terminal.follow_shell = false;
    let exe = std::env::current_exe().unwrap().to_string_lossy().into_owned();
    c.tui.editor = format!("\"{exe}\" --wait");
    for (action, chord) in [("create_archive", "Ctrl+F9"), ("extract_to", "Ctrl+F10"), ("compare_folders", "Ctrl+F11")]
    {
        c.keys.bindings.insert(action.into(), vec![chord.into()]);
    }
    c
}

/// No `$VISUAL` / `$EDITOR`, an empty `PATH`: only `[tui] editor` counts.
fn isolated_env() -> Env {
    Env {
        visual: None,
        editor: None,
        path_dirs: Vec::new(),
        is_file: Box::new(Path::is_file),
        is_windows: cfg!(windows),
    }
}

/// The fake `$EDITOR` saw the bookmark the table editor added (the file was saved first).
static EDITOR_SAW_BOOKMARK: AtomicBool = AtomicBool::new(false);

/// A fake `$EDITOR` on config.toml: notes whether the file holds the `E2E` bookmark, then
/// turns `general.show_hidden` off again.
fn fake_editor(spec: &ChildSpec) -> io::Result<Option<i32>> {
    let path = PathBuf::from(spec.args.last().expect("the file to edit"));
    let mut c = config::load(&path).expect("config.toml").config;
    if c.bookmarks.iter().any(|b| b.name == "E2E") {
        EDITOR_SAW_BOOKMARK.store(true, Ordering::SeqCst);
    }
    c.general.show_hidden = false;
    config::save_pretty(&path, &c).unwrap();
    Ok(Some(0))
}

fn key_with(code: KeyCode, mods: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, mods))
}

fn search_status(a: &App) -> Option<&SearchStatusVm> {
    a.view()?.search.status.as_ref()
}

/// The terminal pane's shell waits at an empty prompt.
fn at_prompt(a: &App) -> bool {
    let t = &a.core.ui.terminal;
    let owner = soyuz_appcore::models::terminal::current_owner(&a.core);
    t.active_id(owner).and_then(|id| t.session(id)).and_then(|s| s.term.as_ref()).is_some_and(|t| t.at_prompt())
}

fn grid(a: &App) -> String {
    a.view().and_then(|v| v.terminal.grid.as_ref()).map(|g| g.text()).unwrap_or_default()
}

/// A plan card of the running AI task waits for an answer and is drawn.
fn card_waits(a: &App) -> bool {
    let Some(r) = &a.core.ai.running else { return false };
    let waits = a
        .core
        .ai
        .chat
        .iter()
        .any(|c| matches!(c, ChatItem::Plan(p) if p.task == r.task && p.state == CardState::Pending));
    waits && a.view().is_some_and(|v| v.ai.lines.iter().any(|l| l.text().contains("[Approve]")))
}

fn ai_task_ended(a: &App) -> bool {
    a.core.ai.running.is_none() && a.core.ai.chat.iter().any(|c| matches!(c, ChatItem::TaskEnd(_)))
}

fn settings_open(a: &App) -> bool {
    a.view().is_some_and(|v| v.settings.visible)
}

/// The settings editor's cursor is on the key named `name`.
fn settings_on(a: &App, name: &str) -> bool {
    a.view().and_then(|v| v.settings.lines.iter().find(|l| l.cursor)).is_some_and(|l| l.label == name)
}

fn compare_table(a: &App) -> Option<&CompareTableVm> {
    match &a.view().map(|v| &v.compare).filter(|c| c.visible)?.body {
        CompareBodyVm::Table(t) => Some(t),
        _ => None,
    }
}

fn pane_at(a: &App, i: usize, dir: &Path) -> bool {
    a.core.ws.tab().panes.get(i).is_some_and(|p| p.location == Location::Dir(dir.to_path_buf()))
}

/// One run through Phase T5 + T6: search → compress and extract an archive → `echo` in the
/// terminal pane → an AI plan card approved (fake model) → a settings change → a table
/// edit (a bookmark) → `$EDITOR` on config.toml (a fake runner) → compare two folders and
/// mirror them → quit. The checks after the run read what it left on disk.
#[test]
fn tui_t5_t6_search_archive_terminal_ai_settings_compare() {
    let transcript = super::ai::fixture("move.toml");
    let run = common::relaunch_env(
        module_path!(),
        "tui_t5_t6_search_archive_terminal_ai_settings_compare",
        &[("SOYUZ_AI_FAKE", OsStr::new(&transcript))],
        |data| {
            for d in ["deep", "pack", "out", "Moved", "left", "right"] {
                std::fs::create_dir_all(data.join(d)).unwrap();
            }
            std::fs::write(data.join("deep").join("needle.txt"), b"n").unwrap();
            std::fs::write(data.join("pack").join("one.txt"), b"1").unwrap();
            std::fs::write(data.join("a.txt"), b"a").unwrap();
            std::fs::write(data.join("left").join("x.txt"), b"left side").unwrap();
        },
    );
    match run {
        Run::Child(data) => run_t56(data),
        Run::Parent(p) => {
            let d = &p.data;
            assert_eq!(std::fs::read(d.join("out").join("pack").join("one.txt")).unwrap(), b"1", "extracted");
            assert!(d.join("Moved").join("a.txt").is_file() && !d.join("a.txt").exists(), "the AI plan ran");
            assert_eq!(std::fs::read(d.join("right").join("x.txt")).unwrap(), b"left side", "mirrored");
            let text = std::fs::read_to_string(config_file(d)).unwrap();
            assert!(text.contains("[[bookmarks]]") && text.contains("name = \"E2E\""), "{text}");
            assert!(text.contains("show_hidden = false"), "{text}");
        }
    }
}

fn run_t56(data: PathBuf) {
    soyuz_appcore::services::os_clipboard::set_system_clipboard(false);
    suspend::set_env_source(isolated_env);
    let c = t56_config(&data);
    // config.toml holds what the app starts with, so the hot reload at start changes nothing.
    let file = config_file(&data);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    config::save_pretty(&file, &c).unwrap();
    let mut app = common::new_app(c, &data);
    app.host.set_local_clipboard(false);
    app.child_runner = fake_editor;
    let (d0, d1, d2, d3) = (data.clone(), data.clone(), data.clone(), data.clone());
    let zip = data.join("pack.zip");
    let (z1, z2) = (zip.clone(), zip);
    let in_zip =
        move |a: &App| matches!(&a.core.ws.pane().location, Location::Archive { archive, .. } if *archive == z1);
    let extracted = data.join("out").join("pack").join("one.txt");
    let moved = data.join("Moved").join("a.txt");
    let (left, right) = (data.join("left"), data.join("right"));
    let (l1, l2, r1, r2) = (left.clone(), left, right.clone(), right.join("x.txt"));
    let ctrl_shift_f = key_with(KeyCode::Char('F'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    let mut b = vec![
        // Search: the bar (Ctrl+Shift+F), a query, Enter; the results; Esc; back.
        beat("the listing", move |a| at(a, &d0) && labels(a).len() == 7, ctrl_shift_f),
        beat("the search bar has the keyboard", |a| a.focus() == TuiFocus::Search, key(KeyCode::Char('n'))),
    ];
    typing(&mut b, "eedle");
    b.extend([
        beat("typed", |a| a.core.ui.search.form.text == "needle", key(KeyCode::Enter)),
        beat(
            "the search found deep/needle.txt",
            |a| {
                matches!(a.core.ws.pane().location, Location::Search(_))
                    && search_status(a).is_some_and(|s| s.progress == Progress::Done && s.head.contains("1 result"))
                    && labels(a).len() == 1
                    && labels(a)[0].ends_with("needle.txt")
            },
            key(KeyCode::Esc),
        ),
        beat(
            "the bar closed",
            |a| a.focus() == TuiFocus::List && a.view().is_some_and(|v| v.search.bar.is_none()),
            key_with(KeyCode::Left, KeyModifiers::ALT),
        ),
        // Archive: compress pack/ (Compress… → Enter), browse the zip, Extract to… out.
        seek("back in the data folder", "pack/", key_with(KeyCode::F(9), KeyModifiers::CONTROL)),
        beat(
            "the compress dialog",
            |a| dialog_title(a).as_deref() == Some(tr!("views-archive-compress")),
            key(KeyCode::Enter),
        ),
        seek("pack.zip", "pack.zip", key(KeyCode::Enter)),
        beat(
            "inside the zip",
            move |a| in_zip(a) && labels(a) == ["pack/"],
            key_with(KeyCode::F(10), KeyModifiers::CONTROL),
        ),
        beat(
            "the extract dialog",
            |a| dialog_title(a).as_deref() == Some(tr!("views-archive-extract")),
            key(KeyCode::End),
        ),
    ]);
    typing(&mut b, &format!("{}out", std::path::MAIN_SEPARATOR));
    b.extend([
        beat("typed", |_| true, key(KeyCode::Enter)),
        beat(
            "extracted",
            move |a| dialog_title(a).is_none() && wait_jobs_idle(a) && extracted.is_file() && z2.is_file(),
            key(KeyCode::Backspace),
        ),
        // The terminal pane: Ctrl+` starts a shell in the folder; echo; exit closes it.
        beat("out of the zip", move |a| at(a, &d1), key_with(KeyCode::Char('`'), KeyModifiers::CONTROL)),
        beat(
            "a prompt in the terminal pane",
            |a| a.view().is_some_and(|v| v.terminal.visible && v.terminal.focused) && at_prompt(a),
            key(KeyCode::Char('e')),
        ),
    ]);
    typing(&mut b, "cho t56-e2e");
    b.push(beat("typed", |_| true, key(KeyCode::Enter)));
    b.push(beat(
        "the echo in the grid",
        |a| grid(a).lines().filter(|l| l.trim_end().ends_with("t56-e2e")).count() >= 2 && at_prompt(a),
        key(KeyCode::Char('e')),
    ));
    typing(&mut b, "xit");
    b.push(beat("typed", |_| true, key(KeyCode::Enter)));
    // The AI panel: Ctrl+I, a prompt, the plan card, Tab to the cards, A approves.
    b.push(beat(
        "the session ended and the pane closed",
        move |a| a.core.ui.terminal.sessions.is_empty() && !a.core.ui.bottom.terminal_shown() && at(a, &d2),
        ctrl('i'),
    ));
    b.push(beat("the AI panel has the keyboard", |a| a.focus() == TuiFocus::Ai, key(KeyCode::Char('m'))));
    typing(&mut b, "ove it");
    b.extend([
        beat("typed", |_| true, key(KeyCode::Enter)),
        beat("a plan card waits", card_waits, key(KeyCode::Tab)),
        beat("the cards have the keyboard", |a| a.focus() == TuiFocus::Ai, key(KeyCode::Char('a'))),
        beat(
            "the approved plan ran as a job",
            move |a| moved.is_file() && ai_task_ended(a) && wait_jobs_idle(a),
            ctrl('i'),
        ),
        // Settings: Ctrl+, ; search a key; Space toggles it (live).
        beat(
            "the panel closed",
            |a| a.focus() == TuiFocus::List && a.view().is_some_and(|v| !v.ai.visible),
            key_with(KeyCode::Char(','), KeyModifiers::CONTROL),
        ),
        beat("the settings editor", settings_open, key(KeyCode::Char('/'))),
    ]);
    typing(&mut b, "general.show_hidden");
    b.extend([
        beat("typed", |_| true, key(KeyCode::Enter)),
        beat("on show_hidden", |a| settings_on(a, "show_hidden"), key(KeyCode::Char(' '))),
        beat("hidden files on", |a| a.core.config.general.show_hidden, key(KeyCode::Char('/'))),
    ]);
    // A table: bookmarks → Enter → Insert → a name → back; live at once.
    for _ in 0.."general.show_hidden".len() {
        b.push(beat("clearing the search", |_| true, key(KeyCode::Backspace)));
    }
    typing(&mut b, "bookmarks");
    b.push(beat("typed", |_| true, key(KeyCode::Enter)));
    b.push(until(
        "the bookmarks row",
        |a| settings_on(a, "bookmarks"),
        settings_open,
        key(KeyCode::Down),
        key(KeyCode::Enter),
    ));
    b.push(beat(
        "the table editor",
        |a| dialog_title(a).is_some_and(|t| t.contains("bookmarks")),
        key(KeyCode::Insert),
    ));
    b.push(beat(
        "the element form",
        |a| dialog_title(a).is_some_and(|t| t.contains("bookmarks ·")),
        key(KeyCode::Char('E')),
    ));
    typing(&mut b, "2E");
    b.push(beat("typed", |_| true, key(KeyCode::Enter)));
    // $EDITOR (Ctrl+E) on config.toml: the fake editor turns show_hidden off again.
    b.push(until(
        "the name is live and the dialogs closed",
        |a| a.core.config.bookmarks.iter().any(|b| b.name == "E2E") && settings_open(a) && dialog_title(a).is_none(),
        |a| a.core.config.bookmarks.iter().any(|b| b.name == "E2E") && dialog_title(a).is_some(),
        key(KeyCode::Esc),
        ctrl('e'),
    ));
    b.push(until(
        "back from $EDITOR: the file's values are live; Esc closes the settings editor",
        |a| !a.core.ui.settings_open && !settings_open(a) && !a.core.config.general.show_hidden,
        |a| {
            !a.core.config.general.show_hidden
                && a.core.config.bookmarks.iter().any(|b| b.name == "E2E")
                && EDITOR_SAW_BOOKMARK.load(Ordering::SeqCst)
                && settings_open(a)
        },
        key(KeyCode::Esc),
        key(KeyCode::F(9)),
    ));
    // Compare: dual panes on left/ and right/, Compare folders, mirror, sync.
    b.extend([
        beat("dual panes", |a| a.core.ws.tab().shown() == 2, key(KeyCode::Tab)),
        beat(
            "the second pane focused",
            move |a| a.core.ws.tab().focused == 1 && pane_at(a, 1, &d3),
            key(KeyCode::Home),
        ),
        seek("right/ in the second pane", "right/", key(KeyCode::Enter)),
        beat("the second pane in right", move |a| pane_at(a, 1, &r1), key(KeyCode::Tab)),
        seek("left/ in the first pane", "left/", key(KeyCode::Enter)),
        beat(
            "the first pane in left",
            move |a| a.core.ws.tab().focused == 0 && pane_at(a, 0, &l1),
            key_with(KeyCode::F(11), KeyModifiers::CONTROL),
        ),
        beat(
            "the compare screen lists x.txt",
            |a| {
                a.focus() == TuiFocus::Tool
                    && compare_table(a).is_some_and(|t| t.total == 1 && t.rows.iter().any(|r| r.rel == "x.txt"))
            },
            key(KeyCode::Char('p')),
        ),
        beat(
            "mirror: x.txt copies to the right",
            |a| compare_table(a).is_some_and(|t| t.rows.iter().all(|r| r.action == RowAction::CopyRight)),
            key(KeyCode::Enter),
        ),
        beat(
            "the plan",
            |a| matches!(a.view().map(|v| &v.compare.body), Some(CompareBodyVm::Plan(p)) if p.total > 0 && !p.danger),
            key(KeyCode::Enter),
        ),
        beat(
            "synced",
            move |a| {
                wait_jobs_idle(a)
                    && r2.is_file()
                    && a.view()
                        .is_some_and(|v| !v.compare.scanning && matches!(v.compare.body, CompareBodyVm::Note(..)))
            },
            key(KeyCode::Esc),
        ),
        beat("the compare screen closed", move |a| a.focus() == TuiFocus::List && pane_at(a, 0, &l2), ctrl('q')),
    ]);
    let mut source = Beats { beats: b.into(), since: Instant::now() };
    let mut terminal = Terminal::new(TestBackend::new(120, 34)).expect("test terminal");
    let mut io = NullIo::default();
    let end = run_loop(&mut app, &mut terminal, &mut source, &mut io).expect("the loop runs");
    assert_eq!(end, LoopEnd::Quit);
    assert!(source.beats.is_empty(), "every beat ran");
    assert_eq!((io.suspended, io.resumed), (1, 1), "one $EDITOR run");
    app.finish();
    common::ok();
}

// ---- Phase T7: one run through every new part ----

const UDISKS: &str = "/org/freedesktop/UDisks2/block_devices/";

/// What the fake UDisks2 backend was asked, in order ("unlock sdb2 right", "mount dm0").
type Calls = Arc<Mutex<Vec<String>>>;

/// A UDisks2 stand-in with one locked BitLocker partition ("Vault"): "right" unlocks it
/// and the mount that follows lands in `mount_at` (a sandbox folder). No real device is
/// ever asked, on any platform.
struct FakeDesk {
    calls: Calls,
    mount_at: PathBuf,
}

impl FakeDesk {
    fn note(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

fn short(object: &str) -> &str {
    object.strip_prefix(UDISKS).unwrap_or(object)
}

impl DeskBackend for FakeDesk {
    fn available(&self) -> bool {
        true
    }
    fn block_devices(&self) -> io::Result<Vec<BlockDevice>> {
        Ok(vec![BlockDevice {
            object: format!("{UDISKS}sdb2"),
            device: PathBuf::from("/dev/sdb2"),
            size: 32_000_000_000,
            connection_bus: "usb".into(),
            removable: true,
            drive: Some("/org/freedesktop/UDisks2/drives/Stick".into()),
            partition_table: Some(format!("{UDISKS}sdb")),
            label: "Vault".into(),
            usage: "crypto".into(),
            fs_type: "BitLocker".into(),
            encrypted: true,
            partition_number: 2,
            ..Default::default()
        }])
    }
    fn mount(&self, object: &str, _options: &MountOptions) -> io::Result<PathBuf> {
        self.note(format!("mount {}", short(object)));
        Ok(self.mount_at.clone())
    }
    fn unmount(&self, object: &str) -> io::Result<()> {
        self.note(format!("unmount {}", short(object)));
        Ok(())
    }
    fn lock(&self, object: &str) -> io::Result<()> {
        self.note(format!("lock {}", short(object)));
        Ok(())
    }
    fn unlock(&self, object: &str, passphrase: &str) -> io::Result<String> {
        self.note(format!("unlock {} {passphrase}", short(object)));
        if passphrase == "right" {
            Ok(format!("{UDISKS}dm0"))
        } else {
            Err(io::Error::other("Failed to activate device: Operation not permitted"))
        }
    }
    fn eject(&self, drive: &str) -> io::Result<()> {
        self.note(format!("eject {drive}"));
        Ok(())
    }
    fn power_off(&self, drive: &str) -> io::Result<()> {
        self.note(format!("power off {drive}"));
        Ok(())
    }
}

fn called(calls: &Calls, what: &str) -> bool {
    calls.lock().unwrap().iter().any(|c| c == what)
}

const MIB: usize = 1024 * 1024;

/// The T7 sandbox: a file to split, a folder to join into, a file for the shelf and its
/// destination, three identical pictures, a small tree for the treemap and a folder the
/// fake device mounts at.
fn t7_fixture(data: &Path) {
    for d in ["dest", "mnt", "out", "pics", "tree/big"] {
        std::fs::create_dir_all(data.join(d)).unwrap();
    }
    let bytes: Vec<u8> = (0..3 * MIB).map(|i| (i * 31 % 251) as u8).collect();
    std::fs::write(data.join("big.bin"), bytes).unwrap();
    std::fs::write(data.join("x.txt"), b"shelf me").unwrap();
    let same = vec![7u8; 200_000];
    for n in ["a.jpg", "b.jpg", "c.jpg"] {
        std::fs::write(data.join("pics").join(n), &same).unwrap();
    }
    let sparse = |p: &Path, len: u64| std::fs::File::create(p).unwrap().set_len(len).unwrap();
    sparse(&data.join("tree").join("big").join("y.bin"), 2_000_000);
    sparse(&data.join("tree").join("small.bin"), 1_000_000);
}

/// Beats that run `action` from the palette: `open` sends Ctrl+K (a [`beat`] or a
/// [`seek`]), then the query goes in one character per frame, Enter once the action is the
/// first result.
fn palette_beats(b: &mut Vec<Beat>, open: Beat, query: &'static str, action: Action) {
    b.push(open);
    for (i, c) in query.char_indices() {
        let typed = &query[..i];
        b.push(beat(
            "typing in the palette",
            move |a| a.view().is_some_and(|v| v.palette.open && v.palette.input == typed),
            key(KeyCode::Char(c)),
        ));
    }
    b.push(beat(
        "the action ranked first in the palette",
        move |a| {
            let first = a.core.palette.results.get(a.core.palette.selected).map(|i| &i.exec);
            a.view().is_some_and(|v| v.palette.open && v.palette.input == query)
                && first == Some(&Exec::Action(action.clone()))
        },
        key(KeyCode::Enter),
    ));
}

/// The context menu key.
fn shift_f10() -> Event {
    key_mods(KeyCode::F(10), KeyModifiers::SHIFT)
}

/// `open` sends Shift+F10 (a [`beat`] or a [`seek`]), then Down until the menu's cursor
/// is on `label`, Enter.
fn menu_beats(b: &mut Vec<Beat>, open: Beat, label: &'static str) {
    b.push(open);
    b.push(until(
        "the menu entry",
        move |a| menu_cursor(a).as_deref() == Some(label),
        |a| !menu_labels(a).is_empty(),
        key(KeyCode::Down),
        key(KeyCode::Enter),
    ));
}

fn screens_shown(a: &App) -> Option<&ScreensVm> {
    a.view().map(|v| &v.screens).filter(|s| s.visible)
}

fn treemap_vm(a: &App) -> Option<&TreemapVm> {
    match &screens_shown(a)?.screen {
        Some(ScreenVm::Treemap(t)) => Some(t.as_ref()),
        _ => None,
    }
}

fn dupes_vm(a: &App) -> Option<&DupesVm> {
    match &screens_shown(a)?.screen {
        Some(ScreenVm::Dupes(d)) => Some(d.as_ref()),
        _ => None,
    }
}

/// The treemap's scan finished and it shows `names` (as tiles, or rows on narrow screens).
fn treemap_shows(a: &App, names: &[&str]) -> bool {
    treemap_vm(a).is_some_and(|t| {
        let shown = |n: &str| t.tiles.iter().any(|x| x.label == n) || t.rows.iter().any(|r| r.name == n);
        matches!(t.status, TreemapStatus::Done(_)) && names.iter().all(|n| shown(n))
    })
}

fn treemap_selected(a: &App) -> Option<String> {
    treemap_vm(a)?.tiles.iter().find(|t| t.selected).map(|t| t.label.clone())
}

fn treemap_crumbs(a: &App) -> Vec<String> {
    treemap_vm(a).map(|t| t.crumbs.iter().map(|(_, n)| n.clone()).collect()).unwrap_or_default()
}

/// The duplicate search ended with one group of three.
fn dupes_one_group(a: &App) -> bool {
    dupes_vm(a).is_some_and(|d| {
        matches!(d.status, DupesStatus::Summary(_))
            && d.lines.iter().filter(|l| matches!(l.kind, LineKind::Header { .. })).count() == 1
            && d.lines.iter().filter(|l| matches!(l.kind, LineKind::File { .. })).count() == 3
    })
}

/// The open dialog's button labelled `label`, pressed by its letter.
fn button_letter(a: &App, label: &str) -> Option<char> {
    a.view()?.dialog.as_ref()?.buttons.iter().find(|b| b.label == label && b.enabled)?.accel
}

/// The open dialog's first button (Start) is enabled: the split / join probe is done.
fn can_start(a: &App) -> bool {
    a.view().and_then(|v| v.dialog.as_ref()).and_then(|d| d.buttons.first()).is_some_and(|b| b.enabled)
}

/// The sidebar's cursor row.
fn sidebar_cursor(a: &App) -> Option<String> {
    a.view()?.sidebar.lines.iter().find(|l| l.cursor).map(|l| l.label.clone())
}

/// The sandbox's `workspaces.toml`.
fn workspaces_file(data: &Path) -> String {
    let dir = data.parent().unwrap().join(config::PORTABLE_DATA_DIR);
    std::fs::read_to_string(dir.join("workspaces.toml")).unwrap_or_default()
}

/// The settings of the T7 run: the sidebar shows the devices (the fake backend's), the
/// mouse is off (keys only), copies stay inside the app.
fn t7_config() -> Config {
    let mut c = common::test_config();
    c.layout.sidebar = true;
    c.sidebar.sections = vec![SidebarSection::Devices];
    c.sidebar.unmounted_devices = UnmountedDevices::All;
    c.tui.osc52 = TuiOsc52::Never;
    c
}

/// One run through Phase T7 with the keys, the palette and the menus a user would press:
/// save a workspace and manage it → go back in the tab history → About (copy info) → add a
/// file to the shelf from its menu and copy it into another folder from the shelf → split
/// a file and join it into a folder → previous versions (the empty state) → the treemap
/// (drill in, up, close) → duplicates (keep the newest, the trash asked for and cancelled)
/// → unlock a device through the sidebar's menu (fake UDisks2) → quit. The checks after
/// the run read what it left on disk; nothing ever reaches the trash.
#[test]
fn tui_t7_workspaces_history_about_shelf_split_versions_tools_devices() {
    let run = common::relaunch(
        module_path!(),
        "tui_t7_workspaces_history_about_shelf_split_versions_tools_devices",
        t7_fixture,
    );
    match run {
        Run::Child(data) => run_t7(data),
        Run::Parent(p) => {
            let d = &p.data;
            assert!(workspaces_file(d).contains("name = \"data\""), "saved: {}", workspaces_file(d));
            assert_eq!(std::fs::read(d.join("dest").join("x.txt")).unwrap(), b"shelf me", "copied from the shelf");
            assert!(d.join("x.txt").is_file(), "a copy, not a move");
            let src = d.join("big.bin");
            assert!(part_path(&src, 1).is_file() && checksum_path(&src).is_file(), "split");
            assert_eq!(std::fs::read(d.join("out").join("big.bin")).unwrap(), std::fs::read(&src).unwrap(), "joined");
            for n in ["a.jpg", "b.jpg", "c.jpg"] {
                assert!(d.join("pics").join(n).is_file(), "{n}: the trash was cancelled");
            }
            assert!(d.join("tree").join("big").join("y.bin").is_file() && d.join("tree").join("small.bin").is_file());
        }
    }
}

fn run_t7(data: PathBuf) {
    soyuz_appcore::services::os_clipboard::set_system_clipboard(false);
    let calls: Calls = Arc::default();
    devices::set_desk_backend(Box::new(FakeDesk { calls: calls.clone(), mount_at: data.join("mnt") }));
    let mut app = super::menu::app(t7_config(), &data);
    let (d0, d1, d2, d3, d4) = (data.clone(), data.clone(), data.clone(), data.clone(), data.clone());
    let (pics, pics2) = (data.join("pics"), data.join("pics"));
    let dest = data.join("dest");
    let (dest1, dest2) = (dest.clone(), dest.join("x.txt"));
    let src = data.join("big.bin");
    let ck = checksum_path(&src);
    let sep = std::path::MAIN_SEPARATOR;
    let mnt = data.join("mnt");
    let ws_title = || Some(tr!("workspaces-save-title").to_string());
    let manage_title = || Some(tr!("workspaces-manage-title").to_string());
    let history_title = || Some(tr!("workspaces-history-title").to_string());
    let about_title = || Some(tr!("dialogs-about-title").to_string());
    let shelf_title = || Some(tr!("tools-shelf-title").to_string());
    let saved = tr!("workspaces-saved", name = "data");
    let split_title = format!("{} — big.bin", tr!("tools-split-title"));
    let join_title = format!("{} — big.bin.001", tr!("tools-join-title"));
    let split_done = tr!("tools-split-done").to_string();
    let joined = tr!("tools-join-done").to_string();
    let versions_title = tr!("tui-versions-title", name = "x.txt");
    let elevate = tr!("tools-versions-elevate").to_string();
    let unlock_title = tr!("dialogs-unlock-title", name = "Vault");
    let copy_here = tr!("tools-shelf-copy-here").to_string();

    // Workspaces: save one (the palette's "Save workspace as…", the folder's name), then
    // "Manage workspaces…" lists it.
    let mut b = Vec::new();
    palette_beats(
        &mut b,
        beat("the listing", move |a| at(a, &d0) && labels(a).len() == 7 && a.focus() == TuiFocus::List, ctrl('k')),
        "save workspace",
        Action::SaveWorkspace,
    );
    b.push(beat(
        "the save dialog with the folder's name",
        move |a| dialog_title(a) == ws_title() && field(a, 0).as_deref() == Some("data"),
        key(KeyCode::Enter),
    ));
    palette_beats(
        &mut b,
        beat("saved", move |a| no_dialog(a) && toasts(a).iter().any(|t| t.contains(&saved)), ctrl('k')),
        "manage workspaces",
        Action::ManageWorkspaces,
    );
    b.push(beat(
        "the saved workspace listed",
        move |a| dialog_title(a) == manage_title() && rows(a).len() == 1 && rows(a)[0].starts_with("data"),
        key(KeyCode::Esc),
    ));
    // Tab history: into pics, up again, then the history goes back to pics.
    b.push(beat("the dialog closed", no_dialog, key(KeyCode::Home)));
    b.push(seek("pics/ in the list", "pics/", key(KeyCode::Enter)));
    b.push(beat("in pics", move |a| at(a, &pics), key(KeyCode::Backspace)));
    palette_beats(
        &mut b,
        beat("up in data", move |a| at(a, &d1) && no_dialog(a), ctrl('k')),
        "tab history",
        Action::TabHistory,
    );
    b.push(until(
        "the history's cursor on pics",
        |a| row_cursor(a).and_then(|i| rows(a).get(i).cloned()).is_some_and(|r| r.contains("pics")),
        move |a| dialog_title(a) == history_title() && rows(a).len() >= 2,
        key(KeyCode::Up),
        key(KeyCode::Enter),
    ));
    // About: Alt+C copies the facts (inside the app only) and closes it.
    b.push(beat("back in pics", move |a| no_dialog(a) && at(a, &pics2), key(KeyCode::Backspace)));
    palette_beats(
        &mut b,
        beat("up in data again", move |a| at(a, &d2) && no_dialog(a), ctrl('k')),
        "about soyuz",
        Action::About,
    );
    b.push(beat("About", move |a| dialog_title(a) == about_title() && !rows(a).is_empty(), alt('c')));
    // The shelf: "Add to shelf" in x.txt's menu opens it; Esc puts it away; in dest the
    // palette's "Toggle shelf" shows it again and its Copy here button copies x.txt.
    b.push(beat(
        "the facts copied",
        |a| no_dialog(a) && a.host.last_copy().is_some_and(|t| t.starts_with("Soyuz\nVersion: ")),
        key(KeyCode::Home),
    ));
    menu_beats(&mut b, seek("x.txt in the list", "x.txt", shift_f10()), tr!("tools-menu-add-to-shelf"));
    b.push(beat(
        "the shelf with x.txt",
        move |a| dialog_title(a) == shelf_title() && rows(a).len() == 1 && rows(a)[0].contains("x.txt"),
        key(KeyCode::Esc),
    ));
    b.push(beat("the shelf put away", |a| no_dialog(a) && !shelf_open(), key(KeyCode::Home)));
    b.push(seek("dest/ in the list", "dest/", key(KeyCode::Enter)));
    palette_beats(
        &mut b,
        beat("in dest", move |a| at(a, &dest1) && no_dialog(a), ctrl('k')),
        "toggle shelf",
        Action::ToggleShelf,
    );
    b.push(Beat {
        what: "the shelf again: Copy here by its letter",
        act: Box::new(move |a| {
            match button_letter(a, &copy_here).filter(|_| dialog_title(a) == shelf_title() && rows(a).len() == 1) {
                Some(c) => Act::Send(key(KeyCode::Char(c))),
                None => Act::Wait,
            }
        }),
    });
    b.push(beat("copied into dest", move |a| dest2.is_file() && wait_jobs_idle(a), key(KeyCode::Esc)));
    // Split big.bin into 1 MB parts (Custom: End, then Tab twice to its field) and join
    // the first part into out (Shift+Tab reaches the output folder).
    b.push(beat("the shelf closed", |a| no_dialog(a) && !shelf_open(), key(KeyCode::Backspace)));
    b.push(beat("up in data once more", move |a| at(a, &d3) && no_dialog(a), key(KeyCode::Home)));
    palette_beats(&mut b, seek("big.bin in the list", "big.bin", ctrl('k')), "split file", Action::SplitFile);
    let st = split_title.clone();
    b.push(beat(
        "the split dialog, the size probed",
        move |a| dialog_title(a).as_deref() == Some(st.as_str()) && can_start(a),
        key(KeyCode::End),
    ));
    b.push(beat("Custom picked", |a| row_cursor(a) == Some(5), key(KeyCode::Tab)));
    b.push(beat("tabbing", |_| true, key(KeyCode::Tab)));
    b.push(until(
        "the custom size field emptied",
        |a| field(a, 0).as_deref() == Some(""),
        |a| a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.focus == 0),
        key(KeyCode::Backspace),
        key(KeyCode::Char('1')),
    ));
    typing(&mut b, " MB");
    b.push(beat("1 MB typed", |a| field(a, 0).as_deref() == Some("1 MB"), key(KeyCode::Enter)));
    b.push(beat(
        "split",
        move |a| toasts(a).iter().any(|t| t.contains(&split_done)) && ck.is_file(),
        key(KeyCode::Enter),
    ));
    b.push(beat("the split dialog closed", no_dialog, key(KeyCode::Home)));
    palette_beats(
        &mut b,
        seek("the first part in the list", "big.bin.001", ctrl('k')),
        "join files",
        Action::JoinFiles,
    );
    let jt = join_title.clone();
    b.push(beat(
        "the join dialog, the parts found",
        move |a| dialog_title(a).as_deref() == Some(jt.as_str()) && can_start(a),
        key_mods(KeyCode::BackTab, KeyModifiers::SHIFT),
    ));
    typing(&mut b, &format!("{sep}out"));
    b.push(beat("into out", |a| field(a, 0).is_some_and(|f| f.ends_with("out")), key(KeyCode::Enter)));
    b.push(beat("joined", move |a| toasts(a).iter().any(|t| t.contains(&joined)), key(KeyCode::Esc)));
    // Previous versions of a new file: the dialog settles on its empty (or, without
    // administrator rights, its message-only) state; never a restart button.
    b.push(beat("the join dialog closed", no_dialog, key(KeyCode::Home)));
    palette_beats(
        &mut b,
        seek("x.txt in the list again", "x.txt", ctrl('k')),
        "restore previous version",
        Action::PreviousVersions,
    );
    b.push(beat(
        "the versions dialog settled, no versions, no administrator button",
        move |a| {
            a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| {
                d.title == versions_title
                    && d.body.len() >= 2
                    && d.body[1] != looking_text()
                    && d.list.is_empty()
                    && !d.buttons.iter().any(|b| b.label == elevate)
            })
        },
        key(KeyCode::Esc),
    ));
    // The treemap of tree/ from its menu ("Analyze disk usage"): the first arrow selects
    // the largest tile (big), Enter drills in, Backspace goes up, Esc closes.
    b.push(beat("the versions dialog closed", no_dialog, key(KeyCode::Home)));
    menu_beats(&mut b, seek("tree/ in the list", "tree/", shift_f10()), tr!("tools-menu-analyze-disk-usage"));
    b.extend([
        beat(
            "the treemap of tree",
            |a| a.focus() == TuiFocus::Tool && treemap_shows(a, &["big", "small.bin"]),
            key(KeyCode::Right),
        ),
        beat("the largest tile selected", |a| treemap_selected(a).as_deref() == Some("big"), key(KeyCode::Enter)),
        beat(
            "drilled into big",
            |a| treemap_crumbs(a).last().map(String::as_str) == Some("big") && treemap_shows(a, &["y.bin"]),
            key(KeyCode::Backspace),
        ),
        beat(
            "up again, big still selected",
            |a| treemap_crumbs(a).len() == 1 && treemap_selected(a).as_deref() == Some("big"),
            key(KeyCode::Esc),
        ),
        // Duplicates in pics/ from its menu: Shift+N keeps the newest of the group, Del
        // asks to trash the other two, Esc cancels (no trash job), Esc closes.
        beat("the treemap closed", |a| screens_shown(a).is_none() && a.focus() == TuiFocus::List, key(KeyCode::Home)),
    ]);
    menu_beats(&mut b, seek("pics/ in the list again", "pics/", shift_f10()), tr!("tools-menu-find-duplicates"));
    b.extend([
        beat(
            "one group of three",
            |a| a.focus() == TuiFocus::Tool && dupes_one_group(a),
            key_mods(KeyCode::Char('N'), KeyModifiers::SHIFT),
        ),
        beat("the newest kept: two marked", |a| dupes_vm(a).is_some_and(|d| d.marked == 2), key(KeyCode::Delete)),
        beat(
            "the trash asked for",
            |a| dupes_vm(a).and_then(|d| d.confirm.as_deref()).is_some_and(|q| q.contains('2')),
            key(KeyCode::Esc),
        ),
        beat(
            "cancelled, the marks kept",
            |a| dupes_vm(a).is_some_and(|d| d.confirm.is_none() && d.marked == 2),
            key(KeyCode::Esc),
        ),
        // Devices: F12 to the sidebar, End to the locked device, its menu's Unlock…, a
        // passphrase; the fake backend unlocks and mounts it.
        beat(
            "the duplicates closed, nothing trashed",
            move |a| screens_shown(a).is_none() && a.focus() == TuiFocus::List && wait_jobs_idle(a) && at(a, &d4),
            key(KeyCode::F(12)),
        ),
        beat("the sidebar has the keyboard", |a| a.key_context() == KeyContext::Sidebar, key(KeyCode::End)),
    ]);
    menu_beats(
        &mut b,
        beat("on the locked device", |a| sidebar_cursor(a).as_deref() == Some("Vault"), shift_f10()),
        "Unlock…",
    );
    let ut = unlock_title.clone();
    b.push(beat(
        "the unlock dialog",
        move |a| dialog_title(a).as_deref() == Some(ut.as_str()),
        key(KeyCode::Char('r')),
    ));
    typing(&mut b, "ight");
    b.push(beat("typed", |_| true, key(KeyCode::Enter)));
    let c = calls.clone();
    b.push(beat(
        // Linux pumps the mount: the pane opens the mounted folder.
        "unlocked and mounted",
        move |a| {
            called(&c, "unlock sdb2 right")
                && called(&c, "mount dm0")
                && dialog_title(a).is_none()
                && (!cfg!(target_os = "linux") || at(a, &mnt))
        },
        ctrl('q'),
    ));
    let mut source = Beats { beats: b.into(), since: Instant::now() };
    let mut terminal = Terminal::new(TestBackend::new(120, 34)).expect("test terminal");
    let mut io = NullIo::default();
    let end = run_loop(&mut app, &mut terminal, &mut source, &mut io).expect("the loop runs");
    assert_eq!(end, LoopEnd::Quit);
    assert!(source.beats.is_empty(), "every beat ran");
    assert_eq!(calls.lock().unwrap().first().map(String::as_str), Some("unlock sdb2 right"));
    app.finish();
    common::ok();
}

// ---- Phase T7: every new dialog and screen at tiny sizes, in each look ----

/// Widths and heights the sweep draws at (down to nothing).
const WIDTHS: [u16; 15] = [0, 1, 2, 3, 4, 5, 7, 10, 14, 20, 28, 40, 60, 90, 120];
const HEIGHTS: [u16; 13] = [0, 1, 2, 3, 4, 5, 6, 8, 10, 13, 17, 24, 34];

/// One frame of `app` on a fresh `w` × `h` test terminal.
fn frame_at(app: &mut App, w: u16, h: u16, events: &[Event]) {
    let mut t = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    app.frame(&mut t, events, &mut NullIo::default()).expect("a frame");
}

/// Frames on an 80 × 24 screen until `ready` holds (30 s at most).
fn settle(app: &mut App, what: &str, ready: &dyn Fn(&App) -> bool) {
    let start = Instant::now();
    loop {
        frame_at(app, 80, 24, &[]);
        if ready(app) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}; dialog {:?}, toasts {:?}",
            dialog_title(app),
            common::toasts(app)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Nothing is open over the panes.
fn bare(a: &App) -> bool {
    dialog_title(a).is_none() && screens_shown(a).is_none() && !a.core.dialogs.is_open()
}

type Opener = Box<dyn Fn(&mut App)>;
type Ready = Box<dyn Fn(&App) -> bool>;

/// Every T7 dialog and screen: how to open it and when it shows (its scan done).
fn t7_views(data: &Path) -> Vec<(&'static str, Opener, Ready)> {
    let has_dialog = |title: String| -> Ready { Box::new(move |a: &App| dialog_title(a) == Some(title.clone())) };
    let action = |act: Action| -> Opener {
        Box::new(move |a: &mut App| a.core.cmds.push(soyuz_appcore::cmd::Cmd::Action(act.clone())))
    };
    let (x, big, part, tree, pics) =
        (data.join("x.txt"), data.join("big.bin"), data.join("j.bin.001"), data.join("tree"), data.join("pics"));
    let gone = data.join("gone.txt");
    vec![
        ("About", action(Action::About), has_dialog(tr!("dialogs-about-title").into())),
        ("Save workspace", action(Action::SaveWorkspace), has_dialog(tr!("workspaces-save-title").into())),
        (
            "Manage workspaces",
            action(Action::ManageWorkspaces),
            Box::new(|a: &App| {
                dialog_title(a).as_deref() == Some(tr!("workspaces-manage-title")) && !rows(a).is_empty()
            }),
        ),
        ("Tab history", action(Action::TabHistory), has_dialog(tr!("workspaces-history-title").into())),
        (
            "Shelf",
            Box::new(move |_: &mut App| queue(ToolCmd::ShelfAdd(vec![x.clone(), gone.clone()]))),
            Box::new(|a: &App| dialog_title(a).as_deref() == Some(tr!("tools-shelf-title")) && rows(a).len() == 2),
        ),
        (
            "Split",
            Box::new(move |_: &mut App| queue(ToolCmd::Split(big.clone()))),
            Box::new(|a: &App| dialog_title(a).is_some_and(|t| t.ends_with("big.bin")) && can_start(a)),
        ),
        (
            "Join",
            Box::new(move |_: &mut App| queue(ToolCmd::Join(part.clone()))),
            Box::new(|a: &App| dialog_title(a).is_some_and(|t| t.ends_with("j.bin.001")) && can_start(a)),
        ),
        (
            "Unlock",
            Box::new(|a: &mut App| {
                let pane = a.core.ws.pane().id;
                a.core.frontend.push(FrontendCmd::UnlockDevice {
                    object: format!("{UDISKS}sdb2"),
                    name: "Vault 日本語 with a long name".into(),
                    kind: "BitLocker".into(),
                    pane,
                });
            }),
            Box::new(|a: &App| dialog_title(a).is_some_and(|t| t.contains("Vault"))),
        ),
        (
            "Previous versions",
            {
                let x = data.join("x.txt");
                Box::new(move |_: &mut App| queue(ToolCmd::Versions(x.clone(), false, None)))
            },
            Box::new(|a: &App| {
                a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| {
                    d.title == tr!("tui-versions-title", name = "x.txt")
                        && d.body.len() >= 2
                        && d.body[1] != looking_text()
                })
            }),
        ),
        (
            "Treemap",
            Box::new(move |_: &mut App| queue(ToolCmd::Treemap(tree.clone()))),
            Box::new(|a: &App| treemap_shows(a, &["big", "small.bin"])),
        ),
        (
            "Duplicates",
            Box::new(move |_: &mut App| queue(ToolCmd::Duplicates(vec![pics.clone()]))),
            Box::new(dupes_one_group),
        ),
    ]
}

/// Open every T7 dialog and screen in turn, draw each at every size of the sweep (no
/// panic), then close it with Esc.
fn sweep_t7(data: PathBuf, look: impl FnOnce(&mut Config)) {
    soyuz_appcore::services::os_clipboard::set_system_clipboard(false);
    let mut c = common::test_config();
    look(&mut c);
    let mut app = super::menu::app(c, &data);
    settle(&mut app, "the listing", &|a| at(a, &data) && !labels(a).is_empty());
    // A workspace to manage and a place in the tab history.
    workspaces::apply(&mut app.core, WsCmd::Save("one".into()));
    let pane = app.core.ws.pane().id;
    ops::navigate(&mut app.core, pane, Location::Dir(data.join("pics")), NavOpts::user());
    ops::navigate(&mut app.core, pane, Location::Dir(data.clone()), NavOpts::user());
    for (what, open, ready) in t7_views(&data) {
        settle(&mut app, "nothing open", &bare);
        open(&mut app);
        settle(&mut app, what, &*ready);
        for w in WIDTHS {
            for h in HEIGHTS {
                frame_at(&mut app, w, h, &[]);
            }
        }
        // Still there after the sweep (a tiny frame closes nothing), then Esc until gone.
        settle(&mut app, what, &*ready);
        for _ in 0..6 {
            if bare(&app) {
                break;
            }
            frame_at(&mut app, 80, 24, &[key(KeyCode::Esc)]);
        }
        settle(&mut app, "closed", &bare);
    }
    assert!(!shelf_open(), "closing the shelf turned it off");
    app.finish();
    common::ok();
}

fn sweep_fixture(data: &Path) {
    t7_fixture(data);
    std::fs::write(data.join("j.bin.001"), b"one").unwrap();
    std::fs::write(data.join("j.bin.002"), b"two").unwrap();
}

/// Unicode icons, rounded lines, 24-bit colour.
#[test]
fn t7_views_at_tiny_sizes_unicode() {
    match common::relaunch(module_path!(), "t7_views_at_tiny_sizes_unicode", sweep_fixture) {
        Run::Child(data) => sweep_t7(data, |c| {
            c.tui.icons = TuiIcons::Unicode;
            c.tui.borders = TuiBorders::Rounded;
            c.tui.color_mode = TuiColorMode::Truecolor;
        }),
        Run::Parent(p) => assert!(p.data.join("pics").join("a.jpg").is_file()),
    }
}

/// A pure ASCII screen: ASCII icons and lines, 16 colours.
#[test]
fn t7_views_at_tiny_sizes_ascii() {
    match common::relaunch(module_path!(), "t7_views_at_tiny_sizes_ascii", sweep_fixture) {
        Run::Child(data) => sweep_t7(data, |c| {
            c.tui.icons = TuiIcons::Ascii;
            c.tui.borders = TuiBorders::Ascii;
            c.tui.color_mode = TuiColorMode::Ansi16;
        }),
        Run::Parent(p) => assert!(p.data.join("pics").join("a.jpg").is_file()),
    }
}

/// Monochrome, Nerd Font icons, no lines.
#[test]
fn t7_views_at_tiny_sizes_mono() {
    match common::relaunch(module_path!(), "t7_views_at_tiny_sizes_mono", sweep_fixture) {
        Run::Child(data) => sweep_t7(data, |c| {
            c.tui.icons = TuiIcons::Nerd;
            c.tui.borders = TuiBorders::None;
            c.tui.color_mode = TuiColorMode::Mono;
        }),
        Run::Parent(p) => assert!(p.data.join("pics").join("a.jpg").is_file()),
    }
}

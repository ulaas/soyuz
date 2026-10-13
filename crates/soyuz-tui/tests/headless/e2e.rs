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
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::ai::state::{CardState, ChatItem};
use soyuz_appcore::editors::DocKind;
use soyuz_appcore::models::compare::RowAction;
use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::{self, Config};
use soyuz_tui::app::{App, EventSource, LoopEnd, NullIo, TuiEvent, run_loop};
use soyuz_tui::input::TuiFocus;
use soyuz_tui::suspend::{self, ChildSpec, Env};
use soyuz_tui::view::compare::{BodyVm as CompareBodyVm, TableVm as CompareTableVm};
use soyuz_tui::view::editor::BodyVm;
use soyuz_tui::view::search::{Progress, StatusVm as SearchStatusVm};

use crate::common::{self, Run, Step, alt, at, ctrl, cursor, dialog_title, key, labels, type_text, wait_jobs_idle};

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

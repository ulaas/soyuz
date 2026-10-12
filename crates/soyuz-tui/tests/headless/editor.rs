//! Headless tests of the editors (Phase T4) and of quitting with unsaved changes. Each
//! test runs in its own sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::editors::DocKind;
use soyuz_appcore::editors::text::buffer::Pos;
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::input::TuiFocus;
use soyuz_tui::view::editor::BodyVm;

use crate::common::{self, Run, Step, ctrl, key, key_mods, type_text};

/// The list shows `name` under the cursor.
fn on(name: &'static str) -> impl Fn(&App) -> bool {
    move |a| common::cursor(a).as_deref() == Some(name)
}

/// Steps that put the cursor on `name` (the only file) and press F4.
fn open(name: &'static str) -> Vec<Step> {
    vec![
        Step::new("the listing", move |a| common::labels(a) == [name], key(KeyCode::Down)),
        Step::new("the file under the cursor", on(name), key(KeyCode::F(4))),
    ]
}

/// The editor has the keyboard on a loaded text document.
fn text_ready(a: &App) -> bool {
    a.view().is_some_and(|v| v.editor.focused && matches!(v.editor.body, BodyVm::Text(_)))
}

/// The active text document's contents.
fn text(a: &App) -> String {
    match a.core.editors.active_doc().map(|d| &d.kind) {
        Some(DocKind::Text(t)) => t.buf.text(),
        _ => String::new(),
    }
}

/// The active text document's caret.
fn caret(a: &App) -> Option<Pos> {
    match a.core.editors.active_doc().map(|d| &d.kind) {
        Some(DocKind::Text(t)) => Some(t.sel.head),
        _ => None,
    }
}

fn clean(a: &App) -> bool {
    a.core.editors.dirty_count() == 0 && !a.core.editors.docs.iter().any(|d| d.saving)
}

/// Open a file with F4, type, save with Ctrl+S: the file on disk changes; Ctrl+Z / Ctrl+Y
/// undo and redo through the document model.
#[test]
fn tui_edit_type_save_undo() {
    let run = common::relaunch(module_path!(), "tui_edit_type_save_undo", |data| {
        std::fs::write(data.join("note.txt"), b"world\n").unwrap();
    });
    match run {
        Run::Child(data) => {
            let mut app = common::new_app(common::test_config(), &data);
            let mut steps = open("note.txt");
            steps.push(Step::new("the text editor has the keyboard", text_ready, key(KeyCode::Char('h'))));
            steps.push(Step::new("typed", |a| text(a) == "hworld\n", ctrl('s')));
            steps.push(Step::new("saved", clean, ctrl('z')));
            steps.push(Step::new("undone", |a| text(a) == "world\n", ctrl('y')));
            steps.push(Step::new("redone", |a| text(a) == "hworld\n", ctrl('q')));
            common::run_script(&mut app, steps, 100, 24);
            assert_eq!(std::fs::read_to_string(data.join("note.txt")).unwrap(), "hworld\n");
            common::ok();
        }
        Run::Parent(p) => {
            assert_eq!(std::fs::read_to_string(p.data.join("note.txt")).unwrap(), "hworld\n");
        }
    }
}

/// Wide characters take two cells: the caret is drawn after them, and Down keeps the
/// terminal column.
#[test]
fn tui_edit_wide_characters() {
    let run = common::relaunch(module_path!(), "tui_edit_wide_characters", |data| {
        std::fs::write(data.join("wide.txt"), "日本語x\nabcdefgh\n".as_bytes()).unwrap();
    });
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let cursor_at = |x: u16, y: u16| {
        move |a: &App| a.view().is_some_and(|v| matches!(&v.editor.body, BodyVm::Text(t) if t.cursor == Some((x, y))))
    };
    let steps = [
        open("wide.txt"),
        vec![
            Step::new("the text editor has the keyboard", text_ready, key(KeyCode::Right)),
            Step::new("after 日", cursor_at(2, 0), key(KeyCode::Right)),
            Step::new("after 本", cursor_at(4, 0), key(KeyCode::Down)),
            Step::new("the same column below", |a| caret(a) == Some(Pos::new(1, 4)), key(KeyCode::Up)),
            Step::new("back after 本", |a| caret(a) == Some(Pos::new(0, 6)), ctrl('q')),
        ],
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    common::run_script(&mut app, steps, 100, 24);
    common::ok();
}

/// Ctrl+F finds, Ctrl+R (the terminal's Replace chord) replaces, Esc closes the bar.
#[test]
fn tui_edit_find_and_replace() {
    let run = common::relaunch(module_path!(), "tui_edit_find_and_replace", |data| {
        std::fs::write(data.join("find.txt"), b"one two one\n").unwrap();
    });
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let selected = |a: &App| match a.core.editors.active_doc().map(|d| &d.kind) {
        Some(DocKind::Text(t)) => Some(t.sel.range()),
        _ => None,
    };
    let bars = |a: &App| match a.view().map(|v| &v.editor.body) {
        Some(BodyVm::Text(t)) => t.bars.len(),
        _ => 0,
    };
    let mut steps = [
        open("find.txt"),
        vec![
            Step::new("the text editor has the keyboard", text_ready, ctrl('f')),
            Step::new("the find bar", move |a| bars(a) == 1, key(KeyCode::Char('o'))),
        ],
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    type_text(&mut steps, "ne");
    steps.push(Step::new(
        "the first match",
        move |a| selected(a) == Some((Pos::new(0, 0), Pos::new(0, 3))),
        key(KeyCode::Enter),
    ));
    steps.push(Step::new("the next match", move |a| selected(a) == Some((Pos::new(0, 8), Pos::new(0, 11))), ctrl('r')));
    steps.push(Step::new("the replace bar", move |a| bars(a) == 2, key(KeyCode::Tab)));
    steps.push(Step::new("in the replace field", |_| true, key(KeyCode::Char('1'))));
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
    steps.push(Step::new("replaced", |a| text(a) == "one two 1\n", key(KeyCode::Esc)));
    steps.push(Step::new("the bar closed", move |a| bars(a) == 0 && text_ready(a), ctrl('q')));
    // Unsaved: the quit asks; Don't save.
    steps.push(Step::new("asked to save", |a| common::dialog_title(a).is_some(), key(KeyCode::Right)));
    steps.push(Step::new("on Don't save", |_| true, key(KeyCode::Enter)));
    common::run_script(&mut app, steps, 100, 24);
    assert_eq!(std::fs::read_to_string(data.join("find.txt")).unwrap(), "one two one\n", "not saved");
    common::ok();
}

/// The hex editor: two digits overwrite a byte, Ctrl+S writes it.
#[test]
fn tui_edit_hex_byte() {
    let run = common::relaunch(module_path!(), "tui_edit_hex_byte", |data| {
        std::fs::write(data.join("blob.bin"), [0u8, 1, 2, 3, 0, 0, 0xFF, 7]).unwrap();
    });
    match run {
        Run::Child(data) => {
            let mut app = common::new_app(common::test_config(), &data);
            let loaded = |a: &App| {
                a.view().is_some_and(|v| {
                    v.editor.focused
                        && matches!(&v.editor.body, BodyVm::Hex(h) if h.rows.first().is_some_and(|r| r.bytes[0].value.is_some()))
                })
            };
            let byte0 = |a: &App| match a.core.editors.active_doc().map(|d| &d.kind) {
                Some(DocKind::Hex(h)) => h.byte(0),
                _ => None,
            };
            let steps = [
                open("blob.bin"),
                vec![
                    Step::new("the hex editor has the keyboard", loaded, key(KeyCode::Char('a'))),
                    Step::new("the high digit", move |a| byte0(a) == Some(0xA0), key(KeyCode::Char('b'))),
                    Step::new("the byte", move |a| byte0(a) == Some(0xAB), ctrl('s')),
                    Step::new("saved", clean, ctrl('q')),
                ],
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            common::run_script(&mut app, steps, 100, 24);
            common::ok();
        }
        Run::Parent(p) => {
            assert_eq!(std::fs::read(p.data.join("blob.bin")).unwrap(), vec![0xAB, 1, 2, 3, 0, 0, 0xFF, 7]);
        }
    }
}

/// Ctrl+W on a document with changes asks first; Don't save closes it and keeps the file.
#[test]
fn tui_close_dirty_tab_discards() {
    let run = common::relaunch(module_path!(), "tui_close_dirty_tab_discards", |data| {
        std::fs::write(data.join("note.txt"), b"keep\n").unwrap();
    });
    match run {
        Run::Child(data) => {
            let mut app = common::new_app(common::test_config(), &data);
            let steps = [
                open("note.txt"),
                vec![
                    Step::new("the text editor has the keyboard", text_ready, key(KeyCode::Char('x'))),
                    Step::new("typed", |a| a.core.editors.dirty_count() == 1, ctrl('w')),
                    Step::new(
                        "asked to save",
                        |a| common::dialog_title(a).is_some_and(|t| t.contains("note.txt")),
                        key(KeyCode::Right),
                    ),
                    Step::new("on Don't save", |_| true, key(KeyCode::Enter)),
                    Step::new("closed", |a| a.core.editors.docs.is_empty() && !a.core.editors.open, ctrl('q')),
                ],
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            common::run_script(&mut app, steps, 100, 24);
            common::ok();
        }
        Run::Parent(p) => {
            assert_eq!(std::fs::read_to_string(p.data.join("note.txt")).unwrap(), "keep\n");
        }
    }
}

/// Quitting with unsaved changes asks; Save all (Enter) saves, then the app quits.
#[test]
fn tui_quit_saves_a_dirty_document() {
    let run = common::relaunch(module_path!(), "tui_quit_saves_a_dirty_document", |data| {
        std::fs::write(data.join("note.txt"), b"world\n").unwrap();
    });
    match run {
        Run::Child(data) => {
            let mut app = common::new_app(common::test_config(), &data);
            let steps = [
                open("note.txt"),
                vec![
                    Step::new("the text editor has the keyboard", text_ready, key(KeyCode::Char('!'))),
                    Step::new("typed", |a| a.core.editors.dirty_count() == 1, ctrl('q')),
                    Step::new("asked to save", |a| common::dialog_title(a).is_some(), key(KeyCode::Enter)),
                ],
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            common::run_script(&mut app, steps, 100, 24);
            assert_eq!(app.core.editors.dirty_count(), 0, "saved before quitting");
            common::ok();
        }
        Run::Parent(p) => {
            let text = std::fs::read_to_string(p.data.join("note.txt")).unwrap();
            assert_eq!(text, "!world\n", "the edit was saved");
        }
    }
}

/// A Windows console paste (a burst of key presses in one batch, Enter included) is one
/// insert: the line break is text, not a command, and one Ctrl+Z takes it back.
#[test]
fn tui_edit_paste_burst() {
    let run = common::relaunch(module_path!(), "tui_edit_paste_burst", |data| {
        std::fs::write(data.join("note.txt"), b"end\n").unwrap();
    });
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let mut io = NullIo::default();
    let mut frame = |app: &mut App, events: &[Event]| app.frame(&mut term, events, &mut io).unwrap();
    let wait = |app: &mut App, frame: &mut dyn FnMut(&mut App, &[Event]), what: &str, f: &dyn Fn(&App) -> bool| {
        let start = Instant::now();
        while !f(app) {
            assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            frame(app, &[]);
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    wait(&mut app, &mut frame, "the listing", &|a: &App| common::labels(a) == ["note.txt"]);
    frame(&mut app, &[key(KeyCode::Down)]);
    wait(&mut app, &mut frame, "the file under the cursor", &on("note.txt"));
    frame(&mut app, &[key(KeyCode::F(4))]);
    wait(&mut app, &mut frame, "the text editor", &text_ready);
    let press = |c: KeyCode| Event::Key(KeyEvent::new(c, KeyModifiers::NONE));
    let burst = vec![
        press(KeyCode::Char('a')),
        press(KeyCode::Char('b')),
        press(KeyCode::Enter),
        key_mods(KeyCode::Char('C'), KeyModifiers::SHIFT),
        press(KeyCode::Char('d')),
        press(KeyCode::Enter),
    ];
    frame(&mut app, &burst);
    assert_eq!(text(&app), "ab\nCd\nend\n", "one insert");
    assert!(common::dialog_title(&app).is_none());
    frame(&mut app, &[ctrl('z')]);
    assert_eq!(text(&app), "end\n", "one undo step");
    // A short run is typing (Enter keeps the line's indentation, as a command).
    frame(&mut app, &[press(KeyCode::Char('x'))]);
    assert_eq!(text(&app), "xend\n");
    common::ok();
}

/// Ctrl+G goes to a line; Alt+M maximizes the editor over the panes and back.
#[test]
fn tui_edit_goto_and_maximize() {
    let run = common::relaunch(module_path!(), "tui_edit_goto_and_maximize", |data| {
        std::fs::write(data.join("lines.txt"), b"a\nb\nc\nd\n").unwrap();
    });
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let steps = [
        open("lines.txt"),
        vec![
            Step::new("the text editor has the keyboard", text_ready, ctrl('g')),
            Step::new(
                "the go-to bar",
                |a| matches!(a.view().map(|v| &v.editor.body), Some(BodyVm::Text(t)) if t.bars.len() == 1),
                key(KeyCode::Char('3')),
            ),
            Step::new("typed", |_| true, key(KeyCode::Enter)),
            Step::new("on line 3", |a| caret(a) == Some(Pos::new(2, 0)), common::alt('m')),
            Step::new("maximized", |a| a.core.editors.maximized && a.regions().editor.width == 100, common::alt('m')),
            Step::new("restored", |a| !a.core.editors.maximized && a.regions().editor.width < 100, ctrl('q')),
        ],
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    common::run_script(&mut app, steps, 100, 24);
    common::ok();
}

/// Alt+Shift+S saves a copy under a new name; the copy replaces the document, the
/// original stays as it was.
#[test]
fn tui_edit_save_as() {
    let run = common::relaunch(module_path!(), "tui_edit_save_as", |data| {
        std::fs::write(data.join("note.txt"), b"old\n").unwrap();
    });
    match run {
        Run::Child(data) => {
            let mut app = common::new_app(common::test_config(), &data);
            let mut steps = open("note.txt");
            steps.push(Step::new("the text editor has the keyboard", text_ready, key(KeyCode::Char('x'))));
            steps.push(Step::new(
                "typed",
                |a| text(a) == "xold\n",
                key_mods(KeyCode::Char('S'), KeyModifiers::ALT | KeyModifiers::SHIFT),
            ));
            // The field holds the document's path: replace the name.
            for _ in 0.."note.txt".len() {
                steps.push(Step::new("the save-as bar", |_| true, key(KeyCode::Backspace)));
            }
            type_text(&mut steps, "copy.txt");
            steps.push(Step::new("named", |_| true, key(KeyCode::Enter)));
            steps.push(Step::new(
                "the copy is open",
                |a| a.core.editors.active_doc().is_some_and(|d| d.name == "copy.txt" && !d.is_dirty()),
                ctrl('q'),
            ));
            common::run_script(&mut app, steps, 100, 24);
            common::ok();
        }
        Run::Parent(p) => {
            assert_eq!(std::fs::read_to_string(p.data.join("copy.txt")).unwrap(), "xold\n");
            assert_eq!(std::fs::read_to_string(p.data.join("note.txt")).unwrap(), "old\n");
        }
    }
}

/// A click in the text places the caret (by terminal cell: after a wide character), a
/// click on the panel's hide button closes the panel.
#[test]
fn tui_edit_mouse() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use soyuz_tui::layout::HitTarget;
    let run = common::relaunch(module_path!(), "tui_edit_mouse", |data| {
        std::fs::write(data.join("note.txt"), "abc\n日本x\n".as_bytes()).unwrap();
    });
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let mut io = NullIo::default();
    let mut frame = |app: &mut App, events: &[Event]| app.frame(&mut term, events, &mut io).unwrap();
    let wait = |app: &mut App, frame: &mut dyn FnMut(&mut App, &[Event]), what: &str, f: &dyn Fn(&App) -> bool| {
        let start = Instant::now();
        while !f(app) {
            assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            frame(app, &[]);
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    wait(&mut app, &mut frame, "the listing", &|a: &App| common::labels(a) == ["note.txt"]);
    frame(&mut app, &[key(KeyCode::Down)]);
    wait(&mut app, &mut frame, "the file under the cursor", &on("note.txt"));
    frame(&mut app, &[key(KeyCode::F(4))]);
    wait(&mut app, &mut frame, "the text editor", &text_ready);
    let target = |app: &App, want: &dyn Fn(&HitTarget) -> bool| {
        app.hits().iter().find(|(_, t)| want(t)).map(|(r, _)| *r).expect("target drawn")
    };
    let press = |x: u16, y: u16| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        })
    };
    let row = target(&app, &|t| matches!(t, HitTarget::EditorCell { top: 1, .. }));
    // Cell 3 is the right half of 本: the caret goes before it.
    frame(&mut app, &[press(row.x + 3, row.y)]);
    assert_eq!(caret(&app), Some(Pos::new(1, 3)));
    frame(&mut app, &[]);
    frame(&mut app, &[press(row.x + 4, row.y)]);
    assert_eq!(caret(&app), Some(Pos::new(1, 6)), "after 本");
    let hide = target(&app, &|t| matches!(t, HitTarget::EditorClose(i) if *i == usize::MAX));
    frame(&mut app, &[press(hide.x, hide.y)]);
    assert!(!app.core.editors.open, "hidden");
    assert_eq!(app.core.editors.docs.len(), 1, "the document stays open");
    common::ok();
}

/// `[keys.bindings] editor_replace` moves Replace: Alt+H opens the replace bar, Ctrl+R (the
/// terminal UI's default) no longer does.
#[test]
fn tui_edit_rebind_replace() {
    let run = common::relaunch(module_path!(), "tui_edit_rebind_replace", |data| {
        std::fs::write(data.join("find.txt"), b"one two\n").unwrap();
    });
    let Run::Child(data) = run else { return };
    let mut c = common::test_config();
    c.keys.bindings.insert("editor_replace".into(), vec!["Alt+H".into()]);
    let mut app = common::new_app(c, &data);
    let replace_open = |a: &App| match a.core.editors.active_doc().map(|d| &d.kind) {
        Some(DocKind::Text(t)) => t.find.open && t.find.replace,
        _ => false,
    };
    let steps = [
        open("find.txt"),
        vec![
            Step::new("the text editor has the keyboard", text_ready, ctrl('r')),
            Step::new("Ctrl+R is not Replace any more", move |a| !replace_open(a) && text_ready(a), common::alt('h')),
            Step::new("Alt+H opens the replace bar", replace_open, key(KeyCode::Esc)),
            Step::new("closed", move |a| !replace_open(a), ctrl('q')),
        ],
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    common::run_script(&mut app, steps, 100, 24);
    common::ok();
}

/// A small PNG in the image editor: R rotates, the resize command (Alt+S) scales it by
/// numbers, C crops by numbers, Ctrl+S writes it; the file on disk has the new pixel
/// size.
#[test]
fn tui_edit_image_rotate_resize_save() {
    let run = common::relaunch(module_path!(), "tui_edit_image_rotate_resize_save", |data| {
        let img = image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8 * 80, y as u8 * 120, 50, 255]));
        img.save(data.join("pic.png")).unwrap();
    });
    let size = |a: &App| match a.core.editors.active_doc().map(|d| &d.kind) {
        Some(DocKind::Image(i)) => Some(i.size()),
        _ => None,
    };
    match run {
        Run::Child(data) => {
            let mut app = common::new_app(common::test_config(), &data);
            let ready =
                |a: &App| a.view().is_some_and(|v| v.editor.focused && matches!(v.editor.body, BodyVm::Image(_)));
            let bars = |n: usize| move |a: &App| matches!(a.view().map(|v| &v.editor.body), Some(BodyVm::Image(i)) if i.bars.len() == n);
            let mut steps = open("pic.png");
            steps.push(Step::new("the image editor has the keyboard", ready, key(KeyCode::Char('r'))));
            steps.push(Step::new("rotated", move |a| size(a) == Some((2, 3)), common::alt('s')));
            // The width field holds "2": make it 4 (the empty height keeps the aspect).
            steps.push(Step::new("the resize bar", bars(2), key(KeyCode::Backspace)));
            type_text(&mut steps, "4");
            steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
            steps.push(Step::new("resized", move |a| size(a) == Some((4, 6)), key(KeyCode::Char('c'))));
            // The crop bar: x, y, width (focused, "4"), height.
            steps.push(Step::new("the crop bar", bars(4), key(KeyCode::Backspace)));
            type_text(&mut steps, "3");
            steps.push(Step::new("typed", |_| true, key(KeyCode::Down)));
            steps.push(Step::new("the height", |_| true, key(KeyCode::Backspace)));
            type_text(&mut steps, "5");
            steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
            steps.push(Step::new("cropped", move |a| size(a) == Some((3, 5)) && bars(0)(a), ctrl('s')));
            steps.push(Step::new("saved", clean, ctrl('q')));
            common::run_script(&mut app, steps, 100, 24);
            assert_eq!(image::image_dimensions(data.join("pic.png")).unwrap(), (3, 5));
            common::ok();
        }
        Run::Parent(p) => {
            assert_eq!(image::image_dimensions(p.data.join("pic.png")).unwrap(), (3, 5), "rotated, resized, cropped");
        }
    }
}

/// The file changes on disk under unsaved edits: the banner shows, the notice key
/// (Alt+E) reloads the file from disk.
#[test]
fn tui_edit_external_change_reload() {
    let run = common::relaunch(module_path!(), "tui_edit_external_change_reload", |data| {
        std::fs::write(data.join("note.txt"), b"old\n").unwrap();
    });
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let mut io = NullIo::default();
    let mut frame = |app: &mut App, events: &[Event]| app.frame(&mut term, events, &mut io).unwrap();
    let banner = |a: &App| a.view().is_some_and(|v| v.editor.banner.is_some());
    let wait = |app: &mut App, frame: &mut dyn FnMut(&mut App, &[Event]), what: &str, f: &dyn Fn(&App) -> bool| {
        let start = Instant::now();
        while !f(app) {
            assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            frame(app, &[]);
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    wait(&mut app, &mut frame, "the listing", &|a: &App| common::labels(a) == ["note.txt"]);
    frame(&mut app, &[key(KeyCode::Down)]);
    wait(&mut app, &mut frame, "the file under the cursor", &on("note.txt"));
    frame(&mut app, &[key(KeyCode::F(4))]);
    wait(&mut app, &mut frame, "the text editor", &text_ready);
    frame(&mut app, &[key(KeyCode::Char('x'))]);
    assert_eq!(text(&app), "xold\n");
    // Change the file until the watcher (registered on a worker) reports it.
    let start = Instant::now();
    let mut n = 0;
    while !banner(&app) {
        assert!(start.elapsed() < Duration::from_secs(30), "no banner");
        if start.elapsed() >= Duration::from_millis(700 * n) {
            n += 1;
            std::fs::write(data.join("note.txt"), format!("new{}\n", "!".repeat(n as usize))).unwrap();
        }
        frame(&mut app, &[]);
        std::thread::sleep(Duration::from_millis(10));
    }
    let text_of = |a: &App| a.view().and_then(|v| v.editor.banner.as_ref()).map(|b| b.text.clone()).unwrap_or_default();
    assert!(text_of(&app).contains("changed on disk"), "{}", text_of(&app));
    frame(&mut app, &[common::alt('e')]);
    let disk = std::fs::read_to_string(data.join("note.txt")).unwrap();
    wait(&mut app, &mut frame, "the reload", &|a: &App| {
        text(a) == disk && !a.core.editors.active_doc().unwrap().is_dirty()
    });
    assert!(!banner(&app));
    common::ok();
}

/// Alt+J moves the keyboard from the editor to the jobs panel; Alt+E (`FocusEditor`)
/// brings it back, also after Esc left the editor.
#[test]
fn tui_edit_focus_keys() {
    let run = common::relaunch(module_path!(), "tui_edit_focus_keys", |data| {
        std::fs::write(data.join("note.txt"), b"abc\n").unwrap();
    });
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let steps = [
        open("note.txt"),
        vec![
            Step::new("the text editor has the keyboard", text_ready, common::alt('j')),
            Step::new("the jobs panel has it", |a| a.focus() == TuiFocus::Jobs, common::alt('e')),
            Step::new("back in the editor", |a| a.focus() == TuiFocus::Editor && text_ready(a), key(KeyCode::Esc)),
            Step::new("Esc left the editor", |a| a.focus() == TuiFocus::List, common::alt('e')),
            Step::new("back again", |a| a.focus() == TuiFocus::Editor, key(KeyCode::Char('z'))),
            Step::new("typing goes to the document", |a| text(a) == "zabc\n", ctrl('w')),
            Step::new("asked to save", |a| common::dialog_title(a).is_some(), key(KeyCode::Right)),
            Step::new("on Don't save", |_| true, key(KeyCode::Enter)),
            Step::new("closed", |a| a.core.editors.docs.is_empty(), ctrl('q')),
        ],
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    common::run_script(&mut app, steps, 100, 24);
    common::ok();
}

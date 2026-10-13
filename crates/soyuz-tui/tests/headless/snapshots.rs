//! Phase T8 (P2): tier-B app snapshots: the real frame (`App::frame`) on a fixture with
//! fixed names, sizes and dates, compared with `tests/snapshots/app__*.snap` (bless with
//! `SOYUZ_BLESS_SNAPSHOTS=1`). What would differ between machines is pinned or masked:
//! - `general.date_format = "%Y-%m-%d"` and every mtime at 12:00 UTC on one day (the same
//!   date in every time zone ±11 h), `general.language = "en-US"`;
//! - `[tui] color_mode = "ansi16"` (the 16-colour tables ignore the OS accent),
//!   `appearance.theme = "dark"`, `icons = "unicode"`, `images = "off"`, sidebar off;
//! - the crumbs row, the pane titles (the sandbox path differs per OS and run) and the
//!   status row (free space) are blanked (`SnapOpts::mask` from `App::regions`).
//!
//! No jobs history (durations, speeds) and no settings editor (OS-specific defaults): the
//! tier-A view snapshots cover both.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use soyuz_core::DateFormat;
use soyuz_core::config::{Config, TuiColorMode, TuiIcons, TuiImages};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::snapshot::SnapOpts;

use crate::common::{self, Run, ctrl, dialog_title, key, key_mods};

const W: u16 = 100;
const H: u16 = 30;

/// 2026-10-01 12:00:00 UTC.
const NOON: u64 = 1_790_856_000;

fn fixture(data: &Path) {
    for dir in ["docs", "photos", "src"] {
        std::fs::create_dir_all(data.join(dir)).unwrap();
        std::fs::write(data.join(dir).join("keep.txt"), b"x").unwrap();
    }
    let files: [(&str, &[u8]); 5] = [
        ("README.md", b"# Soyuz\n\nA keyboard-first file explorer.\n"),
        ("main.rs", b"fn main() {\n    println!(\"hello\");\n}\n"),
        ("notes.txt", b"Shopping list\n\n- coffee\n- bread\n- tea\n"),
        ("report.pdf", &[b'%'; 2048]),
        ("todo.txt", b"- write the snapshot tests\n"),
    ];
    for (name, bytes) in files {
        std::fs::write(data.join(name), bytes).unwrap();
    }
    // Times last: writing into a folder changes its mtime.
    let noon = SystemTime::UNIX_EPOCH + Duration::from_secs(NOON);
    for e in std::fs::read_dir(data).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            for inner in std::fs::read_dir(&p).unwrap() {
                set_mtime(&inner.unwrap().path(), noon);
            }
        }
        set_mtime(&p, noon);
    }
}

/// Set the mtime of a file or folder (folders need backup semantics on Windows).
fn set_mtime(path: &Path, t: SystemTime) {
    let mut o = std::fs::OpenOptions::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_WRITE_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS (opens folders too).
        o.access_mode(0x0100).custom_flags(0x0200_0000);
    }
    #[cfg(not(windows))]
    {
        o.read(true);
    }
    let f = o.open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    f.set_modified(t).unwrap_or_else(|e| panic!("set the mtime of {}: {e}", path.display()));
}

/// The pinned settings (see the module docs).
fn config() -> Config {
    let mut c = common::test_config();
    c.general.language = "en-US".into();
    c.general.date_format = DateFormat::Custom("%Y-%m-%d".into());
    c.appearance.theme = "dark".into();
    c.tui.color_mode = TuiColorMode::Ansi16;
    c.tui.icons = TuiIcons::Unicode;
    c.tui.images = TuiImages::Off;
    c
}

/// The app on a test terminal, one frame at a time.
struct Driver {
    app: App,
    term: Terminal<TestBackend>,
    io: NullIo,
}

impl Driver {
    fn new(data: &Path) -> Driver {
        let app = common::new_app(config(), data);
        let term = Terminal::new(TestBackend::new(W, H)).expect("test terminal");
        Driver { app, term, io: NullIo::default() }
    }

    fn frame(&mut self, events: &[Event]) {
        self.app.frame(&mut self.term, events, &mut self.io).expect("frame");
    }

    fn send(&mut self, e: Event) {
        self.frame(&[e]);
    }

    /// Frames until `cond` holds, then until the screen stops changing (background
    /// loads, the preview).
    fn until(&mut self, what: &str, cond: impl Fn(&App) -> bool) {
        let start = Instant::now();
        loop {
            self.frame(&[]);
            if cond(&self.app) {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut prev = self.term.backend().buffer().clone();
        let mut same = 0;
        while same < 5 {
            assert!(start.elapsed() < Duration::from_secs(60), "the screen of {what} never settles");
            std::thread::sleep(Duration::from_millis(20));
            self.frame(&[]);
            let now = self.term.backend().buffer();
            if *now == prev {
                same += 1;
            } else {
                same = 0;
                prev = now.clone();
            }
        }
        assert!(cond(&self.app), "{what} no longer holds after settling");
    }

    /// Compare the screen with `app__<name>`, the volatile rows blanked.
    fn shot(&self, name: &str) {
        self.shot_masked(name, Vec::new());
    }

    /// [`Self::shot`] with `extra` rects blanked too.
    fn shot_masked(&self, name: &str, extra: Vec<Rect>) {
        let r = self.app.regions();
        let mut mask: Vec<Rect> = vec![r.crumbs, r.status];
        mask.extend(extra);
        mask.extend(r.titles.iter().map(|t| t.rect));
        common::assert_screen(&format!("app__{name}"), &self.term, &SnapOpts::styled().masked(mask));
    }
}

/// The palette's list rows after its first result (under the section header), up to the
/// box's bottom line.
fn palette_tail(term: &Terminal<TestBackend>) -> Vec<Rect> {
    let rows = soyuz_tui::snapshot::lines(term.backend().buffer());
    let Some(first) = rows.iter().position(|l| l.contains("New folder")) else { panic!("no New folder: {rows:#?}") };
    let x = rows[first].find('│').map(|b| rows[first][..b].chars().count() as u16).unwrap_or(0);
    let bottom = (first + 1..rows.len()).find(|&y| rows[y].contains('╰')).unwrap_or(rows.len());
    let w = rows[first].chars().count() as u16 - x;
    (first + 1..bottom).map(|y| Rect::new(x + 1, y as u16, w.saturating_sub(2), 1)).collect()
}

fn labels_ready(a: &App) -> bool {
    common::labels(a).len() == 8
}

fn no_overlay(a: &App) -> bool {
    dialog_title(a).is_none()
        && a.view().is_some_and(|v| v.menu.menu.is_none() && !v.palette.open && !v.quicklook.visible)
}

#[test]
fn tui_app_snapshots() {
    match common::relaunch(module_path!(), "tui_app_snapshots", fixture) {
        Run::Child(data) => {
            run_shots(&data);
            common::ok();
        }
        Run::Parent(_) => {}
    }
}

fn run_shots(data: &Path) {
    let mut d = Driver::new(data);
    d.until("the listing", labels_ready);
    d.shot("start");

    // Down to notes.txt (the first Down places the cursor).
    for _ in 0..8 {
        if common::cursor(&d.app).as_deref() == Some("notes.txt") {
            break;
        }
        d.send(key(KeyCode::Down));
    }
    d.until("the cursor on notes.txt", |a| common::cursor(a).as_deref() == Some("notes.txt"));

    // Quick Look (Space).
    d.send(key(KeyCode::Char(' ')));
    d.until("Quick Look on notes.txt", |a| {
        a.view().is_some_and(|v| {
            v.quicklook.visible && v.quicklook.preview.body_text().iter().any(|l| l.contains("coffee"))
        })
    });
    d.shot("quicklook");
    d.send(key(KeyCode::Esc));
    d.until("Quick Look closed", no_overlay);

    // The context menu (Shift+F10).
    d.send(key_mods(KeyCode::F(10), KeyModifiers::SHIFT));
    d.until("the context menu", |a| a.view().is_some_and(|v| v.menu.menu.is_some()));
    d.shot("menu");
    d.send(key(KeyCode::Esc));
    d.until("the menu closed", no_overlay);

    // The permanent delete prompt (Shift+Del), cancelled.
    d.send(key_mods(KeyCode::Delete, KeyModifiers::SHIFT));
    d.until("the delete prompt", |a| dialog_title(a).is_some());
    d.shot("delete_prompt");
    d.send(key(KeyCode::Esc));
    d.until("the prompt closed", no_overlay);
    assert!(data.join("notes.txt").is_file(), "nothing was deleted");

    // The command palette (Ctrl+K) with a query.
    d.send(ctrl('k'));
    d.until("the palette", |a| a.view().is_some_and(|v| v.palette.open));
    for c in "new folder".chars() {
        d.send(key(KeyCode::Char(c)));
    }
    d.until("the palette's results", |a| a.view().is_some_and(|v| v.palette.input == "new folder"));
    // The first result is pinned; the fuzzy matches after it differ per OS (Windows has
    // "New junction…"), so the list rows below it are blanked.
    d.shot_masked("palette", palette_tail(&d.term));
    for _ in 0..3 {
        if no_overlay(&d.app) {
            break;
        }
        // The first Esc may only clear the query.
        d.send(key(KeyCode::Esc));
    }
    d.until("the palette closed", no_overlay);

    // Dual panes (F9).
    d.send(key(KeyCode::F(9)));
    d.until("dual panes", |a| a.regions().panes.len() == 2 && a.view().is_some_and(|v| v.panes.len() == 2));
    d.shot("dual");
}

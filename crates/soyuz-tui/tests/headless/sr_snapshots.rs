//! Phase T8 (P6): tier-B screen-reader snapshots: the real frame (`App::frame`) with
//! `[tui] screen_reader = true` on a fixture with fixed names, sizes and dates, compared
//! with `tests/snapshots/sr__*.snap` (bless with `SOYUZ_BLESS_SNAPSHOTS=1`). The same pins
//! and masks as the default-look app snapshots (`headless::snapshots`): `general.date_format
//! = "%Y-%m-%d"` with every mtime at 12:00 UTC, en-US, `color_mode = "ansi16"`, the dark
//! theme, `images = "off"`; the crumbs row, the status row (free space) and the "Other
//! pane" line (sandbox paths) are blanked. The tier-A `view__*__sr` snapshots cover those
//! rows with fixed text.
//!
//! The terminal's cursor is not in the buffer: each shot checks it is on the focused
//! element's row.

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

use crate::common::{self, Run, dialog_title, key, key_mods};

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

/// The pinned settings, screen-reader mode on.
fn config() -> Config {
    let mut c = common::test_config();
    c.general.language = "en-US".into();
    c.general.date_format = DateFormat::Custom("%Y-%m-%d".into());
    c.appearance.theme = "dark".into();
    c.tui.color_mode = TuiColorMode::Ansi16;
    c.tui.icons = TuiIcons::Unicode;
    c.tui.images = TuiImages::Off;
    c.tui.screen_reader = true;
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
        assert!(app.screen_reader());
        let term = Terminal::new(TestBackend::new(W, H)).expect("test terminal");
        Driver { app, term, io: NullIo::default() }
    }

    fn frame(&mut self, events: &[Event]) {
        self.app.frame(&mut self.term, events, &mut self.io).expect("frame");
    }

    fn send(&mut self, e: Event) {
        self.frame(&[e]);
    }

    /// Frames until `cond` holds, then until the screen stops changing.
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

    /// The screen's rows as text.
    fn rows(&self) -> Vec<String> {
        soyuz_tui::snapshot::lines(self.term.backend().buffer())
    }

    /// Compare the screen with `sr__<name>` (the volatile rows blanked); the terminal's
    /// cursor is shown on a row that contains `want`.
    fn shot(&mut self, name: &str, want: &str) {
        let r = self.app.regions();
        let mask: Vec<Rect> = vec![r.crumbs, r.status, r.other_pane];
        common::assert_screen(&format!("sr__{name}"), &self.term, &SnapOpts::styled().masked(mask));
        let rows = self.rows();
        assert!(!rows.concat().chars().any(soyuz_tui::a11y::is_box_char), "{name}: box characters\n{rows:#?}");
        let b = self.term.backend_mut();
        assert!(b.cursor_visible(), "{name}: the cursor is hidden");
        let at = b.cursor_position();
        let row = &rows[usize::from(at.y)];
        assert!(row.contains(want), "{name}: the cursor is on row {} ({row:?}), not on {want:?}", at.y);
    }

    /// The status row's text.
    fn status(&self) -> String {
        self.rows()[usize::from(self.app.regions().status.y)].trim_end().to_string()
    }
}

fn labels_ready(a: &App) -> bool {
    common::labels(a).len() == 8
}

fn no_overlay(a: &App) -> bool {
    dialog_title(a).is_none() && a.view().is_some_and(|v| v.menu.menu.is_none() && !v.palette.open)
}

#[test]
fn tui_screen_reader_snapshots() {
    match common::relaunch(module_path!(), "tui_screen_reader_snapshots", fixture) {
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

    // Down to notes.txt (the first Down places the cursor).
    for _ in 0..8 {
        if common::cursor(&d.app).as_deref() == Some("notes.txt") {
            break;
        }
        d.send(key(KeyCode::Down));
    }
    d.until("the cursor on notes.txt", |a| common::cursor(a).as_deref() == Some("notes.txt"));
    d.shot("browse", "notes.txt");
    // The status sentence starts with the focused item (the rest has the free space).
    let status = d.status();
    assert!(status.starts_with("notes.txt, file, "), "{status:?}");
    assert!(status.contains(" of 8") && status.contains("; 8 items"), "{status:?}");

    // The permanent delete prompt (Shift+Del), read top to bottom, then cancelled.
    d.send(key_mods(KeyCode::Delete, KeyModifiers::SHIFT));
    d.until("the delete prompt", |a| dialog_title(a).is_some());
    d.shot("delete_prompt", "[ ");
    d.send(key(KeyCode::Esc));
    d.until("the prompt closed", no_overlay);
    assert!(data.join("notes.txt").is_file(), "nothing was deleted");

    // Dual panes (F9): one pane drawn, the other named on its own line.
    d.send(key(KeyCode::F(9)));
    d.until("dual panes", |a| a.core.ws.tab().shown() == 2 && a.view().is_some_and(|v| v.other_pane.is_some()));
    assert_eq!(d.app.regions().panes.len(), 1, "only the focused pane is drawn");
    let other = d.rows()[usize::from(d.app.regions().other_pane.y)].clone();
    assert!(other.starts_with("Other pane: "), "{other:?}");
    d.shot("dual", "notes.txt");
}

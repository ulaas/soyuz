//! Headless tests of T5: the search bar (W3-search-bar). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! The search index is off in the test config, so every search walks the sandbox. The
//! tests drive `App::frame` directly ([`Drive`]) so a click can use the coordinates of the
//! frame before it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::services::search::KindFilter;
use soyuz_core::config::{Config, KeyContext, SidebarSection};
use soyuz_core::{Location, SearchId};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::hits::SearchButton;
use soyuz_tui::input::TuiFocus;
use soyuz_tui::input::search::{pill, status_button};
use soyuz_tui::layout::HitTarget;
use soyuz_tui::view::search::{BarVm, Progress, SearchVm, StatusVm};

use crate::common::{self, Run, ctrl, key, key_mods, shift};

fn setup(data: &Path) {
    std::fs::write(data.join("a.txt"), b"alpha").unwrap();
    std::fs::write(data.join("b.md"), b"beta").unwrap();
    std::fs::create_dir_all(data.join("sub")).unwrap();
    std::fs::write(data.join("sub").join("inner.log"), b"inner").unwrap();
}

/// A test terminal and the frame loop, by hand.
struct Drive {
    term: Terminal<TestBackend>,
    io: NullIo,
}

impl Drive {
    fn new(w: u16, h: u16) -> Drive {
        Drive { term: Terminal::new(TestBackend::new(w, h)).expect("test terminal"), io: NullIo::default() }
    }

    fn frame(&mut self, app: &mut App, events: &[Event]) {
        app.frame(&mut self.term, events, &mut self.io).expect("a frame");
    }

    /// Send `ev` in one frame, then draw one more.
    fn send(&mut self, app: &mut App, ev: Event) {
        self.frame(app, &[ev]);
        self.frame(app, &[]);
    }

    fn keys(&mut self, app: &mut App, codes: &[KeyCode]) {
        for c in codes {
            self.send(app, key(*c));
        }
    }

    fn type_text(&mut self, app: &mut App, s: &str) {
        for c in s.chars() {
            self.send(app, key(KeyCode::Char(c)));
        }
    }

    /// Run frames until `cond` holds (30 s at most).
    fn wait(&mut self, app: &mut App, what: &str, cond: impl Fn(&App) -> bool) {
        let t0 = Instant::now();
        loop {
            self.frame(app, &[]);
            if cond(app) {
                return;
            }
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A left click on the first cell of target `t` (drawn in the last frame).
    fn click(&mut self, app: &mut App, t: HitTarget) {
        let (x, y) = target_at(app, t).unwrap_or_else(|| panic!("{t:?} is not drawn"));
        self.send(app, mouse(MouseEventKind::Down(MouseButton::Left), x, y));
    }

    fn screen(&self) -> String {
        common::screen(&self.term)
    }
}

fn mouse(kind: MouseEventKind, x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE })
}

fn target_at(a: &App, t: HitTarget) -> Option<(u16, u16)> {
    a.hits().iter().filter(|(_, h)| *h == t).last().map(|(r, _)| (r.x, r.y))
}

fn vm(a: &App) -> Option<&SearchVm> {
    a.view().map(|v| &v.search)
}

fn bar(a: &App) -> Option<&BarVm> {
    vm(a)?.bar.as_ref()
}

fn status(a: &App) -> Option<&StatusVm> {
    vm(a)?.status.as_ref()
}

fn shown(a: &App) -> Option<SearchId> {
    match a.core.ws.pane().location {
        Location::Search(id) => Some(id),
        _ => None,
    }
}

fn pill_focused(a: &App, id: u8) -> bool {
    bar(a).is_some_and(|b| b.pills.iter().any(|p| p.id == id && p.focused))
}

/// The search on screen (not `other`) finished and lists exactly `labels`.
fn finished_with(a: &App, other: Option<SearchId>, labels: &[&str]) -> bool {
    shown(a).is_some()
        && shown(a) != other
        && status(a).is_some_and(|s| s.progress == Progress::Done)
        && common::labels(a) == labels
}

fn ctrl_shift_f() -> Event {
    key_mods(KeyCode::Char('F'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)
}

#[test]
fn bar_runs_a_search_and_closes() {
    if let Run::Child(data) = common::relaunch(module_path!(), "bar_runs_a_search_and_closes", setup) {
        runs_and_closes(data);
    }
}

fn runs_and_closes(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let mut d = Drive::new(100, 24);
    d.wait(&mut app, "the listing", |a| common::at(a, &data) && common::labels(a).len() == 3);
    assert!(vm(&app).is_some_and(|s| s.bar.is_none() && s.status.is_none()));
    d.send(&mut app, ctrl_shift_f());
    assert_eq!(app.focus(), TuiFocus::Search);
    assert_eq!(app.key_context(), KeyContext::Omnibar);
    assert!(bar(&app).is_some() && app.regions().search.height == 2, "{:?}", app.regions().search);
    d.type_text(&mut app, "a");
    assert_eq!(app.core.ui.search.form.text, "a");
    assert!(bar(&app).is_some_and(|b| b.field.shown == "a" && b.field.cursor == Some(1)));
    d.send(&mut app, key(KeyCode::Enter));
    d.wait(&mut app, "the results", |a| finished_with(a, None, &["a.txt"]));
    let s = status(&app).unwrap();
    assert!(s.head.contains("1 result"), "{s:?}");
    assert!(s.detail.contains("scanned"), "{s:?}");
    // The bar keeps the keyboard; the status row is drawn under it.
    assert_eq!(app.focus(), TuiFocus::Search);
    assert_eq!(app.regions().search.height, 3);
    let screen = d.screen();
    assert!(screen.contains("1 result") && screen.contains("[Search]") && screen.contains("[Contains]"), "{screen}");
    d.send(&mut app, key(KeyCode::Esc));
    assert!(bar(&app).is_none() && app.focus() == TuiFocus::List);
    // The results stay with their status row, which offers Refine.
    assert!(shown(&app).is_some());
    assert!(status(&app).is_some_and(|s| s.buttons.iter().any(|b| b.id == status_button::REFINE)));
    // Refine (a click) opens the bar again with the keyboard.
    d.click(&mut app, HitTarget::SearchStatusButton(status_button::REFINE));
    assert!(bar(&app).is_some() && app.focus() == TuiFocus::Search);
    d.send(&mut app, key(KeyCode::Esc));
    // The legacy alternative (F3) opens it too.
    d.send(&mut app, key(KeyCode::F(3)));
    assert!(bar(&app).is_some() && app.focus() == TuiFocus::Search);
    // Down gives the keyboard to the results; the bar stays.
    d.send(&mut app, key(KeyCode::Down));
    assert!(bar(&app).is_some() && app.focus() == TuiFocus::List);
    // A click on the field takes it back.
    d.click(&mut app, HitTarget::SearchField);
    assert_eq!(app.focus(), TuiFocus::Search);
    // The close button.
    d.click(&mut app, HitTarget::SearchButton(SearchButton::Close));
    assert!(bar(&app).is_none() && app.focus() == TuiFocus::List);
    app.finish();
    common::ok();
}

#[test]
fn pills_rerun_the_search() {
    if let Run::Child(data) = common::relaunch(module_path!(), "pills_rerun_the_search", setup) {
        pills(data);
    }
}

fn pills(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let mut d = Drive::new(120, 24);
    d.wait(&mut app, "the listing", |a| common::at(a, &data));
    d.send(&mut app, key(KeyCode::F(3)));
    d.type_text(&mut app, "n");
    d.send(&mut app, key(KeyCode::Enter));
    // "n": sub/inner.log only (folders match by name too: "sub" does not).
    d.wait(&mut app, "the first results", |a| {
        shown(a).is_some() && status(a).is_some_and(|s| s.progress == Progress::Done)
    });
    let labels = common::labels(&app);
    assert!(labels.len() == 1 && labels[0].ends_with("inner.log"), "{labels:?}");
    let first = shown(&app);
    // Field → mode → case → contents → size → date → kind.
    d.keys(&mut app, &[KeyCode::Tab; 6]);
    assert!(pill_focused(&app, pill::KIND) && bar(&app).is_some_and(|b| b.field.cursor.is_none()));
    d.send(&mut app, key(KeyCode::Char(' ')));
    assert_eq!(app.core.ui.search.form.kind, KindFilter::Files);
    d.wait(&mut app, "the search ran again (files)", |a| {
        shown(a) != first && status(a).is_some_and(|s| s.progress == Progress::Done)
    });
    assert!(common::labels(&app).iter().any(|l| l.ends_with("inner.log")));
    let second = shown(&app);
    d.send(&mut app, key(KeyCode::Char(' ')));
    assert_eq!(app.core.ui.search.form.kind, KindFilter::Folders);
    d.wait(&mut app, "the search ran again (folders: nothing)", |a| finished_with(a, second, &[]));
    // Shift: the previous option.
    d.send(&mut app, shift(KeyCode::Char(' ')));
    assert_eq!(app.core.ui.search.form.kind, KindFilter::Files);
    // The mouse: right click cycles backwards, a left click on a toggle toggles it.
    let (x, y) = target_at(&app, HitTarget::SearchPill(pill::KIND)).unwrap();
    d.send(&mut app, mouse(MouseEventKind::Down(MouseButton::Right), x, y));
    assert_eq!(app.core.ui.search.form.kind, KindFilter::Any);
    assert!(!app.core.ui.search.form.include_hidden);
    d.click(&mut app, HitTarget::SearchPill(pill::HIDDEN));
    assert!(app.core.ui.search.form.include_hidden);
    assert!(bar(&app).is_some_and(|b| b.pills.iter().any(|p| p.id == pill::HIDDEN && p.active)));
    d.click(&mut app, HitTarget::SearchPill(pill::CASE));
    assert!(app.core.ui.search.form.case_sensitive);
    d.click(&mut app, HitTarget::SearchPill(pill::MODE));
    assert_ne!(app.core.ui.search.form.mode, soyuz_appcore::services::search::NameMode::Substring);
    // Contents: the glob field appears in the Tab ring and as a click target.
    d.click(&mut app, HitTarget::SearchPill(pill::CONTENT));
    assert!(app.core.ui.search.form.content && bar(&app).is_some_and(|b| b.glob.is_some()));
    d.click(&mut app, HitTarget::SearchPill(pill::GLOB));
    d.type_text(&mut app, "*.md");
    assert_eq!(app.core.ui.search.form.name_glob, "*.md");
    d.click(&mut app, HitTarget::SearchPill(pill::CONTENT));
    assert!(!app.core.ui.search.form.content && bar(&app).is_some_and(|b| b.glob.is_none()));

    // The size pill opens the size dialog.
    d.click(&mut app, HitTarget::SearchField);
    d.keys(&mut app, &[KeyCode::Tab; 4]);
    assert!(pill_focused(&app, pill::SIZE));
    d.send(&mut app, key(KeyCode::Enter));
    assert_eq!(common::dialog_title(&app).as_deref(), Some("File size"));
    d.type_text(&mut app, "1k");
    d.send(&mut app, key(KeyCode::Enter));
    assert!(common::dialog_title(&app).is_none());
    assert_eq!(app.core.ui.search.form.min_size, "1k");
    assert!(bar(&app).is_some_and(|b| b.pills.iter().any(|p| p.id == pill::SIZE && p.active && p.label == "> 1k")));
    // A bad size is refused; Esc leaves the form alone.
    d.send(&mut app, key(KeyCode::Enter));
    d.send(&mut app, key(KeyCode::Tab));
    d.type_text(&mut app, "lots");
    d.send(&mut app, key(KeyCode::Enter));
    assert!(app.view().and_then(|v| v.dialog.as_ref()).is_some_and(|dl| dl.error.is_some()));
    d.send(&mut app, key(KeyCode::Esc));
    assert!(common::dialog_title(&app).is_none() && app.core.ui.search.form.max_size.is_empty());
    // A preset: Tab to the preset row, Right picks "< 16 KB" (min cleared, max 16k).
    d.send(&mut app, key(KeyCode::Enter));
    d.keys(&mut app, &[KeyCode::Tab, KeyCode::Tab, KeyCode::Right, KeyCode::Enter]);
    assert_eq!((app.core.ui.search.form.min_size.as_str(), app.core.ui.search.form.max_size.as_str()), ("", "16k"));
    // The size dialog's buttons take the mouse too.
    d.send(&mut app, key(KeyCode::Enter));
    d.click(&mut app, HitTarget::DialogButton(1));
    assert!(common::dialog_title(&app).is_none());
    app.finish();
    common::ok();
}

#[test]
fn ctrl_s_saves_and_the_sidebar_runs_it() {
    match common::relaunch(module_path!(), "ctrl_s_saves_and_the_sidebar_runs_it", setup) {
        Run::Child(data) => save(data),
        Run::Parent(r) => {
            let text = std::fs::read_to_string(r.config_dir().join("searches.toml")).expect("searches.toml");
            assert!(text.contains("name = \"alpha files\"") && text.contains("text = \"a\""), "{text}");
        }
    }
}

fn config_with_sidebar() -> Config {
    let mut c = common::test_config();
    c.layout.sidebar = true;
    // Saved searches come last; keep the sections above them short (no scrolling).
    c.sidebar.sections = vec![SidebarSection::Trash];
    c
}

fn save(data: PathBuf) {
    let mut app = common::new_app(config_with_sidebar(), &data);
    let mut d = Drive::new(120, 30);
    d.wait(&mut app, "the listing", |a| common::at(a, &data));
    d.send(&mut app, ctrl_shift_f());
    d.type_text(&mut app, "a");
    d.send(&mut app, ctrl('s'));
    assert_eq!(common::dialog_title(&app).as_deref(), Some("Save this search"));
    assert_eq!(app.focus(), TuiFocus::Dialog);
    // The name starts as the form's title; replace it.
    let title = app.core.ui.search.form.title();
    for _ in title.chars() {
        d.send(&mut app, key(KeyCode::Backspace));
    }
    // An empty name is refused.
    d.send(&mut app, key(KeyCode::Enter));
    assert!(app.view().and_then(|v| v.dialog.as_ref()).is_some_and(|dl| dl.error.is_some()));
    d.type_text(&mut app, "alpha files");
    d.send(&mut app, key(KeyCode::Enter));
    assert!(common::dialog_title(&app).is_none());
    assert!(app.core.ui.search.saving.is_none());
    assert_eq!(app.core.ui.search.names(), ["alpha files"]);
    // The bar's Save button opens the same dialog; Esc cancels it.
    // The "Saved" toast sits over the bar's right end: dismiss it first.
    app.core.ui.toasts.live.clear();
    d.frame(&mut app, &[]);
    d.click(&mut app, HitTarget::SearchButton(SearchButton::Save));
    assert_eq!(common::dialog_title(&app).as_deref(), Some("Save this search"));
    d.send(&mut app, key(KeyCode::Esc));
    assert!(common::dialog_title(&app).is_none() && app.core.ui.search.saving.is_none());
    d.send(&mut app, key(KeyCode::Esc));
    assert!(bar(&app).is_none() && shown(&app).is_none());
    // The sidebar lists it; a click runs it.
    let r = app.regions().sidebar;
    let y = app
        .view()
        .and_then(|v| v.sidebar.lines.iter().position(|l| l.label == "alpha files"))
        .expect("the saved search in the sidebar");
    d.send(&mut app, mouse(MouseEventKind::Down(MouseButton::Left), r.x + 2, r.y + y as u16));
    d.wait(&mut app, "the saved search's results", |a| finished_with(a, None, &["a.txt"]));
    app.finish();
    common::ok();
}

#[test]
fn flat_view_round_trip() {
    if let Run::Child(data) = common::relaunch(module_path!(), "flat_view_round_trip", setup) {
        flat(data);
    }
}

fn flat(data: PathBuf) {
    let mut c = common::test_config();
    c.keys.bindings.insert("toggle_flat_view".into(), vec!["F12".into()]);
    let mut app = common::new_app(c, &data);
    let mut d = Drive::new(100, 24);
    d.wait(&mut app, "the listing", |a| common::at(a, &data));
    d.send(&mut app, key(KeyCode::F(12)));
    d.wait(&mut app, "every file below the folder", |a| {
        shown(a).is_some() && status(a).is_some_and(|s| s.progress == Progress::Done) && common::labels(a).len() == 3
    });
    assert!(common::labels(&app).iter().any(|l| l.ends_with("inner.log")));
    // The status row without the bar: one row.
    assert!(bar(&app).is_none() && app.regions().search.height == 1);
    d.send(&mut app, key(KeyCode::F(12)));
    d.wait(&mut app, "back in the folder", |a| common::at(a, &data));
    app.finish();
    common::ok();
}

#[test]
fn tiny_sizes_with_the_bar_open() {
    if let Run::Child(data) = common::relaunch(module_path!(), "tiny_sizes_with_the_bar_open", setup) {
        tiny(data);
    }
}

fn tiny(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let mut d = Drive::new(60, 12);
    d.wait(&mut app, "the listing", |a| common::at(a, &data));
    d.send(&mut app, key(KeyCode::F(3)));
    d.type_text(&mut app, "a");
    d.send(&mut app, key(KeyCode::Enter));
    d.wait(&mut app, "the results", |a| finished_with(a, None, &["a.txt"]));
    // Below 80 columns the pills wrap onto a second row.
    assert_eq!(app.regions().search.height, 4);
    let mut io = NullIo::default();
    for content in [false, true] {
        app.core.ui.search.form.content = content;
        for w in (0..=90).step_by(7) {
            for h in 0..=14 {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                app.frame(&mut terminal, &[], &mut io).expect("a frame");
                app.frame(&mut terminal, &[key(KeyCode::Tab)], &mut io).expect("a frame");
            }
        }
    }
    app.finish();
    common::ok();
}

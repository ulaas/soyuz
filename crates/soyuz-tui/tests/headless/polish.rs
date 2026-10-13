//! Headless tests of the T5 + T6 polish (W4-polish): toasts below the search bar, the
//! bar's key hint, the sidebar's middle click, "Extract to…" folder completion, "Generate
//! token" for MCP clients, the terminal pane's clipboard paste switch and a click in a
//! text field putting the cursor under the pointer. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`. The tests drive
//! `App::frame` directly ([`Ui`]) so a click can use the coordinates of the frame before.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::models::archive::{ArchivePrompt, ExtractForm};
use soyuz_appcore::models::frontend::FrontendCmd;
use soyuz_appcore::models::settings::Section;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::services::ai_service;
use soyuz_appcore::state::session::FavoriteEntry;
use soyuz_appcore::tr;
use soyuz_core::config::{self, SidebarSection};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::input::TuiFocus;
use soyuz_tui::layout::HitTarget;
use soyuz_tui::view::dialogs::DialogVm;
use soyuz_tui::widgets::form::FieldVmKind;

use crate::common::{self, Run, key, key_mods};

/// The app on a test terminal, fed one event per frame.
struct Ui {
    app: App,
    t: Terminal<TestBackend>,
    io: NullIo,
}

impl Ui {
    fn new(app: App, w: u16, h: u16) -> Ui {
        let mut ui = Ui { app, t: Terminal::new(TestBackend::new(w, h)).unwrap(), io: NullIo::default() };
        ui.idle();
        ui
    }

    fn idle(&mut self) {
        self.app.frame(&mut self.t, &[], &mut self.io).expect("a frame");
    }

    /// One event, then a quiet frame.
    fn send(&mut self, ev: Event) {
        self.app.frame(&mut self.t, &[ev], &mut self.io).expect("a frame");
        self.idle();
    }

    fn press(&mut self, code: KeyCode) {
        self.send(key(code));
    }

    fn alt(&mut self, c: char) {
        self.send(key_mods(KeyCode::Char(c), KeyModifiers::ALT));
    }

    fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            self.press(KeyCode::Char(c));
        }
    }

    fn click(&mut self, button: MouseButton, x: u16, y: u16) {
        self.send(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(button),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }));
    }

    /// Run frames until `cond` holds (30 s at most).
    fn wait(&mut self, what: &str, cond: impl Fn(&App) -> bool) {
        let t0 = Instant::now();
        while !cond(&self.app) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
            self.idle();
        }
    }

    fn dialog(&self) -> Option<&DialogVm> {
        self.app.view()?.dialog.as_ref()
    }

    fn title(&self) -> String {
        self.dialog().map(|d| d.title.clone()).unwrap_or_default()
    }

    /// The text and cursor (byte offset) of the dialog's text field `i`.
    fn field_text(&self, i: usize) -> Option<(String, usize)> {
        match &self.dialog()?.fields.get(i)?.kind {
            FieldVmKind::Text { text, edit } => Some((text.clone(), edit.cursor())),
            _ => None,
        }
    }

    /// The text box of dialog field `i` (drawn in the last frame).
    fn field_box(&self, i: usize) -> ratatui::layout::Rect {
        let hits = self.app.hits();
        let row = hits
            .iter()
            .filter(|(_, t)| *t == HitTarget::DialogField(i))
            .map(|(r, _)| *r)
            .last()
            .unwrap_or_else(|| panic!("field {i} is not drawn"));
        (row.x..row.right())
            .find_map(|x| hits.field_text_at(x, row.y).filter(|(_, f)| *f == i).map(|(r, _)| r))
            .expect("the field's text box")
    }
}

/// The search bar: toasts stay below it, the status line shows its key hint, a click in
/// the query field puts the cursor under the pointer.
#[test]
fn search_bar_toasts_hint_and_click() {
    let run = common::relaunch(module_path!(), "search_bar_toasts_hint_and_click", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
    });
    if let Run::Child(data) = run {
        search_bar(data);
    }
}

fn search_bar(data: PathBuf) {
    let mut ui = Ui::new(common::new_app(common::test_config(), &data), 100, 24);
    ui.wait("the listing", |a| common::at(a, &data));
    let hint = |a: &App| a.view().map(|v| v.status.hint.clone()).unwrap_or_default();
    assert_eq!(hint(&ui.app), tr!("tui-hint"));
    ui.send(key_mods(KeyCode::Char('F'), KeyModifiers::CONTROL | KeyModifiers::SHIFT));
    assert_eq!(ui.app.focus(), TuiFocus::Search);
    assert_eq!(hint(&ui.app), tr!("tui-search-hint-field"));
    ui.type_text("abcdef");
    // A click three cells into the text box: the cursor goes before "d".
    let regions = ui.app.regions().clone();
    let start = (regions.search.x..regions.search.right())
        .find(|&x| soyuz_tui::view::search::field_click(&ui.app.core, &regions, x, false).is_some_and(|(c, _)| c == 0))
        .expect("the query box");
    ui.click(MouseButton::Left, start + 3, regions.search.y);
    let cursor = |a: &App| a.view().and_then(|v| v.search.bar.as_ref()).and_then(|b| b.field.cursor);
    assert_eq!(cursor(&ui.app), Some(3));
    ui.type_text("X");
    assert_eq!(ui.app.core.ui.search.form.text, "abcXdef");
    // A click past the text: the end.
    ui.click(MouseButton::Left, start + 40, regions.search.y);
    assert_eq!(cursor(&ui.app), Some(7));
    // On the pills the hint follows.
    ui.press(KeyCode::Tab);
    assert_eq!(hint(&ui.app), tr!("tui-search-hint-controls"));
    // A toast is drawn below the bar, never over its buttons.
    ui.app.core.ui.toasts.push(Toast::info("Copied"));
    ui.idle();
    let search = ui.app.regions().search;
    let toast = ui.app.hits().iter().find(|(_, t)| matches!(t, HitTarget::Toast(_))).map(|(r, _)| *r);
    let toast = toast.expect("the toast is drawn");
    assert!(toast.y >= search.bottom(), "toast {toast:?} under the bar {search:?}");
    ui.press(KeyCode::Esc);
    assert_eq!(hint(&ui.app), tr!("tui-hint"));
    ui.app.finish();
    common::ok();
}

/// A middle click on a sidebar place opens it in a new background tab (the GUI's).
#[test]
fn sidebar_middle_click_opens_a_tab() {
    let run = common::relaunch(module_path!(), "sidebar_middle_click_opens_a_tab", |data| {
        std::fs::create_dir_all(data.join("target")).unwrap();
        std::fs::write(data.join("target").join("inside.txt"), b"i").unwrap();
    });
    if let Run::Child(data) = run {
        middle_click(data);
    }
}

fn middle_click(data: PathBuf) {
    let mut c = common::test_config();
    c.layout.sidebar = true;
    c.sidebar.sections = vec![SidebarSection::Favorites];
    let mut app = common::new_app(c, &data);
    app.core.ui.sidebar.favorites = vec![FavoriteEntry { name: "Target".into(), path: data.join("target") }];
    app.core.ui.sidebar.favorites_customized = true;
    let mut ui = Ui::new(app, 100, 24);
    ui.wait("the sidebar", |a| {
        common::at(a, &data) && a.view().is_some_and(|v| v.sidebar.lines.iter().any(|l| l.label == "Target"))
    });
    let index = ui.app.view().unwrap().sidebar.lines.iter().find(|l| l.label == "Target").unwrap().index;
    let (rect, _) = *ui.app.hits().iter().find(|(_, t)| *t == HitTarget::SidebarRow(index)).expect("the row");
    let focus = ui.app.core.ui.focus;
    ui.click(MouseButton::Middle, rect.x + 1, rect.y);
    ui.wait("the new tab", |a| a.core.ws.tabs.len() == 2);
    // In the background: the first tab stays active on the sandbox, the focus unchanged.
    assert_eq!(ui.app.core.ws.active, 0);
    assert!(common::at(&ui.app, &data));
    assert_eq!(ui.app.core.ui.focus, focus);
    let second = &ui.app.core.ws.tabs[1];
    assert!(second.panes.iter().any(|p| p.location == soyuz_core::Location::Dir(data.join("target"))));
    ui.app.finish();
    common::ok();
}

/// "Extract to…": the destination completes subfolders (Down picks, Tab accepts); a click
/// in the field puts the cursor under the pointer.
#[test]
fn extract_to_completes_and_clicks() {
    let run = common::relaunch(module_path!(), "extract_to_completes_and_clicks", |data| {
        std::fs::create_dir_all(data.join("docs")).unwrap();
        std::fs::create_dir_all(data.join("downloads")).unwrap();
        std::fs::create_dir_all(data.join("music")).unwrap();
        std::fs::write(data.join("a.zip"), b"PK").unwrap();
    });
    if let Run::Child(data) = run {
        extract_to(data);
    }
}

fn extract_to(data: PathBuf) {
    let mut ui = Ui::new(common::new_app(common::test_config(), &data), 100, 30);
    ui.wait("the listing", |a| common::at(a, &data));
    let form = ExtractForm::new(vec![data.join("a.zip")], None, data.clone());
    soyuz_tui::input::dialogs::archive::open(&mut ui.app, ArchivePrompt::Extract(form));
    ui.idle();
    assert_eq!(ui.title(), tr!("views-archive-extract"));
    let sep = std::path::MAIN_SEPARATOR;
    ui.type_text(&format!("{sep}d"));
    ui.wait("the completions", |a| a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.list.len() == 2));
    let rows: Vec<String> = ui.dialog().unwrap().list.iter().map(|r| r.text.clone()).collect();
    assert_eq!(rows, [format!("docs{sep}"), format!("downloads{sep}")]);
    ui.press(KeyCode::Down);
    ui.press(KeyCode::Tab);
    let (text, _) = ui.field_text(0).unwrap();
    assert_eq!(PathBuf::from(text.trim_end_matches(sep)), data.join("downloads"));
    // Clear the field, type a short text and click two cells into it.
    for _ in 0..text.chars().count() {
        ui.press(KeyCode::Backspace);
    }
    ui.type_text("abcdef");
    ui.wait("no completions", |a| a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.list.is_empty()));
    let r = ui.field_box(0);
    ui.click(MouseButton::Left, r.x + 2, r.y);
    assert_eq!(ui.field_text(0), Some(("abcdef".into(), 2)));
    ui.type_text("X");
    assert_eq!(ui.field_text(0).unwrap().0, "abXcdef");
    // The checkbox row is not a text box: a click toggles it as before.
    ui.press(KeyCode::Esc);
    assert!(ui.dialog().is_none());
    ui.app.finish();
    common::ok();
}

/// MCP clients: "Generate token" stores a new token (the in-memory credential store here)
/// and shows it once with the client snippet; `config.toml` keeps only the reference.
#[test]
fn mcp_client_generate_token() {
    let run = common::relaunch(module_path!(), "mcp_client_generate_token", |_| {});
    if let Run::Child(data) = run {
        generate_token(data);
    }
}

fn generate_token(data: PathBuf) {
    ai_service::use_memory_credential_store();
    let mut ui = Ui::new(common::new_app(common::test_config(), &data), 110, 34);
    ui.app.core.frontend.push(FrontendCmd::OpenSettings(Some(Section::Ai)));
    ui.idle();
    ui.press(KeyCode::Char('/'));
    ui.type_text("ai.mcp_clients");
    ui.press(KeyCode::Enter);
    let on = |a: &App| {
        a.view().and_then(|v| v.settings.lines.iter().find(|l| l.cursor)).is_some_and(|l| l.label == "mcp_clients")
    };
    for _ in 0..30 {
        if on(&ui.app) {
            break;
        }
        ui.press(KeyCode::Down);
    }
    ui.press(KeyCode::Enter);
    assert!(ui.title().contains("ai.mcp_clients"), "{:?}", ui.dialog());
    ui.press(KeyCode::Insert);
    ui.type_text("desk");
    ui.press(KeyCode::Enter);
    ui.press(KeyCode::Home);
    for _ in 0..40 {
        let row = ui.dialog().and_then(|d| d.list.get(d.list_state.cursor)).map(|r| r.text.clone());
        if row.as_deref() == Some("token") {
            break;
        }
        ui.press(KeyCode::Down);
    }
    ui.alt('s');
    let d = ui.dialog().expect("the secret dialog");
    assert!(d.buttons.iter().any(|b| b.label == tr!("settings-mcp-generate")), "{d:?}");
    assert_eq!(d.hints, tr!("tui-tables-secret-hint-mcp"));
    ui.alt('g');
    ui.wait("the stored token", |_| ai_service::memory_credential("mcp-client-desk").is_some());
    let token = ai_service::memory_credential("mcp-client-desk").unwrap();
    assert!(token.starts_with("soyuz_"), "{token}");
    let d = ui.dialog().expect("the token stays shown");
    let body = d.body.join("\n");
    assert!(body.contains(&format!("Bearer {token}")) && body.contains("/mcp"), "{body}");
    assert_eq!(d.hints, tr!("tui-tables-token-hint"));
    assert_eq!(ui.app.core.config.ai.mcp_clients.first().map(|c| c.token.as_str()), Some("credential:mcp-client-desk"));
    // Copy config, then close: back in the element form.
    ui.alt('c');
    ui.press(KeyCode::Esc);
    assert!(ui.title().contains("ai.mcp_clients ·"), "back in the form: {}", ui.title());
    for _ in 0..4 {
        if ui.dialog().is_none() {
            break;
        }
        ui.press(KeyCode::Esc);
    }
    soyuz_appcore::models::settings::flush(&mut ui.app.core);
    let name = config::default_path().file_name().expect("config file name").to_owned();
    let file = data.parent().unwrap().join(config::PORTABLE_DATA_DIR).join(name);
    let text = std::fs::read_to_string(file).unwrap_or_default();
    assert!(text.contains("credential:mcp-client-desk") && !text.contains(&token), "{text}");
    ui.app.finish();
    common::ok();
}

/// The terminal pane's Ctrl+Shift+V reads the local clipboard through the core's
/// `os_clipboard::get_text`, which the test switch (`set_system_clipboard(false)`) turns
/// off: tests never read the user's clipboard.
#[test]
fn clipboard_text_is_off_in_tests() {
    let run = common::relaunch(module_path!(), "clipboard_text_is_off_in_tests", |_| {});
    if let Run::Child(data) = run {
        soyuz_appcore::services::os_clipboard::set_system_clipboard(false);
        let mut app = common::new_app(common::test_config(), &data);
        assert_eq!(soyuz_appcore::services::os_clipboard::get_text(), None);
        let paste = crossterm::event::KeyEvent::new(KeyCode::Char('V'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert!(soyuz_tui::input::terminal::is_paste_chord(&paste));
        app.finish();
        common::ok();
    }
}

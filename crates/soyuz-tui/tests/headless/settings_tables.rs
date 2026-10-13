//! Headless tests of T6: arrays / maps of tables and secrets in the settings editor (W3 settings
//! tables). Each test runs in its own sandbox: `crate::common::relaunch(module_path!(), "<test
//! name>", |data| ...)`.
//!
//! The table editor is driven by keys only, one event per frame: the settings editor's
//! search finds the table key, Enter opens the table editor (`TuiDialog::TableEdit`), the
//! element form edits the fields. Checks read the dialog drawn last (`ViewModel::dialog`),
//! the live config and `config.toml` in the sandbox. Secrets go to the in-memory
//! credential store (`ai_service::use_memory_credential_store`), never the OS one.

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use soyuz_appcore::models::frontend::FrontendCmd;
use soyuz_appcore::models::settings::Section;
use soyuz_appcore::services::ai_service;
use soyuz_core::config::{self, AiProviderKind};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::view::dialogs::DialogVm;

use crate::common::{self, Run, key, key_mods};

/// `config.toml` of the sandbox (portable mode: next to the test binary's copy).
fn config_file(data: &Path) -> PathBuf {
    let name = config::default_path().file_name().expect("config file name").to_owned();
    data.parent().expect("sandbox").join(config::PORTABLE_DATA_DIR).join(name)
}

/// The app on a test terminal, fed one event per frame.
struct Ui {
    app: App,
    t: Terminal<TestBackend>,
    io: NullIo,
}

impl Ui {
    fn new(data: &Path, w: u16, h: u16) -> Ui {
        let mut ui = Ui {
            app: common::new_app(common::test_config(), data),
            t: Terminal::new(TestBackend::new(w, h)).unwrap(),
            io: NullIo::default(),
        };
        ui.idle();
        ui
    }

    fn idle(&mut self) {
        self.app.frame(&mut self.t, &[], &mut self.io).expect("a frame");
    }

    /// One event, then a quiet frame (the settings editor applies what dialogs handed back).
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

    /// Clear a text field of up to `n` characters (the cursor is at its end).
    fn clear(&mut self, n: usize) {
        for _ in 0..n {
            self.press(KeyCode::Backspace);
        }
    }

    fn dialog(&self) -> Option<&DialogVm> {
        self.app.view()?.dialog.as_ref()
    }

    fn title(&self) -> String {
        self.dialog().map(|d| d.title.clone()).unwrap_or_default()
    }

    /// The text of the list row under the dialog's cursor.
    fn cursor_row(&self) -> String {
        self.dialog().and_then(|d| d.list.get(d.list_state.cursor)).map(|r| r.text.clone()).unwrap_or_default()
    }

    /// Move the element form's cursor to field `name`.
    fn field(&mut self, name: &str) {
        self.press(KeyCode::Home);
        for _ in 0..40 {
            if self.cursor_row() == name {
                return;
            }
            self.press(KeyCode::Down);
        }
        panic!("no field {name}: {:?}", self.dialog());
    }

    /// Open the settings editor (afresh) and the table editor of `table`.
    fn open_table(&mut self, section: Section, table: &str) {
        for _ in 0..3 {
            if !self.app.core.ui.settings_open {
                break;
            }
            self.press(KeyCode::Esc);
        }
        self.app.core.frontend.push(FrontendCmd::OpenSettings(Some(section)));
        self.idle();
        assert!(self.app.core.ui.settings_open);
        self.press(KeyCode::Char('/'));
        self.type_text(table);
        self.press(KeyCode::Enter);
        let label = table.rsplit('.').next().unwrap();
        let on = |ui: &Ui| {
            ui.app.view().and_then(|v| v.settings.lines.iter().find(|l| l.cursor)).is_some_and(|l| l.label == label)
        };
        for _ in 0..30 {
            if on(self) {
                break;
            }
            self.press(KeyCode::Down);
        }
        assert!(on(self), "the search found {table}");
        self.press(KeyCode::Enter);
        assert!(self.title().contains(table), "the table editor is open: {:?}", self.dialog());
    }

    /// Close every dialog (the settings editor applies the table at once).
    fn close_dialogs(&mut self) {
        for _ in 0..4 {
            if self.dialog().is_none() {
                break;
            }
            self.press(KeyCode::Esc);
        }
        assert!(self.dialog().is_none(), "dialogs closed: {:?}", self.dialog());
    }

    fn saved_text(&mut self, data: &Path) -> String {
        soyuz_appcore::models::settings::flush(&mut self.app.core);
        std::fs::read_to_string(config_file(data)).unwrap_or_default()
    }
}

/// Bookmarks: add one (name, path, hotkey recorded), add a second, move it up, remove the
/// first after the question; `config.toml` follows each step.
#[test]
fn bookmarks_add_edit_move_remove() {
    let run = common::relaunch(module_path!(), "bookmarks_add_edit_move_remove", |_| {});
    let Run::Child(data) = run else { return };
    let mut ui = Ui::new(&data, 110, 34);
    ui.open_table(Section::Bookmarks, "bookmarks");
    assert!(ui.dialog().unwrap().list.iter().any(|r| r.text.contains("No entries")), "{:?}", ui.dialog());
    // Insert adds an element and opens its form on the first field.
    ui.press(KeyCode::Insert);
    assert!(ui.title().contains("bookmarks ·"), "{}", ui.title());
    assert_eq!(ui.cursor_row(), "name");
    ui.type_text("Code");
    ui.press(KeyCode::Enter);
    ui.field("path");
    ui.type_text("/src");
    ui.press(KeyCode::Enter);
    ui.field("hotkey");
    ui.press(KeyCode::Enter);
    assert!(ui.title().contains("bookmarks[0].hotkey"), "the recorder: {}", ui.title());
    ui.press(KeyCode::F(9));
    ui.press(KeyCode::Enter);
    assert!(ui.title().contains("bookmarks · Code"), "back in the form: {}", ui.title());
    let row = ui.dialog().unwrap().list.iter().find(|r| r.text == "hotkey").cloned().unwrap();
    assert_eq!(row.right, "F9");
    // Nothing is live while the editor's dialogs are open; closing them applies.
    ui.close_dialogs();
    let c = &ui.app.core.config;
    assert_eq!(c.bookmarks.len(), 1);
    assert_eq!((c.bookmarks[0].name.as_str(), c.bookmarks[0].hotkey.as_str()), ("Code", "F9"));
    assert_eq!(c.bookmarks[0].path, PathBuf::from("/src"));
    let text = ui.saved_text(&data);
    assert!(
        text.contains("[[bookmarks]]") && text.contains("name = \"Code\"") && text.contains("hotkey = \"F9\""),
        "{text}"
    );

    // A second one after it, moved above the first.
    ui.open_table(Section::Bookmarks, "bookmarks");
    ui.press(KeyCode::Insert);
    ui.type_text("Docs");
    ui.press(KeyCode::Enter);
    ui.press(KeyCode::Esc);
    assert_eq!(ui.cursor_row(), "Docs");
    ui.send(key_mods(KeyCode::Up, KeyModifiers::ALT));
    assert_eq!(ui.dialog().unwrap().list[0].text, "Docs");
    ui.close_dialogs();
    let names: Vec<&str> = ui.app.core.config.bookmarks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Docs", "Code"]);

    // Remove asks first (Cancel is focused), Left + Enter removes.
    ui.open_table(Section::Bookmarks, "bookmarks");
    ui.press(KeyCode::Down);
    ui.press(KeyCode::Delete);
    assert!(ui.dialog().unwrap().alert && ui.dialog().unwrap().body.iter().any(|b| b.contains("Code")));
    ui.press(KeyCode::Enter);
    assert_eq!(ui.dialog().unwrap().list.len(), 2, "Enter on Cancel keeps it");
    ui.press(KeyCode::Delete);
    ui.press(KeyCode::Left);
    ui.press(KeyCode::Enter);
    assert_eq!(ui.dialog().unwrap().list.len(), 1);
    ui.close_dialogs();
    let names: Vec<&str> = ui.app.core.config.bookmarks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Docs"]);
    let text = ui.saved_text(&data);
    assert!(text.contains("name = \"Docs\"") && !text.contains("name = \"Code\""), "{text}");
    ui.app.finish();
    common::ok();
}

/// An AI profile: the name, a choice (kind) changed with Right, numbers typed (an invalid
/// one is refused with a message); a second element may not reuse the name.
#[test]
fn ai_profile_with_enum_and_numbers() {
    let run = common::relaunch(module_path!(), "ai_profile_with_enum_and_numbers", |_| {});
    let Run::Child(data) = run else { return };
    let mut ui = Ui::new(&data, 110, 34);
    ui.open_table(Section::Ai, "ai.profiles");
    ui.press(KeyCode::Insert);
    ui.type_text("work");
    ui.press(KeyCode::Enter);
    ui.field("kind");
    ui.press(KeyCode::Right);
    ui.field("max_tokens");
    ui.clear(8);
    ui.type_text("lots");
    ui.press(KeyCode::Enter);
    assert!(ui.dialog().unwrap().error.as_deref().is_some_and(|e| e.contains("lots")), "{:?}", ui.dialog());
    ui.clear(8);
    ui.type_text("4096");
    ui.press(KeyCode::Enter);
    assert!(ui.dialog().unwrap().error.is_none(), "{:?}", ui.dialog());
    ui.field("monthly_budget_usd");
    ui.clear(8);
    ui.type_text("12.5");
    ui.press(KeyCode::Enter);
    ui.field("vision");
    ui.press(KeyCode::Char(' '));
    // A second profile cannot take the name "work".
    ui.press(KeyCode::Esc);
    ui.press(KeyCode::Insert);
    ui.type_text("work");
    ui.press(KeyCode::Enter);
    assert!(ui.dialog().unwrap().error.as_deref().is_some_and(|e| e.contains("work")), "{:?}", ui.dialog());
    ui.close_dialogs();
    let p = &ui.app.core.config.ai.profiles;
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].name, "work");
    assert_ne!(p[0].kind, AiProviderKind::Anthropic);
    assert_eq!(p[0].max_tokens, 4096);
    assert!((p[0].monthly_budget_usd - 12.5).abs() < 1e-9);
    assert!(p[0].vision);
    assert_eq!(p[1].name, "");
    let text = ui.saved_text(&data);
    assert!(text.contains("[[ai.profiles]]") && text.contains("max_tokens = 4096"), "{text}");
    ui.app.finish();
    common::ok();
}

/// "Set secret…": the secret goes to the credential store (in memory here), the config
/// and `config.toml` only get the `credential:<name>` reference; an MCP client token too.
#[test]
fn secrets_leave_only_the_reference() {
    let run = common::relaunch(module_path!(), "secrets_leave_only_the_reference", |_| {});
    let Run::Child(data) = run else { return };
    ai_service::use_memory_credential_store();
    let mut ui = Ui::new(&data, 110, 34);
    ui.open_table(Section::Ai, "ai.profiles");
    ui.press(KeyCode::Insert);
    ui.type_text("My Claude");
    ui.press(KeyCode::Enter);
    ui.field("key");
    // A raw key typed into the reference field is refused.
    ui.type_text("sk-raw-0001");
    ui.press(KeyCode::Enter);
    assert!(ui.dialog().unwrap().error.is_some(), "{:?}", ui.dialog());
    ui.clear(20);
    ui.alt('s');
    assert!(ui.title().contains("My Claude"), "the secret dialog: {}", ui.title());
    ui.type_text("sk-test-secret-42");
    let shown = format!("{:?}", ui.dialog());
    assert!(!shown.contains("sk-test-secret-42"), "the field is masked");
    ui.press(KeyCode::Enter);
    assert!(ui.title().contains("ai.profiles ·"), "back in the form: {}", ui.title());
    // MCP clients: their token by the GUI's credential name.
    ui.press(KeyCode::Esc);
    ui.close_dialogs();
    ui.open_table(Section::Ai, "ai.mcp_clients");
    ui.press(KeyCode::Insert);
    ui.type_text("desk");
    ui.press(KeyCode::Enter);
    ui.field("token");
    ui.alt('s');
    ui.type_text("tok-123");
    ui.press(KeyCode::Enter);
    ui.close_dialogs();
    // The worker threads report back (a toast); wait for both writes.
    for _ in 0..200 {
        if ai_service::memory_credential("mcp-client-desk").is_some()
            && ai_service::memory_credential("my-claude").is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let c = &ui.app.core.config;
    assert_eq!(c.ai.profiles[0].key, "credential:my-claude");
    assert_eq!(c.ai.mcp_clients[0].token, "credential:mcp-client-desk");
    assert_eq!(ai_service::memory_credential("my-claude").as_deref(), Some("sk-test-secret-42"));
    assert_eq!(ai_service::memory_credential("mcp-client-desk").as_deref(), Some("tok-123"));
    let text = ui.saved_text(&data);
    assert!(text.contains("credential:my-claude") && text.contains("credential:mcp-client-desk"), "{text}");
    assert!(!text.contains("sk-test-secret-42") && !text.contains("tok-123") && !text.contains("sk-raw"), "{text}");
    ui.app.finish();
    common::ok();
}

/// The table editor, an element form and the secret dialog at every small size.
#[test]
fn tables_at_tiny_sizes() {
    let run = common::relaunch(module_path!(), "tables_at_tiny_sizes", |_| {});
    let Run::Child(data) = run else { return };
    ai_service::use_memory_credential_store();
    let mut ui = Ui::new(&data, 110, 34);
    ui.open_table(Section::Ai, "ai.profiles");
    fn sizes(ui: &mut Ui) {
        for w in (0..=40).step_by(3) {
            for h in 0..=14 {
                let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                ui.app.frame(&mut t, &[], &mut ui.io).expect("a frame");
            }
        }
    }
    sizes(&mut ui);
    ui.press(KeyCode::Insert);
    sizes(&mut ui);
    ui.field("headers");
    ui.press(KeyCode::Enter);
    sizes(&mut ui);
    ui.press(KeyCode::Esc);
    ui.field("key");
    ui.alt('s');
    sizes(&mut ui);
    ui.press(KeyCode::Esc);
    ui.press(KeyCode::Esc);
    ui.press(KeyCode::Delete);
    sizes(&mut ui);
    ui.close_dialogs();
    ui.app.finish();
    common::ok();
}

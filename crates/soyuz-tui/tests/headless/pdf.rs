//! Headless tests of the PDF viewer's text view (`view::editor::pdf`): F4 opens a PDF as
//! its page text; Page Down / N / P / Ctrl+Home / Ctrl+End turn pages, Ctrl+G goes to a
//! page, Ctrl+F finds with the matches marked, Ctrl+C copies the page's text, the picture
//! commands do nothing; a password-locked PDF asks for its password on Enter. PDFs are
//! built with lopdf in the sandbox.

use std::path::Path;

use crossterm::event::{KeyCode, KeyModifiers};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use soyuz_appcore::editors::DocKind;
use soyuz_tui::app::App;
use soyuz_tui::view::editor::text::MatchMark;
use soyuz_tui::view::editor::{BodyVm, TabKind};

use crate::common::{self, Run, Step, ctrl, key, key_mods, type_text};

/// A Letter-size document with one line of text per page.
fn document(pages: &[&str]) -> Document {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let mut kids: Vec<Object> = Vec::new();
    for text in pages {
        let ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 24.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal(*text)]),
            Operation::new("ET", vec![]),
        ];
        let content = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content, "Resources" => resources,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        kids.push(page.into());
    }
    let count = kids.len() as i64;
    doc.objects
        .insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc
}

fn save(mut doc: Document, path: &Path) {
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    std::fs::write(path, out).unwrap();
}

/// `pages` encrypted (RC4, 128-bit) with user password `password`.
fn encrypted(pages: &[&str], password: &str, path: &Path) {
    let mut doc = document(pages);
    let id = || Object::String(b"0123456789abcdef".to_vec(), lopdf::StringFormat::Hexadecimal);
    doc.trailer.set("ID", Object::Array(vec![id(), id()]));
    let version = lopdf::EncryptionVersion::V2 {
        document: &doc,
        owner_password: "owner",
        user_password: password,
        key_length: 128,
        permissions: lopdf::Permissions::all(),
    };
    let state = lopdf::EncryptionState::try_from(version).unwrap();
    doc.encrypt(&state).unwrap();
    save(doc, path);
}

/// Steps that put the cursor on `name` (the only file) and press F4.
fn open(name: &'static str) -> Vec<Step> {
    vec![
        Step::new("the listing", move |a| common::labels(a) == [name], key(KeyCode::Down)),
        Step::new("the file under the cursor", move |a| common::cursor(a).as_deref() == Some(name), key(KeyCode::F(4))),
    ]
}

/// The PDF view's visible text rows.
fn rows(a: &App) -> Vec<String> {
    match a.view().map(|v| &v.editor.body) {
        Some(BodyVm::Pdf(p)) => p.rows.iter().map(|r| r.iter().map(|s| s.text.as_str()).collect()).collect(),
        _ => Vec::new(),
    }
}

/// The view has the keyboard, shows page `n` of the status and a row containing `text`.
fn showing(n: usize, text: &'static str) -> impl Fn(&App) -> bool {
    move |a| {
        a.view().is_some_and(|v| {
            v.editor.focused
                && matches!(&v.editor.body, BodyVm::Pdf(p) if p.status_left.starts_with(&format!("Page {n} of ")))
        }) && rows(a).iter().any(|r| r.contains(text))
    }
}

/// The current match is marked, on a row containing `text`.
fn current_match(text: &'static str) -> impl Fn(&App) -> bool {
    move |a| match a.view().map(|v| &v.editor.body) {
        Some(BodyVm::Pdf(p)) => p.rows.iter().any(|r| {
            r.iter().any(|s| s.mark == MatchMark::Current && s.text == "Hello")
                && r.iter().map(|s| s.text.as_str()).collect::<String>().contains(text)
        }),
        _ => false,
    }
}

/// The active PDF's view rotation (the terminal ignores the rotate keys).
fn rotation(a: &App) -> Option<u8> {
    match a.core.editors.active_doc().map(|d| &d.kind) {
        Some(DocKind::Pdf(p)) => Some(p.rotation()),
        _ => None,
    }
}

#[test]
fn tui_pdf_pages_find_and_copy() {
    let run = common::relaunch(module_path!(), "tui_pdf_pages_find_and_copy", |data| {
        save(document(&["Hello from page one", "Second page text", "Third page says Hello"]), &data.join("book.pdf"));
    });
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let mut steps = open("book.pdf");
    steps.extend([
        Step::new(
            "page 1 as text in a PDF tab",
            |a| {
                showing(1, "Hello from page one")(a)
                    && a.view().is_some_and(|v| v.editor.tabs.first().is_some_and(|t| t.kind == TabKind::Pdf))
            },
            key(KeyCode::PageDown),
        ),
        Step::new("Page Down turned the page", showing(2, "Second page text"), key(KeyCode::Char('n'))),
        Step::new("N: the next page", showing(3, "Third page says Hello"), key(KeyCode::Char('p'))),
        Step::new(
            "P: the previous page",
            showing(2, "Second page text"),
            key_mods(KeyCode::Home, KeyModifiers::CONTROL),
        ),
        Step::new("Ctrl+Home: the first page", showing(1, "Hello from page one"), key(KeyCode::Char('r'))),
        Step::new("R does not rotate a text view", |a| rotation(a) == Some(0), ctrl('g')),
        Step::new(
            "the go-to bar",
            |a| a.view().is_some_and(|v| matches!(&v.editor.body, BodyVm::Pdf(p) if !p.bars.is_empty())),
            key(KeyCode::Char('3')),
        ),
    ]);
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
    steps.push(Step::new("went to page 3", showing(3, "Third page says Hello"), ctrl('f')));
    steps.push(Step::new(
        "the find bar",
        |a| a.view().is_some_and(|v| matches!(&v.editor.body, BodyVm::Pdf(p) if !p.bars.is_empty())),
        key(KeyCode::Char('H')),
    ));
    type_text(&mut steps, "ello");
    steps.push(Step::new("typed the query", |_| true, key(KeyCode::Enter)));
    steps.push(Step::new("a marked match", |a| current_match("Hello")(a), key(KeyCode::F(3))));
    steps.push(Step::new(
        "F3: the next match (the other page)",
        |a| current_match("Hello from page one")(a) || current_match("Third page says Hello")(a),
        key(KeyCode::Esc),
    ));
    steps.push(Step::new(
        "the find bar closed",
        |a| a.view().is_some_and(|v| matches!(&v.editor.body, BodyVm::Pdf(p) if p.bars.is_empty())),
        ctrl('c'),
    ));
    steps.push(Step::new("copied the page", |a| a.host.last_copy().is_some_and(|t| t.contains("Hello")), ctrl('q')));
    common::run_script(&mut app, steps, 100, 24);
    common::ok();
}

#[test]
fn tui_pdf_locked_asks_for_the_password() {
    let run = common::relaunch(module_path!(), "tui_pdf_locked_asks_for_the_password", |data| {
        encrypted(&["Secret page one"], "letmein", &data.join("locked.pdf"));
    });
    let Run::Child(data) = run else { return };
    let mut app = common::new_app(common::test_config(), &data);
    let locked = |a: &App| {
        a.view().is_some_and(|v| {
            v.editor.focused
                && matches!(&v.editor.body, BodyVm::Message(l) if l.iter().any(|l| l.starts_with("Enter:")))
        })
    };
    let mut steps = open("locked.pdf");
    steps.push(Step::new("the locked message", locked, key(KeyCode::Enter)));
    steps.push(Step::new("the password prompt", |a| common::dialog_title(a).is_some(), key(KeyCode::Char('l'))));
    type_text(&mut steps, "etmein");
    steps.push(Step::new("typed", |_| true, key(KeyCode::Enter)));
    steps.push(Step::new("unlocked", showing(1, "Secret page one"), ctrl('q')));
    common::run_script(&mut app, steps, 100, 24);
    common::ok();
}

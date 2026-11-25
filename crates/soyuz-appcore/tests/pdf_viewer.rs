//! The PDF viewer in a headless core: F4 / `edit:` opens a PDF as `DocKind::Pdf`, the
//! viewer's keys reach it (R rotates) while text documents and the viewer's own find
//! field keep typing, `SOYUZ_SCRIPT` verbs drive it, copy lands on the host, and
//! `[open] pdf_in_viewer` makes Enter open PDFs in the viewer.
//!
//! The core keeps its state in the config directory, so (like `headless_core.rs`) the test
//! re-runs itself from a copy of the test binary next to a `portable` marker.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use soyuz_appcore::core::{AppCore, Startup};
use soyuz_appcore::editors::{self, DocKind};
use soyuz_appcore::host::{Host, NullHost};
use soyuz_appcore::keys::input::{InputEvent, InputFrame};
use soyuz_core::config::{self, Config, KeyChord, KeyName, Mods};

const CHILD: &str = "SOYUZ_PDF_VIEWER_DATA";
const TEST: &str = "pdf_viewer_in_a_headless_core";

#[test]
fn pdf_viewer_in_a_headless_core() {
    if let Some(data) = std::env::var_os(CHILD) {
        run(PathBuf::from(data));
        return;
    }
    let tmp = tempfile::tempdir().expect("temp dir");
    let exe = std::env::current_exe().expect("test binary");
    let copy = tmp.path().join(exe.file_name().expect("exe name"));
    std::fs::copy(&exe, &copy).expect("copy the test binary");
    std::fs::write(tmp.path().join(config::PORTABLE_MARKER), b"").expect("portable marker");
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("doc.pdf"), pdf(&["Hello world", "Second page", "Third page"])).unwrap();
    std::fs::write(data.join("note.txt"), b"plain text\n").unwrap();
    let out = Command::new(&copy)
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD, &data)
        .output()
        .expect("run the copied test binary");
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "child run failed:\n{log}");
    assert!(log.contains("pdf viewer: ok"), "the child did not run the test:\n{log}");
}

/// Letter pages of Helvetica text, one line each.
fn pdf(pages: &[&str]) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let res = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
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
            "Type" => "Page", "Parent" => pages_id, "Contents" => content, "Resources" => res,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        kids.push(page.into());
    }
    let count = kids.len() as i64;
    doc.objects
        .insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// One headless frame, plus what the viewer's frontend does each frame.
fn tick(app: &mut AppCore, host: &dyn Host) {
    app.begin_frame();
    app.pump_services(host);
    app.pump_jobs(host);
    app.pump_background(host);
    app.apply_cmds(host);
    app.prepare_visible_panes();
    app.apply_cmds(host);
    editors::pump(app, host);
    for d in &mut app.editors.docs {
        if let DocKind::Pdf(p) = &mut d.kind {
            p.set_viewport([800.0, 600.0], 1.0);
            p.pump();
        }
    }
    app.end_frame();
}

fn settle(app: &mut AppCore, host: &dyn Host, what: &str, done: impl Fn(&AppCore) -> bool) {
    let start = Instant::now();
    loop {
        tick(app, host);
        if done(app) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn active_pdf(app: &mut AppCore) -> &mut editors::pdf::PdfDoc {
    match app.editors.active_doc_mut().map(|d| &mut d.kind) {
        Some(DocKind::Pdf(p)) => p,
        _ => panic!("the active document is not a PDF"),
    }
}

fn pdf_ready(app: &AppCore) -> bool {
    matches!(app.editors.active_doc().map(|d| &d.kind), Some(DocKind::Pdf(p)) if p.is_ready())
}

/// Press `chord` with the editor focused; returns whether the editor took it.
fn press(app: &mut AppCore, chord: KeyChord) -> bool {
    let mut frame = InputFrame::new(vec![InputEvent::Chord(chord)]);
    let out = editors::handle_keys(app, &mut frame);
    assert!(out.actions.is_empty(), "no global action: {:?}", out.actions);
    frame.consumed[0]
}

fn run(data: PathBuf) {
    let dir = config::config_dir();
    assert!(dir.ends_with(config::PORTABLE_DATA_DIR), "not in portable mode: {}", dir.display());
    let mut c = Config::default();
    c.updates.auto_check = false;
    c.cli.server_enabled = false;
    c.search.index.enabled = false;
    let startup = Startup { open: Some(data.clone()), secondary: true, safe_mode: false, ..Default::default() };
    let host = NullHost::default();
    let mut app = AppCore::new(c, startup, host.wake(), soyuz_appcore::services::preview_gfx::no_upload());
    let doc = data.join("doc.pdf");

    // `edit:` (what F4 does) opens the PDF in the viewer.
    editors::devscript(&mut app, &host, "edit", &doc.display().to_string());
    settle(&mut app, &host, "the PDF", pdf_ready);
    assert_eq!(active_pdf(&mut app).page_count(), 3);

    // R rotates the PDF.
    let r = KeyChord::new(Mods::empty(), KeyName::R);
    assert!(press(&mut app, r));
    assert_eq!(active_pdf(&mut app).rotation(), 1);
    // In the viewer's find field, R types instead.
    active_pdf(&mut app).field_focus = true;
    assert!(!press(&mut app, r), "left to the find field");
    assert_eq!(active_pdf(&mut app).rotation(), 1);
    active_pdf(&mut app).field_focus = false;
    // Esc closes the selection first, then leaves the viewer.
    editors::devscript(&mut app, &host, "editor", "pdf=select=all");
    assert!(press(&mut app, KeyChord::new(Mods::empty(), KeyName::Escape)));
    assert!(active_pdf(&mut app).selection().is_none());

    // A text document types the letter.
    editors::devscript(&mut app, &host, "edit", &data.join("note.txt").display().to_string());
    settle(&mut app, &host, "the text document", |a| {
        matches!(a.editors.active_doc().map(|d| &d.kind), Some(DocKind::Text(_)))
    });
    assert!(!press(&mut app, r), "typing in a text document");

    // Back to the PDF: script verbs, and copy reaches the host.
    editors::open_path(&mut app, &doc, editors::io::Want::Auto);
    assert!(pdf_ready(&app));
    for v in ["pdf=page=2", "pdf=zoom=150", "pdf=select=all", "pdf=copy"] {
        editors::devscript(&mut app, &host, "editor", v);
    }
    assert_eq!(active_pdf(&mut app).page(), 1);
    assert!((active_pdf(&mut app).percent() - 150.0).abs() < 0.01);
    settle(&mut app, &host, "the copied text", |_| !host.copied.lock().unwrap().is_empty());
    let copied = host.copied.lock().unwrap().clone();
    assert!(copied[0].contains("Hello world") && copied[0].contains("Third page"), "{copied:?}");

    // Closing remembers the position for this session.
    let idx = app.editors.active;
    editors::close(&mut app, idx, false);
    assert!(app.editors.docs.iter().all(|d| d.path != doc));

    // `[open] pdf_in_viewer`: Enter on the PDF opens the viewer (where it was left).
    settle(&mut app, &host, "the listing", |a| a.ws.pane().entry("doc.pdf").is_some());
    app.config.open.pdf_in_viewer = true;
    let pane = app.ws.pane().id;
    soyuz_appcore::ops::open_entry(&mut app, pane, "doc.pdf");
    settle(&mut app, &host, "the PDF again", pdf_ready);
    assert_eq!(active_pdf(&mut app).page(), 1, "the remembered page");
    assert_eq!(active_pdf(&mut app).rotation(), 1, "and rotation");
    assert!(!active_pdf(&mut app).is_dirty());
    assert!(Path::new(&doc).is_file());
    println!("pdf viewer: ok");
}

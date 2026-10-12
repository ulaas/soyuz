//! Headless tests of T4: the preview pane. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! `TogglePreviewPane` is Alt+P; `PreviewPageDown` Alt+PageDown, `PreviewScrollUp`
//! Alt+Shift+Up. `ToggleHexPreview` has no default key: the tests bind it to F7.

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use soyuz_core::Location;
use soyuz_core::config::Config;
use soyuz_tui::app::App;
use soyuz_tui::view::preview::{Body, PreviewVm};

use crate::common::{self, Run, Step, alt, ctrl, key, key_mods};

fn config() -> Config {
    let mut c = common::test_config();
    c.tui.mouse = true;
    c.tui.preview_ratio = 0.5;
    c.keys.bindings.insert("toggle_hex_preview".into(), vec!["F7".into()]);
    c
}

fn vm(app: &App) -> Option<&PreviewVm> {
    app.view().map(|v| &v.preview).filter(|p| p.visible)
}

fn body_has(app: &App, needle: &str) -> bool {
    vm(app).is_some_and(|p| p.body_text().iter().any(|l| l.contains(needle)))
}

fn top(app: &App) -> Option<usize> {
    match &vm(app)?.body {
        Body::Lines { top, .. } => Some(*top),
        _ => None,
    }
}

fn has_file(app: &App, name: &str) -> bool {
    common::labels(app).iter().any(|l| l == name)
}

#[test]
fn code_is_coloured_and_scrolls() {
    let run = common::relaunch(module_path!(), "code_is_coloured_and_scrolls", |data| {
        let mut src = String::from("fn main() {\n    println!(\"hi\");\n}\n");
        for i in 0..300 {
            src.push_str(&format!("// line {i}\n"));
        }
        std::fs::write(data.join("main.rs"), src).unwrap();
    });
    if let Run::Child(data) = run {
        code(data);
    }
}

fn code(data: PathBuf) {
    let mut app = common::new_app(config(), &data);
    let steps = vec![
        Step::new("the listing", |a| has_file(a, "main.rs"), key(KeyCode::Home)),
        Step::new("the cursor on main.rs", |a| common::cursor(a).as_deref() == Some("main.rs"), alt('p')),
        Step::new(
            "the code with syntax colours",
            |a| {
                body_has(a, "fn main() {")
                    && matches!(&vm(a).unwrap().body, Body::Lines { lines, gutter, .. }
                        if *gutter > 0 && lines.iter().flat_map(|l| &l.segs).any(|s| s.color.is_some()))
                    && vm(a).unwrap().info.iter().any(|(_, v)| v.contains("Rust"))
            },
            key_mods(KeyCode::PageDown, KeyModifiers::ALT),
        ),
        Step::new(
            "a page further",
            |a| top(a).is_some_and(|t| t > 3),
            key_mods(KeyCode::Up, KeyModifiers::ALT | KeyModifiers::SHIFT),
        ),
        Step::new("one line back", |a| top(a).is_some_and(|t| t > 2), Event::Mouse(press(70, 10))),
        Step::new("the pane focused by the click", |a| vm(a).is_some_and(|p| p.focused), key(KeyCode::Home)),
        Step::new("Home scrolls to the top", |a| top(a) == Some(0), key(KeyCode::End)),
        Step::new("End scrolls to the end", |a| body_has(a, "line 299"), key(KeyCode::Esc)),
        Step::new("Esc gives the keyboard back", |a| vm(a).is_some_and(|p| !p.focused), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

fn press(column: u16, row: u16) -> MouseEvent {
    MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column, row, modifiers: KeyModifiers::NONE }
}

#[test]
fn hex_toggle_and_hidden_pane_loads_nothing() {
    let run = common::relaunch(module_path!(), "hex_toggle_and_hidden_pane_loads_nothing", |data| {
        std::fs::write(data.join("notes.txt"), "Hello, preview!\n").unwrap();
    });
    if let Run::Child(data) = run {
        hex(data);
    }
}

fn hex(data: PathBuf) {
    let mut app = common::new_app(config(), &data);
    let loads = |a: &App| soyuz_tui::input::preview::target(a).is_some();
    let steps = vec![
        Step::new("the listing", |a| has_file(a, "notes.txt"), key(KeyCode::Home)),
        Step::new(
            "the cursor on notes.txt, nothing loading while hidden",
            move |a| common::cursor(a).as_deref() == Some("notes.txt") && vm(a).is_none() && !loads(a),
            key(KeyCode::Down),
        ),
        Step::new("still nothing", move |a| !loads(a) && a.regions().preview.is_empty(), alt('p')),
        Step::new("the text", |a| body_has(a, "Hello, preview!"), key(KeyCode::F(7))),
        Step::new(
            "the bytes",
            |a| body_has(a, "48 65 6C 6C 6F 2C 20 70") && body_has(a, "Hello, preview!") && vm(a).unwrap().hex,
            key(KeyCode::F(7)),
        ),
        Step::new(
            "text again",
            |a| !vm(a).unwrap().hex && body_has(a, "Hello, preview!") && !body_has(a, "48 65"),
            alt('p'),
        ),
        Step::new("closed: let go", move |a| vm(a).is_none() && !loads(a), ctrl('q')),
    ];
    // Wide enough for the ASCII column of the hex view (beside the 40-column list).
    common::run_script(&mut app, steps, 200, 24);
    common::ok();
}

#[test]
fn zip_listing_opens_the_archive() {
    let run = common::relaunch(module_path!(), "zip_listing_opens_the_archive", |data| {
        write_zip(&data.join("a.zip"), &[("docs/", b""), ("docs/readme.txt", b"read me"), ("top.txt", b"top")]);
    });
    if let Run::Child(data) = run {
        zip(data);
    }
}

fn zip(data: PathBuf) {
    let mut app = common::new_app(config(), &data);
    let archive = data.join("a.zip");
    let steps = vec![
        Step::new("the listing", |a| has_file(a, "a.zip"), key(KeyCode::Home)),
        Step::new("the cursor on a.zip", |a| common::cursor(a).as_deref() == Some("a.zip"), alt('p')),
        Step::new(
            "the archive listing",
            |a| {
                matches!(&vm(a).map(|p| &p.body), Some(Body::Archive { total: 3, rows, .. })
                    if rows.iter().any(|r| r.name == "readme.txt" && r.depth == 1))
            },
            Event::Mouse(press(60, 12)),
        ),
        Step::new("focused", |a| vm(a).is_some_and(|p| p.focused), key(KeyCode::Down)),
        Step::new(
            "the cursor on the second entry",
            |a| matches!(&vm(a).map(|p| &p.body), Some(Body::Archive { list, .. }) if list.cursor == 1),
            key(KeyCode::Enter),
        ),
        Step::new(
            "the pane inside the archive",
            move |a| {
                a.core.ws.pane().location == Location::Archive { archive: archive.clone(), inner: "docs".into() }
                    && has_file(a, "readme.txt")
            },
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 80, 24);
    common::ok();
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// A zip with stored (uncompressed) entries; names ending in `/` are folders.
fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let len = data.len() as u32;
        let n = name.len() as u16;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        for v in [20u16, 0, 0, 0, 0x21] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, len, len] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        for v in [20u16, 20, 0, 0, 0, 0x21] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, len, len] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        for v in [n, 0, 0, 0, 0] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        let ext: u32 = if name.ends_with('/') { 0x10 } else { 0 };
        central.extend_from_slice(&ext.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    let cd_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    let count = entries.len() as u16;
    for v in [0u16, 0, count, count] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    std::fs::write(path, out).unwrap();
}

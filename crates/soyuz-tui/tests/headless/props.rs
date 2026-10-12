//! Headless tests of T3: properties and attributes. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_tui::app::App;

use crate::common::{self, Run, Step, alt, at, ctrl, cursor, dialog_title, key, key_mods, labels, wait_jobs_idle};

/// The rows of the dialog drawn in the last frame.
fn rows(app: &App) -> Vec<String> {
    app.view()
        .and_then(|v| v.dialog.as_ref())
        .map(|d| d.list.iter().map(|r| r.text.clone()).collect())
        .unwrap_or_default()
}

/// The digits of `s`.
fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// The row that starts with `label`.
fn row(app: &App, label: &str) -> Option<String> {
    rows(app).into_iter().find(|r| r.starts_with(label))
}

#[test]
fn properties_shows_the_folder_size() {
    let run = common::relaunch(module_path!(), "properties_shows_the_folder_size", |data| {
        let sub = data.join("sub");
        std::fs::create_dir_all(sub.join("deep")).unwrap();
        std::fs::write(sub.join("a.bin"), [0u8; 1500]).unwrap();
        std::fs::write(sub.join("deep").join("b.bin"), [0u8; 548]).unwrap();
    });
    match run {
        Run::Child(data) => folder_size(data),
        Run::Parent(_) => {}
    }
}

fn folder_size(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d) && labels(a) == ["sub/"], key(KeyCode::Down)),
        Step::new(
            "the cursor on sub/",
            |a| cursor(a).as_deref() == Some("sub/"),
            key_mods(KeyCode::Enter, KeyModifiers::ALT),
        ),
        // 1500 + 548 bytes in two files and one folder, counted in the background.
        Step::new(
            "the size of the folder",
            |a| {
                dialog_title(a).is_some_and(|t| t.contains("sub"))
                    && row(a, "Size ").is_some_and(|r| digits(&r).ends_with("2048") && !r.contains('\u{2026}'))
                    && row(a, "Contains").is_some_and(|r| r.contains("2 files") && r.contains("1 folder"))
                    && row(a, "Type").is_some()
                    && row(a, "Location").is_some_and(|r| r.contains("data"))
            },
            key(KeyCode::Esc),
        ),
        Step::new("closed", |a| dialog_title(a).is_none(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

#[cfg(windows)]
fn flag_on(p: &Path) -> bool {
    std::fs::metadata(p).unwrap().permissions().readonly()
}

#[cfg(unix)]
fn flag_on(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777 == 0o600
}

#[test]
fn attributes_change_a_file_and_undo_restores_it() {
    let run = common::relaunch(module_path!(), "attributes_change_a_file_and_undo_restores_it", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(data.join("a.txt"), std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    });
    match run {
        Run::Child(data) => attributes(data),
        Run::Parent(p) => assert!(!flag_on(&p.data.join("a.txt")), "undo restored the file"),
    }
}

fn attributes(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d = data.clone();
    let (f1, f2) = (data.join("a.txt"), data.join("a.txt"));
    let mut steps = vec![
        Step::new("the listing", move |a| at(a, &d) && labels(a) == ["a.txt"], key(KeyCode::Down)),
        Step::new(
            "the cursor on a.txt",
            |a| cursor(a).as_deref() == Some("a.txt"),
            key_mods(KeyCode::Enter, KeyModifiers::ALT),
        ),
        // The properties' "Attributes…" button opens the attributes dialog.
        Step::new("the properties", |a| row(a, "Size ").is_some(), alt('a')),
        Step::new("the attributes dialog", |a| dialog_title(a).is_some_and(|t| t.starts_with("Attributes of")), {
            if cfg!(windows) { key(KeyCode::Char(' ')) } else { key(KeyCode::Tab) }
        }),
    ];
    if cfg!(unix) {
        // macOS has two flags (locked, hidden), Linux one (immutable); the octal mode follows.
        if cfg!(target_os = "macos") {
            steps.push(Step::new("tabbing", |_| true, key(KeyCode::Tab)));
        }
        for _ in 0..3 {
            steps.push(Step::new("clearing the mode", |_| true, key(KeyCode::Backspace)));
        }
        common::type_text(&mut steps, "600");
    }
    steps.extend([
        Step::new(
            "Apply enabled",
            |a| a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.buttons[0].enabled && d.error.is_none()),
            alt('a'),
        ),
        Step::new(
            "the change on disk",
            move |a| flag_on(&f1) && dialog_title(a).is_none() && wait_jobs_idle(a),
            ctrl('z'),
        ),
        Step::new("undo restored it", move |a| !flag_on(&f2) && wait_jobs_idle(a), ctrl('q')),
    ]);
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

#[test]
fn properties_checksums_compute_compare_and_copy() {
    let run = common::relaunch(module_path!(), "properties_checksums_compute_compare_and_copy", |data| {
        std::fs::write(data.join("abc.txt"), b"abc").unwrap();
    });
    if let Run::Child(data) = run {
        checksums(data);
    }
}

fn checksums(data: PathBuf) {
    use crossterm::event::Event;
    const MD5: &str = "900150983cd24fb0d6963f7d28e17f72";
    const SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let mut app = common::new_app(common::test_config(), &data);
    // Copies never reach the system clipboard.
    app.host.set_local_clipboard(false);
    let d = data.clone();
    let body = |a: &App| a.view().and_then(|v| v.dialog.as_ref()).map(|d| d.body.join("\n")).unwrap_or_default();
    let steps = vec![
        Step::new("the listing", move |a| at(a, &d) && labels(a) == ["abc.txt"], key(KeyCode::Down)),
        Step::new(
            "the cursor on abc.txt",
            |a| cursor(a).as_deref() == Some("abc.txt"),
            key_mods(KeyCode::Enter, KeyModifiers::ALT),
        ),
        Step::new("the properties", |a| row(a, "Size ").is_some() && row(a, "MD5").is_none(), alt('k')),
        // The checksum section: the algorithms not computed yet, MD5 picked.
        Step::new(
            "the checksum section",
            |a| {
                row(a, "MD5").is_some_and(|r| r.contains(soyuz_appcore::tr!("dialogs-hash-not-computed")))
                    && a.view().and_then(|v| v.dialog.as_ref()).is_some_and(|d| d.fields.len() == 2)
            },
            alt('o'),
        ),
        Step::new("MD5 computed", |a| row(a, "MD5").is_some_and(|r| r.contains(MD5)), key(KeyCode::Tab)),
        // A pasted `sha256sum` line starts SHA-256 (and BLAKE3) and matches.
        Step::new("on Compare", |_| true, Event::Paste(format!("{}  abc.txt", SHA256.to_uppercase()))),
        Step::new(
            "the match",
            move |a| {
                row(a, "SHA-256").is_some_and(|r| r.contains(SHA256))
                    && body(a).contains("SHA-256")
                    && !body(a).contains("BLAKE3")
            },
            alt('y'),
        ),
        Step::new("MD5 copied", |a| a.host.last_copy().as_deref() == Some(MD5), key(KeyCode::Esc)),
        Step::new("closed", |a| dialog_title(a).is_none(), ctrl('q')),
    ];
    common::run_script(&mut app, steps, 100, 30);
    common::ok();
}

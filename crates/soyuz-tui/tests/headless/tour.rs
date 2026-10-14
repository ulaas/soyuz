//! Phase T8 (P4): the keyboard tour, in process (`App::with_script`, `common::run_scripted`):
//! the story of `soyuz-app/tests/keyboard_only.rs` and more (browse, file operations, a
//! dialog, the context menu, tabs, dual panes, the palette, the jobs panel, search,
//! settings, quit) in en-US and tr-TR, with the Defaults and Legacy key map overlays (one
//! relaunch each: the overlay is process-wide), keys only (`keys:` steps; `select:` /
//! `cd:` put the cursor, never the mouse). Each step waits for its effect with
//! `wait_for:` (no fixed sleeps); the checks are the runner's results
//! (`Runner::failures`) in the child and the files on disk in the parent.
//!
//! Also here: `--screen-reader` (`App::sr_override`) and the palette's toggle.

use std::path::{Path, PathBuf};

use soyuz_core::config::{Action, Config, KeyChord, KeyContext, KeyPreset};
use soyuz_tui::app::App;
use soyuz_tui::overlay::Overlay;

use crate::common::{self, Run, SCRIPT_CAP};

/// The chords that differ between the overlays: the Defaults use the presets' own chords,
/// the Legacy run the alternatives a legacy terminal can send (`crate::overlay::ALTERNATIVES`).
struct Chords {
    palette: &'static str,
    settings: &'static str,
    next_tab: &'static str,
    search: &'static str,
}

const DEFAULTS: Chords =
    Chords { palette: "Ctrl+Shift+P", settings: "Ctrl+Comma", next_tab: "Ctrl+Tab", search: "Ctrl+Shift+F" };
const LEGACY: Chords =
    Chords { palette: "Alt+Shift+P", settings: "Alt+Comma", next_tab: "Ctrl+PageDown", search: "Alt+Shift+F" };

/// One tour run: language, palette query, the name the palette's "New folder" gives.
pub(crate) struct Lang {
    tag: &'static str,
    query: &'static str,
    new_folder: &'static str,
}

pub(crate) const EN: Lang = Lang { tag: "en-US", query: "new folder", new_folder: "New folder" };
pub(crate) const TR: Lang = Lang { tag: "tr-TR", query: "yeni klas", new_folder: "Yeni klasör" };

fn setup(data: &Path) {
    std::fs::create_dir(data.join("sub")).unwrap();
    for f in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(data.join(f), f.as_bytes()).unwrap();
    }
}

/// The tour as a `SOYUZ_SCRIPT`.
fn script(data: &Path, lang: &Lang, k: &Chords) -> String {
    let name = data.file_name().unwrap().to_string_lossy().into_owned();
    let sub = data.join("sub");
    let parent = data.parent().and_then(Path::file_name).unwrap().to_string_lossy().into_owned();
    let keys = |chord: &str| format!("keys:{{{chord}}}");
    [
        format!("expect:lang={}", lang.tag),
        "wait_for:has=sub".into(),
        "expect:focus=List".into(),
        // Browse: open a folder with Enter, back up with Backspace.
        "select:sub".into(),
        keys("Enter"),
        "wait_for:cwd=sub".into(),
        keys("Backspace"),
        format!("wait_for:cwd={name}"),
        "wait_for:has=a.txt".into(),
        // Duplicate a.txt (no clipboard involved).
        "select:a.txt".into(),
        keys("Ctrl+D"),
        "wait_for:has=a (2).txt".into(),
        // Inline rename: F2 selects the stem, typing replaces it.
        "select:b.txt".into(),
        keys("F2"),
        "wait_for:focus=Rename".into(),
        "keys:renamed{Enter}".into(),
        "wait_for:has=renamed.txt".into(),
        format!("expect:exists={}", data.join("renamed.txt").display()),
        // A dialog: permanent delete, confirmed with Enter.
        "select:c.txt".into(),
        keys("Shift+Delete"),
        "wait_for:dialog=open".into(),
        "wait_for:focus=Dialog".into(),
        keys("Enter"),
        "wait_for:dialog=closed".into(),
        format!("wait_for:missing={}", data.join("c.txt").display()),
        // The context menu.
        keys("Shift+F10"),
        "wait_for:focus=Menu".into(),
        keys("Escape"),
        "wait_for:focus=List".into(),
        // Tabs: a new tab goes into sub, the next-tab chord comes back, Ctrl+W closes.
        keys("Ctrl+T"),
        format!("cd:{}", sub.display()),
        "wait_for:cwd=sub".into(),
        keys(k.next_tab),
        format!("wait_for:cwd={name}"),
        keys(k.next_tab),
        "wait_for:cwd=sub".into(),
        keys("Ctrl+W"),
        format!("wait_for:cwd={name}"),
        // Dual panes: F9 shows the second pane; Tab moves into it, where Backspace goes up
        // (only that pane), and Tab comes back; F9 again shows one pane.
        keys("F9"),
        keys("Tab"),
        keys("Backspace"),
        format!("wait_for:cwd={parent}"),
        keys("Tab"),
        format!("wait_for:cwd={name}"),
        keys("F9"),
        // The command palette: a query in the UI language, Enter runs "New folder", which
        // starts an inline rename; Enter keeps the name.
        keys(k.palette),
        "wait_for:palette=open".into(),
        "wait_for:focus=Palette".into(),
        format!("keys:{}", lang.query),
        format!("wait_for:screen~{}", lang.new_folder),
        keys("Enter"),
        "wait_for:palette=closed".into(),
        "wait_for:focus=Rename".into(),
        keys("Enter"),
        "wait_for:focus=List".into(),
        format!("wait_for:has={}", lang.new_folder),
        // The jobs panel.
        keys("Alt+J"),
        "wait_for:focus=Jobs".into(),
        keys("Escape"),
        "wait_for:focus=List".into(),
        // Search: the search bar takes the keyboard; Esc gives it back.
        keys(k.search),
        "wait_for:focus=Search".into(),
        keys("Escape"),
        "wait_for:focus=List".into(),
        // Settings.
        keys(k.settings),
        "wait_for:settings=open".into(),
        "wait_for:focus=Settings".into(),
        keys("Escape"),
        "wait_for:settings=closed".into(),
        "wait_for:focus=List".into(),
        // Quit with the keyboard.
        keys("Ctrl+Q"),
    ]
    .join(";")
}

/// The child: install the overlay, run the tour, check every expectation held.
fn tour_child(data: PathBuf, lang: &Lang, overlay: Overlay, edit: fn(&mut Config)) {
    // Before the core builds its key map (the overlay hook is set once per process).
    soyuz_appcore::keys::set_keymap_overlay(overlay.function());
    let k = if overlay == Overlay::Legacy { &LEGACY } else { &DEFAULTS };
    let mut c = common::test_config();
    c.general.language = lang.tag.into();
    c.keys.preset = KeyPreset::Explorer;
    edit(&mut c);
    let mut app = common::new_app(c, &data).with_script(&script(&data, lang, k));
    // The chords the tour sends are the overlay's.
    let bound = |a: Action, chord: &str| {
        let chord: KeyChord = chord.parse().unwrap();
        assert_eq!(app.core.keymap.lookup(KeyContext::FileList, chord), Some(&a), "{overlay:?}: {chord}");
    };
    bound(Action::CommandPalette, k.palette);
    bound(Action::Settings, k.settings);
    bound(Action::NextTab, k.next_tab);
    bound(Action::Search, k.search);
    let (terminal, _io) = common::run_scripted(&mut app, 120, 32, SCRIPT_CAP);
    let failures = app.script().map(|r| r.failures()).unwrap_or_default();
    assert!(
        failures.is_empty(),
        "{} / {overlay:?}: failed expectations:\n{}\n\nlast screen:\n{}",
        lang.tag,
        failures.join("\n"),
        common::screen(&terminal)
    );
    assert!(app.script().is_some_and(|r| r.is_done()), "the whole tour ran");
    common::ok();
}

/// On disk: a.txt duplicated, b.txt renamed, c.txt deleted, a folder created.
fn check_disk(data: &Path, lang: &Lang) {
    let names: Vec<String> =
        std::fs::read_dir(data).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    let tag = lang.tag;
    assert!(names.iter().filter(|n| n.starts_with('a') && n.ends_with(".txt")).count() >= 2, "{tag}: {names:?}");
    assert!(names.contains(&"renamed.txt".to_string()) && !names.contains(&"b.txt".to_string()), "{tag}: {names:?}");
    assert!(!names.contains(&"c.txt".to_string()), "{tag}: {names:?}");
    assert!(data.join(lang.new_folder).is_dir(), "{tag}: no new folder: {names:?}");
}

/// Run the tour as test `name` of `module` (pass `module_path!()`) in `lang` with
/// `overlay`; `edit` changes the settings first (a variant, e.g. screen-reader mode:
/// `|c| c.tui.screen_reader = true`). Other test files call it for their variants.
pub(crate) fn run_tour(module: &str, name: &str, lang: &Lang, overlay: Overlay, edit: fn(&mut Config)) {
    match common::relaunch(module, name, setup) {
        Run::Child(data) => tour_child(data, lang, overlay, edit),
        Run::Parent(p) => check_disk(&p.data, lang),
    }
}

fn tour(name: &str, lang: &Lang, overlay: Overlay) {
    run_tour(module_path!(), name, lang, overlay, |_| {});
}

#[test]
fn keyboard_tour_english_defaults() {
    tour("keyboard_tour_english_defaults", &EN, Overlay::Defaults);
}

#[test]
fn keyboard_tour_english_legacy() {
    tour("keyboard_tour_english_legacy", &EN, Overlay::Legacy);
}

#[test]
fn keyboard_tour_turkish_defaults() {
    tour("keyboard_tour_turkish_defaults", &TR, Overlay::Defaults);
}

#[test]
fn keyboard_tour_turkish_legacy() {
    tour("keyboard_tour_turkish_legacy", &TR, Overlay::Legacy);
}

// ---- --screen-reader ----

/// Run `script` with `--screen-reader` on (`App::sr_override`) and `[tui] screen_reader`
/// set to `setting`; returns the app after the script quit it.
fn sr_run(data: &Path, setting: bool, script: &str) -> App {
    let mut c = common::test_config();
    c.tui.screen_reader = setting;
    let mut app = common::new_app(c, data).with_script(script);
    app.sr_override = true;
    assert!(app.screen_reader());
    let _ = common::run_scripted(&mut app, 80, 20, SCRIPT_CAP);
    let failures = app.script().map(|r| r.failures()).unwrap_or_default();
    assert!(failures.is_empty(), "failed expectations:\n{}", failures.join("\n"));
    app
}

/// `--screen-reader` with the setting off: "Toggle screen-reader mode" (the action, as the
/// palette and key bindings run it) ends the mode for the session, saves nothing new and
/// says "off".
#[test]
fn the_toggle_ends_a_screen_reader_override() {
    match common::relaunch(module_path!(), "the_toggle_ends_a_screen_reader_override", setup) {
        Run::Child(data) => {
            let app = sr_run(
                &data,
                false,
                "wait_for:has=sub;toggle_screen_reader;\
                 wait_for:screen~Screen-reader mode off;expect:screen~─;quit",
            );
            assert!(!app.sr_override, "the override is gone");
            assert!(!app.core.config.tui.screen_reader, "the setting stays off");
            assert!(!app.screen_reader());
            common::ok();
        }
        Run::Parent(p) => {
            let saved = std::fs::read_to_string(p.config_dir().join("config.toml")).unwrap_or_default();
            assert!(!saved.contains("screen_reader = true"), "{saved}");
        }
    }
}

/// `--screen-reader` with the setting on too: the toggle turns both off.
#[test]
fn the_toggle_ends_a_screen_reader_override_and_the_setting() {
    match common::relaunch(module_path!(), "the_toggle_ends_a_screen_reader_override_and_the_setting", setup) {
        Run::Child(data) => {
            let app = sr_run(
                &data,
                true,
                "wait_for:has=sub;toggle_screen_reader;wait_for:screen~Screen-reader mode off;quit",
            );
            assert!(!app.sr_override && !app.core.config.tui.screen_reader && !app.screen_reader());
            common::ok();
        }
        Run::Parent(_) => {}
    }
}

/// `--screen-reader` is never saved: a settings save in the session keeps the setting off.
#[test]
fn a_screen_reader_override_is_never_saved() {
    match common::relaunch(module_path!(), "a_screen_reader_override_is_never_saved", setup) {
        Run::Child(data) => {
            let app = sr_run(&data, false, "wait_for:has=sub;config:appearance.high_contrast=true;wait:50;quit");
            assert!(app.sr_override && app.screen_reader() && !app.core.config.tui.screen_reader);
            common::ok();
        }
        Run::Parent(p) => {
            let saved = std::fs::read_to_string(p.config_dir().join("config.toml")).unwrap_or_default();
            assert!(!saved.contains("screen_reader = true"), "{saved}");
        }
    }
}

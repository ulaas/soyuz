//! Colour tags and the other decorations the GUI shows: the terminal UI loads `tags.toml`
//! (the core's per-frame service pumps), draws the tag dots after a name, and writes tags
//! changed in the session back without losing the others.

use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use soyuz_appcore::services::tags::{self, TagStore};
use soyuz_core::config::PORTABLE_DATA_DIR;
use soyuz_tui::app::App;
use soyuz_tui::view::PaneLine;

use crate::common::{self, Run, Step, at, ctrl, key};

/// Red and Blue (`[tags]` order).
const RED: u8 = 0;
const BLUE: u8 = 5;
const GREEN: u8 = 3;

/// The tag mask of the row named `name` in the focused pane's last frame.
fn row_tags(a: &App, name: &str) -> Option<u8> {
    a.view()?.focused_pane()?.lines.iter().find_map(|l| match l {
        PaneLine::Item(r) if r.name == name => Some(r.tags),
        _ => None,
    })
}

#[test]
fn tui_shows_and_keeps_colour_tags() {
    let run = common::relaunch(module_path!(), "tui_shows_and_keeps_colour_tags", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        std::fs::write(data.join("b.txt"), b"b").unwrap();
        let mut store = TagStore::default();
        store.set(&data.join("a.txt"), (1 << RED) | (1 << BLUE));
        let cfg = data.parent().unwrap().join(PORTABLE_DATA_DIR);
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("tags.toml"), store.to_toml()).unwrap();
    });
    match run {
        Run::Child(data) => run_tags(data),
        Run::Parent(r) => {
            let text = std::fs::read_to_string(r.config_dir().join("tags.toml")).expect("tags.toml");
            let store = TagStore::from_toml(&text).expect("tags.toml parses");
            assert_eq!(store.get(&r.data.join("a.txt")), (1 << RED) | (1 << BLUE), "{text}");
            assert_eq!(store.get(&r.data.join("b.txt")), 1 << GREEN, "{text}");
        }
    }
}

fn run_tags(data: PathBuf) {
    let mut app = common::new_app(common::test_config(), &data);
    let d1 = data.clone();
    let both = (1 << RED) | (1 << BLUE);
    let steps = vec![
        Step::new(
            "a.txt with its tags from tags.toml",
            move |a| at(a, &d1) && row_tags(a, "a.txt") == Some(both),
            key(KeyCode::Down),
        ),
        Step::new("b.txt untagged", |a| row_tags(a, "b.txt") == Some(0), ctrl('q')),
    ];
    let (terminal, _) = common::run_script(&mut app, steps, 80, 24);
    let screen = common::screen(&terminal);
    // ASCII icon set (test config): a dot is `o`, one per tag after the name.
    assert!(screen.lines().any(|l| l.contains("a.txt oo")), "no tag dots after a.txt:\n{screen}");
    assert!(!screen.lines().any(|l| l.contains("b.txt o")), "dots after the untagged b.txt:\n{screen}");
    tags::toggle_many(&[Path::new(&data).join("b.txt")], GREEN);
    app.finish();
    common::ok();
}

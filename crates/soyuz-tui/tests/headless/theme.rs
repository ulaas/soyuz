//! Headless tests of step T2.2: theme, colours and icons. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::PathBuf;

use ratatui::style::Color;
use soyuz_appcore::host::ThemePref;
use soyuz_core::config;
use soyuz_tui::theme::{ColorMode, Role, TuiTheme};

use crate::common::{self, Run, Step, ctrl, labels};

/// A user theme in `<config dir>/themes`, `[theme.colors]` and the dark / light choice
/// reach the terminal UI's colours like the GUI's; the theme is cached until the host
/// flags a change, and every frame is filled with the theme's base style.
#[test]
fn tui_theme_follows_user_themes_overrides_and_variant() {
    let run = common::relaunch(module_path!(), "tui_theme_follows_user_themes_overrides_and_variant", |data| {
        std::fs::write(data.join("a.txt"), b"a").unwrap();
    });
    match run {
        Run::Child(data) => theme_child(data),
        Run::Parent(_) => {}
    }
}

fn theme_child(data: PathBuf) {
    let themes = config::config_dir().join("themes");
    std::fs::create_dir_all(&themes).unwrap();
    std::fs::write(
        themes.join("mine.toml"),
        "name = \"Mine\"\n[dark]\n\"bg.window\" = \"#101820\"\n\"text.primary\" = \"#F0F0E0\"\n",
    )
    .unwrap();
    let mut c = common::test_config();
    c.appearance.theme = "dark".into();
    c.appearance.color_theme = "mine".into();
    c.theme.colors.insert("danger".into(), "#FF0010".into());
    let mut app = common::new_app(c, &data);
    app.core.ui.theme = ThemePref::Dark;
    let tui = app.core.config.tui.clone();

    let t = TuiTheme::for_core_in(&app.core, &tui, ColorMode::Truecolor);
    assert!(t.is_dark());
    assert_eq!(t.base().bg, Some(Color::Rgb(0x10, 0x18, 0x20)), "the user theme's window");
    assert_eq!(t.style(Role::Text).fg, Some(Color::Rgb(0xF0, 0xF0, 0xE0)));
    assert_eq!(t.style(Role::Danger).fg, Some(Color::Rgb(0xFF, 0x00, 0x10)), "[theme.colors] override");
    // Orbit's folder colour: the user theme only replaced two tokens.
    assert_eq!(t.style(Role::Folder).fg, Some(Color::Rgb(0xF2, 0xB8, 0x4B)));

    app.core.ui.theme = ThemePref::Light;
    let l = TuiTheme::for_core_in(&app.core, &tui, ColorMode::Truecolor);
    assert!(!l.is_dark());
    assert_eq!(l.base().bg, Some(Color::Rgb(0xF6, 0xF7, 0xF9)), "Orbit Light (the theme has no [light])");
    assert_eq!(l.style(Role::Danger).fg, Some(Color::Rgb(0xFF, 0x00, 0x10)), "overrides apply to both variants");
    app.core.ui.theme = ThemePref::Dark;

    let n = TuiTheme::for_core_in(&app.core, &tui, ColorMode::Mono);
    assert_eq!((n.style(Role::Folder).fg, n.base().bg), (None, Some(Color::Reset)), "NO_COLOR look");

    // The cache: unchanged until the host flags a change.
    let first = TuiTheme::cached(&app.core, true);
    app.core.config.theme.colors.insert("text.primary".into(), "#000000".into());
    assert_eq!(TuiTheme::cached(&app.core, false), first, "no rebuild without the dirty flag");
    assert_eq!(TuiTheme::cached(&app.core, true), TuiTheme::for_core(&app.core, &tui));
    app.core.config.theme.colors.remove("text.primary");
    let _ = TuiTheme::cached(&app.core, true);

    // A real frame: the screen is filled with the theme's base style.
    let d = data.clone();
    let steps = vec![Step::new("the listing", move |a| common::at(a, &d) && labels(a) == ["a.txt"], ctrl('q'))];
    let (terminal, _) = common::run_script(&mut app, steps, 60, 12);
    let base = TuiTheme::cached(&app.core, false).base();
    let buf = terminal.backend().buffer();
    assert_eq!(buf[(59, 5)].bg, base.bg.unwrap_or(Color::Reset), "an empty cell has the window background");
    app.finish();
    common::ok();
}

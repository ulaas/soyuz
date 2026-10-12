//! Headless tests of step T2.2: theme, colours and icons. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use soyuz_appcore::host::ThemePref;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::state::session::FavoriteEntry;
use soyuz_core::config::{self, SidebarSection, TuiBorders, TuiIcons};
use soyuz_tui::app::{App, NullIo};
use soyuz_tui::icons::IconSet;
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

fn fixture(data: &Path) {
    for (dir, file) in [("alpha", "a1.txt"), ("beta", "b1.txt")] {
        std::fs::create_dir_all(data.join(dir)).unwrap();
        std::fs::write(data.join(dir).join(file), b"x").unwrap();
    }
    std::fs::write(data.join("top.txt"), b"t").unwrap();
}

/// Frames with `events` first, then empty frames until `done` holds (30 s at most).
fn frames_until(
    app: &mut App,
    t: &mut Terminal<TestBackend>,
    events: &[Event],
    what: &str,
    done: impl Fn(&App) -> bool,
) {
    let mut io = NullIo::default();
    app.frame(t, events, &mut io).unwrap();
    let start = Instant::now();
    while !done(app) {
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
        app.frame(t, &[], &mut io).unwrap();
    }
}

fn key(code: KeyCode, m: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, m))
}

fn screen(t: &Terminal<TestBackend>) -> Vec<String> {
    let buf = t.backend().buffer();
    (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()).collect()
}

/// `[tui] icons` and `borders` reach every view: the default draws Unicode icons and
/// rounded boxes; `icons = "ascii"` with `borders = "ascii"` draws a pure ASCII screen
/// (tab strip, sidebar, dual panes with their title lines and splitter, a toast, the
/// palette).
#[test]
fn tui_icons_and_borders_follow_the_config() {
    match common::relaunch(module_path!(), "tui_icons_and_borders_follow_the_config", fixture) {
        Run::Child(data) => icons_and_borders(data),
        Run::Parent(_) => {}
    }
}

fn icons_and_borders(data: PathBuf) {
    // The default: Unicode icons.
    let mut c = common::test_config();
    c.tui.icons = TuiIcons::default();
    let mut app = common::new_app(c, &data);
    let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let d = data.clone();
    frames_until(&mut app, &mut t, &[], "the listing", move |a| common::at(a, &d) && labels(a).len() == 3);
    assert_eq!(TuiTheme::cached(&app.core, false).icons(), IconSet::Unicode);
    let s = screen(&t).join(
        "
",
    );
    assert!(s.contains(" ▸ ") && s.contains("■"), "Unicode separators and folder icons: {s}");
    app.finish();
    drop(app);

    let mut c = common::test_config();
    c.tui.icons = TuiIcons::Ascii;
    c.tui.borders = TuiBorders::Ascii;
    c.layout.sidebar = true;
    c.sidebar.sections = vec![SidebarSection::Favorites];
    let mut app = common::new_app(c, &data);
    // Only folders of the sandbox, never the user's own.
    app.core.ui.sidebar.favorites = vec![FavoriteEntry { name: "Alpha".into(), path: data.join("alpha") }];
    app.core.ui.sidebar.favorites_customized = true;
    let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let d = data.clone();
    frames_until(&mut app, &mut t, &[], "the listing", move |a| common::at(a, &d) && labels(a).len() == 3);
    // A second tab (Ctrl+Enter on alpha/ opens it in the background): the tab strip.
    frames_until(&mut app, &mut t, &[key(KeyCode::Down, KeyModifiers::NONE)], "the cursor", |a| {
        common::cursor(a).as_deref() == Some("alpha/")
    });
    frames_until(&mut app, &mut t, &[key(KeyCode::Enter, KeyModifiers::CONTROL)], "two tabs", |a| {
        a.core.ws.tabs.len() == 2 && !a.regions().tabs.is_empty()
    });
    frames_until(&mut app, &mut t, &[key(KeyCode::F(9), KeyModifiers::NONE)], "dual panes", |a| {
        a.regions().panes.len() == 2 && !a.regions().titles.is_empty() && !a.regions().splitters.is_empty()
    });
    // Typographic punctuation from a translation-like text: replaced too.
    app.core.ui.toasts.push(Toast::info("Copied “a.txt” — 3 items…").sticky());
    frames_until(&mut app, &mut t, &[], "the toast", |a| a.view().is_some_and(|v| !v.toasts.lines.is_empty()));
    let all = screen(&t).join(
        "
",
    );
    assert!(all.contains("Copied \"a.txt\" - 3 items~"), "{all}");
    frames_until(&mut app, &mut t, &[key(KeyCode::Char('k'), KeyModifiers::CONTROL)], "the palette", |a| {
        a.view().is_some_and(|v| v.palette.open && !v.palette.lines.is_empty() && !v.toasts.lines.is_empty())
    });
    let lines = screen(&t);
    let all = lines.join(
        "
",
    );
    for (y, l) in lines.iter().enumerate() {
        assert!(
            l.is_ascii(),
            "row {y} is not ASCII: {l:?}
{all}"
        );
    }
    // The boxes and lines are there, in ASCII.
    assert!(all.contains("+-") && all.contains('|') && all.contains("v Favorites"), "{all}");
    app.finish();
    common::ok();
}

//! Headless tests of step T2.6: the sidebar. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
//!
//! `FocusSidebar` is F12, the terminal UI's own default (`soyuz_tui::overlay::tui_defaults`).

use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use soyuz_appcore::core::Focus;
use soyuz_appcore::state::session::FavoriteEntry;
use soyuz_core::config::{Config, KeyContext, SidebarMode, SidebarSection};
use soyuz_tui::app::App;
use soyuz_tui::layout::HitTarget;
use soyuz_tui::view::sidebar::LineKind;

use crate::common::{self, Run, Step, at, ctrl, key};

fn config() -> Config {
    let mut c = common::test_config();
    c.layout.sidebar = true;
    c
}

/// Label of the sidebar's cursor row in the last frame.
fn sidebar_cursor(app: &App) -> Option<String> {
    app.view()?.sidebar.lines.iter().find(|l| l.cursor).map(|l| l.label.clone())
}

fn sidebar_has(app: &App, label: &str) -> bool {
    app.view().is_some_and(|v| v.sidebar.lines.iter().any(|l| l.label == label))
}

#[test]
fn sidebar_places_navigate_the_pane() {
    let run = common::relaunch(module_path!(), "sidebar_places_navigate_the_pane", |data| {
        std::fs::create_dir_all(data.join("first")).unwrap();
        std::fs::create_dir_all(data.join("target")).unwrap();
        std::fs::write(data.join("target").join("inside.txt"), b"i").unwrap();
    });
    if let Run::Child(data) = run {
        places(data);
    }
}

fn places(data: PathBuf) {
    let mut c = config();
    c.sidebar.sections = vec![SidebarSection::Favorites, SidebarSection::Trash];
    let mut app = common::new_app(c, &data);
    // Only folders of the sandbox, never the user's own.
    app.core.ui.sidebar.favorites = vec![
        FavoriteEntry { name: "First".into(), path: data.join("first") },
        FavoriteEntry { name: "Target".into(), path: data.join("target") },
    ];
    app.core.ui.sidebar.favorites_customized = true;
    let target = data.join("target");
    let d = data.clone();
    let steps = vec![
        Step::new(
            "the sidebar next to the listing",
            move |a| at(a, &d) && sidebar_has(a, "Target") && sidebar_has(a, "Favorites"),
            key(KeyCode::F(12)),
        ),
        Step::new(
            "the sidebar focused on the first place",
            |a| a.key_context() == KeyContext::Sidebar && sidebar_cursor(a).as_deref() == Some("First"),
            key(KeyCode::Down),
        ),
        Step::new("the cursor on Target", |a| sidebar_cursor(a).as_deref() == Some("Target"), key(KeyCode::Enter)),
        Step::new(
            // Like the GUI's click (`FocusPane`), the keyboard goes to the list.
            "the pane in the target folder, the list focused",
            move |a| {
                at(a, &target)
                    && common::labels(a) == ["inside.txt"]
                    && a.core.ui.focus == Focus::List
                    && a.view().is_some_and(|v| v.sidebar.lines.iter().any(|l| l.label == "Target" && l.active))
            },
            key(KeyCode::F(12)),
        ),
        Step::new(
            "back in the sidebar on the current place",
            |a| a.key_context() == KeyContext::Sidebar && sidebar_cursor(a).as_deref() == Some("Target"),
            key(KeyCode::Home),
        ),
        Step::new(
            "the cursor on the header",
            |a| sidebar_cursor(a).as_deref() == Some("Favorites"),
            key(KeyCode::Enter),
        ),
        Step::new(
            "the section collapsed",
            |a| !sidebar_has(a, "Target") && !a.core.ui.sidebar.is_open(SidebarSection::Favorites),
            key(KeyCode::Right),
        ),
        Step::new("the section open again", |a| sidebar_has(a, "Target"), key(KeyCode::Esc)),
        Step::new(
            "the list focused again",
            |a| a.key_context() == KeyContext::FileList && sidebar_cursor(a).is_none(),
            ctrl('q'),
        ),
    ];
    let (terminal, _) = common::run_script(&mut app, steps, 100, 24);
    let screen = common::screen(&terminal);
    assert!(screen.contains("v Favorites") && screen.contains("  Target"), "{screen}");
    // Every sidebar row was a click target, left of the pane.
    let rows: Vec<_> = app.hits().iter().filter(|(_, t)| matches!(t, HitTarget::SidebarRow(_))).collect();
    assert!(rows.len() >= 3, "{rows:?}");
    assert!(rows.iter().all(|(r, _)| r.x == 0 && r.width == app.regions().sidebar.width));
    assert!(app.regions().panes[0].rect.x > app.regions().sidebar.width);
    common::ok();
}

/// Sidebar row `label` is drawn at row `y` and is the click target there.
fn sidebar_row_at(app: &App, y: u16, label: &str) -> bool {
    let Some(v) = app.view() else { return false };
    let r = app.regions().sidebar;
    let Some(line) = v.sidebar.lines.get(usize::from(y.saturating_sub(r.y))) else { return false };
    line.label == label && app.hits().at(r.x + 2, y) == Some(HitTarget::SidebarRow(line.index))
}

fn click(column: u16, row: u16) -> crossterm::event::Event {
    use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// Clicks (T2.9 through `input::sidebar::click`): a place opens in the pane, a header
/// collapses its section.
#[test]
fn sidebar_clicks_open_places_and_toggle_headers() {
    let run = common::relaunch(module_path!(), "sidebar_clicks_open_places_and_toggle_headers", |data| {
        std::fs::create_dir_all(data.join("first")).unwrap();
        std::fs::create_dir_all(data.join("target")).unwrap();
        std::fs::write(data.join("target").join("inside.txt"), b"i").unwrap();
    });
    if let Run::Child(data) = run {
        clicks(data);
    }
}

fn clicks(data: PathBuf) {
    let mut c = config();
    c.sidebar.sections = vec![SidebarSection::Favorites, SidebarSection::Trash];
    let mut app = common::new_app(c, &data);
    app.core.ui.sidebar.favorites = vec![
        FavoriteEntry { name: "First".into(), path: data.join("first") },
        FavoriteEntry { name: "Target".into(), path: data.join("target") },
    ];
    app.core.ui.sidebar.favorites_customized = true;
    let target = data.join("target");
    let d = data.clone();
    // Row 0 is the location line; the sidebar starts below it.
    let steps = vec![
        Step::new("Target in the sidebar", move |a| at(a, &d) && sidebar_row_at(a, 3, "Target"), click(2, 3)),
        Step::new(
            "the pane in the target folder",
            move |a| at(a, &target) && common::labels(a) == ["inside.txt"] && sidebar_row_at(a, 1, "Favorites"),
            click(2, 1),
        ),
        Step::new(
            "the section collapsed",
            |a| !sidebar_has(a, "Target") && !a.core.ui.sidebar.is_open(SidebarSection::Favorites),
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 100, 24);
    common::ok();
}

#[test]
fn sidebar_tree_expands_and_opens() {
    let run = common::relaunch(module_path!(), "sidebar_tree_expands_and_opens", |data| {
        std::fs::create_dir_all(data.join("sub").join("inner")).unwrap();
        std::fs::write(data.join("sub").join("inner").join("deep.txt"), b"d").unwrap();
    });
    if let Run::Child(data) = run {
        tree(data);
    }
}

/// The tree row of `path` in the last frame: (depth, expanded).
fn tree_row(app: &App, path: &Path) -> Option<(usize, bool)> {
    let rows = app.core.ui.tree.rows();
    let r = rows.iter().find(|r| r.key.path == path)?;
    Some((r.depth, r.expanded))
}

fn tree(data: PathBuf) {
    let mut c = config();
    c.sidebar.mode = SidebarMode::Tree;
    let sub = data.join("sub");
    let mut app = common::new_app(c, &sub);
    let inner = sub.join("inner");
    let (s1, s2, s3) = (sub.clone(), sub.clone(), sub.clone());
    let (i1, i2) = (inner.clone(), inner.clone());
    let steps = vec![
        // The tree follows the pane down to `sub` (listing its parents on the way).
        Step::new(
            "the tree revealing the pane's folder",
            move |a| {
                at(a, &s1)
                    && a.core.ui.tree.selected.as_ref().is_some_and(|k| k.path == s1)
                    && tree_row(a, &s1).is_some_and(|(_, open)| !open)
            },
            key(KeyCode::F(12)),
        ),
        Step::new(
            "the cursor on sub",
            |a| a.key_context() == KeyContext::Sidebar && sidebar_cursor(a).as_deref() == Some("sub"),
            key(KeyCode::Right),
        ),
        Step::new(
            "sub expanded with inner below it",
            move |a| {
                let (Some((d, open)), Some((di, _))) = (tree_row(a, &s2), tree_row(a, &i1)) else { return false };
                open && di == d + 1
            },
            key(KeyCode::Down),
        ),
        // The view scrolls with the cursor.
        Step::new(
            "the cursor on inner",
            |a| {
                a.view().is_some_and(|v| {
                    v.sidebar
                        .lines
                        .iter()
                        .any(|l| l.cursor && l.label == "inner" && matches!(l.kind, LineKind::Folder { .. }))
                })
            },
            key(KeyCode::Enter),
        ),
        Step::new(
            "the pane in inner",
            move |a| at(a, &i2) && common::labels(a) == ["deep.txt"] && a.core.ui.focus == Focus::Sidebar,
            key(KeyCode::Left),
        ),
        // Left on a collapsed folder goes to its parent.
        Step::new("the cursor back on sub", |a| sidebar_cursor(a).as_deref() == Some("sub"), key(KeyCode::Left)),
        Step::new(
            "sub collapsed",
            move |a| tree_row(a, &s3).is_some_and(|(_, open)| !open) && !a.core.ui.tree.model.is_expanded(&s3),
            ctrl('q'),
        ),
    ];
    common::run_script(&mut app, steps, 100, 24);
    common::ok();
}

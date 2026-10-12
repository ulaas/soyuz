//! Keys while the sidebar has focus (step T2.6). The core's dispatcher runs first in the
//! sidebar context (global shortcuts keep working), then what it left:
//!
//! | Key | Place rows / headers | Tree rows |
//! |---|---|---|
//! | Up / Down | previous / next row (into and out of the tree) | `models::tree::keyboard` |
//! | Home / End, PageUp / PageDown | first / last row, a page | the tree's (tree-only mode) |
//! | Enter | open the place (`FocusPane` + `Navigate`, the GUI's click); toggle a header | open the folder |
//! | Right / Left | expand / collapse a header; Left on a place goes to its header | expand / collapse, child / parent |
//! | Esc | back to the list (`Action::FocusList`) | the same (the tree's Esc) |
//! | typing | the next place starting with the letter | type-ahead |
//!
//! The cursor itself (new compared with the GUI) is `crate::view::sidebar::SidebarNav`,
//! shared with the view. Enter on a place moves the keyboard to the list (the GUI's
//! `FocusPane`); on a tree row it stays in the sidebar, like the GUI's tree.

use crossterm::event::Event;
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::{AppCore, Focus};
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::smart::SmartCmd;
use soyuz_appcore::models::tree::model::Nav;
use soyuz_appcore::models::tree::{KeyInput, TreeIo};
use soyuz_appcore::services::places::drive_volumes;
use soyuz_core::config::{Action, KeyName, Mods, SidebarMode};

use super::{TuiFocus, core_input, key_context};
use crate::app::App;
use crate::view::sidebar::{Entry, Target, cursor_index, entries, nav, set_nav, sync_focus};

/// What the sidebar keeps between frames. The cursor lives in
/// `crate::view::sidebar::SidebarNav` (the view model needs it too).
#[derive(Debug, Default)]
pub struct State {}

pub fn route(app: &mut App, events: &[Event]) {
    let (kctx, text) = key_context(TuiFocus::Sidebar);
    let frame = core_input(app, events, kctx, text);
    for i in frame.live() {
        // A shortcut may have moved the focus (palette, address bar, a prompt).
        if crate::input::focus(&app.core, &app.input) != TuiFocus::Sidebar {
            return;
        }
        match &frame.events[i] {
            InputEvent::Chord(c) if !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) => key(app, c.key),
            InputEvent::Text(t) if !frame.command_held => typed(&mut app.core, t),
            _ => {}
        }
    }
}

/// The entry the cursor is on, with the list.
fn current(core: &AppCore) -> (Vec<Entry>, Option<usize>) {
    let list = entries(core);
    sync_focus(core, &list);
    let cur = cursor_index(core, &list, &nav());
    (list, cur)
}

fn key(app: &mut App, k: KeyName) {
    let core = &mut app.core;
    let (list, cur) = current(core);
    let on = cur.and_then(|c| list.get(c));
    let tree_only = core.config.sidebar.mode == SidebarMode::Tree;
    let page = usize::from(nav().rows.max(2) - 1);
    match (k, on) {
        (KeyName::Escape, Some(Entry::Tree(_))) => tree_key(core, KeyInput { escape: true, ..KeyInput::default() }),
        (KeyName::Escape, _) => core.cmds.push(Cmd::Action(Action::FocusList)),
        (KeyName::ArrowUp | KeyName::ArrowDown, _) => {
            let down = k == KeyName::ArrowDown;
            if let Some(Entry::Tree(j)) = on {
                let edge = if down { j + 1 >= core.ui.tree.rows().len() } else { *j == 0 };
                if !edge {
                    tree_step(core, if down { Nav::Down } else { Nav::Up });
                    return;
                }
            }
            match cur {
                Some(c) => step(core, &list, c, down),
                None => first(core, &list),
            }
        }
        (KeyName::Home | KeyName::End | KeyName::PageUp | KeyName::PageDown, Some(Entry::Tree(_))) if tree_only => {
            tree_step(
                core,
                match k {
                    KeyName::Home => Nav::Home,
                    KeyName::End => Nav::End,
                    KeyName::PageUp => Nav::PageUp(page),
                    _ => Nav::PageDown(page),
                },
            );
        }
        (KeyName::Home, _) => first(core, &list),
        (KeyName::End, _) => {
            if let Some(t) = list.iter().rposition(Entry::selectable) {
                select(core, &list, t);
            }
        }
        (KeyName::PageUp | KeyName::PageDown, _) => {
            let c = cur.unwrap_or(0);
            let t = if k == KeyName::PageUp { c.saturating_sub(page) } else { (c + page).min(list.len() - 1) };
            let near = (t..list.len()).chain((0..t).rev()).find(|&i| list[i].selectable());
            if let Some(t) = near {
                select(core, &list, t);
            }
        }
        (KeyName::ArrowLeft, Some(Entry::Tree(_))) => tree_step(core, Nav::Left),
        (KeyName::ArrowRight, Some(Entry::Tree(_))) => tree_step(core, Nav::Right),
        (KeyName::Enter, Some(Entry::Tree(_))) => tree_key(core, KeyInput { enter: true, ..KeyInput::default() }),
        (KeyName::ArrowLeft | KeyName::ArrowRight | KeyName::Enter, Some(Entry::Header { section, open, .. })) => {
            let want = match k {
                KeyName::ArrowLeft => false,
                KeyName::ArrowRight => true,
                _ => !*open,
            };
            section.set_open(core, want);
        }
        (KeyName::ArrowLeft, Some(Entry::Place { .. })) => {
            if let Some(h) = cur.and_then(|c| list[..c].iter().rposition(|e| matches!(e, Entry::Header { .. }))) {
                select(core, &list, h);
            }
        }
        (KeyName::Enter, Some(Entry::Place { target, .. })) => activate(core, target.clone()),
        _ => {}
    }
}

/// Move the cursor to the next selectable entry above / below `c`.
fn step(core: &mut AppCore, list: &[Entry], c: usize, down: bool) {
    let next = if down {
        (c + 1..list.len()).find(|&i| list[i].selectable())
    } else {
        (0..c).rev().find(|&i| list[i].selectable())
    };
    if let Some(t) = next {
        select(core, list, t);
    }
}

fn first(core: &mut AppCore, list: &[Entry]) {
    if let Some(t) = list.iter().position(Entry::selectable) {
        select(core, list, t);
    }
}

/// Put the cursor on entry `t`. Entering the tree selects its row without opening it.
fn select(core: &mut AppCore, list: &[Entry], t: usize) {
    let mut n = nav();
    match list.get(t) {
        Some(Entry::Tree(j)) => {
            let Some(key) = core.ui.tree.rows().get(*j).map(|r| r.key.clone()) else { return };
            core.ui.tree.selected = Some(key);
            n.on_tree = true;
        }
        Some(_) => {
            n.on_tree = false;
            n.line = t;
        }
        None => return,
    }
    set_nav(n);
}

/// Do what a place row stands for (the GUI's click).
pub fn activate(core: &mut AppCore, target: Target) {
    let pane = core.ws.pane().id;
    match target {
        Target::Go(location) => {
            core.cmds.push(Cmd::FocusPane(pane));
            core.cmds.push(Cmd::Navigate { pane, location, select: None });
        }
        Target::Smart(index) => core.cmds.push(Cmd::Smart(SmartCmd::Open { index, new_tab: false })),
        Target::Search(i) => core.cmds.push(Cmd::RunSavedSearch(i)),
    }
}

/// A click on entry `index` (`HitTarget::SidebarRow`, for the mouse handler): the
/// keyboard moves to the sidebar with the cursor there; a header toggles, a place opens,
/// a tree row opens its folder (like the GUI's click).
pub fn click(app: &mut App, index: usize) {
    let core = &mut app.core;
    let list = entries(core);
    let Some(e) = list.get(index) else { return };
    core.ui.focus = Focus::Sidebar;
    sync_focus(core, &list);
    select(core, &list, index);
    match e {
        Entry::Header { section, open, .. } => section.set_open(core, !*open),
        Entry::Place { target, .. } => activate(core, target.clone()),
        Entry::Tree(j) => {
            let pane = core.ws.pane().id;
            if let Some(key) = core.ui.tree.rows().get(*j).map(|r| r.key.clone()) {
                core.ui.tree.open_row(key, pane, &mut core.cmds);
            }
        }
        Entry::Message(_) => {}
    }
}

fn tree_step(core: &mut AppCore, step: Nav) {
    tree_key(core, KeyInput { steps: vec![step], ..KeyInput::default() });
}

/// One key for the folder tree (`models::tree::keyboard`).
fn tree_key(core: &mut AppCore, k: KeyInput) {
    let current = core.ws.pane().location.clone();
    let pane = core.ws.pane().id;
    let vols = drive_volumes(&core.ui.volumes, &core.ui.cloud);
    let mut io = TreeIo { volumes: &vols, current: &current, pane, cmds: &mut core.cmds };
    soyuz_appcore::models::tree::keyboard(&mut core.ui.tree, k, &mut io);
    let mut n = nav();
    n.on_tree = true;
    set_nav(n);
}

/// Typed text: type-ahead in the tree; on places, the next place whose name starts
/// with it (wrapping).
fn typed(core: &mut AppCore, t: &str) {
    let (list, cur) = current(core);
    if let Some(Entry::Tree(_)) = cur.and_then(|c| list.get(c)) {
        tree_key(core, KeyInput { typed: t.to_string(), ..KeyInput::default() });
        return;
    }
    let needle = soyuz_core::fold_for_match(t.trim());
    if needle.is_empty() || list.is_empty() {
        return;
    }
    let start = cur.map_or(0, |c| c + 1);
    let hit = (start..list.len()).chain(0..start).find(|&i| match &list[i] {
        Entry::Place { label, .. } => soyuz_core::fold_for_match(label).starts_with(&needle),
        _ => false,
    });
    if let Some(i) = hit {
        select(core, &list, i);
    }
}

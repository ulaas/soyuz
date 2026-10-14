//! Headless tests of Phase T7 (places): the Trash and Computer pages in the pane. Each
//! test runs in its own sandbox: `crate::common::relaunch(module_path!(), "<test name>",
//! |data| ...)`, like the other files here.
//!
//! Neither page ever sees the real trash or the real drives: the trash comes from a fake
//! bin in the sandbox (`models::trash::set_lister`; Recycle Bin style `$R` / `$I` pairs,
//! so on Windows the real restore and purge code works on them) and the drives from
//! made-up volumes on sandbox folders (`services::places::set_volume_source`). Emptying
//! the trash would go through every real bin, so the tests only open its confirmation and
//! cancel it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::models::trash;
use soyuz_appcore::services::places;
use soyuz_appcore::tr;
use soyuz_core::{EntryKind, Location};
use soyuz_fs::{TrashItem, VolumeInfo, VolumeKind};
use soyuz_tui::app::App;
use soyuz_tui::layout::HitTarget;
use soyuz_tui::view::pane::places::{PlacesVm, button};

use super::menu::{drive, menu_labels, open_menu};
use crate::common::{self, Run, at, ctrl, dialog_title, key, key_mods, shift};

/// The fake bin: `bin/$R<name>` (the item) and `bin/$I<name>` (a Recycle Bin record of
/// `orig/<name>`) for each of a.txt, b.txt, c.txt.
fn fake_bin(data: &Path) {
    let bin = data.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(bin.join(format!("$R{name}")), name.as_bytes()).unwrap();
        std::fs::write(bin.join(format!("$I{name}")), i_record(&data.join("orig").join(name), 5)).unwrap();
    }
}

/// A version-2 `$I` record for `path`.
fn i_record(path: &Path, size: u64) -> Vec<u8> {
    let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    let mut v = Vec::new();
    v.extend_from_slice(&2u64.to_le_bytes());
    v.extend_from_slice(&size.to_le_bytes());
    v.extend_from_slice(&133_000_000_000_000_000u64.to_le_bytes());
    v.extend_from_slice(&(wide.len() as u32).to_le_bytes());
    for c in wide {
        v.extend_from_slice(&c.to_le_bytes());
    }
    v
}

/// List the fake bin: the items whose `$R` file is still there, in name order.
fn use_fake_bin(data: &Path) {
    let data = data.to_path_buf();
    trash::set_lister(Some(Arc::new(move || {
        let bin = data.join("bin");
        let mut names: Vec<String> = std::fs::read_dir(&bin)
            .map_err(|e| e.to_string())?
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_prefix("$R")).map(str::to_string))
            .collect();
        names.sort();
        Ok(names
            .into_iter()
            .enumerate()
            .map(|(i, n)| TrashItem {
                id: format!("$R{n}"),
                original: data.join("orig").join(&n),
                deleted_at: SystemTime::UNIX_EPOCH + Duration::from_secs(86_400 * (i as u64 + 1)),
                size: 5,
                kind: EntryKind::File,
                trash_root: bin.clone(),
            })
            .collect())
    })));
}

/// The focused pane's page in the last frame.
fn page(a: &App) -> Option<&PlacesVm> {
    a.view()?.focused_pane()?.places.as_ref()
}

/// The names of the page's rows.
fn names(a: &App) -> Vec<String> {
    page(a).map(|p| p.rows.iter().map(|r| r.name.clone()).collect()).unwrap_or_default()
}

/// The names of the selected rows.
fn selected(a: &App) -> Vec<String> {
    page(a).map(|p| p.rows.iter().filter(|r| r.selected).map(|r| r.name.clone()).collect()).unwrap_or_default()
}

/// The cursor row's name.
fn cursor(a: &App) -> Option<String> {
    page(a)?.rows.iter().find(|r| r.cursor).map(|r| r.name.clone())
}

/// A drive's name without its mount (`Fake vol1 (…/vol1)` → `Fake vol1`).
fn drive_name(name: &str) -> String {
    name.split(" (").next().unwrap_or(name).to_string()
}

/// A cell of row `index`.
fn row_at(a: &App, index: usize) -> Option<(u16, u16)> {
    a.hits()
        .iter()
        .find(|(_, t)| matches!(*t, HitTarget::PlacesRow { index: i, .. } if i == index))
        .map(|(r, _)| (r.x + 3, r.y))
}

/// A cell of header button `id`.
fn button_at(a: &App, id: u8) -> Option<(u16, u16)> {
    a.hits()
        .iter()
        .find(|(_, t)| matches!(*t, HitTarget::PlacesButton { button: b, .. } if b == id))
        .map(|(r, _)| (r.x + 1, r.y))
}

fn press(at: (u16, u16), b: MouseButton, modifiers: KeyModifiers) -> Event {
    Event::Mouse(MouseEvent { kind: MouseEventKind::Down(b), column: at.0, row: at.1, modifiers })
}

/// A terminal UI on `data` (mouse on) that opens `location` first.
fn app_at(data: &Path, location: Location) -> App {
    let mut c = common::test_config();
    c.tui.mouse = true;
    let mut app = super::menu::app(c, data);
    let pane = app.core.ws.pane().id;
    app.core.cmds.push(Cmd::Navigate { pane, location, select: None });
    app
}

#[test]
fn trash_page_selects_and_confirms() {
    match common::relaunch(module_path!(), "trash_page_selects_and_confirms", fake_bin) {
        Run::Child(data) => trash_page(data),
        Run::Parent(r) => {
            for name in ["a.txt", "b.txt", "c.txt"] {
                assert!(r.data.join("bin").join(format!("$R{name}")).is_file(), "{name} is still in the bin");
            }
            assert!(!r.data.join("orig").exists(), "nothing was restored");
        }
    }
}

fn trash_page(data: PathBuf) {
    use_fake_bin(&data);
    let mut app = app_at(&data, Location::Trash);
    let purge_title = tr!("dialogs-delete-permanent-many", count = 1);
    let empty_title = tr!("dialogs-delete-empty-title").to_string();
    let restore = tr!("views-trash-restore").to_string();
    let mut phase = 0;
    let three = vec!["a.txt".to_string(), "b.txt".into(), "c.txt".into()];
    drive(&mut app, |a| {
        let p = page(a);
        let ev = match phase {
            // Three items, nothing selected; Down selects b.txt.
            0 if names(a) == three && selected(a).is_empty() && p.is_some_and(|p| p.columns.len() == 3) => {
                key(KeyCode::Down)
            }
            // Shift+Down extends to c.txt.
            1 if selected(a) == ["b.txt"] && cursor(a).as_deref() == Some("b.txt") => shift(KeyCode::Down),
            // Space toggles c.txt off again.
            2 if selected(a) == ["b.txt", "c.txt"] => key(KeyCode::Char(' ')),
            // Ctrl+A selects everything; the header says so.
            3 if selected(a) == ["b.txt"] => ctrl('a'),
            4 if selected(a).len() == 3 && p.is_some_and(|p| p.summary.contains('3')) => key(KeyCode::Esc),
            // A click selects a.txt (the GUI's row click).
            5 if selected(a).is_empty() => press(row_at(a, 0)?, MouseButton::Left, KeyModifiers::NONE),
            // Del asks to delete it permanently: cancel.
            6 if selected(a) == ["a.txt"] => key(KeyCode::Delete),
            7 if dialog_title(a).as_deref() == Some(purge_title.as_str()) => key(KeyCode::Esc),
            // The header's Delete permanently button asks the same.
            8 if dialog_title(a).is_none() => {
                press(button_at(a, button::PURGE)?, MouseButton::Left, KeyModifiers::NONE)
            }
            9 if dialog_title(a).as_deref() == Some(purge_title.as_str()) => key(KeyCode::Esc),
            // E asks to empty the trash: cancel (it would empty every real bin).
            10 if dialog_title(a).is_none() => key(KeyCode::Char('e')),
            11 if dialog_title(a).as_deref() == Some(empty_title.as_str()) => key(KeyCode::Esc),
            // Shift+F10: the item's menu, Restore first.
            12 if dialog_title(a).is_none() => shift(KeyCode::F(10)),
            13 if menu_labels(a).first() == Some(&restore) => key(KeyCode::Esc),
            // A right click on c.txt selects it and opens its menu.
            14 if open_menu(a).is_none() => press(row_at(a, 2)?, MouseButton::Right, KeyModifiers::NONE),
            15 if open_menu(a).is_some() && selected(a) == ["c.txt"] => key(KeyCode::Esc),
            // Ctrl+click adds a.txt; a click on the empty area clears.
            16 if open_menu(a).is_none() => press(row_at(a, 0)?, MouseButton::Left, KeyModifiers::CONTROL),
            17 if selected(a) == ["a.txt", "c.txt"] => {
                let r = a.hits().iter().find(|(_, t)| matches!(t, HitTarget::PlacesArea { .. }))?.0;
                press((r.x + 2, r.bottom() - 1), MouseButton::Left, KeyModifiers::NONE)
            }
            18 if selected(a).is_empty() => ctrl('q'),
            _ => return None,
        };
        phase += 1;
        Some(ev)
    });
    assert_eq!(phase, 19);
    common::ok();
}

/// Restore and purge for real, on the fake bin (the Recycle Bin code accepts any folder
/// with `$R` / `$I` pairs; the unix bins only take their real locations).
#[cfg(windows)]
#[test]
fn trash_page_restores_and_purges() {
    match common::relaunch(module_path!(), "trash_page_restores_and_purges", fake_bin) {
        Run::Child(data) => restores_and_purges(data),
        Run::Parent(r) => {
            let bin = r.data.join("bin");
            assert_eq!(std::fs::read(r.data.join("orig").join("b.txt")).unwrap(), b"b.txt", "b restored");
            assert!(!bin.join("$Rb.txt").exists() && !bin.join("$Ib.txt").exists());
            assert!(!bin.join("$Ra.txt").exists() && !r.data.join("orig").join("a.txt").exists(), "a purged");
            assert!(bin.join("$Rc.txt").is_file(), "c stays");
        }
    }
}

#[cfg(windows)]
fn restores_and_purges(data: PathBuf) {
    use_fake_bin(&data);
    let mut app = app_at(&data, Location::Trash);
    let purge_title = tr!("dialogs-delete-permanent-many", count = 1);
    let mut phase = 0;
    drive(&mut app, |a| {
        let ev = match phase {
            // Down selects b.txt (the cursor starts on a.txt); R restores it.
            0 if names(a).len() == 3 => key(KeyCode::Down),
            1 if selected(a) == ["b.txt"] => key(KeyCode::Char('r')),
            // The job ran and the list was read again.
            2 if common::wait_jobs_idle(a) && names(a) == ["a.txt", "c.txt"] => {
                press(row_at(a, 0)?, MouseButton::Left, KeyModifiers::NONE)
            }
            3 if selected(a) == ["a.txt"] => key_mods(KeyCode::Delete, KeyModifiers::SHIFT),
            4 if dialog_title(a).as_deref() == Some(purge_title.as_str()) => key(KeyCode::Enter),
            5 if dialog_title(a).is_none() && common::wait_jobs_idle(a) && names(a) == ["c.txt"] => ctrl('q'),
            _ => return None,
        };
        phase += 1;
        Some(ev)
    });
    assert_eq!(phase, 6);
    common::ok();
}

fn drives(data: &Path) {
    std::fs::create_dir_all(data.join("vol1").join("inside")).unwrap();
    std::fs::create_dir_all(data.join("vol2")).unwrap();
}

fn use_fake_drives(data: &Path) {
    let vol = |name: &str, free: u64, kind: VolumeKind| VolumeInfo {
        mount: data.join(name),
        label: format!("Fake {name}"),
        fs: "fakefs".into(),
        kind,
        total: 1_000_000_000,
        free,
        read_only: false,
        device: soyuz_core::entry::DeviceId(7),
        supports: Default::default(),
    };
    let vols = vec![vol("vol1", 600_000_000, VolumeKind::Fixed), vol("vol2", 20_000_000, VolumeKind::Removable)];
    places::set_volume_source(Some(Arc::new(move || vols.clone())));
}

#[test]
fn computer_page_lists_and_opens_drives() {
    match common::relaunch(module_path!(), "computer_page_lists_and_opens_drives", drives) {
        Run::Child(data) => computer_page(data),
        Run::Parent(_) => {}
    }
}

fn computer_page(data: PathBuf) {
    use_fake_drives(&data);
    let mut app = app_at(&data, Location::Computer);
    let open = tr!("app-ctx-open").to_string();
    let mut phase = 0;
    let (vol1, vol2) = (data.join("vol1"), data.join("vol2"));
    let computer = |a: &App| a.core.ws.pane().location == Location::Computer;
    drive(&mut app, |a| {
        let ev = match phase {
            // Both drives with usage bars (vol2 nearly full); Down, Enter opens vol2.
            0 if names(a).iter().map(|n| drive_name(n)).collect::<Vec<_>>() == ["Fake vol1", "Fake vol2"]
                && page(a).is_some_and(|p| p.rows[1].usage.is_some_and(|(u, _)| u == 980)) =>
            {
                key(KeyCode::Down)
            }
            1 if cursor(a).as_deref().map(drive_name).as_deref() == Some("Fake vol2") => key(KeyCode::Enter),
            // Back to the Computer page (Alt+Left), where the cursor stayed.
            2 if at(a, &vol2) => key_mods(KeyCode::Left, KeyModifiers::ALT),
            3 if computer(a) && cursor(a).as_deref().map(drive_name).as_deref() == Some("Fake vol2") => {
                shift(KeyCode::F(10))
            }
            // The drive's menu opens with Open first.
            4 if menu_labels(a).first() == Some(&open) => key(KeyCode::Esc),
            // A right click on vol1 opens its menu; a left click opens the drive.
            5 if open_menu(a).is_none() => press(row_at(a, 0)?, MouseButton::Right, KeyModifiers::NONE),
            6 if open_menu(a).is_some() => key(KeyCode::Esc),
            7 if open_menu(a).is_none() => press(row_at(a, 0)?, MouseButton::Left, KeyModifiers::NONE),
            8 if at(a, &vol1) => ctrl('q'),
            _ => return None,
        };
        phase += 1;
        Some(ev)
    });
    assert_eq!(phase, 9);
    common::ok();
}

/// The row menu also opens from the Menu key and from the palette's "Context menu"
/// (`FrontendCmd::View(ContextMenu)` → `input::menu::open_context`), not the folder's
/// background menu.
#[test]
fn trash_menu_from_the_menu_key_and_the_palette() {
    match common::relaunch(module_path!(), "trash_menu_from_the_menu_key_and_the_palette", fake_bin) {
        Run::Child(data) => menu_key_and_palette(data),
        Run::Parent(_) => {}
    }
}

fn menu_key_and_palette(data: PathBuf) {
    use soyuz_appcore::palette::Exec;
    use soyuz_core::config::Action;
    use soyuz_tui::input::TuiFocus;

    use_fake_bin(&data);
    let mut app = app_at(&data, Location::Trash);
    let restore = tr!("views-trash-restore").to_string();
    let query = "right click";
    let mut typed = 0;
    let mut phase = 0;
    drive(&mut app, |a| {
        let palette_on = a.core.palette.open && a.focus() == TuiFocus::Palette;
        let ev = match phase {
            // The Menu key with only a cursor: the cursor's item is selected, its menu opens.
            0 if names(a).len() == 3 && selected(a).is_empty() => key(KeyCode::Menu),
            1 if menu_labels(a).first() == Some(&restore) && selected(a) == ["a.txt"] => key(KeyCode::Esc),
            2 if open_menu(a).is_none() => ctrl('k'),
            // Type the query one key per frame, then run the palette's top result.
            3 if palette_on && a.view().is_some_and(|v| v.palette.input == query[..typed]) => {
                if typed == query.len() {
                    let top = a.core.palette.results.get(a.core.palette.selected).map(|i| &i.exec);
                    if top != Some(&Exec::Action(Action::ContextMenu)) {
                        return None;
                    }
                    key(KeyCode::Enter)
                } else {
                    typed += 1;
                    return Some(key(KeyCode::Char(query[typed - 1..].chars().next()?)));
                }
            }
            4 if !a.core.palette.open && menu_labels(a).first() == Some(&restore) => key(KeyCode::Esc),
            5 if open_menu(a).is_none() => ctrl('q'),
            _ => return None,
        };
        phase += 1;
        Some(ev)
    });
    assert_eq!(phase, 6);
    common::ok();
}

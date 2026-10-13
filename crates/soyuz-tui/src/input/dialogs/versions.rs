//! Previous versions of a file or folder (`TuiDialog::Versions`, Phase T7, agent B): the
//! list from `models::versions::VersionsModel` (the scan on a worker) with open, compare,
//! copy to and restore, like the GUI's `ui/tools/versions.rs` window. Opened by
//! `ToolCmd::Versions` (the palette's "Restore previous version…", the item menu, the
//! Properties dialog's "Previous versions…").
//!
//! Without administrator rights it shows a message only (owner decision: no elevated
//! relaunch from the terminal UI, on any OS): `models::versions::elevate_hint` explains
//! how to start the terminal UI as administrator; there is no button and no key for it.
//!
//! | Key | Does |
//! |---|---|
//! | Up / Down, PageUp / PageDown, Home / End, wheel | pick a version |
//! | Enter / O | open (a folder in the pane, which closes the dialog; a file from a temp copy) |
//! | C | compare with the current file (files) |
//! | T | copy to… (the transfer prompt) |
//! | R | restore over the original (a Copy job: it asks before replacing, undoable) |
//! | F5 / L | look again |
//! | Esc | close |

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::tools::{VersionCmd, apply_version};
use soyuz_appcore::models::versions::{VersionsCfg, VersionsModel};
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog, form_mouse};
use crate::app::App;
use crate::view::dialogs::MAX_LIST_ROWS;
use crate::widgets::{Button, Form, FormOutcome, SelectList};

/// A button of the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Open,
    Compare,
    CopyTo,
    Restore,
    LookAgain,
    Close,
}

/// Two clicks on one row this close are a double click (open).
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

/// What the dialog keeps while it is open.
#[derive(Debug)]
pub struct State {
    pub title: String,
    pub model: VersionsModel,
    pub form: Form,
    /// What each of the form's buttons is.
    pub bmap: Vec<Btn>,
    /// Cursor and scroll of the versions (the cursor is the model's selection).
    pub list: SelectList,
    /// The last row click (a second one soon after on the same row opens it).
    pub last_click: Option<(usize, Instant)>,
}

impl State {
    pub fn new(model: VersionsModel) -> State {
        let name = soyuz_core::entry::display_name(&model.path);
        let bmap: Vec<Btn> = if model.is_dir {
            vec![Btn::Open, Btn::CopyTo, Btn::Restore, Btn::LookAgain, Btn::Close]
        } else {
            vec![Btn::Open, Btn::Compare, Btn::CopyTo, Btn::Restore, Btn::LookAgain, Btn::Close]
        };
        let buttons = bmap
            .iter()
            .map(|b| match b {
                Btn::Open => Button::new(tr!("tools-open")).accel('o'),
                Btn::Compare => Button::new(tr!("tools-compare-start")).accel('c'),
                Btn::CopyTo => Button::new(tr!("tools-versions-copy-to")).accel('t'),
                Btn::Restore => Button::new(tr!("tools-versions-restore")).accel('r'),
                Btn::LookAgain => Button::new(tr!("tools-versions-look-again")).accel('l'),
                Btn::Close => Button::new(tr!("common-close")),
            })
            .collect();
        let mut s = State {
            title: tr!("tui-versions-title", name = crate::dialog::safe(&name)),
            model,
            form: Form::new(Vec::new(), buttons),
            bmap,
            list: SelectList::new(),
            last_click: None,
        };
        s.sync();
        s.focus_open();
        s
    }

    /// Put the focus on Open once there is something to open (Enter opens).
    fn focus_open(&mut self) {
        if self.model.selection().is_some() {
            self.form.focus_button(0);
        }
    }

    /// The selection follows the list cursor; the buttons follow the selection.
    fn sync(&mut self) {
        let len = self.model.versions().len();
        self.list.clamp(len, usize::from(MAX_LIST_ROWS));
        if len > 0 {
            self.model.select(self.list.cursor);
        } else {
            self.model.selected = None;
        }
        let on = self.model.selection().is_some();
        let looking = self.model.looking();
        for (b, btn) in self.form.buttons.iter_mut().zip(&self.bmap) {
            b.enabled = match btn {
                Btn::LookAgain => !looking,
                Btn::Close => true,
                _ => on,
            };
        }
        if self.form.focused_button().is_none_or(|b| !self.form.buttons[b].enabled) {
            let first = self.form.buttons.iter().position(|b| b.enabled).unwrap_or(0);
            self.form.focus_button(first);
        }
        self.form.default_button = Some(0);
    }

    fn button(&self, b: Btn) -> Option<usize> {
        self.bmap.iter().position(|x| *x == b)
    }
}

/// Open the dialog for `ToolCmd::Versions` (`current`: the size and modified time now).
pub fn open(app: &mut App, path: PathBuf, is_dir: bool, current: Option<(u64, Option<SystemTime>)>) {
    let cfg = VersionsCfg::from_config(&app.core.config);
    let model = VersionsModel::new(path, is_dir, current, &cfg);
    app.push_dialog(TuiDialog::Versions(State::new(model)));
}

/// Run `cmd` (open, copy to, restore). Opening a folder closes the dialog: the pane shows
/// the snapshot folder.
fn run(core: &mut AppCore, cmd: Option<VersionCmd>) -> Outcome {
    let Some(cmd) = cmd else { return Outcome::Keep };
    let close = matches!(cmd, VersionCmd::Open { is_dir: true, .. });
    apply_version(core, cmd);
    if close { Outcome::Close(None) } else { Outcome::Keep }
}

fn press(core: &mut AppCore, s: &mut State, b: usize) -> Outcome {
    let Some(&btn) = s.bmap.get(b) else { return Outcome::Close(None) };
    if !s.form.buttons.get(b).is_some_and(|x| x.enabled) {
        return Outcome::Keep;
    }
    let out = match btn {
        Btn::Open => run(core, s.model.open_cmd(&core.config.general.date_format)),
        Btn::Compare => {
            s.model.start_compare();
            Outcome::Keep
        }
        Btn::CopyTo => run(core, s.model.copy_to_cmd()),
        Btn::Restore => run(core, s.model.restore_cmd()),
        Btn::LookAgain => {
            s.model.refresh(&VersionsCfg::from_config(&core.config));
            s.list = SelectList::new();
            Outcome::Keep
        }
        Btn::Close => Outcome::Close(None),
    };
    s.sync();
    out
}

/// F5 (look again) without modifiers.
fn is_f5(ev: &InputEvent) -> bool {
    matches!(ev, InputEvent::Chord(c) if c.key == KeyName::F5 && !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT))
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let page = usize::from(MAX_LIST_ROWS);
    for ev in &i.events {
        let len = s.model.versions().len();
        if s.list.apply(ev, len, page) {
            s.sync();
            continue;
        }
        if is_f5(ev) {
            if let Some(b) = s.button(Btn::LookAgain) {
                press(core, s, b);
            }
            continue;
        }
        match s.form.apply(ev) {
            FormOutcome::Cancel => return Outcome::Close(None),
            FormOutcome::Pressed(b) => {
                if let Outcome::Close(c) = press(core, s, b) {
                    return Outcome::Close(c);
                }
            }
            _ => {}
        }
    }
    Outcome::Keep
}

/// A mouse event: rows pick (a double click opens), the wheel scrolls, buttons press.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    use crossterm::event::{MouseButton, MouseEventKind};
    let len = s.model.versions().len();
    if let (MouseEventKind::Down(MouseButton::Left), Some(crate::layout::HitTarget::DialogRow(row))) =
        (m.event.kind, m.target)
    {
        let double = s.last_click.is_some_and(|(r, t)| r == row && t.elapsed() < DOUBLE_CLICK);
        s.last_click = (!double).then(|| (row, Instant::now()));
        s.list.select(row, len);
        s.sync();
        if double && let Some(b) = s.button(Btn::Open) {
            return press(core, s, b);
        }
        return Outcome::Keep;
    }
    let out = match form_mouse(&mut s.form, Some((&mut s.list, len)), m) {
        FormOutcome::Pressed(b) => press(core, s, b),
        _ => Outcome::Keep,
    };
    s.sync();
    out
}

/// The dialog went away by itself: never.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

/// Every frame while it is open (`input::dialogs::tick`, from `view::prepare`): the scan's
/// and the compare's results (their workers wake the frame loop).
pub fn tick(_core: &mut AppCore, s: &mut State) {
    let looking = s.model.looking();
    if s.model.poll() {
        s.sync();
        if looking && !s.model.looking() {
            s.focus_open();
        }
    }
}

#[cfg(test)]
mod tests {
    use soyuz_fs::snapshots::{PathVersion, Snapshot, VersionsReport};

    use super::*;

    fn report(n: u32) -> VersionsReport {
        let versions = (1..=n)
            .map(|seq| PathVersion {
                snapshot: Snapshot {
                    provider: "Shadow copy",
                    id: format!("s{seq}"),
                    root: PathBuf::from(format!("snap{seq}")),
                    volume: PathBuf::from("vol"),
                    created: None,
                    seq,
                },
                path: PathBuf::from(format!("snap{seq}")).join("a.txt"),
                size: 10,
                modified: None,
                is_dir: false,
            })
            .collect();
        VersionsReport { versions, ..VersionsReport::default() }
    }

    #[test]
    fn buttons_follow_the_selection_and_never_elevate() {
        let s = State::new(VersionsModel::with_report(PathBuf::from("a.txt"), false, Ok(report(0))));
        assert_eq!(s.bmap, [Btn::Open, Btn::Compare, Btn::CopyTo, Btn::Restore, Btn::LookAgain, Btn::Close]);
        assert!(!s.form.buttons[0].enabled, "nothing to open");
        assert!(s.form.buttons[s.button(Btn::LookAgain).unwrap()].enabled);
        let s = State::new(VersionsModel::with_report(PathBuf::from("a.txt"), false, Ok(report(2))));
        assert!(s.form.buttons.iter().all(|b| b.enabled));
        assert_eq!(s.model.selected, Some(0), "the cursor's version");
        // Folders have no compare; nothing anywhere offers administrator rights.
        let s = State::new(VersionsModel::with_report(PathBuf::from("dir"), true, Ok(report(1))));
        assert!(!s.bmap.contains(&Btn::Compare));
        assert!(!s.form.buttons.iter().any(|b| b.label == tr!("tools-versions-elevate")));
    }

    #[test]
    fn the_cursor_is_the_selection() {
        let mut s = State::new(VersionsModel::with_report(PathBuf::from("a.txt"), false, Ok(report(3))));
        let down = InputEvent::Chord("Down".parse().unwrap());
        assert!(s.list.apply(&down, 3, 10));
        s.sync();
        assert_eq!(s.model.selected, Some(1));
        assert!(is_f5(&InputEvent::Chord("F5".parse().unwrap())));
        assert!(!is_f5(&InputEvent::Chord("Ctrl+F5".parse().unwrap())));
    }
}

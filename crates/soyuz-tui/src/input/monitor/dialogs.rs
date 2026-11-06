//! The monitor's dialogs: the column chooser (`c`: Space toggles, Shift+Up / Down moves, `r`
//! resets), the processors of a process (`a`) and the name of a snapshot (`n`, `F2`).

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::monitor::MonitorCmd;
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::config::monitor::{Monitor, ProcColumn};
use soyuz_core::config::{KeyName, Mods};

use crate::app::App;
use crate::input::dialogs::confirm::{ListClick, click, list_mouse};
use crate::input::dialogs::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::widgets::form::FieldKind;
use crate::widgets::{Button, Field, Form, FormOutcome, SelectList};

/// The column chooser's state; the columns themselves are the config's.
#[derive(Debug)]
pub struct ColumnsState {
    pub pane: PaneId,
    pub form: Form,
    pub list: SelectList,
}

/// Shown columns in their order, then the others in their fixed order.
pub fn ordered_columns(cfg: &Monitor) -> Vec<ProcColumn> {
    let mut out: Vec<ProcColumn> = cfg.columns.iter().map(|c| c.id).collect();
    for c in ProcColumn::ALL {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// Open the column chooser (`FrontendCmd::MonitorColumns`).
pub fn open_columns(app: &mut App, pane: PaneId) {
    let form = Form::new(Vec::new(), vec![Button::new(tr!("dialogs-done"))]);
    app.push_dialog(TuiDialog::MonitorColumns(ColumnsState { pane, form, list: SelectList::new() }));
}

/// The dialog went away by itself: its pane closed.
pub fn pane_gone(core: &AppCore, pane: PaneId) -> bool {
    core.ws.find_pane(pane).is_none()
}

/// A key press in the column chooser.
pub fn columns_input(core: &mut AppCore, s: &mut ColumnsState, i: &DialogInput) -> Outcome {
    let cols = ordered_columns(&core.config.monitor);
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        if let InputEvent::Chord(c) = ev {
            let cur = cols.get(s.list.cursor).copied();
            let shown = core.config.monitor.columns.len();
            match (c.mods, c.key) {
                (m, KeyName::Space) if m.is_empty() => {
                    if let Some(col) = cur {
                        core.cmds.push(Cmd::Monitor(s.pane, MonitorCmd::ToggleColumn(col)));
                    }
                    continue;
                }
                (Mods::SHIFT, KeyName::ArrowUp) if s.list.cursor > 0 && s.list.cursor < shown => {
                    if let Some(col) = cur {
                        s.list.cursor -= 1;
                        core.cmds.push(Cmd::Monitor(s.pane, MonitorCmd::MoveColumn(col, s.list.cursor)));
                    }
                    continue;
                }
                (Mods::SHIFT, KeyName::ArrowDown) if s.list.cursor + 1 < shown => {
                    if let Some(col) = cur {
                        s.list.cursor += 1;
                        core.cmds.push(Cmd::Monitor(s.pane, MonitorCmd::MoveColumn(col, s.list.cursor)));
                    }
                    continue;
                }
                (m, KeyName::R) if m.is_empty() => {
                    core.cmds.push(Cmd::Monitor(s.pane, MonitorCmd::ResetColumns));
                    continue;
                }
                _ => {}
            }
        }
        if s.list.apply(ev, cols.len(), height) {
            continue;
        }
        if let FormOutcome::Pressed(_) | FormOutcome::Cancel = s.form.apply(ev) {
            return Outcome::Close(None);
        }
    }
    Outcome::Keep
}

/// A mouse event in the column chooser.
pub fn columns_mouse(core: &mut AppCore, s: &mut ColumnsState, e: &DialogMouse) -> Outcome {
    let cols = ordered_columns(&core.config.monitor);
    if let Some(ListClick::Row { i, .. }) = list_mouse(&mut s.list, cols.len(), e) {
        if let Some(col) = cols.get(i) {
            core.cmds.push(Cmd::Monitor(s.pane, MonitorCmd::ToggleColumn(*col)));
        }
        return Outcome::Keep;
    }
    match click(&mut s.form, e) {
        Some(FormOutcome::Pressed(_)) => Outcome::Close(None),
        _ => Outcome::Keep,
    }
}

/// The processors dialog's state; the ticks are the core's `AffinityEdit`.
#[derive(Debug)]
pub struct AffinityState {
    pub pane: PaneId,
    pub form: Form,
    pub list: SelectList,
}

/// The dialog of pane `pane`'s `AffinityEdit`.
pub fn affinity_dialog(pane: PaneId) -> TuiDialog {
    let form = Form::new(Vec::new(), vec![Button::new(tr!("common-apply")), Button::new(tr!("common-cancel"))])
        .default_button(0);
    TuiDialog::MonitorAffinity(AffinityState { pane, form, list: SelectList::new() })
}

fn cpu_count(core: &AppCore, pane: PaneId) -> usize {
    core.ui.monitor.peek(pane).and_then(|v| v.affinity.as_ref()).map_or(0, |e| e.cpus.len())
}

/// The core's edit is gone (applied, or the pane closed).
pub fn affinity_stale(core: &AppCore, s: &AffinityState) -> bool {
    cpu_count(core, s.pane) == 0
}

fn affinity_outcome(core: &mut AppCore, s: &AffinityState, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => {
            core.cmds.push(Cmd::Monitor(s.pane, MonitorCmd::ApplyAffinity));
            Some(Outcome::Close(None))
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => {
            core.cmds.push(Cmd::Monitor(s.pane, MonitorCmd::CancelAffinity));
            Some(Outcome::Close(None))
        }
        _ => None,
    }
}

/// A key press in the processors dialog.
pub fn affinity_input(core: &mut AppCore, s: &mut AffinityState, i: &DialogInput) -> Outcome {
    let n = cpu_count(core, s.pane);
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        if let InputEvent::Chord(c) = ev
            && c.key == KeyName::Space
            && c.mods.is_empty()
        {
            core.cmds.push(Cmd::Monitor(s.pane, MonitorCmd::ToggleAffinityCpu(s.list.cursor)));
            continue;
        }
        if s.list.apply(ev, n, height) {
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = affinity_outcome(core, s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event in the processors dialog.
pub fn affinity_mouse(core: &mut AppCore, s: &mut AffinityState, e: &DialogMouse) -> Outcome {
    let n = cpu_count(core, s.pane);
    if let Some(ListClick::Row { i, .. }) = list_mouse(&mut s.list, n, e) {
        core.cmds.push(Cmd::Monitor(s.pane, MonitorCmd::ToggleAffinityCpu(i)));
        return Outcome::Keep;
    }
    match click(&mut s.form, e) {
        Some(o) => affinity_outcome(core, s, o).unwrap_or(Outcome::Keep),
        None => Outcome::Keep,
    }
}

/// What the name dialog names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameKind {
    New,
    Rename(String),
}

/// The snapshot name dialog's state.
#[derive(Debug)]
pub struct NameState {
    pub pane: PaneId,
    pub kind: NameKind,
    pub form: Form,
}

/// Open the name dialog with `name` in the field.
pub fn open_name(app: &mut App, pane: PaneId, kind: NameKind, name: &str) {
    let form = Form::new(
        vec![Field::text(tr!("tui-monitor-snapshot-name"), name)],
        vec![Button::new(tr!("common-save")), Button::new(tr!("common-cancel"))],
    )
    .default_button(0);
    app.push_dialog(TuiDialog::MonitorName(NameState { pane, kind, form }));
}

fn name_of(s: &NameState) -> String {
    match s.form.fields.first().map(|f| &f.kind) {
        Some(FieldKind::Text { text, .. }) => text.trim().to_string(),
        _ => String::new(),
    }
}

fn name_outcome(core: &mut AppCore, s: &NameState, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => {
            let name = name_of(s);
            let cmds: Vec<MonitorCmd> = match &s.kind {
                NameKind::New => vec![MonitorCmd::TakeSnapshot(name)],
                NameKind::Rename(id) => {
                    vec![MonitorCmd::StartRename(id.clone()), MonitorCmd::RenameText(name), MonitorCmd::CommitRename]
                }
            };
            core.cmds.extend(cmds.into_iter().map(|c| Cmd::Monitor(s.pane, c)));
            Some(Outcome::Close(None))
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        _ => None,
    }
}

/// A key press or paste in the name dialog.
pub fn name_input(core: &mut AppCore, s: &mut NameState, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        let o = s.form.apply(ev);
        if let Some(out) = name_outcome(core, s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event in the name dialog.
pub fn name_mouse(core: &mut AppCore, s: &mut NameState, m: &DialogMouse) -> Outcome {
    match click(&mut s.form, m) {
        Some(o) => name_outcome(core, s, o).unwrap_or(Outcome::Keep),
        None => Outcome::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_columns_come_first_in_their_order() {
        let cfg = Monitor::default();
        let all = ordered_columns(&cfg);
        assert_eq!(all.len(), ProcColumn::ALL.len());
        assert_eq!(all[0], cfg.columns[0].id);
        assert_eq!(all[cfg.columns.len() - 1], cfg.columns[cfg.columns.len() - 1].id);
    }
}

//! The views of the monitor's dialogs (`crate::input::monitor::dialogs`): the column
//! chooser, the processors of a process and the name of a snapshot.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::monitor::labels;
use soyuz_appcore::tr;

use crate::input::monitor::dialogs::{AffinityState, ColumnsState, NameKind, NameState, ordered_columns};
use crate::theme::Role;
use crate::view::dialogs::DialogVm;
use crate::widgets::ListRow;

/// What the column chooser shows: shown columns in their order, then the others.
pub fn columns_vm(core: &AppCore, s: &ColumnsState) -> DialogVm {
    let cfg = &core.config.monitor;
    let list = ordered_columns(cfg)
        .into_iter()
        .map(|col| {
            let on = cfg.columns.iter().any(|c| c.id == col);
            let mut row = ListRow::new(format!("[{}] {}", if on { 'x' } else { ' ' }, labels::column_title(col)));
            row.right = crate::view::text::truncate_end(labels::column_desc(col), 44);
            if col == soyuz_core::config::monitor::ProcColumn::Name {
                row.role = Some(Role::Disabled);
            }
            row
        })
        .collect();
    DialogVm {
        list,
        list_state: s.list.clone(),
        hints: tr!("tui-monitor-columns-hints").into(),
        width: 84,
        ..DialogVm::from_form(tr!("tui-monitor-columns-title"), &s.form)
    }
}

/// What the processors dialog shows.
pub fn affinity_vm(core: &AppCore, s: &AffinityState) -> DialogVm {
    let edit = core.ui.monitor.peek(s.pane).and_then(|v| v.affinity.as_ref());
    let (name, cpus) = edit.map_or((String::new(), Vec::new()), |e| (e.name.clone(), e.cpus.clone()));
    let list = cpus
        .iter()
        .enumerate()
        .map(|(i, on)| ListRow::new(format!("[{}] {}", if *on { 'x' } else { ' ' }, tr!("tui-monitor-cpu-n", n = i))))
        .collect();
    DialogVm {
        list,
        list_state: s.list.clone(),
        hints: tr!("tui-monitor-affinity-hints").into(),
        width: 50,
        ..DialogVm::from_form(&tr!("tui-monitor-affinity-title", name = name), &s.form)
    }
}

/// What the snapshot name dialog shows.
pub fn name_vm(_core: &AppCore, s: &NameState) -> DialogVm {
    let title = match s.kind {
        NameKind::New => tr!("tui-monitor-snapshot-new"),
        NameKind::Rename(_) => tr!("tui-monitor-snapshot-rename"),
    };
    DialogVm { hints: tr!("tui-monitor-snapshot-hints").into(), width: 60, ..DialogVm::from_form(title, &s.form) }
}

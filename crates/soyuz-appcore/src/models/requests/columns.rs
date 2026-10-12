//! The column chooser (`FrontendCmd::ChooseColumns`): which columns a details view can
//! show, their labels, and showing / hiding one. Name always stays.

use soyuz_core::Column;

use crate::state::pane::Pane;

/// Columns offered (in order): every column except those filled only on demand and
/// script columns without a loaded provider.
pub fn choosable() -> Vec<Column> {
    Column::ALL
        .iter()
        .copied()
        .filter(|&col| {
            !matches!(
                col,
                Column::Dimensions | Column::Duration | Column::Checksum | Column::ItemCount | Column::LinkTarget
            )
        })
        .filter(|&col| col.script_slot().is_none() || crate::automation::scripts::column_label(col).is_some())
        .collect()
}

/// A column's header label (a script column's own title).
pub fn label(col: Column) -> String {
    crate::automation::scripts::column_label(col).unwrap_or_else(|| crate::i18n::labels::column(col).to_string())
}

/// Name cannot be hidden.
pub fn can_toggle(col: Column) -> bool {
    col != Column::Name
}

/// `col` is shown in `pane`'s details view.
pub fn is_visible(pane: &Pane, col: Column) -> bool {
    pane.view.visible_columns().iter().any(|c| c.column == col)
}

/// Show or hide `col` in `pane` (the frontend then queues `Cmd::ViewChanged`). Returns
/// whether anything changed.
pub fn set_visible(pane: &mut Pane, col: Column, on: bool) -> bool {
    if !can_toggle(col) || is_visible(pane, col) == on {
        return false;
    }
    pane.view.set_column_visible(col, on);
    pane.layout_sig = None;
    true
}

#[cfg(test)]
mod tests {
    use soyuz_core::Location;

    use super::*;
    use crate::state::view_memory::ViewState;

    #[test]
    fn toggles_a_column_but_never_name() {
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        let cols = choosable();
        assert!(cols.contains(&Column::Name) && cols.contains(&Column::Created));
        assert!(!cols.contains(&Column::Checksum));
        let was = is_visible(&p, Column::Created);
        assert!(set_visible(&mut p, Column::Created, !was));
        assert_eq!(is_visible(&p, Column::Created), !was);
        assert!(!set_visible(&mut p, Column::Created, !was), "no change");
        assert!(!set_visible(&mut p, Column::Name, false));
        assert!(is_visible(&p, Column::Name));
        assert!(!label(Column::Size).is_empty());
    }
}

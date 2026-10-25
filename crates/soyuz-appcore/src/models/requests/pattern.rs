//! Select by pattern (`FrontendCmd::SelectByPattern`): a glob (`*`, `?`) or plain text
//! matched against the names of the pane's visible items.

use soyuz_core::{NamePatternSpec, PatternKind};

use crate::cmd::Cmd;
use crate::models::toasts::Toast;
use crate::state::pane::{Pane, PaneId};
use crate::state::workspace::Workspace;

/// What the pattern field starts with.
pub const DEFAULT_PATTERN: &str = "*.";

/// A glob when the text has a wildcard, else a substring.
pub fn kind_of(text: &str) -> PatternKind {
    if text.contains(['*', '?']) { PatternKind::Glob } else { PatternKind::Substring }
}

/// Select the visible items of `pane` whose name matches `text` (replacing the selection
/// unless `add`). Returns how many matched, or `None` when the pattern does not compile
/// (nothing changes then).
pub fn select_matching(pane: &mut Pane, text: &str, add: bool) -> Option<usize> {
    let pat = NamePatternSpec::new(kind_of(text), text.trim()).compile().ok()?;
    if !add {
        pane.selection.clear();
    }
    let keys: Vec<_> =
        pane.visible.iter().filter(|&&i| pat.is_match(&pane.entries[i].name)).map(|&i| pane.key_of(i)).collect();
    let n = keys.len();
    for k in keys {
        pane.selection.insert(k);
    }
    Some(n)
}

/// The dialog's Select: select in pane `id` and return the "N selected" toast to queue.
pub fn apply(ws: &mut Workspace, id: PaneId, text: &str, add: bool) -> Option<Cmd> {
    let pane = ws.find_pane_mut(id)?;
    let n = select_matching(pane, text, add)?;
    Some(Cmd::Toast(Toast::info(tr!("dialogs-pattern-selected", count = n))))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use soyuz_core::{EntryInfo, EntryKind, Location};

    use super::*;
    use crate::state::view_memory::ViewState;

    fn pane_with(names: &[&str]) -> Pane {
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        let entries = names.iter().map(|n| EntryInfo::minimal(PathBuf::from("/d").join(n), EntryKind::File)).collect();
        p.on_listing(entries, true, None);
        p.refresh_visible(true, true);
        p
    }

    #[test]
    fn globs_and_substrings_select_matching_names() {
        let mut p = pane_with(&["a.txt", "b.txt", "c.rs", "notes.md"]);
        assert_eq!(select_matching(&mut p, "*.txt", false), Some(2));
        assert!(p.selection.contains("a.txt") && p.selection.contains("b.txt") && !p.selection.contains("c.rs"));
        // Replace, then add.
        assert_eq!(select_matching(&mut p, "note", false), Some(1));
        assert!(!p.selection.contains("a.txt") && p.selection.contains("notes.md"));
        assert_eq!(select_matching(&mut p, "*.rs", true), Some(1));
        assert!(p.selection.contains("notes.md") && p.selection.contains("c.rs"));
        assert_eq!(kind_of("a?"), PatternKind::Glob);
        assert_eq!(kind_of("abc"), PatternKind::Substring);
    }
}

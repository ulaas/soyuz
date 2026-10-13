//! The view of the tab history dialog (`TuiDialog::TabHistory`): the intro, then one row
//! per place as a tree (guides `│ `, `├─`, `└─`; `| `, `|-`, `` `- `` with ASCII icons or
//! lines), its title and full path (cut first when the row is narrow), and on the right
//! when it was last visited and how often. The current place is in the accent colour.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::workspaces::when_label;
use soyuz_appcore::state::crumbs::location_title;
use soyuz_appcore::state::workspace_visits::VisitRow;
use soyuz_appcore::tr;
use soyuz_core::config::TuiBorders;

use super::DialogVm;
use crate::dialog::safe;
use crate::icons::IconSet;
use crate::input::dialogs::tab_history::State;
use crate::theme::Role;
use crate::widgets::ListRow;

/// The characters of the tree guides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guides {
    /// A level whose node has later children: `│ `.
    pub through: &'static str,
    /// The elbow into a row with later siblings: `├─`.
    pub tee: &'static str,
    /// The elbow into the last sibling: `└─`.
    pub last: &'static str,
}

/// Box-drawing guides.
pub const UNICODE: Guides = Guides { through: "│ ", tee: "├─", last: "└─" };
/// ASCII guides.
pub const ASCII: Guides = Guides { through: "| ", tee: "|-", last: "`-" };

/// The guides for the core's `[tui]` settings: ASCII with ASCII icons or ASCII lines.
pub fn guides(core: &AppCore) -> Guides {
    let tui = &core.config.tui;
    if IconSet::from_config(tui) == IconSet::Ascii || tui.borders == TuiBorders::Ascii { ASCII } else { UNICODE }
}

/// Whether a row after `k` sits at `depth` before the tree leaves that level.
fn next_at(rows: &[VisitRow], k: usize, depth: usize) -> bool {
    for r in &rows[k + 1..] {
        if r.depth < depth {
            return false;
        }
        if r.depth == depth {
            return true;
        }
    }
    false
}

/// The tree prefix of row `k` (rows depth-first, `VisitGraph::rows`).
pub fn prefix(rows: &[VisitRow], k: usize, g: Guides) -> String {
    let depth = rows[k].depth;
    let mut s = String::new();
    for d in 1..depth {
        s.push_str(if next_at(rows, k, d) { g.through } else { "  " });
    }
    if depth > 0 {
        s.push_str(if next_at(rows, k, depth) { g.tee } else { g.last });
    }
    s
}

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let g = guides(core);
    let now = soyuz_appcore::services::recent::unix_now();
    let mut d = DialogVm {
        body: vec![tr!("workspaces-history-intro").into()],
        hints: tr!("tui-workspaces-history-hints").into(),
        width: 84,
        ..DialogVm::from_form(tr!("workspaces-history-title"), &s.form)
    };
    if s.rows.is_empty() {
        d.body.push(tr!("workspaces-history-empty").into());
    }
    d.list = s
        .rows
        .iter()
        .enumerate()
        .map(|(k, r)| {
            let title = safe(&location_title(&r.location));
            let detail = safe(&r.location.display());
            let count = if r.count > 1 { format!(" ×{}", r.count) } else { String::new() };
            ListRow {
                text: format!("{}{title}  {detail}", prefix(&s.rows, k, g)),
                right: format!("{}{count}", when_label(r.last, now)),
                role: r.current.then_some(Role::Accent),
            }
        })
        .collect();
    d.list_state = s.list.clone();
    d
}

#[cfg(test)]
mod tests {
    use soyuz_core::Location;

    use super::*;

    fn row(depth: usize) -> VisitRow {
        VisitRow {
            node: 0,
            depth,
            location: Location::Dir("/x".into()),
            first: 0,
            last: 0,
            count: 1,
            current: false,
            branches: false,
        }
    }

    #[test]
    fn guides_draw_the_tree() {
        // a
        // ├─b
        // │ └─c
        // └─d
        //   └─e
        let rows: Vec<VisitRow> = [0, 1, 2, 1, 2].into_iter().map(row).collect();
        let p: Vec<String> = (0..rows.len()).map(|k| prefix(&rows, k, UNICODE)).collect();
        assert_eq!(p, ["", "├─", "│ └─", "└─", "  └─"]);
        let a: Vec<String> = (0..rows.len()).map(|k| prefix(&rows, k, ASCII)).collect();
        assert_eq!(a, ["", "|-", "| `-", "`-", "  `-"]);
        assert!(a.iter().all(|s| s.is_ascii()));
    }
}

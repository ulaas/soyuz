//! What the terminal UI draws, in three steps per frame (see `App::frame`):
//!
//! 1. [`prepare`] (`&mut AppCore`, `&mut InputState`, after `prepare_visible_panes`): per
//!    drawn pane the cell layout, scroll position and page size; data fetched lazily
//!    (Miller columns); the scroll positions the TUI keeps itself (sidebar, palette, the
//!    filter field) in its [`InputState`].
//! 2. [`ViewModel::from_core`] (`&AppCore`, `&InputState`): plain data, no ratatui types,
//!    easy to test.
//! 3. [`draw`]: renders the model into the [`Regions`] with the [`TuiTheme`] (colours,
//!    icon set, box lines), recording click targets in [`Hits`]. A pure ASCII theme
//!    ([`TuiTheme::ascii_only`]) gets its typographic punctuation replaced last
//!    ([`crate::borders::asciify`]).
//!
//! One sub-model and one draw function per part, each in its own file, so the Phase T2
//! steps do not touch each other's code:
//!
//! | File | Model | Draw | Step |
//! |---|---|---|---|
//! | `pane.rs` | [`PaneVm`] (one per drawn pane) | `draw_pane` | T2.3 |
//! | `tabs.rs` | [`TabsVm`] | `draw_tabs` | T2.4 |
//! | `crumbs.rs` | [`CrumbsVm`] | `draw_crumbs` | T2.5 |
//! | `sidebar.rs` | [`SidebarVm`] | `draw_sidebar` | T2.6 |
//! | `palette.rs` | [`PaletteVm`] | `draw_palette` | T2.7 |
//! | `status.rs` / `toasts.rs` | [`StatusVm`] / [`ToastsVm`] | `draw_status` / `draw_toasts` | T2.8 |
//! | `dialog.rs` | [`DialogView`] (`crate::dialog`) | `draw_dialog` | T3 |
//! | `text.rs` | width-aware truncation / wrapping | | shared |
//!
//! All names pass through [`crate::dialog::safe`], so a file name can never write escape
//! sequences to the terminal.

pub mod crumbs;
pub mod dialog;
pub mod palette;
pub mod pane;
pub mod sidebar;
pub mod status;
pub mod tabs;
pub mod text;
pub mod toasts;

use ratatui::Frame;
use soyuz_appcore::core::AppCore;

pub use self::crumbs::CrumbsVm;
pub use self::palette::PaletteVm;
pub use self::pane::{PaneLine, PaneMode, PaneVm, Row, RowKind};
pub use self::sidebar::SidebarVm;
pub use self::status::StatusVm;
pub use self::tabs::TabsVm;
pub use self::toasts::{MAX_TOASTS, ToastLine, ToastsVm};
pub use crate::dialog::DialogView;
use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// Everything one frame draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewModel {
    pub tabs: TabsVm,
    /// The drawn panes, in the order of `Regions::panes`.
    pub panes: Vec<PaneVm>,
    pub crumbs: CrumbsVm,
    pub sidebar: SidebarVm,
    pub status: StatusVm,
    pub toasts: ToastsVm,
    pub palette: PaletteVm,
    /// The top-most core prompt.
    pub dialog: Option<DialogView>,
}

impl ViewModel {
    /// Build the frame's data from the core and the TUI's input state, after [`prepare`]
    /// ran for `regions`.
    pub fn from_core(core: &AppCore, input: &InputState, regions: &Regions) -> ViewModel {
        let mut status = StatusVm::from_core(core);
        crate::input::filter::decorate(&input.filter, core, &mut status, regions.area.width);
        ViewModel {
            tabs: TabsVm::from_core(core),
            panes: regions.panes.iter().filter_map(|p| PaneVm::from_core(core, p.index)).collect(),
            crumbs: CrumbsVm::from_core(core, &input.address),
            sidebar: SidebarVm::from_core(core, &input.sidebar.nav),
            status,
            toasts: ToastsVm::from_core(core),
            palette: PaletteVm::from_core(core, &input.palette.view),
            dialog: core.dialogs.top().map(crate::dialog::fallback),
        }
    }

    /// The focused pane's model (when it is drawn).
    pub fn focused_pane(&self) -> Option<&PaneVm> {
        self.panes.iter().find(|p| p.focused).or(self.panes.first())
    }
}

/// Step 1: what the views need from `&mut AppCore` (and the TUI's own view state in
/// `input`) before the model is built.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    pane::prepare(core, regions);
    sidebar::prepare(core, &mut input.sidebar.nav, regions);
    palette::prepare(core, &mut input.palette.view, regions);
    crate::input::filter::prepare(&mut input.filter, core, regions.area.width);
}

/// Draw a frame. Clears `hits` first.
pub fn draw(f: &mut Frame<'_>, m: &ViewModel, regions: &Regions, theme: &TuiTheme, hits: &mut Hits) {
    hits.clear();
    tabs::draw_tabs(f, regions.tabs, &m.tabs, theme, hits);
    crumbs::draw_crumbs(f, regions.crumbs, &m.crumbs, theme, hits);
    sidebar::draw_sidebar(f, regions.sidebar, &m.sidebar, theme, hits);
    for (vm, p) in m.panes.iter().zip(&regions.panes) {
        pane::draw_pane(f, p.rect, vm, theme, hits);
    }
    tabs::draw_pane_chrome(f, regions, &m.tabs, theme, hits);
    toasts::draw_toasts(f, toasts::area(regions), &m.toasts, theme, hits);
    status::draw_status(f, regions.status, &m.status, theme, hits);
    crumbs::draw_completions(f, regions.crumbs, regions.area, &m.crumbs, theme, hits);
    palette::draw_palette(f, regions.area, &m.palette, theme, hits);
    if let Some(d) = &m.dialog {
        dialog::draw_dialog(f, regions.area, d, theme, hits);
    }
    if theme.ascii_only() {
        crate::borders::asciify(f.buffer_mut());
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;
    use soyuz_appcore::file_class::FileClass;
    use soyuz_appcore::models::toasts::ToastKind;
    use soyuz_appcore::state::pane::PaneId;
    use soyuz_core::Column;
    use soyuz_core::config::{Orientation, TuiSection};

    use super::*;
    use crate::layout::{Facts, HitTarget};

    pub(crate) fn screen(m: &ViewModel, facts: &Facts, w: u16, h: u16) -> (Vec<String>, ratatui::buffer::Buffer, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let regions = Regions::compute(Rect::new(0, 0, w, h), &TuiSection::default(), facts);
        let mut hits = Hits::default();
        t.draw(|f| draw(f, m, &regions, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect();
        (lines, buf, hits)
    }

    fn row(name: &str, kind: RowKind, cursor: bool, selected: bool, index: usize) -> PaneLine {
        let size: String = if kind == RowKind::Dir { String::new() } else { "1 KiB".into() };
        PaneLine::Item(Row {
            index,
            name: name.into(),
            kind,
            class: if kind == RowKind::Dir { FileClass::Folder } else { FileClass::Text },
            cells: vec![String::new(), size.clone()],
            size,
            cursor,
            selected,
            ..Row::default()
        })
    }

    fn pane(index: usize, focused: bool) -> PaneVm {
        PaneVm {
            index,
            id: PaneId(7 + index as u64),
            vis_version: 3,
            focused,
            lines: vec![
                row("sub", RowKind::Dir, false, false, 0),
                row("a.txt", RowKind::File, true, false, 1),
                row("b.txt", RowKind::File, false, true, 2),
            ],
            top: 0,
            total: 3,
            message: None,
            columns: vec![pane::columns::ColumnVm::new(Column::Name), pane::columns::ColumnVm::new(Column::Size)],
            ..PaneVm::default()
        }
    }

    pub(crate) fn model() -> ViewModel {
        ViewModel {
            tabs: TabsVm::default(),
            panes: vec![pane(0, true)],
            crumbs: CrumbsVm::plain("/data"),
            sidebar: SidebarVm::default(),
            status: StatusVm {
                visible: true,
                counts: "3 items · 1 selected".into(),
                hint: "Ctrl+Q quit".into(),
                ..StatusVm::default()
            },
            toasts: ToastsVm::default(),
            palette: PaletteVm::default(),
            dialog: None,
        }
    }

    #[test]
    fn list_with_cursor_and_selection() {
        let (lines, buf, hits) = screen(&model(), &Facts::default(), 40, 8);
        assert_eq!(lines[0], "/data");
        // T2.3: the details view's column header, then the rows.
        assert!(lines[1].starts_with("  Name") && lines[1].ends_with("Size"), "{:?}", lines[1]);
        assert_eq!(lines[2], "  sub/");
        assert!(lines[3].starts_with("  a.txt") && lines[3].ends_with("1 KiB"), "{:?}", lines[3]);
        assert!(lines[4].starts_with("* b.txt"), "{:?}", lines[4]);
        assert!(lines[7].starts_with(" 3 items · 1 selected") && lines[7].ends_with("Ctrl+Q quit"), "{:?}", lines[7]);
        // The cursor row is reversed across the whole width, the others are not.
        assert!(buf[(0, 3)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(39, 3)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(0, 2)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(2, 4)].modifier.contains(Modifier::BOLD));
        // Rows are click targets; below them is the pane's background.
        assert_eq!(hits.at(5, 3), Some(HitTarget::PaneRow { pane: PaneId(7), vis_version: 3, index: 1 }));
        assert_eq!(hits.at(5, 5), Some(HitTarget::PaneBackground { pane: PaneId(7) }));
    }

    #[test]
    fn long_names_keep_the_extension() {
        let mut m = model();
        m.panes[0].lines[1] = row("a_very_long_file_name_indeed.txt", RowKind::File, true, false, 1);
        let (lines, _, _) = screen(&m, &Facts::default(), 24, 8);
        // 24 columns: no room for the size column (11 + 1) next to a 12-cell name, so the
        // name takes all but the mark's 2 cells.
        assert_eq!(lines[3], "  a_very_lo…e_indeed.txt", "{:?}", lines[3]);
    }

    #[test]
    fn empty_message_and_toasts() {
        let mut m = model();
        m.panes[0].lines.clear();
        m.panes[0].total = 0;
        m.panes[0].message = Some("This folder is empty".into());
        let toast = |id: u64, kind: ToastKind, title: &str, body: Option<&str>| ToastLine {
            id,
            kind,
            title: title.into(),
            body: body.map(Into::into),
            action: None,
            sticky: false,
        };
        m.toasts.lines =
            vec![toast(1, ToastKind::Info, "Copied", None), toast(2, ToastKind::Error, "Failed", Some("why"))];
        let (lines, buf, hits) = screen(&m, &Facts::default(), 80, 12);
        // Row 1 is the details header (T2.3), the message is on row 2.
        assert!(lines[2].contains("This folder is empty"), "{lines:?}");
        // Toasts float over the list in the top-right corner, newest first.
        assert!(lines[2].ends_with("│ [x] Failed           │"), "{lines:?}");
        assert!(lines[3].contains("│ why "), "{lines:?}");
        assert!(lines[6].contains("[i] Copied"), "{lines:?}");
        assert!(buf[(62, 2)].modifier.contains(Modifier::BOLD));
        assert_eq!(hits.at(60, 2), Some(HitTarget::Toast(1)));
        assert_eq!(hits.at(60, 6), Some(HitTarget::Toast(0)));
    }

    #[test]
    fn dialog_fallback_box() {
        let mut m = model();
        m.dialog = Some(DialogView {
            title: "Move “a.txt” to the trash?".into(),
            body: vec!["You can restore it from the trash.".into()],
            hints: "Enter/Y confirm · N/Esc cancel".into(),
        });
        let (lines, _, _) = screen(&m, &Facts::default(), 60, 12);
        let all = lines.join("\n");
        assert!(all.contains("Move “a.txt” to the trash?"), "{all}");
        assert!(all.contains("You can restore it from the trash."), "{all}");
        assert!(all.contains("Enter/Y confirm · N/Esc cancel"), "{all}");
    }

    #[test]
    fn dual_panes_draw_side_by_side() {
        let mut m = model();
        m.panes = vec![pane(0, false), pane(1, true)];
        let facts = Facts { shown: 2, focused: 1, ..Facts::default() };
        let (lines, buf, hits) = screen(&m, &facts, 80, 8);
        // Row 1 holds the pane titles (T2.4), row 2 the details headers (T2.3), the
        // lists start on row 3.
        assert!(lines[3].starts_with("  sub/"), "{:?}", lines[3]);
        assert_eq!(buf[(43, 3)].symbol(), "s", "the second pane starts after the gap");
        // Only the focused pane's cursor is reversed.
        assert!(!buf[(0, 4)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(41, 4)].modifier.contains(Modifier::REVERSED));
        assert_eq!(hits.at(45, 4), Some(HitTarget::PaneRow { pane: PaneId(8), vis_version: 3, index: 1 }));
    }

    /// Every view at every tiny size: nothing may panic (ratatui panics on writes outside
    /// the buffer), whatever is open.
    #[test]
    fn draw_every_view_at_tiny_sizes() {
        let mut m = model();
        let warning = ToastLine {
            id: 1,
            kind: ToastKind::Warning,
            title: "日本語 warning".into(),
            body: Some("body".into()),
            action: Some("Undo (Ctrl+Z)".into()),
            sticky: true,
        };
        m.toasts.lines = vec![warning; 3];
        m.panes[0].lines.insert(0, PaneLine::Header { group: 0, label: "Today".into(), count: 3 });
        let mut with_dialog = m.clone();
        with_dialog.dialog = Some(DialogView {
            title: "A long title with 日本語 and 👨\u{200d}👩\u{200d}👧".into(),
            body: vec!["body ".repeat(20), String::new()],
            hints: "Enter confirm".into(),
        });
        let mut quad = m.clone();
        quad.panes = (0..4).map(|i| pane(i, i == 2)).collect();
        let mut dual = m.clone();
        dual.panes = vec![pane(0, true), pane(1, false)];
        let cases = [
            (&m, Facts { toasts: 3, ..Facts::default() }),
            (&with_dialog, Facts { toasts: 3, sidebar: true, tab_strip: true, ..Facts::default() }),
            (&dual, Facts { shown: 2, ..Facts::default() }),
            (&dual, Facts { shown: 2, orientation: Orientation::Horizontal, ..Facts::default() }),
            (&quad, Facts { shown: 4, focused: 2, ..Facts::default() }),
        ];
        for (vm, facts) in cases {
            for w in 0..=20 {
                for h in 0..=6 {
                    let regions = Regions::compute(Rect::new(0, 0, w, h), &TuiSection::default(), &facts);
                    // The model must match the regions: as many panes as drawn.
                    let mut vm = vm.clone();
                    vm.panes = regions
                        .panes
                        .iter()
                        .map(|p| {
                            vm.panes.iter().find(|v| v.index == p.index).cloned().unwrap_or_else(|| pane(p.index, true))
                        })
                        .collect();
                    let _ = screen(&vm, &facts, w, h);
                }
            }
        }
    }
}

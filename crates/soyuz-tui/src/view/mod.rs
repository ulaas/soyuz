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
//! | `dialogs/*` | [`DialogVm`] (one file per dialog kind) | `draw_dialog` | T3 |
//! | `menu.rs` | [`MenuVm`] | `draw_menu` | T3 |
//! | `rename.rs` | `EditVm` (on `Row::edit`) | `draw_edit` (from `draw_pane`) | T3 |
//! | `jobs.rs` | [`JobsVm`] | `draw_jobs` | T3 |
//! | `preview.rs` | [`PreviewVm`] | `draw_preview` | T4 |
//! | `quicklook.rs` | [`QuickLookVm`] | `draw_quicklook` | T4 |
//! | `editor/` | [`EditorVm`] | `draw_editor` | T4 |
//! | `pane/grid.rs` | grid / gallery (in `PaneVm`) | | T4 |
//! | `terminal.rs` | [`TerminalVm`] | `draw_terminal` | T5 |
//! | `search.rs` | [`SearchVm`] | `draw_search` | T5 |
//! | `ai.rs` | [`AiVm`] | `draw_ai` | T5 |
//! | `compare.rs` | [`CompareVm`] | `draw_compare` | T5 |
//! | `settings.rs` | [`SettingsVm`] | `draw_settings` | T6 |
//! | `screens/` | [`ScreensVm`] (treemap, duplicate finder) | `draw_screens` | T7 |
//! | `text.rs` | width-aware truncation / wrapping | | shared |
//!
//! Drawing order (later on top): tabs, crumbs, sidebar, panes, pane chrome, preview,
//! editor, AI panel, the bottom panel (jobs or terminal), search bar, toasts, status,
//! address completions, Quick Look, compare screen, tool screens, settings editor,
//! palette, menu, dialog.
//!
//! All names pass through [`crate::dialog::safe`], so a file name can never write escape
//! sequences to the terminal.

pub mod ai;
pub mod compare;
pub mod crumbs;
pub mod dialogs;
pub mod editor;
pub mod jobs;
pub mod menu;
pub mod palette;
pub mod pane;
pub mod preview;
pub mod quicklook;
pub mod rename;
pub mod screens;
pub mod search;
pub mod settings;
pub mod settings_keys;
pub mod sidebar;
pub mod status;
pub mod tabs;
pub mod terminal;
pub mod text;
pub mod toasts;

/// Phase T8 tier-A snapshots: hand-built models drawn and compared with
/// `tests/snapshots/view__*.snap` (`crate::snapshot`).
#[cfg(test)]
mod snapshots;

/// Phase T8 screen-reader mode: the look, the caret, the status sentence (`crate::a11y`).
#[cfg(test)]
mod sr_tests;

use ratatui::Frame;
use soyuz_appcore::core::AppCore;

pub use self::ai::AiVm;
pub use self::compare::CompareVm;
pub use self::crumbs::CrumbsVm;
pub use self::dialogs::DialogVm;
pub use self::editor::EditorVm;
pub use self::jobs::JobsVm;
pub use self::menu::MenuVm;
pub use self::palette::PaletteVm;
pub use self::pane::{PaneLine, PaneMode, PaneVm, Row, RowKind};
pub use self::preview::PreviewVm;
pub use self::quicklook::QuickLookVm;
pub use self::screens::ScreensVm;
pub use self::search::SearchVm;
pub use self::settings::SettingsVm;
pub use self::sidebar::SidebarVm;
pub use self::status::StatusVm;
pub use self::tabs::TabsVm;
pub use self::terminal::TerminalVm;
pub use self::toasts::{MAX_TOASTS, ToastLine, ToastsVm};
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
    /// The bottom panel (T3).
    pub jobs: JobsVm,
    /// The preview pane (T4).
    pub preview: PreviewVm,
    /// The editor panel (T4).
    pub editor: EditorVm,
    /// Quick Look (T4).
    pub quicklook: QuickLookVm,
    /// The open popup menu (T3).
    pub menu: MenuVm,
    /// The top-most dialog of either stack.
    pub dialog: Option<DialogVm>,
    /// The search bar and its status row (T5).
    pub search: SearchVm,
    /// The embedded terminal pane (T5).
    pub terminal: TerminalVm,
    /// The AI assistant panel (T5).
    pub ai: AiVm,
    /// The compare screen (T5).
    pub compare: CompareVm,
    /// The settings editor (T6).
    pub settings: SettingsVm,
    /// The full-screen tools (T7).
    pub screens: ScreensVm,
    /// Screen-reader mode (T8): the location of the panes not drawn, for the "Other pane:
    /// <path>" line (`crate::a11y`); `None` outside screen-reader mode or with one pane.
    pub other_pane: Option<String>,
}

impl ViewModel {
    /// Build the frame's data from the core and the TUI's input state, after [`prepare`]
    /// ran for `regions`.
    pub fn from_core(core: &AppCore, input: &InputState, regions: &Regions) -> ViewModel {
        let mut status = StatusVm::from_core(core);
        crate::input::filter::decorate(&input.filter, core, &mut status, regions.area.width);
        status::decorate_search(core, input, &mut status);
        let mut panes: Vec<PaneVm> = regions.panes.iter().filter_map(|p| PaneVm::from_core(core, p.index)).collect();
        rename::decorate(core, input, &mut panes);
        ViewModel {
            tabs: TabsVm::from_core(core),
            panes,
            crumbs: CrumbsVm::from_core(core, &input.address),
            sidebar: SidebarVm::from_core(core, &input.sidebar.nav),
            status,
            toasts: ToastsVm::from_core(core),
            palette: PaletteVm::from_core(core, &input.palette.view),
            jobs: JobsVm::from_core(core, input),
            preview: PreviewVm::from_core(core, input),
            editor: EditorVm::from_core(core, input),
            quicklook: QuickLookVm::from_core(core, input),
            menu: MenuVm::from_input(input),
            dialog: dialogs::from_core(core, input),
            search: SearchVm::from_core(core, input, regions),
            terminal: TerminalVm::from_core(core, input, regions),
            ai: AiVm::from_core(core, input, regions),
            compare: CompareVm::from_core(core, input, regions),
            settings: SettingsVm::from_core(core, input, regions),
            screens: ScreensVm::from_core(core, input, regions),
            other_pane: crate::a11y::other_pane(core, regions),
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
    jobs::prepare(core, input, regions);
    preview::prepare(core, input, regions);
    editor::prepare(core, input, regions);
    quicklook::prepare(core, input, regions);
    // The media player (Phase V3b): for the file Quick Look or the preview pane shows.
    crate::player::prepare(core, input);
    search::prepare(core, input, regions);
    terminal::prepare(core, input, regions);
    ai::prepare(core, input, regions);
    compare::prepare(core, input, regions);
    screens::prepare(core, input, regions);
    settings::prepare(core, input, regions);
    crate::input::dialogs::prepare(core, &mut input.dialogs);
    crate::input::dialogs::batch_rename::tick(core, &mut input.dialogs);
    crate::input::dialogs::attributes::tick(core, &mut input.dialogs);
    crate::input::dialogs::link::tick(core, &mut input.dialogs);
    crate::input::dialogs::tick(core, &mut input.dialogs);
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
    preview::draw_preview(f, regions.preview, &m.preview, theme, hits);
    editor::draw_editor(f, regions.editor, &m.editor, theme, hits);
    ai::draw_ai(f, regions.ai, &m.ai, theme, hits);
    // The bottom panel: whichever of the two is shown draws (the other's model is hidden).
    jobs::draw_jobs(f, regions.bottom, &m.jobs, theme, hits);
    terminal::draw_terminal(f, regions.bottom, &m.terminal, theme, hits);
    search::draw_search(f, regions.search, &m.search, theme, hits);
    toasts::draw_toasts(f, toasts::area(regions), &m.toasts, theme, hits);
    status::draw_status(f, regions.status, &m.status, theme, hits);
    crumbs::draw_completions(f, regions.crumbs, regions.area, &m.crumbs, theme, hits);
    quicklook::draw_quicklook(f, regions.area, &m.quicklook, theme, hits);
    compare::draw_compare(f, regions.area, &m.compare, theme, hits);
    screens::draw_screens(f, regions.area, &m.screens, theme, hits);
    settings::draw_settings(f, regions.area, &m.settings, theme, hits);
    palette::draw_palette(f, regions.area, &m.palette, theme, hits);
    menu::draw_menu(f, regions.area, &m.menu, theme, hits);
    if let Some(d) = &m.dialog {
        dialogs::draw_dialog(f, regions.area, d, theme, hits);
    }
    // A wide character whose second cell an overlay's edge drew over would hide that edge.
    text::unstraddle(f.buffer_mut());
    // Screen-reader mode (T8): its own lines ("Other pane: …", the status sentence) and the
    // terminal's cursor on the focused element (the caret recorded last).
    crate::a11y::draw(f, regions, m, theme, hits);
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
            jobs: JobsVm::default(),
            preview: PreviewVm::default(),
            editor: EditorVm::default(),
            quicklook: QuickLookVm::default(),
            menu: MenuVm::default(),
            dialog: None,
            search: SearchVm::default(),
            terminal: TerminalVm::default(),
            ai: AiVm::default(),
            compare: CompareVm::default(),
            settings: SettingsVm::default(),
            screens: ScreensVm::default(),
            other_pane: None,
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
        m.dialog = Some(DialogVm {
            title: "Move “a.txt” to the trash?".into(),
            body: vec!["You can restore it from the trash.".into()],
            hints: "Enter/Y confirm · N/Esc cancel".into(),
            ..DialogVm::default()
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
        let mut form = crate::widgets::Form::new(
            vec![crate::widgets::Field::text("Name", "日本語.txt"), crate::widgets::Field::check("All", true)],
            vec![crate::widgets::Button::new("OK"), crate::widgets::Button::new("Cancel")],
        );
        form.error = Some("error ".repeat(10));
        with_dialog.dialog = Some(DialogVm {
            title: "A long title with 日本語 and 👨\u{200d}👩\u{200d}👧".into(),
            body: vec!["body ".repeat(20), String::new()],
            hints: "Enter confirm".into(),
            list: vec![crate::widgets::ListRow::new("row"); 4],
            ..DialogVm::from_form("t", &form)
        });
        let mut with_menu = m.clone();
        let cmd = crate::input::menu::MenuCmd::Action;
        with_menu.menu.menu = Some(crate::widgets::Menu::new(
            vec![
                crate::widgets::MenuItem::run("Open 日本語", cmd(soyuz_core::config::Action::Open)).chord("Enter"),
                crate::widgets::MenuItem::Separator,
                crate::widgets::MenuItem::submenu(
                    "More",
                    vec![crate::widgets::MenuItem::run("Copy", cmd(soyuz_core::config::Action::Copy))],
                ),
            ],
            (3, 2),
        ));
        let panels = Facts { bottom_rows: 3, preview: true, editor: true, ..Facts::default() };
        let mut quad = m.clone();
        quad.panes = (0..4).map(|i| pane(i, i == 2)).collect();
        let mut dual = m.clone();
        dual.panes = vec![pane(0, true), pane(1, false)];
        // Phase T5 / T6 (W4): the models of the new regions and overlays, as their own
        // modules' tests build them (the AI panel by hand: its file belongs to the composer).
        let mut t56 = dual.clone();
        t56.search = search::tests::sample();
        t56.terminal = terminal::tests::vm();
        t56.ai = AiVm {
            visible: true,
            focused: true,
            title: "Assistant 日本語".into(),
            status: "Waiting for your approval".into(),
            running: true,
            chips: vec![ai::AiLine::plain(
                "[Stop Esc] [fake · move] [Contents] [Tools] 1.2k tok",
                crate::theme::Role::Muted,
            )],
            tabs: vec!["Chat".into(), "Recipes".into(), "Audit log".into()],
            lines: vec![ai::AiLine::plain("> move it ".repeat(8), crate::theme::Role::Title); 5],
            footer: vec![ai::AiLine::plain("2 plans wait", crate::theme::Role::Warning)],
            hints: "Enter send · Tab cards".into(),
            ..AiVm::default()
        };
        let mut compare = t56.clone();
        compare.compare = compare::tests::vm();
        let mut settings = t56.clone();
        settings.settings = settings::tests::vm();
        let mut tables = settings.clone();
        tables.dialog = with_dialog.dialog.clone();
        let cases = [
            (&m, Facts { toasts: 3, ..Facts::default() }),
            (&with_dialog, Facts { toasts: 3, sidebar: true, tab_strip: true, ..Facts::default() }),
            (&dual, Facts { shown: 2, ..Facts::default() }),
            (&dual, Facts { shown: 2, orientation: Orientation::Horizontal, ..Facts::default() }),
            (&quad, Facts { shown: 4, focused: 2, ..Facts::default() }),
            (&with_menu, panels.clone()),
            (&with_dialog, Facts { editor_maximized: true, ..panels.clone() }),
            // Phase T5 / T6: the search bar, the AI panel (beside or over the body).
            (&dual, Facts { shown: 2, search_rows: 2, ai: true, ..Facts::default() }),
            (&with_menu, Facts { search_rows: 1, ai: true, ai_ratio: 0.6, ..panels.clone() }),
            // W4: every T5 / T6 region filled at once (search bar + status row, terminal in
            // the bottom panel, AI panel beside or over the body) ...
            (&t56, Facts { shown: 2, search_rows: 3, ai: true, bottom_rows: 3, ..Facts::default() }),
            (&t56, Facts { search_rows: 2, ai: true, ai_ratio: 0.25, ..panels.clone() }),
            // ... and the full-screen overlays: the compare screen, the settings editor and
            // a table editor dialog over it.
            (&compare, Facts { bottom_rows: 3, search_rows: 1, ..Facts::default() }),
            (&settings, Facts { shown: 2, ai: true, ..Facts::default() }),
            (&tables, Facts { toasts: 3, ..panels }),
        ];
        // Phase T7: the full-screen tools (`view::screens`), over every region at once.
        let t7: Vec<ViewModel> =
            screens::tests::tiny_vms().into_iter().map(|s| ViewModel { screens: s, ..t56.clone() }).collect();
        let t7_facts = Facts { shown: 2, search_rows: 1, ai: true, bottom_rows: 3, ..Facts::default() };
        let cases = cases.into_iter().chain(t7.iter().map(|vm| (vm, t7_facts.clone())));
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

    /// The dialog boxes of Phase T7, shaped as their kinds build them (the kinds' `vm`s
    /// need a core; `headless::e2e::t7_views_at_tiny_sizes_*` draws the real ones).
    fn t7_dialogs() -> Vec<DialogVm> {
        use soyuz_appcore::tr;

        use crate::widgets::{Button, Field, Form, ListRow};
        let dialog = |title: &str, fields: Vec<Field>, buttons: &[&str], error: Option<&str>| {
            let mut form = Form::new(fields, buttons.iter().map(|b| Button::new(*b).accel('x')).collect());
            form.error = error.map(Into::into);
            DialogVm::from_form(title, &form)
        };
        let row = |text: &str, right: &str| ListRow { text: text.into(), right: right.into(), role: None };
        let long = "日本語 ".repeat(12);
        let about = DialogVm {
            body: vec!["Soyuz 0.2.0".into(), "A keyboard-first file explorer".into()],
            list: vec![row("Commit", "4329018"), row("Config folder", &long), row("Memory", "512 MiB")],
            hints: tr!("tui-workspaces-about-hints").into(),
            ..dialog(tr!("dialogs-about-title"), Vec::new(), &["Copy info", "Close"], None)
        };
        let save = DialogVm {
            hints: tr!("tui-workspaces-save-hints").into(),
            ..dialog(
                tr!("workspaces-save-title"),
                vec![Field::text(tr!("tui-workspaces-name"), long.clone())],
                &["Save", "Cancel"],
                Some(tr!("tui-workspaces-name-empty")),
            )
        };
        let manage = DialogVm {
            list: vec![row("one", "Ctrl+F12"), row(&long, ""), row("three", "Ctrl+Alt+Shift+F11")],
            hints: tr!("tui-workspaces-manage-hints").into(),
            ..dialog(
                tr!("workspaces-manage-title"),
                vec![
                    Field::text(tr!("tui-workspaces-name"), "one"),
                    Field::text(tr!("tui-workspaces-hotkey"), "Nope+"),
                ],
                &["Open", "Edit", "Move up", "Delete", "Close"],
                Some("“Nope+” is not a key chord"),
            )
        };
        let history = DialogVm {
            list: ["data", "`-a", "  |-b", "  `-c 日本語", "    `-d"].iter().map(|t| row(t, "12:00")).collect(),
            hints: tr!("tui-workspaces-history-hints").into(),
            ..dialog(tr!("workspaces-history-title"), Vec::new(), &["Go", "Close"], None)
        };
        let shelf = DialogVm {
            list: vec![
                row("[x] a.txt", "1 KiB · data"),
                row("[ ] gone.txt", tr!("tools-shelf-missing")),
                row(&long, ""),
            ],
            hints: tr!("tui-shelf-hints").into(),
            ..dialog(
                tr!("tools-shelf-title"),
                Vec::new(),
                &[tr!("tools-shelf-copy-here"), tr!("tools-shelf-move-here"), "Remove", "Clear", "Close"],
                None,
            )
        };
        let empty_shelf =
            DialogVm { list: Vec::new(), body: vec![tr!("tui-shelf-empty-body").into()], ..shelf.clone() };
        let split = DialogVm {
            body: vec![format!("File: {long} · 3 MiB"), "Part 2 of 3 · 45%".into()],
            list: ["700 MB", "1 GB", "2 GB", "4 GB", "4.7 GB", "Custom size"].iter().map(|t| row(t, "")).collect(),
            hints: tr!("tui-split-running-hints").into(),
            ..dialog(
                tr!("tools-split-title"),
                vec![Field::text(tr!("tui-split-custom"), "1 MB"), Field::text(tr!("tui-split-output"), long.clone())],
                &["Split", "Close"],
                None,
            )
        };
        let mut secret = Field::text("Passphrase", "hunter2");
        secret.set_secret(true);
        let unlock = DialogVm {
            body: vec!["Enter the passphrase or the 48-digit recovery key.".into()],
            hints: tr!("tui-devices-unlock-hints").into(),
            ..dialog(&format!("Unlock {long}"), vec![secret], &["Unlock", "Cancel"], Some("Operation not permitted"))
        };
        let versions = DialogVm {
            body: vec![tr!("tools-versions-admin-needed").into(), "only administrators can read ".repeat(6)],
            list: vec![row("Shadow copy · 8 Oct 2026 14:00", "2.0 KiB"), row(&long, "")],
            hints: tr!("tui-versions-hints").into(),
            ..dialog(
                &format!("Previous versions: {long}"),
                Vec::new(),
                &["Open", "Compare", "Copy to", "Restore"],
                None,
            )
        };
        vec![about, save, manage, history, shelf, empty_shelf, split, unlock, versions]
    }

    /// Phase T7 (wave 2): every new dialog and both full-screen tools at tiny sizes (down
    /// to nothing) in every look: mono, ASCII lines, no lines with Nerd Font icons,
    /// 24-bit colour with Unicode icons and 16 colours with ASCII icons.
    #[test]
    fn draw_t7_views_at_tiny_sizes_in_every_look() {
        use soyuz_appcore::theme::tokens::{ORBIT_DARK, ORBIT_LIGHT};
        use soyuz_core::config::TuiBorders;

        use crate::icons::IconSet;
        use crate::theme::ColorMode;
        let looks = [
            TuiTheme::plain(),
            TuiTheme::plain().with_borders(TuiBorders::Ascii),
            TuiTheme::plain().with_icons(IconSet::Nerd).with_borders(TuiBorders::None),
            TuiTheme::from_tokens(&ORBIT_DARK, ColorMode::Truecolor, IconSet::Unicode),
            TuiTheme::from_tokens(&ORBIT_LIGHT, ColorMode::Ansi16, IconSet::Ascii).with_borders(TuiBorders::Ascii),
        ];
        let base = model();
        let mut vms: Vec<ViewModel> =
            t7_dialogs().into_iter().map(|d| ViewModel { dialog: Some(d), ..base.clone() }).collect();
        vms.extend(screens::tests::tiny_vms().into_iter().map(|s| ViewModel { screens: s, ..base.clone() }));
        let facts = [Facts::default(), Facts { sidebar: true, tab_strip: true, toasts: 1, ..Facts::default() }];
        let sizes = (0..=24).flat_map(|w| (0..=8).map(move |h| (w, h))).chain([(40, 12), (80, 24), (120, 34)]);
        for (w, h) in sizes {
            for f in &facts {
                let regions = Regions::compute(Rect::new(0, 0, w, h), &TuiSection::default(), f);
                for vm in &vms {
                    let mut vm = vm.clone();
                    vm.panes = regions.panes.iter().map(|p| pane(p.index, true)).collect();
                    for theme in &looks {
                        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                        let mut hits = Hits::default();
                        t.draw(|fr| draw(fr, &vm, &regions, theme, &mut hits)).unwrap();
                    }
                }
            }
        }
    }
}

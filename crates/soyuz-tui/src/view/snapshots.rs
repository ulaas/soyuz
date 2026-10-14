//! Tier-A snapshots (Phase T8 P2): hand-built `ViewModel`s drawn at fixed sizes and
//! compared with `tests/snapshots/view__*.snap` (`crate::snapshot::assert_snapshot`,
//! bless with `SOYUZ_BLESS_SNAPSHOTS=1`). Fully deterministic: no file system, no clock,
//! fixed themes (`TuiTheme::from_tokens` with the built-in Orbit tokens, or
//! `TuiTheme::plain`; never `for_core`, which reads the OS look), en-US.
//!
//! - Every part in the default look: Unicode icons, rounded lines, 16 colours.
//! - Four composites (browse dual, the delete prompt, the palette, the jobs panel) also in
//!   the `ascii` look (ASCII icons and lines) and the `mono` look (no colours); three in
//!   24-bit colour.
//!
//! A new snapshot: build the model, call [`snap`] with a new name, run the tests with
//! `SOYUZ_BLESS_SNAPSHOTS=1`, read the new `.snap` file, then run them again without it.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use soyuz_appcore::file_class::FileClass;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::models::jobs_rows::Tone;
use soyuz_appcore::models::toasts::ToastKind;
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::theme::tokens::ORBIT_DARK;
use soyuz_core::config::{Action, Orientation, TuiBorders, TuiSection};
use soyuz_core::{Column, SortDir};

use super::*;
use crate::hits::{CardButton, JobButton};
use crate::icons::{self, IconSet};
use crate::input::TuiFocus;
use crate::input::menu::MenuCmd;
use crate::layout::{Facts, HitTarget};
use crate::snapshot::{SnapOpts, assert_snapshot};
use crate::theme::{ColorMode, Role};
use crate::view::palette::PaletteLine;
use crate::widgets::{Button, Field, Form, ListRow, Menu, MenuItem};

/// The standard size.
const W: u16 = 100;
const H: u16 = 30;

/// A theme and icon set to draw in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    /// Unicode icons, rounded lines, 16 colours (`[tui]` defaults on a dark terminal).
    Default,
    /// ASCII icons and lines (`icons = "ascii"`, `borders = "ascii"`).
    Ascii,
    /// No colours (`color_mode = "mono"`).
    Mono,
    /// 24-bit colour from the built-in dark tokens.
    Truecolor,
    /// Screen-reader mode (`crate::a11y`) over the default look: no box lines, ASCII
    /// markers, the one-pane layout (`Facts::screen_reader` is set by [`render`]).
    ScreenReader,
}

impl Look {
    fn theme(self) -> TuiTheme {
        match self {
            Look::Default => TuiTheme::from_tokens(&ORBIT_DARK, ColorMode::Ansi16, IconSet::Unicode),
            Look::Ascii => {
                TuiTheme::from_tokens(&ORBIT_DARK, ColorMode::Ansi16, IconSet::Ascii).with_borders(TuiBorders::Ascii)
            }
            Look::Mono => TuiTheme::from_tokens(&ORBIT_DARK, ColorMode::Mono, IconSet::Unicode),
            Look::Truecolor => TuiTheme::from_tokens(&ORBIT_DARK, ColorMode::Truecolor, IconSet::Unicode),
            Look::ScreenReader => Look::Default.theme().with_screen_reader(true),
        }
    }

    fn icons(self) -> IconSet {
        if matches!(self, Look::Ascii | Look::ScreenReader) { IconSet::Ascii } else { IconSet::Unicode }
    }

    /// The snapshot name's suffix.
    fn suffix(self) -> &'static str {
        match self {
            Look::Default => "",
            Look::Ascii => "__ascii",
            Look::Mono => "__mono",
            Look::Truecolor => "__truecolor",
            Look::ScreenReader => "__sr",
        }
    }
}

/// Draw `m` like `App::frame` does (base style, then `view::draw`) into a `w` × `h`
/// terminal.
fn render(m: &ViewModel, facts: &Facts, w: u16, h: u16, look: Look) -> (Buffer, Regions) {
    let (buf, regions, _) = render_with_cursor(m, facts, w, h, look);
    (buf, regions)
}

/// [`render`], plus the terminal's cursor when it is shown (screen-reader mode puts it on
/// the focused element; it is not part of the buffer).
fn render_with_cursor(
    m: &ViewModel,
    facts: &Facts,
    w: u16,
    h: u16,
    look: Look,
) -> (Buffer, Regions, Option<ratatui::layout::Position>) {
    let theme = look.theme();
    let sr = look == Look::ScreenReader;
    let facts = Facts { screen_reader: sr || facts.screen_reader, ..facts.clone() };
    let regions = Regions::compute(Rect::new(0, 0, w, h), &TuiSection::default(), &facts);
    let mut m = m.clone();
    for p in &mut m.panes {
        p.icons = look.icons();
    }
    if sr {
        // Like `ViewModel::from_core`: models only for the drawn panes (the focused one).
        m.panes.retain(|p| regions.panes.iter().any(|r| r.index == p.index));
    }
    assert_eq!(m.panes.len(), regions.panes.len(), "one pane model per drawn pane");
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut hits = Hits::default();
    t.draw(|f| {
        let area = f.area();
        f.buffer_mut().set_style(area, theme.base());
        draw(f, &m, &regions, &theme, &mut hits);
    })
    .unwrap();
    let b = t.backend_mut();
    let cursor = b.cursor_visible().then(|| b.cursor_position());
    (t.backend().buffer().clone(), regions, cursor)
}

/// Draw and compare with `view__<name>[__<look>]`.
fn snap(name: &str, m: &ViewModel, facts: &Facts, (w, h): (u16, u16), look: Look) {
    let (buf, _) = render(m, facts, w, h, look);
    assert_snapshot(&format!("view__{name}{}", look.suffix()), &buf, &SnapOpts::styled());
}

// ---- Models ----

fn item(index: usize, name: &str, class: FileClass, cells: [&str; 3]) -> Row {
    let kind = if class == FileClass::Folder { RowKind::Dir } else { RowKind::File };
    Row {
        index,
        name: name.into(),
        kind,
        class,
        icon: icons::class_icon(class),
        size: cells[0].into(),
        cells: vec![String::new(), cells[0].into(), cells[1].into(), cells[2].into()],
        ..Row::default()
    }
}

/// A folder listing: folders first, a wide name, a hidden file, the cursor on
/// `notes.txt`, `report.pdf` selected.
fn rows() -> Vec<Row> {
    let date = "2026-10-01 12:00";
    let mut rows = vec![
        item(0, "docs", FileClass::Folder, ["", "Folder", date]),
        item(1, "photos", FileClass::Folder, ["", "Folder", date]),
        item(2, "src", FileClass::Folder, ["", "Folder", date]),
        item(3, "README.md", FileClass::Text, ["2.1 KB", "Markdown", date]),
        item(4, "archive.zip", FileClass::Archive, ["14.8 MB", "ZIP archive", date]),
        item(5, "main.rs", FileClass::Code, ["812 B", "Rust source", date]),
        item(6, "notes.txt", FileClass::Text, ["1.0 KB", "Text", date]),
        item(7, "report.pdf", FileClass::Pdf, ["384 KB", "PDF document", date]),
        item(8, "sunset.jpg", FileClass::Image, ["3.2 MB", "JPEG image", date]),
        item(9, "日本語のメモ.txt", FileClass::Text, ["96 B", "Text", date]),
        item(10, ".env", FileClass::Other, ["64 B", "File", date]),
    ];
    rows[6].cursor = true;
    rows[7].selected = true;
    rows[10].hidden = true;
    rows
}

fn details_columns() -> Vec<pane::columns::ColumnVm> {
    let mut name = pane::columns::ColumnVm::new(Column::Name);
    name.sort = Some(SortDir::Asc);
    vec![
        name,
        pane::columns::ColumnVm::new(Column::Size),
        pane::columns::ColumnVm::new(Column::Kind),
        pane::columns::ColumnVm::new(Column::Modified),
    ]
}

fn pane_vm(index: usize, focused: bool, mode: PaneMode) -> PaneVm {
    let mut rows = rows();
    if !focused {
        for r in &mut rows {
            r.selected = false;
        }
    }
    let total = rows.len();
    PaneVm {
        index,
        id: PaneId(10 + index as u64),
        vis_version: 1,
        focused,
        mode,
        lines: rows.into_iter().map(PaneLine::Item).collect(),
        top: 0,
        total,
        columns: if mode == PaneMode::Details { details_columns() } else { Vec::new() },
        icons: IconSet::Unicode,
        ..PaneVm::default()
    }
}

fn base() -> ViewModel {
    ViewModel {
        tabs: TabsVm::default(),
        panes: vec![pane_vm(0, true, PaneMode::Details)],
        crumbs: CrumbsVm { segments: vec!["/".into(), "home".into(), "ada".into(), "data".into()], edit: None },
        sidebar: SidebarVm::default(),
        status: StatusVm {
            visible: true,
            counts: "11 items".into(),
            hidden: String::new(),
            selected: "1 selected (384 KB)".into(),
            smart: String::new(),
            free: "120 GB free".into(),
            mode: "Details".into(),
            sort: "Name ↑".into(),
            hint: "Ctrl+K commands".into(),
            filter: None,
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

fn titles(n: usize, focused: usize) -> Vec<tabs::PaneTitle> {
    let paths = ["/home/ada/data", "/home/ada/backup", "/srv/media", "/tmp"];
    (0..n)
        .map(|i| tabs::PaneTitle { index: i, id: PaneId(10 + i as u64), title: paths[i].into(), focused: i == focused })
        .collect()
}

/// `n` panes (`n` > 1), pane `focused` focused.
fn multi(n: usize, focused: usize) -> (ViewModel, Facts) {
    let mut m = base();
    m.panes = (0..n).map(|i| pane_vm(i, i == focused, PaneMode::Details)).collect();
    m.tabs.panes = titles(n, focused);
    (m, Facts { shown: n, focused, ..Facts::default() })
}

fn dual() -> (ViewModel, Facts) {
    multi(2, 0)
}

fn delete_prompt() -> DialogVm {
    use soyuz_appcore::models::dialogs::{DeleteConfirmModel, DialogModel};
    let paths = ["notes.txt", "report.pdf", "old draft.md"].map(std::path::PathBuf::from).to_vec();
    let m: DialogModel = DeleteConfirmModel::new(paths, soyuz_jobs::DeleteMode::Permanent).into();
    dialogs::delete::vm(&m, &crate::input::dialogs::delete::ui(&m))
}

fn conflict_prompt() -> DialogVm {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use soyuz_appcore::models::dialogs::{DialogModel, PromptModel, PromptShared};
    use soyuz_core::{EntryInfo, EntryKind};
    use soyuz_jobs::{ConflictInfo, ConflictKind, JobId, JobPrompt};
    // No times: the dialog would show them in local time.
    let mut src = EntryInfo::minimal(PathBuf::from("data").join("notes.txt"), EntryKind::File);
    src.size = 4096;
    let mut dst = EntryInfo::minimal(PathBuf::from("backup").join("notes.txt"), EntryKind::File);
    dst.size = 1024;
    let c = ConflictInfo { src, dst, kind: ConflictKind::FileExists, suggested_name: "notes (2).txt".into() };
    let shared = Rc::new(RefCell::new(PromptShared { alive: true, total: 2, ..Default::default() }));
    let m: DialogModel = PromptModel::new(JobId(1), JobPrompt::Conflict(c), "Copy 3 items".into(), shared).into();
    dialogs::prompt::vm(&m, &crate::input::dialogs::prompt::ui(&m))
}

fn form_dialog() -> DialogVm {
    let mut form = Form::new(
        vec![
            Field::text("Name", "New folder"),
            Field::check("Open it after creating", true),
            Field::radio("Kind", vec!["Folder".into(), "Text file".into(), "Link".into()], 0),
        ],
        vec![Button::new("Create").accel('c'), Button::new("Cancel")],
    )
    .default_button(0);
    form.error = Some("“New folder” already exists".into());
    DialogVm { hints: "Enter create · Esc cancel · Tab next".into(), ..DialogVm::from_form("New folder", &form) }
}

fn about_dialog() -> DialogVm {
    let row = |text: &str, right: &str| ListRow { text: text.into(), right: right.into(), role: None };
    let form = Form::new(Vec::new(), vec![Button::new("Copy info").accel('c'), Button::new("Close")]);
    DialogVm {
        body: vec!["Soyuz 0.2.0".into(), "A keyboard-first file explorer".into()],
        // A fixed commit and paths: never `GIT_SHA` or the real config folder.
        list: vec![
            row("Commit", "0123abc"),
            row("Frontend", "Terminal UI (ratatui)"),
            row("Config folder", "/home/ada/.config/soyuz"),
            row("License", "MIT"),
        ],
        hints: "C copy · Esc close".into(),
        ..DialogVm::from_form("About Soyuz", &form)
    }
}

fn palette() -> PaletteVm {
    let row = |index: usize, title: &[(&str, bool)], detail: &str, tag: &str, chord: Option<&str>, selected| {
        PaletteLine::Item(palette::PaletteRow {
            index,
            icon: "›",
            tint: Role::Accent,
            title: title.iter().map(|(t, hl)| (t.to_string(), *hl)).collect(),
            detail: detail.into(),
            tag: Some(tag.into()),
            chord: chord.map(Into::into),
            selected,
        })
    };
    PaletteVm {
        open: true,
        glyph: "›",
        chip: None,
        input: "new".into(),
        cursor: 3,
        placeholder: "Type a command".into(),
        lines: vec![
            PaletteLine::Header("FILE".into()),
            row(0, &[("New", true), (" folder", false)], "Create a folder here", "File", Some("Ctrl+Shift+N"), true),
            row(1, &[("New", true), (" file", false)], "Create an empty file", "File", Some("Ctrl+Alt+N"), false),
            PaletteLine::Header("VIEW".into()),
            row(2, &[("New", true), (" tab", false)], "Open a tab", "View", Some("Ctrl+T"), false),
            row(3, &[("New", true), (" window", false)], "Not in the terminal UI", "View", None, false),
        ],
        top: 0,
        empty: "No matches".into(),
        hints: "↑↓ move · Enter run · Esc close".into(),
        legend: vec![(": path".into(), false), ("@ bookmarks".into(), false), ("> commands".into(), true)],
    }
}

fn job(title: &str, permille: Option<u16>, line: &str) -> jobs::JobItem {
    jobs::JobItem {
        icon: IconId::Copy,
        icon_tone: Tone::Accent,
        title: title.into(),
        dim: false,
        tag: None,
        line: line.into(),
        line_tone: Tone::Muted,
        bar: Some(jobs::Bar { permille, tone: Tone::Accent }),
        stats: "45 MB of 100 MB · 3 of 12 files".into(),
        stats_tone: Tone::Faint,
        right: "12 MB/s · 5 s left".into(),
        buttons: vec![
            jobs::ButtonVm { kind: JobButton::Pause, label: "Pause".into(), enabled: true },
            jobs::ButtonVm { kind: JobButton::Cancel, label: "Cancel".into(), enabled: true },
        ],
        log: Vec::new(),
    }
}

fn jobs_panel() -> JobsVm {
    let mut waiting = job("Moving 2 items to backup", None, "Waiting for the disk");
    waiting.bar = Some(jobs::Bar { permille: None, tone: Tone::Muted });
    waiting.stats = "0 of 2 items".into();
    waiting.right = String::new();
    let mut failed = job("Deleting old-logs", Some(1000), "1 item failed");
    failed.icon = IconId::Trash;
    failed.line_tone = Tone::Danger;
    failed.tag = Some(("Failed".into(), Tone::Danger));
    failed.bar = Some(jobs::Bar { permille: Some(1000), tone: Tone::Danger });
    failed.buttons = vec![jobs::ButtonVm { kind: JobButton::Retry, label: "Retry".into(), enabled: true }];
    failed.log = vec![("old-logs/app.log: Access is denied".into(), Tone::Danger)];
    JobsVm {
        visible: true,
        focused: true,
        tabs: vec!["Jobs · 3".into(), "History".into()],
        tab: 0,
        toolbar: Vec::new(),
        items: vec![job("Copying 12 files to backup", Some(450), "photos/sunset.jpg"), waiting, failed],
        first: 0,
        len: 3,
        cursor: 0,
        empty: None,
        hints: "P pause · C cancel · Enter details".into(),
    }
}

fn menu() -> MenuVm {
    let run = |label: &str, a: Action| MenuItem::run(label, MenuCmd::Action(a));
    MenuVm {
        menu: Some(Menu::new(
            vec![
                run("Open", Action::Open).chord("Enter"),
                run("Open with…", Action::OpenWith),
                MenuItem::Separator,
                run("Cut", Action::Cut).chord("Ctrl+X"),
                run("Copy", Action::Copy).chord("Ctrl+C"),
                run("Rename", Action::Rename).chord("F2"),
                MenuItem::submenu("Copy path", vec![run("Full path", Action::CopyPath)]),
                MenuItem::Separator,
                run("Delete", Action::Delete).chord("Del"),
                run("Properties", Action::Properties).chord("Alt+Enter"),
            ],
            (12, 9),
        )),
    }
}

fn toasts() -> ToastsVm {
    let toast = |id: u64, kind: ToastKind, title: &str, body: Option<&str>, action: Option<&str>| ToastLine {
        id,
        kind,
        title: title.into(),
        body: body.map(Into::into),
        action: action.map(Into::into),
        sticky: false,
    };
    ToastsVm {
        lines: vec![
            toast(1, ToastKind::Success, "Copied 12 files", None, Some("Undo (Ctrl+Z)")),
            toast(2, ToastKind::Warning, "2 items skipped", Some("They already exist in backup"), None),
            toast(3, ToastKind::Error, "Cannot delete old-logs", Some("Access is denied"), None),
        ],
    }
}

fn sidebar() -> SidebarVm {
    use sidebar::{LineKind, Mark, SidebarLine};
    let line = |index: usize, kind: LineKind, label: &str| SidebarLine {
        index,
        kind,
        label: label.into(),
        depth: 0,
        active: false,
        cursor: false,
        dim: false,
        mark: Mark::None,
        right: String::new(),
        eject: false,
    };
    let folder = |expanded| LineKind::Folder { expandable: true, expanded, loading: false, failed: false };
    SidebarVm {
        lines: vec![
            line(0, LineKind::Header { open: true }, "Favorites"),
            SidebarLine { active: true, ..line(1, LineKind::Place, "data") },
            line(2, LineKind::Place, "Documents"),
            line(3, LineKind::Place, "Downloads"),
            line(4, LineKind::Header { open: true }, "Devices"),
            SidebarLine { mark: Mark::Volume, right: "ext4".into(), ..line(5, LineKind::Place, "System") },
            SidebarLine {
                mark: Mark::Volume,
                eject: true,
                right: "exFAT".into(),
                ..line(6, LineKind::Place, "USB stick")
            },
            line(7, LineKind::Header { open: true }, "Folders"),
            SidebarLine { depth: 0, ..line(8, folder(true), "home") },
            SidebarLine { depth: 1, ..line(9, folder(true), "ada") },
            SidebarLine { depth: 2, cursor: true, ..line(10, folder(false), "data") },
            SidebarLine { depth: 2, dim: true, ..line(11, folder(false), ".cache") },
            line(12, LineKind::Header { open: false }, "Servers"),
        ],
        focused: true,
    }
}

fn preview() -> PreviewVm {
    use preview::{Body, PLine};
    let code = [
        "# Soyuz",
        "",
        "A keyboard-first file explorer for Windows, macOS and Linux.",
        "",
        "## Build",
        "",
        "    cargo build --release",
        "",
        "Every file operation is a background job that you can pause,",
        "resume, cancel and undo.",
    ];
    PreviewVm {
        visible: true,
        focused: false,
        title: "README.md".into(),
        class: Some(FileClass::Text),
        kind: "Markdown".into(),
        hex: false,
        body: Body::Lines {
            lines: code.iter().map(|l| PLine::plain(*l, Role::Text)).collect(),
            top: 0,
            total: code.len(),
            gutter: 2,
        },
        info: vec![
            ("Size".into(), "2.1 KB".into()),
            ("Modified".into(), "2026-10-01 12:00".into()),
            ("Type".into(), "Markdown".into()),
        ],
        textual: true,
    }
}

fn editor() -> EditorVm {
    use editor::{BodyVm, TabKind, TabVm};
    EditorVm {
        visible: true,
        focused: true,
        doc: 1,
        tabs: vec![
            TabVm { name: "notes.txt".into(), kind: TabKind::Text, active: true, dirty: true, saving: false },
            TabVm { name: "logo.png".into(), kind: TabKind::Image, ..TabVm::default() },
        ],
        body: BodyVm::Text(editor::text::tests::sample_vm()),
        empty: ("No files open".into(), "Select a file and press F4".into()),
        ..EditorVm::default()
    }
}

fn quicklook() -> QuickLookVm {
    use preview::{Body, PLine};
    QuickLookVm {
        visible: true,
        title: "notes.txt".into(),
        class: Some(FileClass::Text),
        position: "7 / 11".into(),
        size: "1.0 KB".into(),
        preview: PreviewVm {
            visible: true,
            focused: true,
            body: Body::Lines {
                lines: ["Shopping list", "", "- coffee", "- bread", "- 日本茶"]
                    .iter()
                    .map(|l| PLine::plain(*l, Role::Text))
                    .collect(),
                top: 0,
                total: 5,
                gutter: 0,
            },
            ..PreviewVm::default()
        },
        image: None,
        summary: "Text · 1.0 KB · 2026-10-01 12:00".into(),
        hints: "←→ previous / next · Esc close".into(),
    }
}

/// The AI panel with a plan card waiting for approval.
fn ai_plan() -> AiVm {
    use ai::{AiLine, FieldVm, Span};
    let card = HitTarget::AiCard { id: 1, task: 2 };
    let button =
        |label: &str, b: CardButton| Span::button(label, HitTarget::AiCardButton { id: 1, task: 2, button: b }, true);
    let gutter_line = |spans: Vec<Span>| {
        let mut all = vec![Span::gutter(Role::Accent)];
        all.extend(spans);
        AiLine::new(all).with_target(card)
    };
    AiVm {
        visible: true,
        focused: true,
        title: "Assistant".into(),
        status: "Waiting for your approval".into(),
        running: true,
        chips: vec![AiLine::new(vec![
            Span::new("fake · local", Role::Muted),
            Span::new("  ", Role::Text),
            Span::new("Folder: data", Role::Muted),
        ])],
        new_chat: false,
        tabs: vec!["Chat".into(), "Recipes".into(), "Audit log".into()],
        tab: 0,
        lines: vec![
            AiLine::plain("> sort the photos into folders by year", Role::Title),
            AiLine::plain("", Role::Text),
            AiLine::plain("I will move 3 photos into new year folders.", Role::Text),
            AiLine::plain("", Role::Text),
            gutter_line(vec![Span::bold("move_items", Role::Accent), Span::new("  plan · 3 changes", Role::Muted)]),
            gutter_line(vec![Span::new("photos/a.jpg → photos/2024/a.jpg", Role::Text)]),
            gutter_line(vec![Span::new("photos/b.jpg → photos/2025/b.jpg", Role::Text)]),
            gutter_line(vec![Span::new("photos/c.jpg → photos/2025/c.jpg", Role::Text)]),
            gutter_line(vec![
                button("Approve", CardButton::Approve),
                Span::new(" ", Role::Text),
                button("Edit", CardButton::Edit),
                Span::new(" ", Role::Text),
                button("Reject", CardButton::Reject),
            ]),
        ],
        footer: vec![AiLine::plain("1 plan waits for approval", Role::Warning)],
        footer_cursor: None,
        field: Some(FieldVm {
            prefix: "> ".into(),
            lines: vec![String::new()],
            placeholder: "Ask about this folder".into(),
            cursor: None,
            target: Some(HitTarget::AiComposer),
        }),
        hints: "Tab cards · A approve · R reject".into(),
        menu: None,
        menu_screen: Rect::default(),
    }
}

// ---- Snapshots: the default look, every part ----

#[test]
fn browse_details() {
    snap("browse_details", &base(), &Facts::default(), (W, H), Look::Default);
}

#[test]
fn browse_list() {
    let mut m = base();
    m.panes = vec![pane_vm(0, true, PaneMode::List)];
    snap("browse_list", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn browse_grid() {
    let mut m = base();
    let mut p = pane_vm(0, true, PaneMode::Grid);
    p.grid = Some(pane::grid::GridVm { spec: pane::grid::GridSpec::new(96, (10, 20)), ..Default::default() });
    m.panes = vec![p];
    snap("browse_grid", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn browse_miller() {
    use pane::miller::{AuxRow, MillerCol, MillerVm};
    let aux = |index: usize, name: &str, class: FileClass, on_path: bool| AuxRow {
        index,
        name: name.into(),
        kind: if class == FileClass::Folder { RowKind::Dir } else { RowKind::File },
        class,
        icon: icons::class_icon(class),
        hidden: false,
        on_path,
    };
    let mut m = base();
    let mut p = pane_vm(0, true, PaneMode::Miller);
    p.miller = Some(MillerVm {
        parent: MillerCol {
            rows: vec![
                aux(0, "backup", FileClass::Folder, false),
                aux(1, "data", FileClass::Folder, true),
                aux(2, "music", FileClass::Folder, false),
                aux(3, "todo.txt", FileClass::Text, false),
            ],
            message: None,
        },
        child: MillerCol { rows: Vec::new(), message: Some("Text · 1.0 KB".into()) },
    });
    m.panes = vec![p];
    snap("browse_miller", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn browse_dual() {
    let (m, facts) = dual();
    snap("browse_dual", &m, &facts, (W, H), Look::Default);
}

#[test]
fn browse_dual_horizontal() {
    let (m, facts) = dual();
    snap("browse_dual_horizontal", &m, &Facts { orientation: Orientation::Horizontal, ..facts }, (W, H), Look::Default);
}

#[test]
fn browse_quad() {
    let (m, facts) = multi(4, 3);
    snap("browse_quad", &m, &facts, (W, H), Look::Default);
}

#[test]
fn browse_quad_stacked() {
    let (m, facts) = multi(4, 1);
    let (_, regions) = render(&m, &facts, 58, 24, Look::Default);
    assert!(regions.stacked, "58 columns stack the quad layout");
    snap("browse_quad_stacked", &m, &facts, (58, 24), Look::Default);
}

#[test]
fn tabs_strip() {
    use tabs::{GroupChip, StripItem, TabItem};
    let tab = |index: usize, title: &str, pinned: bool, color: Option<u8>| {
        StripItem::Tab(TabItem { index, title: title.into(), pinned, color, closable: !pinned })
    };
    let mut m = base();
    m.tabs.items = vec![
        tab(0, "data", true, None),
        StripItem::Group(GroupChip { label: "work".into(), color: 2, collapsed: false }),
        tab(1, "src", false, Some(2)),
        tab(2, "docs", false, Some(2)),
        StripItem::Group(GroupChip { label: "old 3".into(), color: 5, collapsed: true }),
        tab(6, "日本語のフォルダ", false, None),
    ];
    m.tabs.active = 1;
    snap("tabs_strip", &m, &Facts { tab_strip: true, ..Facts::default() }, (W, H), Look::Default);
}

#[test]
fn crumbs_completion() {
    let mut m = base();
    m.crumbs.edit = Some(crumbs::AddressVm {
        text: "/home/ada/pro".into(),
        cursor: 13,
        focused: true,
        completions: vec!["/home/ada/projects".into(), "/home/ada/prototypes".into(), "/home/ada/profile".into()],
        selected: Some(1),
    });
    snap("crumbs_completion", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn sidebar_places_and_tree() {
    let mut m = base();
    m.sidebar = sidebar();
    snap("sidebar", &m, &Facts { sidebar: true, ..Facts::default() }, (W, H), Look::Default);
}

#[test]
fn status_filter() {
    let mut m = base();
    m.status.counts = "3 of 11 items".into();
    m.status.filter = Some(status::FilterVm { label: "Filter".into(), text: ".txt".into(), cursor: Some(4) });
    m.panes[0].lines.retain(|l| matches!(l, PaneLine::Item(r) if r.name.ends_with(".txt")));
    m.panes[0].total = 3;
    snap("status_filter", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn toasts_stack() {
    let mut m = base();
    m.toasts = toasts();
    snap("toasts", &m, &Facts { toasts: 3, ..Facts::default() }, (W, H), Look::Default);
}

#[test]
fn palette_commands() {
    let mut m = base();
    m.palette = palette();
    snap("palette", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn context_menu() {
    let mut m = base();
    m.menu = menu();
    snap("menu", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn dialog_delete() {
    let mut m = base();
    m.dialog = Some(delete_prompt());
    snap("dialog_delete", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn dialog_conflict() {
    let mut m = base();
    m.dialog = Some(conflict_prompt());
    snap("dialog_conflict", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn dialog_form() {
    let mut m = base();
    m.dialog = Some(form_dialog());
    snap("dialog_form", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn dialog_about() {
    let mut m = base();
    m.dialog = Some(about_dialog());
    snap("dialog_about", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn jobs_progress() {
    let mut m = base();
    m.jobs = jobs_panel();
    snap("jobs", &m, &Facts { bottom_rows: 9, ..Facts::default() }, (W, H), Look::Default);
}

#[test]
fn jobs_empty() {
    let mut m = base();
    m.jobs = JobsVm {
        items: Vec::new(),
        len: 0,
        focused: false,
        tabs: vec!["Jobs".into(), "History".into()],
        empty: Some(("No operations".into(), "Copies, moves and deletes show here.".into())),
        ..jobs_panel()
    };
    snap("jobs_empty", &m, &Facts { bottom_rows: 6, ..Facts::default() }, (W, H), Look::Default);
}

#[test]
fn preview_text() {
    let mut m = base();
    m.preview = preview();
    snap("preview", &m, &Facts { preview: true, ..Facts::default() }, (W, H), Look::Default);
}

#[test]
fn editor_text() {
    let mut m = base();
    m.editor = editor();
    snap("editor", &m, &Facts { editor: true, ..Facts::default() }, (W, H), Look::Default);
}

#[test]
fn quicklook_text() {
    let mut m = base();
    m.quicklook = quicklook();
    snap("quicklook", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn search_bar() {
    let mut m = base();
    m.search = search::tests::sample();
    snap("search", &m, &Facts { search_rows: 3, ..Facts::default() }, (W, H), Look::Default);
}

#[test]
fn ai_plan_card() {
    let mut m = base();
    m.ai = ai_plan();
    snap("ai_plan", &m, &Facts { ai: true, ai_ratio: 0.45, ..Facts::default() }, (W, H), Look::Default);
}

#[test]
fn compare_screen() {
    let mut m = base();
    m.compare = compare::tests::vm();
    snap("compare", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn settings_page() {
    let mut m = base();
    m.settings = settings::tests::vm();
    snap("settings", &m, &Facts::default(), (W, H), Look::Default);
}

#[test]
fn treemap_and_duplicates() {
    let [treemap, dupes]: [ScreensVm; 2] = screens::tests::tiny_vms().try_into().expect("two screens");
    let mut m = base();
    m.screens = treemap;
    snap("treemap", &m, &Facts::default(), (W, H), Look::Default);
    m.screens = dupes;
    snap("duplicates", &m, &Facts::default(), (W, H), Look::Default);
}

// ---- The composites in the other looks ----

const LOOKS: [Look; 2] = [Look::Ascii, Look::Mono];

#[test]
fn composites_in_ascii_and_mono() {
    let (dual_m, dual_f) = dual();
    let mut delete = base();
    delete.dialog = Some(delete_prompt());
    let mut pal = base();
    pal.palette = palette();
    let mut jobs = base();
    jobs.jobs = jobs_panel();
    for look in LOOKS {
        snap("browse_dual", &dual_m, &dual_f, (W, H), look);
        snap("dialog_delete", &delete, &Facts::default(), (W, H), look);
        snap("palette", &pal, &Facts::default(), (W, H), look);
        snap("jobs", &jobs, &Facts { bottom_rows: 9, ..Facts::default() }, (W, H), look);
    }
}

#[test]
fn composites_in_truecolor() {
    let (dual_m, dual_f) = dual();
    let mut delete = base();
    delete.dialog = Some(delete_prompt());
    let mut pal = base();
    pal.palette = palette();
    snap("browse_dual", &dual_m, &dual_f, (W, H), Look::Truecolor);
    snap("dialog_delete", &delete, &Facts::default(), (W, H), Look::Truecolor);
    snap("palette", &pal, &Facts::default(), (W, H), Look::Truecolor);
}

/// The ASCII look leaves no box-drawing or block character on screen.
#[test]
fn the_ascii_look_is_pure_ascii_in_its_lines() {
    let (m, facts) = dual();
    let (buf, _) = render(&m, &facts, W, H, Look::Ascii);
    let text = crate::snapshot::lines(&buf).concat();
    assert!(!text.chars().any(|c| ('\u{2500}'..='\u{259F}').contains(&c)), "{text}");
}

// ---- Screen-reader mode (Phase T8 P6, `crate::a11y`) ----

/// Draw `m` in screen-reader mode with the keyboard on `facts.focus`, compare with
/// `view__<name>__sr`, and check what the buffer cannot show: the terminal's cursor is on
/// the row of the focused element, which contains `want`; no box character is left.
fn snap_sr(name: &str, m: &ViewModel, facts: &Facts, want: &str) {
    let (buf, _, cursor) = render_with_cursor(m, facts, W, H, Look::ScreenReader);
    assert_snapshot(&format!("view__{name}__sr"), &buf, &SnapOpts::styled());
    let rows = crate::snapshot::lines(&buf);
    assert!(!rows.concat().chars().any(crate::a11y::is_box_char), "{name}: box characters left\n{rows:#?}");
    let at = cursor.unwrap_or_else(|| panic!("{name}: the cursor is hidden"));
    let row = &rows[usize::from(at.y)];
    assert!(row.contains(want), "{name}: the cursor is on row {} ({row:?}), not on {want:?}", at.y);
}

fn sr_facts(focus: TuiFocus) -> Facts {
    Facts { focus, ..Facts::default() }
}

#[test]
fn screen_reader_browse() {
    snap_sr("browse_details", &base(), &sr_facts(TuiFocus::List), "notes.txt");
}

#[test]
fn screen_reader_dual_with_the_other_pane_line() {
    let (mut m, facts) = dual();
    m.other_pane = Some("Other pane: /home/ada/backup".into());
    snap_sr("browse_dual", &m, &facts, "notes.txt");
    // The second pane focused: its model is drawn, the first pane's folder is named.
    let (mut m, facts) = multi(2, 1);
    m.other_pane = Some("Other pane: /home/ada/data".into());
    let (buf, regions, _) = render_with_cursor(&m, &facts, W, H, Look::ScreenReader);
    assert_eq!(regions.panes.len(), 1);
    assert_eq!(regions.panes[0].index, 1);
    let rows = crate::snapshot::lines(&buf);
    assert_eq!(rows[usize::from(regions.other_pane.y)].trim_end(), "Other pane: /home/ada/data");
}

#[test]
fn screen_reader_miller_draws_the_current_folder_alone() {
    use pane::miller::{AuxRow, MillerCol, MillerVm};
    let aux = |index: usize, name: &str| AuxRow {
        index,
        name: name.into(),
        kind: RowKind::Dir,
        class: FileClass::Folder,
        icon: icons::class_icon(FileClass::Folder),
        hidden: false,
        on_path: name == "data",
    };
    let mut m = base();
    let mut p = pane_vm(0, true, PaneMode::Miller);
    p.miller = Some(MillerVm {
        parent: MillerCol { rows: vec![aux(0, "backup"), aux(1, "data"), aux(2, "music")], message: None },
        child: MillerCol { rows: Vec::new(), message: Some("Text · 1.0 KB".into()) },
    });
    m.panes = vec![p];
    snap_sr("browse_miller", &m, &sr_facts(TuiFocus::List), "notes.txt");
    let (buf, ..) = render_with_cursor(&m, &sr_facts(TuiFocus::List), W, H, Look::ScreenReader);
    let text = crate::snapshot::lines(&buf).concat();
    assert!(!text.contains("backup") && !text.contains("music"), "no side column shares a row: {text}");
}

#[test]
fn screen_reader_dialogs() {
    let mut m = base();
    m.dialog = Some(delete_prompt());
    snap_sr("dialog_delete", &m, &sr_facts(TuiFocus::Dialog), "[ ");
    // Fields read "Label: value".
    m.dialog = Some(form_dialog());
    snap_sr("dialog_form", &m, &sr_facts(TuiFocus::Dialog), "Name: New folder");
}

#[test]
fn screen_reader_palette() {
    let mut m = base();
    m.palette = palette();
    snap_sr("palette", &m, &sr_facts(TuiFocus::Palette), "New folder");
}

#[test]
fn screen_reader_menu() {
    let mut m = base();
    m.menu = menu();
    snap_sr("menu", &m, &sr_facts(TuiFocus::Menu), "Open");
}

#[test]
fn screen_reader_jobs_with_progress() {
    let mut m = base();
    m.jobs = jobs_panel();
    snap_sr("jobs", &m, &Facts { bottom_rows: 9, ..sr_facts(TuiFocus::Jobs) }, "Copying 12 files to backup");
}

#[test]
fn screen_reader_settings() {
    let mut m = base();
    m.settings = settings::tests::vm();
    snap_sr("settings", &m, &sr_facts(TuiFocus::Settings), "borders");
    // The section list has the keyboard: it alone, at full width.
    m.settings.nav_focused = true;
    m.settings.rows_focused = false;
    snap_sr("settings_nav", &m, &sr_facts(TuiFocus::Settings), "Terminal UI");
    let (buf, ..) = render_with_cursor(&m, &sr_facts(TuiFocus::Settings), W, H, Look::ScreenReader);
    assert!(!crate::snapshot::lines(&buf).concat().contains("sidebar_width"), "no key shares a row with the list");
}

#[test]
fn screen_reader_toast() {
    let mut m = base();
    m.toasts = toasts();
    snap_sr("toasts", &m, &Facts { toasts: 3, ..sr_facts(TuiFocus::List) }, "notes.txt");
    let (buf, regions, _) = render_with_cursor(&m, &Facts { toasts: 3, ..Facts::default() }, W, H, Look::ScreenReader);
    let status = &crate::snapshot::lines(&buf)[usize::from(regions.status.y)];
    assert_eq!(status.trim_end(), "Error: Cannot delete old-logs. Access is denied");
}

#[test]
fn screen_reader_sidebar() {
    let mut m = base();
    m.sidebar = sidebar();
    snap_sr("sidebar", &m, &Facts { sidebar: true, ..sr_facts(TuiFocus::Sidebar) }, "data");
}

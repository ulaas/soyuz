//! Portal: a sidebar section of folders pinned by dropping them on it.
//!
//! * Dropping folders on the section (header or rows) pins them: internal drags (file
//!   list, tree, shelf, tool windows) carry a [`DragPayload`], OS drops from Explorer
//!   arrive as `raw.dropped_files` over the section's last rect ([`begin_frame`], before
//!   the panes would copy them). Nothing is copied or moved: a drop only pins.
//! * Only existing local folders are pinned (files, duplicates and missing paths are
//!   skipped); the check runs on a worker thread, never on the UI thread.
//! * Pins live in `[[sidebar.portal]]` and are written through
//!   [`update_config`](crate::ui::settings::update_config), like a Settings edit.
//! * Rows navigate like favorites (click, middle-click = new tab), drag to reorder, and
//!   have a context menu (open, new tab, other pane, reveal, copy path, rename label,
//!   unpin). Pins whose folder is gone are dimmed and marked "missing" (checked every
//!   few seconds on a worker while the window is focused).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use egui::{Align2, CursorIcon, DragAndDrop, FontId, Id, Key, Rect, Sense, Stroke, TextEdit, Ui, pos2, vec2};
use soyuz_core::Location;
use soyuz_core::config::PortalPin;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::state::pane::PaneId;
use crate::theme::tokens::radius;
use crate::theme::{icons::ph, palette};
use crate::ui::dnd::{self, DragPayload};
use crate::ui::toasts::Toast;
use crate::ui::widgets::{NavRow, menu_item, menu_separator};

/// How often pinned folders are checked for existence (while the window is focused).
const CHECK_EVERY: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq)]
pub enum PortalCmd {
    /// Pin dropped paths (folders only; checked on a worker first).
    Pin(Vec<PathBuf>),
    Unpin(PathBuf),
    /// Move the pin at `from` before the pin at `to` (`to == len` = to the end).
    Move {
        from: usize,
        to: usize,
    },
    /// Set a pin's label (empty = the folder's name).
    Rename {
        path: PathBuf,
        name: String,
    },
}

/// Result of a pin check: the folders that may be pinned and how many paths were dropped.
struct Checked {
    dirs: Vec<PathBuf>,
    dropped: usize,
}

/// Per-window Portal UI state (lives in the sidebar state).
#[derive(Default)]
pub struct PortalState {
    /// Pin being dragged to a new position (index).
    drag: Option<usize>,
    /// The section's rect last frame: OS drops over it are pinned.
    pub rect: Option<Rect>,
    /// Inline label editor: the pin's path, the text, frames left to take the focus.
    renaming: Option<(PathBuf, String, u8)>,
    /// Pinned folders that were not found at the last check.
    missing: HashSet<PathBuf>,
    last_check: Option<Instant>,
    checking: Option<Receiver<HashSet<PathBuf>>>,
    pinning: Vec<Receiver<Checked>>,
}

/// Paths are compared case-insensitively on Windows (NTFS names are).
pub fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// The label shown for a pin.
pub fn label(pin: &PortalPin) -> String {
    if pin.name.trim().is_empty() { soyuz_core::entry::display_name(&pin.path) } else { pin.name.clone() }
}

/// New pins for `dirs`: duplicates (of `existing` or each other) are skipped.
pub fn new_pins(existing: &[PortalPin], dirs: &[PathBuf]) -> Vec<PortalPin> {
    let mut out: Vec<PortalPin> = Vec::new();
    for d in dirs {
        let dup = existing.iter().chain(out.iter()).any(|p| same_path(&p.path, d));
        if !dup {
            out.push(PortalPin { path: d.clone(), name: String::new() });
        }
    }
    out
}

/// Pins after moving `from` before `to` (a no-op move returns `None`).
pub fn moved(pins: &[PortalPin], from: usize, to: usize) -> Option<Vec<PortalPin>> {
    if from >= pins.len() || to == from || to == from + 1 {
        return None;
    }
    let mut v = pins.to_vec();
    let p = v.remove(from);
    let to = if to > from { to - 1 } else { to };
    v.insert(to.min(v.len()), p);
    Some(v)
}

/// Worker: keep the dropped paths that are existing local folders.
fn check_dirs(paths: Vec<PathBuf>) -> Checked {
    let dropped = paths.len();
    let dirs =
        paths.into_iter().map(|p| soyuz_fs::strip_verbatim(&p)).filter(|p| p.is_absolute() && p.is_dir()).collect();
    Checked { dirs, dropped }
}

fn spawn_check(app: &mut SoyuzApp, ctx: &egui::Context, paths: Vec<PathBuf>) {
    let (tx, rx) = channel();
    let wake = ctx.clone();
    let spawned = std::thread::Builder::new().name("soyuz-portal-pin".into()).spawn(move || {
        let _ = tx.send(check_dirs(paths));
        wake.request_repaint();
    });
    if spawned.is_ok() {
        app.ui.sidebar.portal.pinning.push(rx);
    }
}

pub fn apply(app: &mut SoyuzApp, ctx: &egui::Context, cmd: PortalCmd) {
    let pins = &app.config.sidebar.portal;
    match cmd {
        PortalCmd::Pin(paths) => spawn_check(app, ctx, paths),
        PortalCmd::Unpin(path) => {
            let keep: Vec<PortalPin> = pins.iter().filter(|p| !same_path(&p.path, &path)).cloned().collect();
            crate::ui::settings::update_config(app, ctx, |c| c.sidebar.portal = keep);
        }
        PortalCmd::Move { from, to } => {
            if let Some(v) = moved(pins, from, to) {
                crate::ui::settings::update_config(app, ctx, |c| c.sidebar.portal = v);
            }
        }
        PortalCmd::Rename { path, name } => {
            let mut v = pins.clone();
            if let Some(p) = v.iter_mut().find(|p| same_path(&p.path, &path)) {
                let name = name.trim().to_string();
                p.name = if name == soyuz_core::entry::display_name(&p.path) { String::new() } else { name };
            }
            crate::ui::settings::update_config(app, ctx, |c| c.sidebar.portal = v);
        }
    }
}

/// Before the panes see them: OS drops over the Portal section, finished pin checks and
/// the periodic existence check.
pub fn begin_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    // OS drops (Explorer) over the section pin instead of copying into a pane.
    if app.ui.sidebar_visible
        && let Some(rect) = app.ui.sidebar.portal.rect
    {
        let over = ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| rect.contains(p));
        let dropped: Vec<PathBuf> = if over {
            ctx.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            })
        } else {
            Vec::new()
        };
        if !dropped.is_empty() {
            ctx.input_mut(|i| i.raw.dropped_files.clear());
            spawn_check(app, ctx, dropped);
        }
    }
    let st = &mut app.ui.sidebar.portal;
    let mut done: Vec<Checked> = Vec::new();
    st.pinning.retain(|rx| match rx.try_recv() {
        Ok(c) => {
            done.push(c);
            false
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => true,
        Err(_) => false,
    });
    if let Some(rx) = &st.checking {
        match rx.try_recv() {
            Ok(m) => {
                st.missing = m;
                st.checking = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => st.checking = None,
            Err(_) => {}
        }
    }
    let pins: Vec<PathBuf> = app.config.sidebar.portal.iter().map(|p| p.path.clone()).collect();
    let due = st.last_check.is_none_or(|t| t.elapsed() >= CHECK_EVERY);
    // The first check runs right away; later ones only while the window is focused.
    let focused = st.last_check.is_none() || ctx.input(|i| i.focused);
    if !pins.is_empty() && st.checking.is_none() && due && focused {
        st.last_check = Some(Instant::now());
        let (tx, rx) = channel();
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name("soyuz-portal-check".into()).spawn(move || {
            let missing: HashSet<PathBuf> = pins.into_iter().filter(|p| !p.is_dir()).collect();
            let _ = tx.send(missing);
            wake.request_repaint();
        });
        if spawned.is_ok() {
            st.checking = Some(rx);
        }
    }
    if !app.config.sidebar.portal.is_empty() && ctx.input(|i| i.focused) {
        ctx.request_repaint_after(CHECK_EVERY);
    }
    for c in done {
        finish_pin(app, ctx, c);
    }
}

fn finish_pin(app: &mut SoyuzApp, ctx: &egui::Context, c: Checked) {
    if c.dirs.is_empty() {
        if c.dropped > 0 {
            app.ui.toasts.push(Toast::info(tr!("app-portal-folders-only")));
        }
        return;
    }
    let add = new_pins(&app.config.sidebar.portal, &c.dirs);
    if add.is_empty() {
        app.ui.toasts.push(Toast::info(tr!("app-portal-already")));
        return;
    }
    for p in &add {
        app.ui.sidebar.portal.missing.remove(&p.path);
    }
    let msg = match add.as_slice() {
        [one] => tr!("app-portal-pinned", name = label(one)),
        many => tr!("app-portal-pinned-many", count = many.len()),
    };
    crate::ui::settings::update_config(app, ctx, |cfg| cfg.sidebar.portal.extend(add));
    app.ui.toasts.push(Toast::success(msg));
}

/// Inputs for drawing the section.
pub struct PortalCtx<'a> {
    pub pins: &'a [PortalPin],
    pub current: &'a Location,
    pub pane: PaneId,
    pub cmds: &'a mut Vec<Cmd>,
}

/// The section body below its header (`header` = the header's rect; `open` = expanded).
/// The whole section is a drop target that pins folders.
pub fn section(ui: &mut Ui, st: &mut PortalState, header: Rect, open: bool, c: &mut PortalCtx<'_>) {
    let mut area = header;
    if open {
        let rows = rows(ui, st, c);
        area = area.union(rows);
    } else {
        st.drag = None;
    }
    st.rect = Some(area.intersect(ui.clip_rect()));
    drop_zone(ui, st, area, c.cmds);
}

/// Pin rows (or the empty hint); returns their rect.
fn rows(ui: &mut Ui, st: &mut PortalState, c: &mut PortalCtx<'_>) -> Rect {
    let p = palette(ui.ctx());
    let top = ui.cursor().top();
    if c.pins.is_empty() {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
        crate::ui::a11y::name(&resp, tr!("app-portal-empty"));
        if ui.is_rect_visible(rect) {
            ui.painter().text(
                pos2(rect.left() + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                ph::PUSH_PIN,
                crate::theme::icons::font(14.0),
                p.text_tertiary,
            );
            let g = crate::ui::widgets::trunc_galley(
                ui.painter(),
                tr!("app-portal-empty"),
                FontId::proportional(12.0),
                p.text_tertiary,
                rect.width() - 42.0,
            );
            ui.painter().galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, p.text_tertiary);
        }
        return Rect::from_min_max(pos2(rect.left(), top), rect.max);
    }
    let mut rects: Vec<Rect> = Vec::with_capacity(c.pins.len());
    for (i, pin) in c.pins.iter().enumerate() {
        if let Some((path, text, focus)) = st.renaming.as_mut()
            && same_path(path, &pin.path)
        {
            let resp = ui.add(
                TextEdit::singleline(text)
                    .id(Id::new(("soyuz.portal.rename", i)))
                    .hint_text(soyuz_core::entry::display_name(&pin.path))
                    .desired_width(f32::INFINITY),
            );
            if *focus > 0 {
                // Select the whole label until the editor owns the focus (gaining focus
                // resets the cursor); give up after a few frames.
                let n = text.chars().count();
                let mut s = egui::text_edit::TextEditState::load(ui.ctx(), resp.id).unwrap_or_default();
                s.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(n),
                )));
                s.store(ui.ctx(), resp.id);
                if resp.has_focus() {
                    *focus = 0;
                } else {
                    *focus -= 1;
                    resp.request_focus();
                    ui.ctx().request_repaint();
                }
            }
            rects.push(resp.rect);
            let (esc, enter) = ui.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
            if esc {
                st.renaming = None;
            } else if resp.lost_focus() || (enter && resp.has_focus()) {
                c.cmds.push(Cmd::Portal(PortalCmd::Rename { path: pin.path.clone(), name: text.clone() }));
                st.renaming = None;
            }
            continue;
        }
        let missing = st.missing.contains(&pin.path);
        let name = label(pin);
        let shown_label = if missing { format!("{name}  \u{b7}  {}", tr!("app-portal-missing")) } else { name.clone() };
        let r = ui
            .scope(|ui| {
                if missing {
                    ui.multiply_opacity(0.5);
                }
                NavRow {
                    icon: if missing {
                        ph::FOLDER_SIMPLE_DASHED
                    } else {
                        crate::ui::sidebar::favorite_glyph(&pin.path)
                    },
                    label: &shown_label,
                    active: matches!(c.current, Location::Dir(d) if d == &pin.path),
                    icon_color: crate::services::folder_colors::color_of(&pin.path, &p),
                    usage: None,
                }
                .show(ui)
            })
            .inner;
        rects.push(r.rect);
        let shown = soyuz_fs::strip_verbatim(&pin.path).display().to_string();
        let tip = if missing { tr!("app-portal-missing-tip", path = shown) } else { shown };
        let r = r.on_hover_text(tip);
        if r.clicked() {
            c.cmds.push(Cmd::FocusPane(c.pane));
            c.cmds.push(Cmd::Navigate { pane: c.pane, location: Location::Dir(pin.path.clone()), select: None });
        }
        if r.middle_clicked() {
            c.cmds.push(Cmd::OpenInNewTab { location: Location::Dir(pin.path.clone()), activate: false });
        }
        if r.drag_started() {
            st.drag = Some(i);
        }
        let cmds = &mut *c.cmds;
        let mut rename = false;
        crate::ui::native::popup::context_menu(&r, |ui| {
            rename = menu(ui, cmds, pin, c.pane);
        });
        if rename {
            st.renaming = Some((pin.path.clone(), name.clone(), 8));
        }
    }
    reorder(ui, st, &rects, c.cmds);
    let last = rects.last().copied().unwrap_or(Rect::NOTHING);
    Rect::from_min_max(pos2(last.left(), top), last.max)
}

/// Drag a pin to a new position (an accent line marks the gap).
fn reorder(ui: &Ui, st: &mut PortalState, rects: &[Rect], cmds: &mut Vec<Cmd>) {
    let Some(from) = st.drag else { return };
    let p = palette(ui.ctx());
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let down = ui.input(|i| i.pointer.primary_down());
    let to = pointer.map(|pos| rects.iter().position(|r| pos.y < r.center().y).unwrap_or(rects.len()));
    if let (Some(to), Some(first), Some(last)) = (to, rects.first(), rects.last()) {
        let y = if to < rects.len() { rects[to].top() - 0.5 } else { last.bottom() + 0.5 };
        let y = y.clamp(first.top() - 1.0, last.bottom() + 1.0);
        ui.painter()
            .line_segment([pos2(first.left() + 6.0, y), pos2(first.right() - 6.0, y)], Stroke::new(2.0, p.accent));
        ui.painter().circle_filled(pos2(first.left() + 6.0, y), 3.0, p.accent);
    }
    if !down {
        if let Some(to) = to {
            cmds.push(Cmd::Portal(PortalCmd::Move { from, to }));
        }
        st.drag = None;
    }
}

/// Context menu of a pin; returns `true` when "Rename label" was chosen.
fn menu(ui: &mut Ui, cmds: &mut Vec<Cmd>, pin: &PortalPin, pane: PaneId) -> bool {
    ui.set_width(230.0);
    let loc = || Location::Dir(pin.path.clone());
    let mut rename = false;
    if menu_item(ui, Some(ph::FOLDER_OPEN), tr!("app-portal-open"), None, true).clicked() {
        cmds.push(Cmd::FocusPane(pane));
        cmds.push(Cmd::Navigate { pane, location: loc(), select: None });
        ui.close();
    }
    if menu_item(ui, Some(ph::BROWSERS), tr!("app-ctx-open-in-new-tab"), None, true).clicked() {
        cmds.push(Cmd::OpenInNewTab { location: loc(), activate: true });
        ui.close();
    }
    if menu_item(ui, Some(ph::SQUARE_SPLIT_HORIZONTAL), tr!("app-ctx-open-in-other-pane"), None, true).clicked() {
        cmds.push(Cmd::OpenInOtherPane(loc()));
        ui.close();
    }
    menu_separator(ui);
    if menu_item(ui, Some(ph::ARROW_SQUARE_UP_RIGHT), tr!("app-ctx-reveal-in-system"), None, true).clicked() {
        cmds.push(Cmd::RevealInSystem(pin.path.clone()));
        ui.close();
    }
    if menu_item(ui, Some(ph::CLIPBOARD_TEXT), tr!("app-ctx-copy-path"), None, true).clicked() {
        cmds.push(Cmd::CopyText(soyuz_fs::strip_verbatim(&pin.path).to_string_lossy().into_owned()));
        ui.close();
    }
    menu_separator(ui);
    if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("app-portal-rename"), None, true).clicked() {
        rename = true;
        ui.close();
    }
    if !pin.name.is_empty()
        && menu_item(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("app-portal-reset-name"), None, true).clicked()
    {
        cmds.push(Cmd::Portal(PortalCmd::Rename { path: pin.path.clone(), name: String::new() }));
        ui.close();
    }
    if menu_item(ui, Some(ph::PUSH_PIN_SLASH), tr!("app-portal-unpin"), None, true).clicked() {
        cmds.push(Cmd::Portal(PortalCmd::Unpin(pin.path.clone())));
        ui.close();
    }
    rename
}

/// The section as a drop target: internal drags pin on release, OS drags are
/// highlighted (their drop is taken in [`begin_frame`]).
fn drop_zone(ui: &Ui, st: &PortalState, area: Rect, cmds: &mut Vec<Cmd>) {
    if st.drag.is_some() {
        return;
    }
    let ctx = ui.ctx();
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return };
    if !area.contains(pos) || !ui.clip_rect().contains(pos) {
        return;
    }
    let external = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if external {
        dnd::highlight(ui, area, radius::ROW);
        return;
    }
    if dnd::payload(ctx).is_none() {
        return;
    }
    dnd::highlight(ui, area, radius::ROW);
    dnd::set_pin_hover(ctx, tr!("app-portal-drop-badge").to_string());
    if ctx.input(|i| i.pointer.any_released())
        && let Some(p) = DragAndDrop::take_payload::<DragPayload>(ctx)
    {
        cmds.push(Cmd::Portal(PortalCmd::Pin(p.paths.clone())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(p: &str) -> PortalPin {
        PortalPin { path: PathBuf::from(p), name: String::new() }
    }

    #[test]
    fn duplicates_are_skipped() {
        let existing = vec![pin("/a"), pin("/b")];
        let add = new_pins(&existing, &[PathBuf::from("/b"), PathBuf::from("/c"), PathBuf::from("/c")]);
        assert_eq!(add, vec![pin("/c")]);
        if cfg!(windows) {
            assert!(new_pins(&[pin("C:\\Work")], &[PathBuf::from("c:\\work")]).is_empty());
        }
    }

    #[test]
    fn move_semantics_match_favorites() {
        let v = vec![pin("/a"), pin("/b"), pin("/c")];
        let names = |v: Vec<PortalPin>| v.into_iter().map(|p| p.path.display().to_string()).collect::<Vec<_>>();
        assert_eq!(names(moved(&v, 0, 3).unwrap()), ["/b", "/c", "/a"]);
        assert_eq!(names(moved(&v, 2, 0).unwrap()), ["/c", "/a", "/b"]);
        assert!(moved(&v, 1, 1).is_none() && moved(&v, 1, 2).is_none() && moved(&v, 5, 0).is_none());
    }

    #[test]
    fn only_existing_folders_pass_the_check() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("folder");
        std::fs::create_dir(&dir).unwrap();
        let file = d.path().join("file.txt");
        std::fs::write(&file, b"x").unwrap();
        let c = check_dirs(vec![dir.clone(), file, d.path().join("gone"), PathBuf::from("relative")]);
        assert_eq!(c.dropped, 4);
        assert_eq!(c.dirs, vec![soyuz_fs::strip_verbatim(&dir)]);
    }

    #[test]
    fn labels_fall_back_to_the_folder_name() {
        assert_eq!(label(&pin("/x/Projects")), "Projects");
        assert_eq!(label(&PortalPin { path: "/x/p".into(), name: "Work".into() }), "Work");
        assert_eq!(label(&PortalPin { path: "/x/p".into(), name: "  ".into() }), "p");
    }
}

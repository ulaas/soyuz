//! Miller's side columns: the parent folder (the current folder highlighted) on the
//! left, the folder under the cursor on the right. Both come from the core's one-shot
//! aux listings (`services.aux_entries`, requested in [`prepare`], refreshed by the core
//! after a few seconds).
//!
//! The hidden-filtered order of a listing is cached per pane and rebuilt only when the
//! listing, the folder or `show_hidden` changes, so a frame never walks a large folder:
//! [`vm`] builds only the rows in view.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::file_class::{FileClass, classify};
use soyuz_appcore::icon::IconId;
use soyuz_appcore::services::{AuxState, Services};
use soyuz_appcore::state::pane::{Pane, PaneId};
use soyuz_appcore::tr;
use soyuz_core::{EntryInfo, Location};

use super::{PaneGeom, PaneMode, PaneVm, RowKind, fitted, geometry, put};
use crate::dialog::safe;
use crate::icons::{self, IconSet};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// One row of a side column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuxRow {
    /// Position in the column's (hidden-filtered) order.
    pub index: usize,
    pub name: String,
    pub kind: RowKind,
    pub class: FileClass,
    pub icon: IconId,
    pub hidden: bool,
    /// The parent column's entry for the current folder.
    pub on_path: bool,
}

/// A side column in view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MillerCol {
    /// The rows in view.
    pub rows: Vec<AuxRow>,
    /// Shown instead of rows: loading, unreadable, empty.
    pub message: Option<String>,
}

/// Miller's side columns of a pane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MillerVm {
    pub parent: MillerCol,
    pub child: MillerCol,
}

/// A cached side column.
#[derive(Default)]
enum Slot {
    /// No such column (root folder, cursor not on a folder).
    #[default]
    None,
    Loading,
    Failed,
    Ready(Col),
}

struct Col {
    dir: PathBuf,
    entries: Arc<Vec<EntryInfo>>,
    show_hidden: bool,
    /// Indices into `entries` of the entries shown.
    shown: Vec<usize>,
    /// The name to highlight, and its position in `shown`.
    highlight_name: Option<String>,
    highlight: Option<usize>,
}

#[derive(Default)]
struct Cols {
    parent: Slot,
    child: Slot,
}

thread_local! {
    /// Per drawn Miller pane (the frame loop runs on one thread).
    static CACHE: RefCell<HashMap<PaneId, Cols>> = RefCell::new(HashMap::new());
}

/// The folders a Miller pane shows beside its own: `(parent, current folder's name)` and
/// the folder under the cursor.
fn wanted(pane: &Pane) -> (Option<(PathBuf, Option<String>)>, Option<PathBuf>) {
    let Location::Dir(cur) = &pane.location else { return (None, None) };
    let parent = cur.parent().map(|p| (p.to_path_buf(), cur.file_name().map(|n| n.to_string_lossy().into_owned())));
    let child = pane.cursor.as_deref().and_then(|c| pane.entry(c)).filter(|e| e.is_dir_like()).map(|e| e.path.clone());
    (parent, child)
}

/// Request the side columns of every drawn Miller pane and refresh the cache.
pub fn prepare(core: &mut AppCore, regions: &Regions) {
    let show_hidden = core.ui.show_hidden;
    let tab = core.ws.tab();
    let mut want = Vec::new();
    for p in &regions.panes {
        let Some(pane) = tab.panes.get(p.index) else { continue };
        let mode = PaneMode::of(pane);
        if mode == PaneMode::Miller && !geometry(mode, p.rect).parent.is_empty() {
            want.push((pane.id, wanted(pane)));
        }
    }
    let services = &mut core.services;
    CACHE.with_borrow_mut(|cache| {
        cache.retain(|id, _| want.iter().any(|(w, _)| w == id));
        for (id, (parent, child)) in want {
            let cols = cache.entry(id).or_default();
            update(&mut cols.parent, services, parent.as_ref().map(|(d, h)| (d.as_path(), h.as_deref())), show_hidden);
            update(&mut cols.child, services, child.as_deref().map(|d| (d, None)), show_hidden);
        }
    });
}

fn update(slot: &mut Slot, services: &mut Services, want: Option<(&Path, Option<&str>)>, show_hidden: bool) {
    let Some((dir, highlight)) = want else {
        *slot = Slot::None;
        return;
    };
    let Some(entries) = services.aux_entries(dir) else {
        *slot = if matches!(services.aux_state(dir), Some(AuxState::Failed(_))) { Slot::Failed } else { Slot::Loading };
        return;
    };
    let same = matches!(slot, Slot::Ready(c) if c.dir == dir && Arc::ptr_eq(&c.entries, &entries) && c.show_hidden == show_hidden);
    if !same {
        let shown = entries.iter().enumerate().filter(|(_, e)| show_hidden || !e.is_hidden()).map(|(i, _)| i).collect();
        *slot = Slot::Ready(Col {
            dir: dir.to_path_buf(),
            entries,
            show_hidden,
            shown,
            highlight_name: None,
            highlight: None,
        });
    }
    if let Slot::Ready(c) = slot
        && (!same || c.highlight_name.as_deref() != highlight)
    {
        c.highlight_name = highlight.map(str::to_string);
        c.highlight = highlight.and_then(|h| c.shown.iter().position(|&i| c.entries[i].name == h));
    }
}

/// The side columns of `pane`, `rows` rows each.
pub fn vm(pane: &Pane, rows: usize) -> MillerVm {
    CACHE.with_borrow(|cache| {
        let Some(cols) = cache.get(&pane.id) else { return MillerVm::default() };
        MillerVm { parent: column(&cols.parent, rows, true), child: column(&cols.child, rows, false) }
    })
}

fn column(slot: &Slot, rows: usize, center: bool) -> MillerCol {
    let c = match slot {
        Slot::None => return MillerCol::default(),
        Slot::Loading => return MillerCol { rows: Vec::new(), message: Some(tr!("tui-loading").to_string()) },
        Slot::Failed => {
            return MillerCol { rows: Vec::new(), message: Some(tr!("views-miller-unreadable").to_string()) };
        }
        Slot::Ready(c) => c,
    };
    if c.shown.is_empty() {
        return MillerCol { rows: Vec::new(), message: Some(tr!("views-miller-empty").to_string()) };
    }
    // The parent column keeps the current folder in the middle.
    let top = match (center, c.highlight) {
        (true, Some(h)) => h.saturating_sub(rows / 2).min(c.shown.len().saturating_sub(rows)),
        _ => 0,
    };
    let end = (top + rows).min(c.shown.len());
    let rows = (top..end)
        .map(|pos| {
            let e = &c.entries[c.shown[pos]];
            let class = classify(e);
            AuxRow {
                index: pos,
                name: safe(&e.name),
                kind: RowKind::of(e),
                class,
                icon: icons::class_icon(class),
                hidden: e.is_hidden(),
                on_path: c.highlight == Some(pos),
            }
        })
        .collect();
    MillerCol { rows, message: None }
}

/// The entry of a side column row (for clicks): its path and whether it is a folder.
/// `parent`: the parent column, else the child column; `index` as in [`AuxRow::index`].
pub fn entry(pane: PaneId, parent: bool, index: usize) -> Option<(PathBuf, bool)> {
    CACHE.with_borrow(|cache| {
        let cols = cache.get(&pane)?;
        let Slot::Ready(c) = (if parent { &cols.parent } else { &cols.child }) else { return None };
        let e = &c.entries[*c.shown.get(index)?];
        Some((e.path.clone(), e.is_dir_like()))
    })
}

/// Draw the side columns and their separators.
pub fn draw(buf: &mut Buffer, g: &PaneGeom, m: &PaneVm, mv: &MillerVm, theme: &TuiTheme, hits: &mut Hits) {
    if g.parent.is_empty() {
        return;
    }
    let border = theme.style(Role::Border);
    for x in [g.parent.right(), g.list.right()] {
        for y in g.list.top()..g.list.bottom() {
            put(buf, x, y, "│", 1, border);
        }
    }
    draw_col(buf, g.parent, m, &mv.parent, true, theme, hits);
    draw_col(buf, g.child, m, &mv.child, false, theme, hits);
}

fn draw_col(buf: &mut Buffer, r: Rect, m: &PaneVm, c: &MillerCol, parent: bool, theme: &TuiTheme, hits: &mut Hits) {
    if let Some(msg) = &c.message {
        let t = text::truncate_end(msg, usize::from(r.width));
        let x = r.x + (r.width - text::width(&t) as u16) / 2;
        put(buf, x, r.y, &t, r.width, theme.style(Role::Muted));
    }
    for (i, row) in c.rows.iter().enumerate().take(usize::from(r.height)) {
        let line = Rect { y: r.y + i as u16, height: 1, ..r };
        let role = if row.kind == RowKind::Dir { Role::Folder } else { Role::of_class(row.class) };
        let mut style = theme.style(Role::Text).patch(theme.style(role));
        if row.hidden {
            style = style.patch(theme.style(Role::Muted));
        }
        let mut x = line.x + 1;
        let end = line.right();
        if m.icons != IconSet::Ascii && x < end {
            put(buf, x, line.y, icons::glyph(row.icon, m.icons), end - x, style);
            x = (x + 3).min(end);
        }
        let label = fitted(&row.name, row.kind, usize::from(end.saturating_sub(x + 1)));
        put(buf, x, line.y, &label, end.saturating_sub(x), style);
        if row.on_path {
            buf.set_style(line, theme.style(Role::Selected).patch(theme.style(Role::CursorInactive)));
        }
        hits.push(line, HitTarget::MillerRow { pane: m.id, parent, index: row.index });
    }
}

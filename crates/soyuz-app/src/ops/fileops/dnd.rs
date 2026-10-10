//! Drag and drop. Internal drags carry a [`DragPayload`] through egui's
//! `DragAndDrop` (panes, tabs, sidebar favorites/devices/trash and breadcrumb segments
//! are drop targets); external drops come from `raw.dropped_files` and always copy.
//!
//! Modifier semantics (Explorer-like): Ctrl = copy, Shift = move, Alt (or Ctrl+Shift) =
//! link; otherwise `[behaviour] drag_default` (auto = move on the same volume, copy
//! across volumes; `ask` shows a small menu at the drop point).
//!
//! Spring-loaded folders: resting on a folder item, breadcrumb segment, tab or
//! sidebar row for `[behaviour] spring_load_delay_ms` opens it, so a drag can go deeper.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, DragAndDrop, FontId, Id, Order, Pos2, Rect, Response, Stroke,
    StrokeKind, Ui, pos2, vec2,
};
use soyuz_core::config::DragDefault;

use super::FileOpCmd;
use crate::cmd::Cmd;
use crate::state::pane::{Pane, PaneId};
use crate::theme::tokens::radius;
use crate::theme::{icons, icons::ph, palette};

/// What an internal drag carries.
#[derive(Clone, Debug)]
pub struct DragPayload {
    pub paths: Vec<PathBuf>,
    pub source_pane: Option<PaneId>,
    pub default: DragDefault,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropEffect {
    Copy,
    Move,
    Link,
}

impl DropEffect {
    fn glyph(self) -> &'static str {
        match self {
            DropEffect::Copy => ph::COPY,
            DropEffect::Move => ph::ARROW_BEND_DOWN_RIGHT,
            DropEffect::Link => ph::LINK,
        }
    }
}

/// Modifier state relevant to a drop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropMods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl DropMods {
    pub fn from_egui(m: egui::Modifiers) -> Self {
        DropMods { ctrl: m.command || m.ctrl, shift: m.shift, alt: m.alt }
    }
}

/// Effect of a drop; `None` = ask the user (`drag_default = "ask"` without modifiers).
pub fn resolve_effect(m: DropMods, same_volume: bool, default: DragDefault) -> Option<DropEffect> {
    if m.alt || (m.ctrl && m.shift) {
        return Some(DropEffect::Link);
    }
    if m.ctrl {
        return Some(DropEffect::Copy);
    }
    if m.shift {
        return Some(DropEffect::Move);
    }
    match default {
        DragDefault::Auto => Some(if same_volume { DropEffect::Move } else { DropEffect::Copy }),
        DragDefault::Copy => Some(DropEffect::Copy),
        DragDefault::Move => Some(DropEffect::Move),
        DragDefault::Ask => None,
    }
}

/// Can `paths` be dropped into `dest`? Not into one of the dragged items or below it.
pub fn valid_target(paths: &[PathBuf], dest: &Path) -> bool {
    !paths.is_empty() && !paths.iter().any(|p| dest == p.as_path() || dest.starts_with(p))
}

/// A move into the folder the items already live in does nothing.
pub fn is_noop(paths: &[PathBuf], dest: &Path, effect: Option<DropEffect>) -> bool {
    effect == Some(DropEffect::Move) && paths.iter().all(|p| p.parent() == Some(dest))
}

/// Volume identity derived from the path alone (drive letter / UNC share) on Windows.
pub fn volume_key(p: &Path) -> Option<String> {
    let stripped = soyuz_fs::strip_verbatim(p);
    match stripped.components().next()? {
        Component::Prefix(pre) => {
            use std::path::Prefix as P;
            Some(match pre.kind() {
                P::Disk(d) | P::VerbatimDisk(d) => (d as char).to_ascii_uppercase().to_string(),
                P::UNC(s, h) | P::VerbatimUNC(s, h) => {
                    format!("\\\\{}\\{}", s.to_string_lossy().to_lowercase(), h.to_string_lossy().to_lowercase())
                }
                _ => return None,
            })
        }
        _ => None,
    }
}

thread_local! {
    static DEVICES: RefCell<HashMap<PathBuf, Option<u64>>> = RefCell::new(HashMap::new());
}

/// Whether two paths live on the same volume (decides move vs copy for plain drags).
pub fn same_volume(a: &Path, b: &Path) -> bool {
    if let (Some(x), Some(y)) = (volume_key(a), volume_key(b)) {
        return x == y;
    }
    let dev = |p: &Path| {
        DEVICES.with(|c| {
            *c.borrow_mut().entry(p.to_path_buf()).or_insert_with(|| soyuz_fs::device_of(p).ok().map(|d| d.0))
        })
    };
    match (dev(a), dev(b)) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// Effect for the current modifiers, dropping `payload` into `dest`.
pub fn effect_for(ctx: &egui::Context, payload: &DragPayload, dest: &Path) -> Option<DropEffect> {
    let mods = DropMods::from_egui(ctx.input(|i| i.modifiers));
    let same = payload.paths.first().is_none_or(|p| same_volume(p, dest));
    resolve_effect(mods, same, payload.default)
}

/// Start an internal drag with the pane's selection.
pub fn begin(ctx: &egui::Context, pane: &Pane, default: DragDefault) {
    let paths: Vec<PathBuf> = pane.selected_entries().iter().map(|e| e.path.clone()).collect();
    let draggable = matches!(pane.location, soyuz_core::Location::Dir(_))
        || crate::ui::tools::tags_ui::is_virtual_listing(&pane.location);
    if paths.is_empty() || !draggable {
        return;
    }
    DEVICES.with(|c| c.borrow_mut().clear());
    DragAndDrop::set_payload(ctx, DragPayload { paths, source_pane: Some(pane.id), default });
}

/// The active internal drag, if any.
pub fn payload(ctx: &egui::Context) -> Option<std::sync::Arc<DragPayload>> {
    DragAndDrop::payload::<DragPayload>(ctx)
}

/// What the badge says about the current target.
#[derive(Clone, Debug, Default)]
struct HoverInfo {
    label: String,
    effect: Option<DropEffect>,
}

fn hover_id() -> Id {
    Id::new("soyuz.dnd.hover")
}

fn set_hover(ctx: &egui::Context, label: String, effect: Option<DropEffect>) {
    ctx.data_mut(|d| d.insert_temp(hover_id(), HoverInfo { label, effect }));
}

/// Paint the drop highlight (2 px accent inset stroke + faint fill).
pub fn highlight(ui: &Ui, rect: Rect, rounding: u8) {
    let rect = rect.intersect(ui.clip_rect());
    if !rect.is_positive() {
        return;
    }
    let p = palette(ui.ctx());
    let painter = ui.ctx().layer_painter(egui::LayerId::new(Order::Foreground, Id::new("soyuz.dnd.hl")));
    painter.rect_filled(rect, CornerRadius::same(rounding), p.accent.gamma_multiply(0.08));
    painter.rect_stroke(rect, CornerRadius::same(rounding), Stroke::new(2.0, p.accent), StrokeKind::Inside);
}

/// Handle hover + release of an internal drag over `rect` targeting `dest`. Returns
/// `true` while this target is hovered.
pub fn target_rect(ui: &Ui, rect: Rect, dest: &Path, rounding: u8, cmds: &mut Vec<Cmd>) -> bool {
    target_rect_spring(ui, rect, dest, rounding, cmds, None)
}

fn spring_id() -> Id {
    Id::new("soyuz.dnd.spring")
}

/// Spring-loading: run `open` once the pointer rested on `dest` long enough; paints a
/// filling hint meanwhile.
fn spring(ui: &Ui, rect: Rect, dest: &Path, rounding: u8, open: Cmd, cmds: &mut Vec<Cmd>) {
    let Some(delay) = crate::ui::looks::spring_delay() else { return };
    let ctx = ui.ctx();
    let now = ctx.input(|i| i.time);
    let mut timer: crate::ui::motion::HoverTimer = ctx.data(|d| d.get_temp(spring_id())).unwrap_or_default();
    let fire = timer.update(dest, now, delay);
    let progress = timer.progress(now, delay);
    let fired = timer.fired;
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        d.insert_temp(spring_id(), timer);
        d.insert_temp(spring_id().with("pass"), pass);
    });
    if fire {
        cmds.push(open);
    } else if !fired {
        ctx.request_repaint();
        // A fill that grows from the left edge while the timer runs.
        if progress > 0.15 && !crate::theme::reduced_motion(ctx) {
            let p = palette(ctx);
            let r = rect.intersect(ui.clip_rect());
            let w = r.width() * crate::ui::motion::ease_out((progress - 0.15) / 0.85);
            let fill = Rect::from_min_size(r.min, vec2(w, r.height()));
            let painter = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("soyuz.dnd.hl")));
            painter.rect_filled(fill, CornerRadius::same(rounding), p.accent.gamma_multiply(0.10));
        }
    }
}

/// [`target_rect`] that also spring-loads: `open` runs after the hover delay.
pub fn target_rect_spring(
    ui: &Ui,
    rect: Rect,
    dest: &Path,
    rounding: u8,
    cmds: &mut Vec<Cmd>,
    open: Option<Cmd>,
) -> bool {
    let ctx = ui.ctx();
    let Some(payload) = payload(ctx) else { return false };
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return false };
    if !rect.contains(pos) || !ui.clip_rect().contains(pos) || !valid_target(&payload.paths, dest) {
        return false;
    }
    let released = ctx.input(|i| i.pointer.any_released());
    // Modifiers are latched while hovering: a modifier key let go in the release frame
    // must not change the effect the badge promised.
    let key = Id::new(("soyuz.dnd.effect", dest));
    let effect = match ctx.data(|d| d.get_temp::<Option<DropEffect>>(key)) {
        Some(e) if released => e,
        _ => effect_for(ctx, &payload, dest),
    };
    if is_noop(&payload.paths, dest, effect) {
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(key, effect));
    highlight(ui, rect, rounding);
    if !released && let Some(open) = open {
        spring(ui, rect, dest, rounding, open, cmds);
    }
    set_hover(ctx, format!("\u{201c}{}\u{201d}", soyuz_core::entry::display_name(dest)), effect);
    if released && let Some(p) = DragAndDrop::take_payload::<DragPayload>(ctx) {
        ctx.data_mut(|d| d.remove::<Option<DropEffect>>(key));
        cmds.push(Cmd::FileOp(FileOpCmd::Drop { paths: p.paths.clone(), dest: dest.to_path_buf(), effect, pos }));
    }
    true
}

/// Drop target on any response (sidebar rows, breadcrumb segments, tabs).
pub fn target(ui: &Ui, resp: &Response, dest: &Path, cmds: &mut Vec<Cmd>) -> bool {
    target_rect(ui, resp.rect, dest, radius::ROW, cmds)
}

/// [`target`] that spring-loads: hovering opens `dest` in `pane`.
pub fn target_open(ui: &Ui, resp: &Response, dest: &Path, pane: PaneId, cmds: &mut Vec<Cmd>) -> bool {
    let open = Cmd::Navigate { pane, location: soyuz_core::Location::Dir(dest.to_path_buf()), select: None };
    target_rect_spring(ui, resp.rect, dest, radius::ROW, cmds, Some(open))
}

/// [`target`] that spring-loads with a custom command (tabs: select the tab).
pub fn target_with(ui: &Ui, resp: &Response, dest: &Path, open: Cmd, cmds: &mut Vec<Cmd>) -> bool {
    target_rect_spring(ui, resp.rect, dest, radius::ROW, cmds, Some(open))
}

/// The sidebar Trash row: dropping moves the items to the trash.
pub fn trash_target(ui: &Ui, resp: &Response, cmds: &mut Vec<Cmd>) -> bool {
    let ctx = ui.ctx();
    let Some(_) = payload(ctx) else { return false };
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return false };
    if !resp.rect.contains(pos) {
        return false;
    }
    highlight(ui, resp.rect, radius::ROW);
    set_hover(ctx, tr!("ops-dnd-trash").into(), None);
    ctx.data_mut(|d| d.insert_temp(Id::new("soyuz.dnd.trash"), true));
    if ctx.input(|i| i.pointer.any_released())
        && let Some(p) = DragAndDrop::take_payload::<DragPayload>(ctx)
    {
        cmds.push(Cmd::FileOp(FileOpCmd::TrashPaths(p.paths.clone())));
    }
    true
}

/// Where the pointer was over a pane last frame (external drops have no pointer
/// position on some platforms, so the last known pane wins, else the focused one).
#[derive(Clone, Debug, Default)]
pub struct DndFrame {
    pub panes: Vec<(PaneId, Rect, Option<PathBuf>)>,
    /// A folder item claimed the hover this frame (the pane background yields).
    pub item_claimed: bool,
    pub ask: Option<AskMenu>,
}

/// "Copy here / Move here / Link here" menu for `drag_default = "ask"`.
#[derive(Clone, Debug)]
pub struct AskMenu {
    pub paths: Vec<PathBuf>,
    pub dest: PathBuf,
    pub pos: Pos2,
}

/// Pane-level target: drop into the pane's folder (background, empty folder state),
/// plus the external-drop highlight. Called once per drawn pane.
pub fn pane_zone(ui: &Ui, rect: Rect, pane: &Pane, frame: &mut DndFrame, cmds: &mut Vec<Cmd>) {
    let dir = match &pane.location {
        soyuz_core::Location::Dir(d) => Some(d.clone()),
        _ => None,
    };
    frame.panes.push((pane.id, rect, dir.clone()));
    let Some(dir) = dir else { return };
    let ctx = ui.ctx();
    let external = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if external {
        let pos = ctx.input(|i| i.pointer.latest_pos());
        if pos.is_some_and(|p| rect.contains(p)) {
            highlight(ui, rect.shrink(2.0), radius::CARD);
        }
        return;
    }
    if frame.item_claimed {
        return;
    }
    target_rect(ui, rect.shrink(2.0), &dir, radius::CARD, cmds);
}

/// Folder items inside a view: `hit` is the item under the pointer and its rect.
pub fn item_target(ui: &Ui, pane: &Pane, hit: Option<(usize, Rect)>, frame: &mut DndFrame, cmds: &mut Vec<Cmd>) {
    let Some((i, rect)) = hit else { return };
    let Some(e) = pane.entry_at(i) else { return };
    if !e.is_dir_like() {
        return;
    }
    let path = e.path.clone();
    let open = Cmd::Navigate { pane: pane.id, location: soyuz_core::Location::Dir(path.clone()), select: None };
    if target_rect_spring(ui, rect, &path, radius::ROW, cmds, Some(open)) {
        frame.item_claimed = true;
    }
}

/// External drop target: the pane under the pointer, else the focused pane.
pub fn external_target(frame: &DndFrame, pointer: Option<Pos2>, focused: PaneId) -> Option<PathBuf> {
    if let Some(pos) = pointer
        && let Some((_, _, dir)) = frame.panes.iter().find(|(_, r, _)| r.contains(pos))
    {
        return dir.clone();
    }
    frame.panes.iter().find(|(id, _, _)| *id == focused).and_then(|(_, _, d)| d.clone())
}

/// Floating badge next to the pointer: item count, effect and target.
pub fn paint_badge(ctx: &egui::Context) {
    let hover: Option<HoverInfo> = ctx.data_mut(|d| d.remove_temp(hover_id()));
    let pass = ctx.cumulative_pass_nr();
    if ctx.data(|d| d.get_temp::<u64>(spring_id().with("pass"))) != Some(pass) {
        ctx.data_mut(|d| d.remove::<crate::ui::motion::HoverTimer>(spring_id()));
    }
    let to_trash: bool = ctx.data_mut(|d| d.remove_temp::<bool>(Id::new("soyuz.dnd.trash"))).unwrap_or(false);
    let Some(payload) = payload(ctx) else { return };
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return };
    if !ctx.input(|i| i.pointer.primary_down()) {
        return;
    }
    let p = palette(ctx);
    let n = payload.paths.len();
    let what = if n == 1 { soyuz_core::entry::display_name(&payload.paths[0]) } else { tr!("common-items", count = n) };
    let (glyph, line2, accent) = match &hover {
        Some(h) if to_trash => (ph::TRASH, tr!("ops-dnd-badge-trash", target = &h.label), p.danger),
        Some(h) => match h.effect {
            Some(e) => {
                let line = match e {
                    DropEffect::Copy => tr!("ops-dnd-badge-copy", target = &h.label),
                    DropEffect::Move => tr!("ops-dnd-badge-move", target = &h.label),
                    DropEffect::Link => tr!("ops-dnd-badge-link", target = &h.label),
                };
                (e.glyph(), line, p.accent)
            }
            None => (ph::QUESTION, tr!("ops-dnd-badge-drop", target = &h.label), p.accent),
        },
        None => (ph::FILES, String::new(), p.text_tertiary),
    };
    ctx.set_cursor_icon(match hover.as_ref().and_then(|h| h.effect) {
        _ if hover.is_none() => CursorIcon::NoDrop,
        Some(DropEffect::Copy) => CursorIcon::Copy,
        Some(DropEffect::Link) => CursorIcon::Alias,
        _ => CursorIcon::Grabbing,
    });
    let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("soyuz.dnd.badge")));
    let title = crate::ui::widgets::trunc_galley(
        &painter,
        &what,
        crate::ui::views::flow::semibold(12.5),
        p.text_primary,
        220.0,
    );
    let sub = (!line2.is_empty())
        .then(|| crate::ui::widgets::trunc_galley(&painter, &line2, FontId::proportional(11.5), accent, 220.0));
    let w = 46.0 + title.size().x.max(sub.as_ref().map_or(0.0, |g| g.size().x)) + 14.0;
    let h = if sub.is_some() { 44.0 } else { 32.0 };
    let r = Rect::from_min_size(pos + vec2(16.0, 14.0), vec2(w, h));
    let shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha((p.shadow_alpha * 255.0) as u8),
    };
    painter.add(shadow.as_shape(r, CornerRadius::same(radius::CARD)));
    painter.rect_filled(r, CornerRadius::same(radius::CARD), p.bg_surface3);
    painter.rect_stroke(r, CornerRadius::same(radius::CARD), Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    // Stacked-documents glyph with a count bubble.
    let gc = pos2(r.left() + 22.0, r.center().y);
    if n > 1 {
        // Two cards peeking out behind the glyph: "many items".
        for k in [2.0f32, 1.0] {
            let card = Rect::from_center_size(gc + vec2(k * 2.5, -k * 2.5), vec2(16.0, 19.0));
            painter.rect_filled(card, CornerRadius::same(3), p.bg_surface2);
            painter.rect_stroke(card, CornerRadius::same(3), Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        }
    }
    painter.text(gc, Align2::CENTER_CENTER, glyph, icons::font_fill(18.0), accent);
    // Effect chip (+ copy, ↗ link) at the glyph's bottom-right, like the OS cursors.
    let chip = match hover.as_ref().and_then(|h| h.effect) {
        Some(DropEffect::Copy) if !to_trash => Some((ph::PLUS, p.success)),
        Some(DropEffect::Link) if !to_trash => Some((ph::ARROW_UP_RIGHT, p.info)),
        _ => None,
    };
    if let Some((g, c)) = chip {
        let cc = gc + vec2(8.0, 8.0);
        painter.circle_filled(cc, 7.0, p.bg_surface3);
        painter.circle_filled(cc, 5.8, c);
        painter.text(cc, Align2::CENTER_CENTER, g, icons::font(9.0), Color32::WHITE);
    }
    if n > 1 {
        let b = pos2(r.left() + 32.0, r.top() + 9.0);
        let label = if n > 99 { "99+".to_string() } else { n.to_string() };
        let g = painter.layout_no_wrap(label, FontId::proportional(9.5), Color32::WHITE);
        let br = Rect::from_center_size(b, vec2((g.size().x + 8.0).max(15.0), 15.0));
        painter.rect_filled(br, CornerRadius::same(8), p.accent);
        painter.galley(br.center() - g.size() / 2.0, g, Color32::WHITE);
    }
    let tx = r.left() + 44.0;
    if let Some(sub) = sub {
        painter.galley(pos2(tx, r.top() + 7.0), title, p.text_primary);
        painter.galley(pos2(tx, r.top() + 24.0), sub, accent);
    } else {
        painter.galley(pos2(tx, r.center().y - title.size().y / 2.0), title, p.text_primary);
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: DropMods = DropMods { ctrl: false, shift: false, alt: false };

    #[test]
    fn modifier_semantics() {
        let ctrl = DropMods { ctrl: true, ..NONE };
        let shift = DropMods { shift: true, ..NONE };
        let alt = DropMods { alt: true, ..NONE };
        let ctrl_shift = DropMods { ctrl: true, shift: true, alt: false };
        for same in [true, false] {
            assert_eq!(resolve_effect(ctrl, same, DragDefault::Auto), Some(DropEffect::Copy));
            assert_eq!(resolve_effect(shift, same, DragDefault::Auto), Some(DropEffect::Move));
            assert_eq!(resolve_effect(alt, same, DragDefault::Auto), Some(DropEffect::Link));
            assert_eq!(resolve_effect(ctrl_shift, same, DragDefault::Copy), Some(DropEffect::Link));
        }
        assert_eq!(resolve_effect(NONE, true, DragDefault::Auto), Some(DropEffect::Move), "same device moves");
        assert_eq!(resolve_effect(NONE, false, DragDefault::Auto), Some(DropEffect::Copy), "cross device copies");
        assert_eq!(resolve_effect(NONE, true, DragDefault::Copy), Some(DropEffect::Copy));
        assert_eq!(resolve_effect(NONE, false, DragDefault::Move), Some(DropEffect::Move));
        assert_eq!(resolve_effect(NONE, true, DragDefault::Ask), None);
        assert_eq!(resolve_effect(ctrl, true, DragDefault::Ask), Some(DropEffect::Copy), "modifiers win over ask");
    }

    #[test]
    fn targets() {
        let paths = vec![PathBuf::from("/a/x"), PathBuf::from("/a/y")];
        assert!(valid_target(&paths, Path::new("/b")));
        assert!(!valid_target(&paths, Path::new("/a/x")), "not into itself");
        assert!(!valid_target(&paths, Path::new("/a/x/sub")), "not below itself");
        assert!(valid_target(&paths, Path::new("/a")));
        assert!(!valid_target(&[], Path::new("/a")));
        assert!(is_noop(&paths, Path::new("/a"), Some(DropEffect::Move)));
        assert!(!is_noop(&paths, Path::new("/a"), Some(DropEffect::Copy)), "copy into the same folder duplicates");
        assert!(!is_noop(&paths, Path::new("/b"), Some(DropEffect::Move)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_volume_keys() {
        assert_eq!(volume_key(Path::new(r"c:\Users\x")), Some("C".into()));
        assert_eq!(volume_key(Path::new(r"\\?\C:\Users\x")), Some("C".into()));
        assert_eq!(volume_key(Path::new(r"\\Server\Share\dir")), Some(r"\\server\share".into()));
        assert!(same_volume(Path::new(r"C:\a"), Path::new(r"c:\b\c")));
        assert!(!same_volume(Path::new(r"C:\a"), Path::new(r"D:\a")));
        assert!(!same_volume(Path::new(r"\\s\one\a"), Path::new(r"\\s\two\a")));
    }

    #[test]
    fn external_target_prefers_pane_under_pointer() {
        let a = PaneId(1);
        let b = PaneId(2);
        let f = DndFrame {
            panes: vec![
                (a, Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)), Some("/a".into())),
                (b, Rect::from_min_size(pos2(100.0, 0.0), vec2(100.0, 100.0)), Some("/b".into())),
            ],
            ..Default::default()
        };
        assert_eq!(external_target(&f, Some(pos2(150.0, 50.0)), a), Some("/b".into()));
        assert_eq!(external_target(&f, None, a), Some("/a".into()));
        assert_eq!(external_target(&f, Some(pos2(500.0, 50.0)), b), Some("/b".into()));
    }
}

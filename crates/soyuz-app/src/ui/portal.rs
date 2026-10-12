//! Portal section of the sidebar (drawing and egui input): folders pinned by dropping
//! them on it. The pins, their checks and `PortalCmd` live in [`crate::models::portal`].
//!
//! * Dropping folders on the section (header or rows) pins them: internal drags (file
//!   list, tree, shelf, tool windows) carry a [`DragPayload`], OS drops from Explorer
//!   arrive as `raw.dropped_files` over the section's last rect ([`begin_frame`], before
//!   the panes would copy them). Nothing is copied or moved: a drop only pins.
//! * Rows navigate like favorites (click, middle-click = new tab), drag to reorder, and
//!   have a context menu (open, new tab, other pane, reveal, copy path, rename label,
//!   unpin). Pins whose folder is gone are dimmed and marked "missing".

use std::path::PathBuf;

use egui::{Align2, CursorIcon, DragAndDrop, FontId, Id, Key, Rect, Sense, Stroke, TextEdit, Ui, pos2, vec2};
use soyuz_core::Location;
use soyuz_core::config::PortalPin;

pub use crate::models::portal::{PortalCmd, PortalState};

use crate::models::portal::{label, same_path};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::state::pane::PaneId;
use crate::theme::tokens::radius;
use crate::theme::{icons::ph, palette};
use crate::ui::dnd::{self, DragPayload};
use crate::ui::widgets::{NavRow, menu_item, menu_separator};

/// Before the panes see them: OS drops over the Portal section, then the model's finished
/// pin checks and the periodic existence check.
pub fn begin_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    // OS drops (Explorer) over the section pin instead of copying into a pane.
    if app.core.ui.sidebar_visible
        && let Some(rect) = app.core.ui.sidebar.portal.rect
    {
        let over = ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| rect.contains(crate::ui::geom::point(p)));
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
            crate::models::portal::pin_dropped(&mut app.core, ctx, dropped);
        }
    }
    let focused = ctx.input(|i| i.focused);
    crate::models::portal::pump(&mut app.core, ctx, focused);
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
    st.rect = Some(crate::ui::geom::area(area.intersect(ui.clip_rect())));
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
        let missing = st.is_missing(&pin.path);
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
                    icon_color: crate::ui::looks::folder_color_of(&pin.path, &p),
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

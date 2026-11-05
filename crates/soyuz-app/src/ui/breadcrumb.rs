//! The omnibar: breadcrumb segments with per-segment sibling dropdowns that turns into
//! an editable path field (click empty space or Ctrl+L) with directory completion.

use egui::text::{CCursor, CCursorRange};
use egui::{
    Align2, Color32, Frame, Id, Key, Popup, Rect, RectAlign, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2,
    vec2,
};
use soyuz_core::Location;

use crate::app::AddressState;
use crate::cmd::Cmd;
use crate::services::{AuxState, Services};
use crate::state::crumbs::{self, Crumb, CrumbKind};
use crate::state::pane::PaneId;
use crate::theme::font::semibold;
use crate::theme::tokens::{Rail, mix, radius};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::widgets::{
    fit, hidden_controls_tip, menu_item, menu_item_marked, paint_hidden_badge, spinner, trunc_galley,
};

pub struct OmnibarCtx<'a> {
    pub pane: PaneId,
    pub location: &'a Location,
    pub services: &'a mut Services,
    pub cmds: &'a mut Vec<Cmd>,
    pub address: &'a mut AddressState,
    /// Volume labels for root crumbs (`C:` → `Local Disk (C:)`).
    pub root_label: Option<String>,
    pub loading: bool,
}

const SEP_W: f32 = 18.0;
const CRUMB_PAD: f32 = 8.0;
/// The "…" button of the collapsed crumbs.
const MORE_W: f32 = 28.0;

fn kind_glyph(c: &Crumb) -> &'static str {
    match c.kind {
        CrumbKind::Root => ph::HARD_DRIVE,
        CrumbKind::Home => ph::HOUSE,
        CrumbKind::Archive => ph::FILE_ZIP,
        CrumbKind::Folder => ph::FOLDER,
        CrumbKind::Server => ph::GLOBE,
        CrumbKind::Virtual => match c.location {
            Location::Trash => ph::TRASH,
            Location::Computer => ph::DESKTOP_TOWER,
            Location::Search(_) => ph::MAGNIFYING_GLASS,
            _ => ph::FOLDER,
        },
    }
}

/// Draw the omnibar inside `rect`.
/// Width of the folder-color dot before a crumb's name.
const DOT_W: f32 = 12.0;

pub fn show(ui: &mut Ui, rect: Rect, c: &mut OmnibarCtx<'_>) {
    let p = palette(ui.ctx());
    let editing = c.address.editing;
    let field = rect;
    let bg = ui.interact(field, Id::unique("soyuz.omnibar.bg"), Sense::click());
    crate::ui::a11y::container(ui.ctx(), bg.id, egui::accesskit::Role::Navigation, tr!("app-crumb-address"));
    let hover = bg.hovered() && !editing;
    // Borderless field: hover and editing change the fill (high contrast keeps outlines).
    let fill = if editing || bg.has_focus() {
        p.rail(Rail::FieldFocus)
    } else if hover {
        p.rail(Rail::FieldHover)
    } else {
        p.rail(Rail::Field)
    };
    ui.painter().rect_filled(field, crate::theme::tokens::radius::same(radius::CARD), fill);
    if p.high_contrast() {
        let stroke = if editing {
            Stroke::new(2.0, p.accent)
        } else if hover {
            Stroke::new(1.0, p.border_strong)
        } else {
            Stroke::new(1.0, p.border_subtle)
        };
        ui.painter().rect_stroke(field, crate::theme::tokens::radius::same(radius::CARD), stroke, StrokeKind::Inside);
    }
    if editing {
        editor(ui, field, c);
        return;
    }
    let home = soyuz_fs::home();
    let mut list = crumbs::crumbs(c.location, Some(&home));
    if let (Some(first), Some(label)) = (list.first_mut(), c.root_label.as_ref())
        && first.kind == CrumbKind::Root
    {
        first.label = label.clone();
    }
    if list.is_empty() {
        return;
    }
    // Leading glyph of the location kind.
    let lead = list.first().map(kind_glyph).unwrap_or(ph::FOLDER);
    let lead_c = pos2(field.left() + 18.0, field.center().y);
    if c.loading {
        spinner(ui, lead_c, 6.0, p.accent);
    } else {
        ui.painter().text(lead_c, Align2::CENTER_CENTER, lead, icons::font_fill(15.0), p.text_secondary);
    }
    let left = field.left() + 34.0;
    let right = field.right() - 34.0;
    let avail = right - left;
    let font = crate::theme::font::ui(13.0);
    // Folder colors show as a dot before the crumb's name.
    let dots: Vec<Option<Color32>> = list
        .iter()
        .map(|cr| match &cr.location {
            soyuz_core::Location::Dir(d) => crate::ui::looks::folder_color_of(d, &p),
            _ => None,
        })
        .collect();
    let widths: Vec<f32> = list
        .iter()
        .enumerate()
        .map(|(i, cr)| {
            let f = if i + 1 == list.len() { semibold(13.0) } else { font.clone() };
            let badge = if soyuz_core::naming::has_hidden_controls(&cr.label) { 14.0 } else { 0.0 };
            let w = trunc_galley(ui.painter(), &cr.label, f, Color32::WHITE, f32::INFINITY).size().x;
            // The current folder's name may take all the room there is.
            (if i + 1 == list.len() { w } else { w.min(crate::theme::font::grow(220.0)) })
                + badge
                + 2.0 * CRUMB_PAD
                + if dots[i].is_some() { DOT_W } else { 0.0 }
        })
        .collect();
    let hidden = collapsed(&widths, avail);
    let mut x = left;
    let h = field.height() - 8.0;
    let cy = field.center().y;
    let n = list.len();
    for i in 0..n {
        if hidden.contains(&i) {
            if hidden.first() == Some(&i) {
                // Overflow dropdown with the collapsed ancestors.
                let r = Rect::from_min_size(pos2(x, cy - h / 2.0), vec2(MORE_W, h));
                let resp = ui.interact(r, Id::unique("soyuz.crumb.more"), Sense::click());
                crate::ui::a11y::button(&resp, tr!("app-crumb-hidden-parents"));
                if resp.hovered() {
                    ui.painter().rect_filled(r, crate::theme::tokens::radius::same(radius::ROW), seg_hover(&p));
                }
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    ph::DOTS_THREE,
                    icons::font(14.0),
                    p.text_secondary,
                );
                crate::ui::native::popup::Menu::menu(&resp).show(|ui| {
                    ui.set_width(220.0);
                    for &j in hidden.iter().rev() {
                        if menu_item(ui, Some(ph::FOLDER), &list[j].label, None, true).clicked() {
                            c.cmds.push(Cmd::Navigate {
                                pane: c.pane,
                                location: list[j].location.clone(),
                                select: None,
                            });
                            ui.close();
                        }
                    }
                });
                x += MORE_W;
                // The chevron after "…" lists the children of the last collapsed crumb.
                let last_hidden = *hidden.last().unwrap_or(&i);
                let parent = list.get(last_hidden).map(|cr| cr.location.clone());
                let child = list.get(last_hidden + 1).map(|cr| cr.label.clone());
                sep(ui, &mut x, cy, h, c, parent, child);
            }
            continue;
        }
        let last = i + 1 == n;
        let w = widths[i];
        let r = Rect::from_min_size(pos2(x, cy - h / 2.0), vec2(w.min(right - x).max(0.0), h));
        if r.width() <= 4.0 {
            break;
        }
        let resp = ui.interact(r, Id::unique(("soyuz.crumb", i)), Sense::click());
        crate::ui::a11y::button(
            &resp,
            &if last {
                tr!("app-crumb-edit-path", name = &list[i].label)
            } else {
                tr!("app-crumb-go-to", name = &list[i].label)
            },
        );
        if resp.hovered() {
            ui.painter().rect_filled(r, crate::theme::tokens::radius::same(radius::ROW), seg_hover(&p));
        }
        let color = if last || resp.hovered() { p.text_primary } else { p.text_secondary };
        let f = if last { semibold(13.0) } else { font.clone() };
        let hidden = soyuz_core::naming::has_hidden_controls(&list[i].label);
        let badge = if hidden { 14.0 } else { 0.0 };
        let dot_w = if dots[i].is_some() { DOT_W } else { 0.0 };
        if let Some(dc) = dots[i] {
            let ring = if resp.hovered() { seg_hover(&p) } else { fill };
            crate::ui::looks::paint_dot(ui.painter(), pos2(r.left() + CRUMB_PAD + 3.5, cy), dc, ring);
        }
        let tx = r.left() + CRUMB_PAD + dot_w;
        let room = r.width() - 2.0 * CRUMB_PAD - badge - dot_w;
        let g = fit::ends(ui.painter(), &list[i].label, f, color, room).galley;
        let gw = g.size().x;
        ui.painter().galley(pos2(tx, cy - g.size().y / 2.0), g, color);
        let resp = if hidden {
            paint_hidden_badge(ui.painter(), &p, pos2(tx + gw, cy), &list[i].label);
            resp.on_hover_text(hidden_controls_tip())
        } else {
            resp
        };
        // Shift: open it in the other pane. Ctrl: copy its path.
        let mods = ui.input(|i| i.modifiers);
        if resp.clicked() {
            if mods.shift {
                c.cmds.push(Cmd::OpenInOtherPane(list[i].location.clone()));
            } else if mods.command {
                c.cmds.push(Cmd::CopyText(list[i].location.display()));
            } else if last {
                start_edit(c);
            } else {
                c.cmds.push(Cmd::FocusPane(c.pane));
                c.cmds.push(Cmd::Navigate { pane: c.pane, location: list[i].location.clone(), select: None });
            }
        }
        if resp.middle_clicked() {
            c.cmds.push(Cmd::OpenInNewTab { location: list[i].location.clone(), activate: false });
        }
        if let soyuz_core::Location::Dir(d) = &list[i].location {
            crate::ui::dnd::target_open(ui, &resp, d, c.pane, c.cmds);
        }
        x += r.width();
        if x + SEP_W > right {
            break;
        }
        let child = list.get(i + 1).map(|cr| cr.label.clone());
        sep(ui, &mut x, cy, h, c, Some(list[i].location.clone()), child);
    }
    // Refresh button at the right end.
    let rr = Rect::from_center_size(pos2(field.right() - 18.0, cy), vec2(26.0, 26.0));
    let rresp = ui.interact(rr, Id::unique("soyuz.omnibar.refresh"), Sense::click());
    crate::ui::a11y::button(&rresp, tr!("app-crumb-refresh"));
    if rresp.hovered() {
        ui.painter().rect_filled(rr, crate::theme::tokens::radius::same(radius::ROW), seg_hover(&p));
    }
    ui.painter().text(
        rr.center(),
        Align2::CENTER_CENTER,
        ph::ARROW_CLOCKWISE,
        icons::font(14.0),
        if rresp.hovered() { p.text_primary } else { p.text_tertiary },
    );
    if rresp.on_hover_text(tr!("app-crumb-refresh-tip")).clicked() {
        c.cmds.push(Cmd::PaneAction(c.pane, soyuz_core::config::Action::Refresh));
    }
    if bg.clicked() {
        start_edit(c);
    }
}

/// The crumbs that go under the "…" button when the path does not fit in `avail`. The
/// current folder's name matters most: the middle goes first, keeping as many folders next
/// to the current one as fit, then the first crumb too. What is left for the name, if it
/// still is too long, is its own width minus the cut `fit::ends` makes.
fn collapsed(widths: &[f32], avail: f32) -> Vec<usize> {
    let n = widths.len();
    let total: f32 = widths.iter().sum::<f32>() + SEP_W * (n.saturating_sub(1)) as f32;
    if total <= avail || n < 2 {
        return Vec::new();
    }
    let more = MORE_W + SEP_W;
    let last = widths[n - 1];
    // No room for the first crumb next to "…" and the name.
    let first = widths[0] + SEP_W;
    if n == 2 || first + more + last > avail {
        let mut used = more + last;
        let mut keep_from = n - 1;
        while keep_from > 0 && used + widths[keep_from - 1] + SEP_W <= avail {
            used += widths[keep_from - 1] + SEP_W;
            keep_from -= 1;
        }
        return (0..keep_from).collect();
    }
    let mut used = first + more + last;
    let mut keep_from = n - 1;
    while keep_from > 1 && used + widths[keep_from - 1] + SEP_W <= avail {
        used += widths[keep_from - 1] + SEP_W;
        keep_from -= 1;
    }
    (1..keep_from).collect()
}

/// Hover fill of a crumb / chevron / button inside the (hovered) omnibar field.
fn seg_hover(p: &crate::theme::Palette) -> Color32 {
    mix(p.rail(Rail::FieldHover), p.text_primary, if p.dark { 0.06 } else { 0.05 })
}

fn start_edit(c: &mut OmnibarCtx<'_>) {
    crumbs::start_edit(c.address, c.location);
}

/// Chevron separator that opens a dropdown of the folders inside `parent`.
fn sep(
    ui: &mut Ui,
    x: &mut f32,
    cy: f32,
    h: f32,
    c: &mut OmnibarCtx<'_>,
    parent: Option<Location>,
    current_child: Option<String>,
) {
    let p = palette(ui.ctx());
    let r = Rect::from_min_size(pos2(*x, cy - h / 2.0), vec2(SEP_W, h));
    *x += SEP_W;
    let Some(Location::Dir(dir)) = parent else {
        ui.painter().text(r.center(), Align2::CENTER_CENTER, ph::CARET_RIGHT, icons::font(11.0), p.text_tertiary);
        return;
    };
    let id = Id::unique(("soyuz.crumb.sep", &dir));
    let resp = ui.interact(r, id, Sense::click());
    crate::ui::a11y::button(&resp, tr!("app-crumb-subfolders"));
    let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&resp));
    if resp.hovered() || open {
        ui.painter().rect_filled(r, crate::theme::tokens::radius::same(radius::CONTROL), seg_hover(&p));
    }
    let glyph = if open { ph::CARET_DOWN } else { ph::CARET_RIGHT };
    ui.painter().text(
        r.center(),
        Align2::CENTER_CENTER,
        glyph,
        icons::font(11.0),
        if resp.hovered() { p.text_primary } else { p.text_tertiary },
    );
    crate::ui::native::popup::Menu::menu(&resp).align(RectAlign::BOTTOM_START).show(|ui| {
        ui.set_width(240.0);
        let entries = c.services.aux_entries(&dir);
        match entries {
            None => {
                let failed = matches!(c.services.aux_state(&dir), Some(AuxState::Failed(_)));
                let (r, _) = ui.allocate_exact_size(vec2(240.0, 32.0), Sense::hover());
                if failed {
                    ui.painter().text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        tr!("app-crumb-cant-read"),
                        crate::theme::font::ui(12.0),
                        p.text_tertiary,
                    );
                } else {
                    spinner(ui, r.center(), 7.0, p.text_tertiary);
                }
            }
            Some(entries) => {
                let dirs: Vec<_> = entries.iter().filter(|e| e.is_dir_like() && !e.is_hidden()).collect();
                if dirs.is_empty() {
                    menu_item(ui, Some(ph::FOLDER_DASHED), tr!("app-crumb-no-folders"), None, false);
                }
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    for e in dirs {
                        let is_current = current_child.as_deref() == Some(e.name.as_str());
                        let r = menu_item(
                            ui,
                            Some(if is_current { ph::FOLDER_OPEN } else { ph::FOLDER }),
                            &e.name,
                            None,
                            true,
                        );
                        if is_current {
                            ui.painter().rect_stroke(
                                r.rect,
                                crate::theme::tokens::radius::same(radius::ROW),
                                Stroke::new(1.0, p.accent.gamma_multiply(0.5)),
                                StrokeKind::Inside,
                            );
                        }
                        if r.clicked() {
                            c.cmds.push(Cmd::Navigate {
                                pane: c.pane,
                                location: Location::Dir(e.path.clone()),
                                select: None,
                            });
                            ui.close();
                        }
                    }
                });
            }
        }
    });
}

/// Path editor with completion (the editing logic is `state::crumbs`, shared with the
/// terminal UI).
fn editor(ui: &mut Ui, field: Rect, c: &mut OmnibarCtx<'_>) {
    // Full field height: the text edit centers its row (see the filter in toolbar.rs).
    let inner = field.shrink2(vec2(12.0, 0.0));
    let id = Id::unique("soyuz.omnibar.edit");
    // Completion candidates for the typed parent folder.
    let comp = crumbs::completion(c.services, &c.address.text);
    let names = &comp.names;
    // Keyboard handling before the text edit consumes keys.
    let (up, down, tab, esc) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, Key::Tab),
            i.consume_key(egui::Modifiers::NONE, Key::Escape),
        )
    });
    if esc {
        crumbs::cancel(c.address);
        return;
    }
    if down {
        crumbs::cycle(c.address, names.len(), true);
    }
    if up {
        crumbs::cycle(c.address, names.len(), false);
    }
    let moved_cursor = tab && crumbs::tab(c.address, names);
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
    let te = TextEdit::singleline(&mut c.address.text)
        .id(id)
        .frame(Frame::NONE)
        .font(crate::theme::font::ui(13.0))
        .desired_width(inner.width())
        .min_size(vec2(0.0, inner.height()))
        .align(egui::Align2::LEFT_CENTER);
    let out = te.show(&mut child);
    let resp = out.response;
    if c.address.request_focus {
        resp.request_focus();
        let mut state = out.state;
        let len = c.address.text.chars().count();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(len))));
        state.store(ui.ctx(), id);
        c.address.request_focus = false;
    } else if moved_cursor {
        let mut state = out.state;
        let len = c.address.text.chars().count();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(len))));
        state.store(ui.ctx(), id);
    }
    if resp.changed() {
        c.address.completion = None;
    }
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    if enter {
        crumbs::enter(c.address, c.services, c.pane, c.cmds, &comp);
        return;
    }
    if resp.lost_focus() && !tab {
        c.address.editing = false;
        return;
    }
    // Completion popup.
    if !names.is_empty() && resp.has_focus() {
        let popup_rect =
            Rect::from_min_size(pos2(field.left(), field.bottom() + 4.0), vec2(field.width().min(520.0), 0.0));
        egui::Area::new(Id::unique("soyuz.omnibar.complete"))
            .order(egui::Order::Foreground)
            .fixed_pos(popup_rect.min)
            .show(ui.ctx(), |ui| {
                Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_width(popup_rect.width() - 14.0);
                    for (i, name) in names.iter().enumerate() {
                        let shortcut = if i == 0 { Some("Tab") } else { None };
                        let on = c.address.completion == Some(i);
                        let r = menu_item_marked(ui, Some(ph::FOLDER), name, shortcut, true, on);
                        if r.clicked() {
                            crumbs::accept(&mut c.address.text, name);
                            c.address.request_focus = false;
                            resp.request_focus();
                        }
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_that_fits_stays_whole() {
        assert!(collapsed(&[80.0, 60.0, 100.0], 400.0).is_empty());
        assert!(collapsed(&[80.0], 10.0).is_empty(), "nothing to collapse");
    }

    #[test]
    fn the_middle_goes_first_and_folders_near_the_name_stay() {
        // 80 > … > 50 > 100 fits, 60 and 70 go.
        let w = [80.0, 70.0, 60.0, 50.0, 100.0];
        assert_eq!(collapsed(&w, 80.0 + 18.0 + 28.0 + 18.0 + 50.0 + 18.0 + 100.0), vec![1, 2]);
    }

    #[test]
    fn the_first_crumb_goes_before_the_name_is_cut() {
        // "…" and the name fit, the first crumb does not.
        assert_eq!(collapsed(&[80.0, 70.0, 100.0], 28.0 + 18.0 + 100.0 + 20.0), vec![0, 1]);
        assert_eq!(collapsed(&[80.0, 100.0], 150.0), vec![0], "also with two crumbs");
        // Too narrow even for the name alone: still only the name, which gets cut.
        assert_eq!(collapsed(&[80.0, 70.0, 300.0], 120.0), vec![0, 1]);
    }

    #[test]
    fn freed_room_goes_to_the_folders_next_to_the_name() {
        assert_eq!(collapsed(&[200.0, 70.0, 60.0, 100.0], 28.0 + 18.0 + 60.0 + 18.0 + 100.0), vec![0, 1]);
    }
}

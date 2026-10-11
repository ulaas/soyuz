//! The command palette's GUI: the animated overlay (input row, ranked list, footer with
//! the prefix legend), its keys, and the pending key-sequence hint. State, ranking and
//! execution live in [`crate::palette`].

pub mod widgets;

use egui::emath::TSTransform;
use egui::{
    Align2, Color32, CornerRadius, FontId, Frame, Id, Key, LayerId, Margin, Modal, Modifiers, Order, Rect, RichText,
    Sense, Stroke, StrokeKind, TextEdit, Ui, pos2, vec2,
};
use soyuz_core::config::{Action, KeyContext};

use crate::app::SoyuzApp;
use crate::palette::query::Mode;
use crate::palette::{self, Exec, Item, KeyInput, RunMods, Tint};
use crate::theme::icons::ph;
use crate::theme::{Palette, icons};
use crate::ui::geom;
use crate::ui::widgets::trunc_galley;

use widgets::{highlighted, icon_tile, paint_chips};

const ROW_H: f32 = 46.0;
const HEADER_H: f32 = 26.0;
const INPUT_H: f32 = 54.0;
const FOOTER_H: f32 = 36.0;
const MAX_LIST_H: f32 = 430.0;

fn tint_color(p: &Palette, t: Tint) -> Color32 {
    match t {
        Tint::Accent => p.accent,
        Tint::Folder => p.glyph_folder,
        Tint::Neutral => p.text_tertiary,
        Tint::Success => p.success,
        Tint::Warning => p.warning,
        Tint::Info => p.info,
    }
}

/// Draw the palette (when open) and the pending key-sequence hint.
pub fn show(ctx: &egui::Context, app: &mut SoyuzApp) {
    sequence_hint(ctx, app);
    if !app.palette.open {
        return;
    }
    let p = crate::theme::palette(ctx);
    let now = ctx.input(|i| i.time);
    if app.palette.opened_at.is_nan() {
        app.palette.opened_at = now;
    }
    let t = if app.ui.reduced_motion { 1.0 } else { ((now - app.palette.opened_at) / 0.16).clamp(0.0, 1.0) as f32 };
    let ease = 1.0 - (1.0 - t).powi(3);
    if t < 1.0 {
        ctx.request_repaint();
    }

    // Keyboard first (before the text field sees the keys).
    let keys = read_keys(ctx, app);
    palette::apply_keys(app, ctx, keys);
    if !app.palette.open {
        return;
    }
    palette::refresh_results(app);

    let screen = ctx.content_rect();
    let width = (screen.width() - 32.0).clamp(320.0, 660.0);
    let top = (screen.height() * 0.13).clamp(24.0, 120.0);
    let id = Id::new("soyuz.palette");
    let area = Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, top));
    let frame = Frame::new()
        .fill(p.bg_surface3)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::ZERO)
        .shadow(egui::epaint::Shadow {
            offset: [0, 18],
            blur: 56,
            spread: 0,
            color: Color32::from_black_alpha((p.shadow_alpha * 255.0 * 1.2).min(255.0) as u8),
        });
    let mut clicked: Option<(Exec, RunMods)> = None;
    let resp = Modal::new(id)
        .area(area)
        .frame(frame)
        .backdrop_color(Color32::from_black_alpha(((if p.dark { 140.0 } else { 80.0 }) * ease) as u8))
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.set_opacity(ease);
            crate::ui::a11y::container(
                ui.ctx(),
                ui.unique_id(),
                egui::accesskit::Role::Dialog,
                tr!("action-command-palette"),
            );
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            input_row(ui, app, &p, width);
            divider(ui, &p);
            clicked = list(ui, app, &p, width);
            footer(ui, app, &p, width, &mut clicked);
        });
    // Scale-in from 97 % around the top center, sliding down 8 px.
    let s = 0.97 + 0.03 * ease;
    let pivot = pos2(screen.center().x, top);
    let tr = TSTransform::new(pivot.to_vec2() * (1.0 - s) + vec2(0.0, -8.0 * (1.0 - ease)), s);
    ctx.set_transform_layer(LayerId::new(Order::Foreground, id), tr);
    if let Some((exec, mods)) = clicked {
        palette::run(app, ctx, exec, mods);
    } else if resp.backdrop_response.clicked() {
        palette::close(app);
    }
}

/// This frame's palette keys, consumed before the text field sees them.
fn read_keys(ctx: &egui::Context, app: &SoyuzApp) -> KeyInput {
    let mut k = KeyInput::default();
    let cmd_is_ctrl = app.config.keys.cmd_is_ctrl;
    let km = &app.keymap;
    ctx.input_mut(|i| {
        let mods = i.modifiers;
        // Global palette chords toggle / switch modes.
        i.events.retain(|ev| {
            let Some(chord) = crate::keys::event_chord(ev, mods, cmd_is_ctrl) else { return true };
            if chord.mods.is_empty() {
                return true;
            }
            match km.lookup(KeyContext::Global, chord) {
                Some(Action::CommandPalette) => {
                    k.toggle = Some(Mode::Commands);
                    false
                }
                Some(Action::JumpToFolderDialog) => {
                    k.toggle = Some(Mode::Folders);
                    false
                }
                _ => true,
            }
        });
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            k.escape = true;
            return;
        }
        k.down = i.consume_key(Modifiers::NONE, Key::ArrowDown)
            || i.consume_key(Modifiers::CTRL, Key::J)
            || i.consume_key(Modifiers::CTRL, Key::N);
        k.up = i.consume_key(Modifiers::NONE, Key::ArrowUp)
            || i.consume_key(Modifiers::CTRL, Key::K)
            || i.consume_key(Modifiers::CTRL, Key::P);
        k.page_down = i.consume_key(Modifiers::NONE, Key::PageDown);
        k.page_up = i.consume_key(Modifiers::NONE, Key::PageUp);
        let enter_mods = [Modifiers::NONE, Modifiers::CTRL, Modifiers::SHIFT, Modifiers::COMMAND];
        for m in enter_mods {
            if i.consume_key(m, Key::Enter) {
                k.enter = Some(run_mods(m));
                break;
            }
        }
        k.tab = i.consume_key(Modifiers::NONE, Key::Tab);
        i.consume_key(Modifiers::SHIFT, Key::Tab);
    });
    k
}

fn run_mods(m: Modifiers) -> RunMods {
    RunMods { command: m.command, shift: m.shift }
}

fn divider(ui: &mut Ui, p: &Palette) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, p.border_subtle);
}

fn input_row(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, INPUT_H), Sense::hover());
    let mode = app.palette.mode();
    let painter = ui.painter();
    let mut x = rect.left() + 18.0;
    let glyph = match mode {
        Mode::Commands => ph::COMMAND,
        Mode::Folders | Mode::Path => ph::FOLDER_OPEN,
        Mode::Bookmarks => ph::BOOKMARK_SIMPLE,
        Mode::Search => ph::MAGNIFYING_GLASS,
        Mode::Shell => ph::TERMINAL,
        Mode::Tabs => ph::BROWSERS,
    };
    painter.text(pos2(x + 9.0, rect.center().y), Align2::CENTER_CENTER, glyph, icons::font(19.0), p.accent);
    x += 30.0;
    // Mode chip for prefixed / folder modes.
    if mode != Mode::Commands {
        let label = mode.label();
        let g = painter.layout_no_wrap(label.to_string(), FontId::proportional(11.5), p.accent);
        let chip = Rect::from_min_size(pos2(x, rect.center().y - 11.0), vec2(g.size().x + 16.0, 22.0));
        painter.rect_filled(chip, CornerRadius::same(11), p.accent_subtle);
        painter.galley(pos2(chip.left() + 8.0, chip.center().y - g.size().y / 2.0), g, p.accent);
        x = chip.right() + 8.0;
    }
    // "esc" hint on the right.
    let esc_left = paint_chips(ui, p, "Esc", rect.right() - 16.0, rect.center().y, false);
    let edit_rect = Rect::from_min_max(pos2(x, rect.top() + 8.0), pos2(esc_left - 10.0, rect.bottom() - 8.0));
    let hint = mode.placeholder();
    let mut child = ui
        .new_child(egui::UiBuilder::new().max_rect(edit_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let te = TextEdit::singleline(&mut app.palette.input)
        .id(Id::new("soyuz.palette.input"))
        .frame(Frame::NONE)
        .font(FontId::proportional(16.5))
        .text_color(p.text_primary)
        .hint_text(RichText::new(hint).color(p.text_tertiary).size(15.0))
        .desired_width(edit_rect.width())
        .margin(Margin::symmetric(0, 4));
    let out = te.show(&mut child);
    crate::ui::a11y::name(&out.response, hint);
    if app.palette.focus_input {
        out.response.request_focus();
        app.palette.focus_input = false;
    }
    if !out.response.has_focus() {
        // Keep typing in the palette even after a click elsewhere in it.
        out.response.request_focus();
    }
    if app.palette.cursor_to_end {
        app.palette.cursor_to_end = false;
        let mut state = out.state.clone();
        let end = egui::text::CCursor::new(app.palette.input.chars().count());
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
        state.store(ui.ctx(), out.response.id);
    }
}

fn list(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, width: f32) -> Option<(Exec, RunMods)> {
    let items = &app.palette.results;
    let mut clicked = None;
    if items.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 120.0), Sense::hover());
        let painter = ui.painter();
        painter.text(
            rect.center() - vec2(0.0, 14.0),
            Align2::CENTER_CENTER,
            ph::MAGNIFYING_GLASS,
            icons::font(26.0),
            p.text_tertiary,
        );
        painter.text(
            rect.center() + vec2(0.0, 16.0),
            Align2::CENTER_CENTER,
            tr!("actions-palette-no-matches"),
            FontId::proportional(13.0),
            p.text_secondary,
        );
        return None;
    }
    // Heights with section headers.
    let mut total = 8.0;
    let mut last: Option<&str> = None;
    for it in items {
        if last != Some(it.section) {
            total += HEADER_H;
            last = Some(it.section);
        }
        total += ROW_H;
    }
    let height = total.min(MAX_LIST_H);
    let selected = app.palette.selected;
    let scroll_to = std::mem::take(&mut app.palette.scroll_to_selected);
    // Mouse hover selects only after the pointer moved (not when the palette opens under it).
    let pointer = ui.ctx().input(|i| i.pointer.hover_pos());
    let moved = match (app.palette.pointer_at_open, pointer) {
        (None, Some(pp)) => {
            app.palette.pointer_at_open = Some(geom::point(pp));
            false
        }
        (Some(a), Some(b)) => geom::pos(a).distance(b) > 3.0,
        _ => false,
    };
    let mut hover_sel = None;
    egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).auto_shrink([false, true]).show(
        ui,
        |ui| {
            ui.set_width(width);
            ui.add_space(4.0);
            let mut last: Option<&str> = None;
            for (i, it) in items.iter().enumerate() {
                if last != Some(it.section) {
                    let (r, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
                    ui.painter().text(
                        pos2(r.left() + 20.0, r.bottom() - 7.0),
                        Align2::LEFT_BOTTOM,
                        palette::upper(it.section),
                        FontId::proportional(10.5),
                        p.text_tertiary,
                    );
                    last = Some(it.section);
                }
                let (outer, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
                let r = outer.shrink2(vec2(8.0, 2.0));
                let is_sel = i == selected;
                let total = items.len();
                crate::ui::a11y::with(ui.ctx(), resp.id, |n| {
                    n.set_role(egui::accesskit::Role::ListBoxOption);
                    if it.detail.is_empty() {
                        n.set_label(it.title.as_str());
                    } else {
                        n.set_label(format!("{}, {}", it.title, it.detail));
                    }
                    if let Some(k) = &it.chord {
                        n.set_keyboard_shortcut(k.as_str());
                    }
                    n.set_selected(is_sel);
                    n.set_position_in_set(i + 1);
                    n.set_size_of_set(total);
                });
                if scroll_to && is_sel {
                    ui.scroll_to_rect(outer.expand2(vec2(0.0, 6.0)), None);
                }
                if moved && resp.hovered() && !is_sel {
                    hover_sel = Some(i);
                }
                if resp.clicked() {
                    clicked = Some((it.exec.clone(), run_mods(ui.input(|i| i.modifiers))));
                }
                if !ui.is_rect_visible(outer) {
                    continue;
                }
                let sel_t = ui.ctx().animate_bool_with_time(
                    Id::new(("soyuz.palette.row", i)),
                    is_sel,
                    crate::theme::anim(ui.ctx(), 0.08),
                );
                if sel_t > 0.0 {
                    ui.painter().rect_filled(r, CornerRadius::same(9), p.accent_subtle.gamma_multiply(sel_t));
                    let bar = Rect::from_min_size(pos2(r.left(), r.center().y - 9.0), vec2(3.0, 18.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(2), p.accent.gamma_multiply(sel_t));
                }
                row(ui, p, it, r, is_sel);
            }
            ui.add_space(4.0);
        },
    );
    if let Some(h) = hover_sel {
        app.palette.selected = h;
    }
    clicked
}

fn row(ui: &Ui, p: &Palette, it: &Item, r: Rect, selected: bool) {
    let tile = Rect::from_center_size(pos2(r.left() + 24.0, r.center().y), vec2(30.0, 30.0));
    let tint = tint_color(p, it.tint);
    icon_tile(ui, tile, it.icon, tint, 16.0);
    // Right side: chips + tag.
    let mut right = r.right() - 12.0;
    if let Some(c) = &it.chord {
        right = paint_chips(ui, p, c, right, r.center().y, selected) - 6.0;
    }
    if let Some(tag) = &it.tag {
        let g = ui.painter().layout_no_wrap(tag.clone(), FontId::proportional(10.5), p.text_tertiary);
        let pos = pos2(right - g.size().x, r.center().y - g.size().y / 2.0);
        right = pos.x - 10.0;
        ui.painter().galley(pos, g, p.text_tertiary);
    }
    let text_left = tile.right() + 12.0;
    let max_w = (right - text_left).max(40.0);
    let title_color = if selected { p.text_primary } else { p.text_primary.gamma_multiply(0.92) };
    let mut job = highlighted(&it.title, &it.hl, 13.5, title_color, p.accent);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    let title = ui.painter().layout_job(job);
    let has_detail = !it.detail.is_empty();
    let ty = if has_detail { r.center().y - 9.0 } else { r.center().y };
    ui.painter().galley(pos2(text_left, ty - title.size().y / 2.0), title, title_color);
    if has_detail {
        let g = trunc_galley(ui.painter(), &it.detail, FontId::proportional(11.5), p.text_tertiary, max_w);
        ui.painter().galley(pos2(text_left, r.center().y + 9.0 - g.size().y / 2.0), g, p.text_tertiary);
    }
}

fn footer(ui: &mut Ui, app: &SoyuzApp, p: &Palette, width: f32, clicked: &mut Option<(Exec, RunMods)>) {
    divider(ui, p);
    let (rect, _) = ui.allocate_exact_size(vec2(width, FOOTER_H), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius { nw: 0, ne: 0, sw: 14, se: 14 },
        if p.dark { p.bg_surface2.gamma_multiply(0.6) } else { p.bg_surface2 },
    );
    let cy = rect.center().y;
    let mut x = rect.left() + 16.0;
    let hint = |x: &mut f32, keys: &str, label: &str| {
        let w = widgets::chips_width(ui, keys);
        paint_chips(ui, p, keys, *x + w, cy, false);
        *x += w + 4.0;
        let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(11.0), p.text_tertiary);
        ui.painter().galley(pos2(*x, cy - g.size().y / 2.0), g.clone(), p.text_tertiary);
        *x += g.size().x + 12.0;
    };
    hint(&mut x, "↑↓", tr!("actions-palette-hint-move"));
    hint(&mut x, "Enter", tr!("actions-palette-hint-run"));
    if matches!(app.palette.mode(), Mode::Path | Mode::Shell | Mode::Folders | Mode::Bookmarks) {
        hint(&mut x, "Tab", tr!("actions-palette-hint-complete"));
    }
    // Prefix legend (clickable), right-aligned.
    let mut rx = rect.right() - 12.0;
    for m in Mode::PREFIXED.iter().rev() {
        let c = m.prefix().unwrap_or(' ');
        let label = format!("{c} {}", m.legend());
        let g = ui.painter().layout_no_wrap(label, FontId::proportional(11.0), p.text_secondary);
        let r = Rect::from_min_size(pos2(rx - g.size().x - 12.0, cy - 10.0), vec2(g.size().x + 12.0, 20.0));
        if r.left() < x {
            break;
        }
        let resp = ui.interact(r, Id::new(("soyuz.palette.prefix", c)), Sense::click());
        let active = app.palette.mode() == *m;
        crate::ui::a11y::toggle(&resp, &format!("{c} {}", m.legend()), active);
        let fill = if active {
            p.accent_subtle
        } else if resp.hovered() {
            p.bg_surface3
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(r, CornerRadius::same(10), fill);
        if !active {
            ui.painter().rect_stroke(r, CornerRadius::same(10), Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
        }
        let color = if active { p.accent } else { p.text_secondary };
        ui.painter().galley(pos2(r.left() + 6.0, cy - g.size().y / 2.0), g, color);
        if resp.clicked() {
            *clicked = Some((Exec::Prefix(c), RunMods::NONE));
        }
        rx = r.left() - 5.0;
    }
}

/// Bottom-center chip while a key sequence or mark is pending ("G …").
fn sequence_hint(ctx: &egui::Context, app: &SoyuzApp) {
    let Some(label) = app.keyseq.pending_label() else { return };
    let p = crate::theme::palette(ctx);
    let screen = ctx.content_rect();
    let font = FontId::proportional(12.5);
    let g = ctx.fonts_mut(|f| f.layout_no_wrap(label, font, p.text_primary));
    let size = vec2(g.size().x + 46.0, 30.0);
    let rect = Rect::from_center_size(pos2(screen.center().x, screen.bottom() - 60.0), size);
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("soyuz.keyseq")));
    painter.add(
        egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(80) }
            .as_shape(rect, CornerRadius::same(15)),
    );
    painter.rect_filled(rect, CornerRadius::same(15), p.bg_surface3);
    painter.rect_stroke(
        rect,
        CornerRadius::same(15),
        Stroke::new(1.0, p.accent.gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
    painter.text(
        pos2(rect.left() + 21.0, rect.center().y),
        Align2::CENTER_CENTER,
        ph::KEYBOARD,
        icons::font(14.0),
        p.accent,
    );
    painter.galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, p.text_primary);
}

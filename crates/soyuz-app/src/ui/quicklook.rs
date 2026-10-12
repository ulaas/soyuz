//! Quick Look: Space opens a large centered overlay previewing the cursor
//! item with the same renderers as the preview pane. Arrow keys move through the list
//! (the overlay follows), Esc / Space close, images zoom with the wheel / pinch and pan
//! by dragging (double-click toggles fit ↔ 100%). Open state and list movement are in
//! [`crate::models::quicklook`]; this module keeps the renderer state and the keys.

use std::cell::RefCell;

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Order, Rect, Sense, Stroke, UiBuilder, pos2, vec2,
};
use soyuz_core::{Location, format_size};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::models::quicklook::Step;
use crate::theme::icons::{self, ph};
use crate::theme::palette;
use crate::theme::tokens::radius;
use crate::ui::preview::render::{self, PreviewAction, PreviewState};
use crate::ui::views::flow::semibold;
use crate::ui::views::icons as file_icons;
use crate::ui::widgets::{icon_toggle, kbd, trunc_galley};

pub use crate::models::quicklook::{QuickLook, toggle};

/// Renderer state of the overlay (GUI-only).
#[derive(Default)]
struct View {
    state: Option<PreviewState>,
    /// Two-file diff when exactly two text files are selected.
    diff: crate::ui::preview::diff::DiffView,
    /// Something was shown since the overlay last closed.
    shown: bool,
}

thread_local! {
    static VIEW: RefCell<View> = RefCell::new(View::default());
}

/// Drop what the overlay rendered (it closed).
fn clear_view(v: &mut View) {
    if let Some(s) = &mut v.state {
        s.clear();
    }
    v.diff.clear();
    v.shown = false;
}

/// Keyboard while open: consumes Esc / Space / arrows / Enter so the file list doesn't
/// see them. Returns `true` when Quick Look handled input this frame.
pub fn handle_keys(app: &mut SoyuzApp, ctx: &egui::Context) -> bool {
    if !app.core.ui.quicklook.open {
        return false;
    }
    let (esc, space, up, down, left, right, enter, home, end) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::Escape),
            i.consume_key(Modifiers::NONE, Key::Space),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowLeft),
            i.consume_key(Modifiers::NONE, Key::ArrowRight),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Home),
            i.consume_key(Modifiers::NONE, Key::End),
        )
    });
    if esc || space {
        crate::models::quicklook::close(&mut app.core);
        return true;
    }
    let dir = if up || left {
        Some(Step::Prev)
    } else if down || right {
        Some(Step::Next)
    } else if home {
        Some(Step::First)
    } else if end {
        Some(Step::Last)
    } else {
        None
    };
    if let Some(d) = dir {
        crate::models::quicklook::step(&mut app.core, d);
    }
    if enter {
        crate::models::quicklook::open_item(&mut app.core);
    }
    true
}

/// The overlay (drawn over everything while open).
pub fn show(ctx: &egui::Context, app: &mut SoyuzApp) {
    VIEW.with_borrow_mut(|v| {
        if app.core.ui.quicklook.open {
            v.shown = true;
            show_open(ctx, app, v);
        }
        if !app.core.ui.quicklook.open && v.shown {
            clear_view(v);
        }
    });
}

fn show_open(ctx: &egui::Context, app: &mut SoyuzApp, v: &mut View) {
    if !app.core.ui.quicklook.open {
        return;
    }
    let p = palette(ctx);
    let target = {
        let pane = app.core.ws.pane();
        pane.cursor.as_deref().and_then(|c| pane.entry(c)).cloned()
    };
    let Some(target) = target else {
        app.core.ui.quicklook.close();
        return;
    };
    let (pos, count) = {
        let pane = app.core.ws.pane();
        (pane.cursor_pos().map_or(0, |i| i + 1), pane.visible.len())
    };
    let pane_id = app.core.ws.pane().id;
    let pair = crate::ui::preview::diff::pair(app);
    let diff_cap = app.core.config.preview.max_diff_kib;
    let st = v.state.get_or_insert_with(|| PreviewState::new(ctx, true));
    let limits = st.limits(&app.core.config, &p);
    st.set_target(Some(&target), limits);
    st.poll(ctx);

    let t = ctx.animate_bool_with_time(
        Id::new("soyuz.ql.anim"),
        true,
        crate::theme::anim(ctx, crate::theme::tokens::motion::MODAL),
    );
    let screen = ctx.content_rect();
    let dim = Color32::from_black_alpha((if p.dark { 150.0 } else { 90.0 } * t) as u8);

    let size = vec2((screen.width() * 0.86).min(1500.0), (screen.height() * 0.88).min(1100.0));
    let scale = 0.98 + 0.02 * t;
    let card = Rect::from_center_size(screen.center(), size * scale);
    let mut close = false;
    let mut action = None;
    egui::Area::new(Id::new("soyuz.quicklook")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        ui.set_opacity(t);
        // Full-screen catcher: clicking outside the card closes. Allocated (not just
        // interacted) so the overlay layer covers the whole window and the panes beneath get
        // no clicks, hover or wheel; the modal layer also keeps keyboard focus off them.
        let bg = ui.allocate_rect(screen, Sense::click());
        ui.ctx().memory_mut(|m| m.set_modal_layer(ui.layer_id()));
        if bg.clicked() && !bg.interact_pointer_pos().is_some_and(|ptr| card.contains(ptr)) {
            close = true;
        }
        let painter = ui.painter().clone();
        let painter = &painter;
        painter.rect_filled(screen, 0.0, dim);
        painter.add(
            egui::epaint::Shadow {
                offset: [0, 16],
                blur: 48,
                spread: 0,
                color: Color32::from_black_alpha((p.shadow_alpha * 255.0) as u8),
            }
            .as_shape(card, CornerRadius::same(radius::MODAL + 2)),
        );
        painter.rect_filled(card, CornerRadius::same(radius::MODAL + 2), p.bg_surface1);
        painter.rect_stroke(
            card,
            CornerRadius::same(radius::MODAL + 2),
            Stroke::new(1.0, p.border_strong),
            egui::StrokeKind::Inside,
        );
        let card_resp = ui.interact(card, Id::new("soyuz.ql.card"), Sense::click());
        let _ = card_resp;

        // Header.
        let header = Rect::from_min_size(card.min, vec2(card.width(), 52.0));
        painter.text(
            pos2(header.left() + 24.0, header.center().y),
            Align2::CENTER_CENTER,
            file_icons::glyph(&target),
            icons::font_fill(20.0),
            file_icons::color(&target, &p),
        );
        let tg = trunc_galley(painter, &target.name, semibold(14.5), p.text_primary, header.width() * 0.55);
        painter.galley(pos2(header.left() + 44.0, header.center().y - tg.size().y / 2.0), tg, p.text_primary);
        let close_r = Rect::from_min_size(pos2(header.right() - 42.0, header.center().y - 15.0), vec2(30.0, 30.0));
        if ui
            .scope_builder(UiBuilder::new().max_rect(close_r), |ui| {
                icon_toggle(ui, ph::X, tr!("views-quicklook-close"), true, false)
            })
            .inner
            .clicked()
        {
            close = true;
        }
        let hex_r = Rect::from_min_size(pos2(close_r.left() - 34.0, close_r.top()), vec2(30.0, 30.0));
        let hex_on = st.force_hex;
        if ui
            .scope_builder(UiBuilder::new().max_rect(hex_r), |ui| {
                icon_toggle(ui, ph::BINARY, tr!("views-preview-show-hex"), !target.is_dir_like(), hex_on)
            })
            .inner
            .clicked()
        {
            action = Some(PreviewAction::ToggleHex);
        }
        painter.text(
            pos2(hex_r.left() - 12.0, header.center().y),
            Align2::RIGHT_CENTER,
            tr!("views-quicklook-position", pos = pos, count = count),
            FontId::proportional(12.0),
            p.text_tertiary,
        );
        painter.line_segment(
            [pos2(card.left(), header.bottom()), pos2(card.right(), header.bottom())],
            Stroke::new(1.0, p.border_subtle),
        );

        // Footer: key hints + a short info line.
        let footer = Rect::from_min_max(pos2(card.left(), card.bottom() - 40.0), card.max);
        painter.line_segment(
            [pos2(card.left(), footer.top()), pos2(card.right(), footer.top())],
            Stroke::new(1.0, p.border_subtle),
        );
        let rows = render::info_rows(&target, st.loaded.as_ref(), st.folder_size(), &app.core.config);
        let mut summary: Vec<String> = Vec::new();
        for k in ["Type", "Size", "Dimensions", "Duration", "Modified"] {
            if let Some((_, v)) = rows.iter().find(|(a, _)| a == k) {
                summary.push(v.clone());
            }
        }
        if target.is_dir_like()
            && let Some((b, ..)) = st.folder_size()
        {
            summary.push(format_size(b, app.core.config.general.size_style));
        }
        let sg = trunc_galley(
            painter,
            &summary.join("  ·  "),
            FontId::proportional(12.0),
            p.text_secondary,
            footer.width() * 0.6,
        );
        painter.galley(pos2(footer.left() + 18.0, footer.center().y - sg.size().y / 2.0), sg, p.text_secondary);
        let mut hints = ui.new_child(
            UiBuilder::new()
                .max_rect(footer.shrink2(vec2(14.0, 8.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        for (k, label) in [
            ("Esc", tr!("views-quicklook-hint-close")),
            ("↑↓", tr!("views-quicklook-hint-browse")),
            ("Enter", tr!("views-quicklook-hint-open")),
        ] {
            hints.label(egui::RichText::new(label).size(11.0).color(p.text_tertiary));
            kbd(&mut hints, k);
            hints.add_space(8.0);
        }

        let body = Rect::from_min_max(
            pos2(card.left() + 1.0, header.bottom() + 1.0),
            pos2(card.right() - 1.0, footer.top() - 1.0),
        );
        let mut child = ui.new_child(UiBuilder::new().max_rect(body));
        child.set_clip_rect(body);
        if let Some((a, b)) = &pair {
            crate::ui::preview::diff::show(&mut child, body, &mut v.diff, a, b, diff_cap);
        } else if let Some(a) = render::content(&mut child, body, st, true) {
            action = Some(a);
        }
    });
    if close {
        app.core.ui.quicklook.close();
    }
    match action {
        Some(PreviewAction::ToggleHex) => {
            if let Some(s) = &mut v.state {
                s.force_hex = !s.force_hex;
                s.invalidate();
            }
        }
        Some(PreviewAction::Open(path)) => {
            app.core.ui.quicklook.close();
            app.core.cmds.push(Cmd::Navigate {
                pane: pane_id,
                location: Location::Archive { archive: path, inner: String::new() },
                select: None,
            });
        }
        None => {}
    }
}

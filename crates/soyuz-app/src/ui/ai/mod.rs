//! The AI side panel (right dock, Ctrl+I): header with profile / model and
//! folder-access chips, a live token and cost meter and a stop button; tabs for the chat,
//! recipes and the audit log. Chat items, plan cards and diffs live in the submodules.
//! Widgets never mutate app state while drawing: they push [`PanelAct`]s that [`apply`]
//! runs after the panel is drawn.

pub mod cards;
pub mod chat;
pub mod diff;
pub mod mcp_area;
pub mod pages;
pub mod touch;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Panel, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use soyuz_core::Location;
use soyuz_core::config::{Action, AiAccess};

use crate::ai::model::{self, Route};
pub use crate::ai::panel::{self, PanelAct, tokens};
use crate::ai::state::{self, AiCmd, PanelTab};
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::theme::icons::{self, ph};
use crate::theme::tokens::radius;
use crate::theme::{Palette, palette};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{icon_toggle, menu_item, menu_separator};

/// Small rounded chip; `strong` uses the accent.
pub fn chip(ui: &mut Ui, icon: Option<&str>, label: &str, strong: bool) -> Response {
    chip_impl(ui, icon, label, strong, false)
}

/// A chip that opens a menu (caret on the right).
pub fn chip_menu(ui: &mut Ui, icon: Option<&str>, label: &str, strong: bool) -> Response {
    chip_impl(ui, icon, label, strong, true)
}

fn chip_impl(ui: &mut Ui, icon: Option<&str>, label: &str, strong: bool, caret: bool) -> Response {
    let p = palette(ui.ctx());
    let font = FontId::proportional(11.5);
    let g = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let iw = if icon.is_some() { 16.0 } else { 0.0 };
    let cw = if caret { 14.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0 + iw + cw, 22.0), Sense::click());
    let hover = resp.hovered();
    let (bg, fg) = if strong {
        (p.accent_subtle, p.accent)
    } else {
        (if hover { p.bg_surface3 } else { p.bg_surface2 }, p.text_secondary)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(11), bg);
    if hover {
        ui.painter().rect_stroke(rect, CornerRadius::same(11), Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    }
    let mut x = rect.left() + 8.0;
    if let Some(i) = icon {
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, i, icons::font(12.0), fg);
        x += iw;
    }
    ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
    if caret {
        ui.painter().text(
            pos2(rect.right() - 9.0, rect.center().y),
            Align2::CENTER_CENTER,
            ph::CARET_DOWN,
            icons::font(10.0),
            fg.gamma_multiply(0.8),
        );
    }
    crate::ui::a11y::button(&resp, label);
    resp
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Normal,
    Danger,
    Quiet,
}

/// Compact button used on cards.
pub fn button(ui: &mut Ui, icon: Option<&str>, label: &str, tone: Tone, enabled: bool) -> Response {
    let p = palette(ui.ctx());
    let font = FontId::proportional(12.0);
    let g = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let iw = if icon.is_some() { if label.is_empty() { 4.0 } else { 18.0 } } else { 0.0 };
    let w = g.size().x + 20.0 + iw;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), if enabled { Sense::click() } else { Sense::hover() });
    let hover = enabled && resp.hovered();
    let (bg, fg, stroke) = match tone {
        Tone::Primary => (if hover { p.accent_hover } else { p.accent }, Color32::WHITE, None),
        Tone::Danger => (
            if hover { p.danger.gamma_multiply(0.18) } else { p.danger.gamma_multiply(0.08) },
            p.danger,
            Some(p.danger.gamma_multiply(0.35)),
        ),
        Tone::Normal => (if hover { p.bg_surface3 } else { p.bg_surface2 }, p.text_primary, Some(p.border_subtle)),
        Tone::Quiet => (if hover { p.bg_surface2 } else { Color32::TRANSPARENT }, p.text_secondary, None),
    };
    let (bg, fg) = if enabled { (bg, fg) } else { (p.bg_surface2.gamma_multiply(0.6), p.text_tertiary) };
    ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), bg);
    if let Some(s) = stroke {
        ui.painter().rect_stroke(rect, CornerRadius::same(radius::ROW), Stroke::new(1.0, s), StrokeKind::Inside);
    }
    let mut x = rect.left() + 10.0;
    if let Some(i) = icon {
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, i, icons::font(13.0), fg);
        x += iw;
    }
    ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
    crate::ui::a11y::button(&resp, label);
    resp
}

fn access_icon(a: AiAccess) -> &'static str {
    match a {
        AiAccess::None => ph::LOCK_SIMPLE,
        AiAccess::Metadata => ph::EYE,
        AiAccess::Contents => ph::FILE_TEXT,
    }
}

pub fn access_short(a: AiAccess) -> &'static str {
    match a {
        AiAccess::None => tr!("ai-access-short-none"),
        AiAccess::Metadata => tr!("ai-access-short-metadata"),
        AiAccess::Contents => tr!("ai-access-short-contents"),
    }
}

/// Menu with the three access levels plus "inherit" for `path`.
pub fn access_menu(ui: &mut Ui, path: &std::path::Path, current: AiAccess, explicit: bool, acts: &mut Vec<PanelAct>) {
    ui.set_width(250.0);
    crate::ui::widgets::menu_caption(ui, tr!("ai-access-menu-caption"));
    for a in AiAccess::ALL {
        let label = match a {
            AiAccess::None => tr!("ai-access-menu-none"),
            AiAccess::Metadata => tr!("ai-access-menu-metadata"),
            AiAccess::Contents => tr!("ai-access-menu-contents"),
        };
        let on = explicit && a == current;
        if menu_item(ui, Some(if on { ph::CHECK } else { access_icon(a) }), label, None, true).clicked() {
            acts.push(PanelAct::FolderAccess(path.to_path_buf(), Some(a)));
            ui.close();
        }
    }
    menu_separator(ui);
    if menu_item(ui, Some(if explicit { ph::ARROW_U_UP_LEFT } else { ph::CHECK }), tr!("ai-access-inherit"), None, true)
        .clicked()
    {
        acts.push(PanelAct::FolderAccess(path.to_path_buf(), None));
        ui.close();
    }
}

/// "AI access" row in the properties dialog (folders).
pub fn properties_row(ui: &mut Ui, path: &std::path::Path, cmds: &mut Vec<Cmd>) {
    let p = palette(ui.ctx());
    let (access, explicit) = crate::ai::state::cached_access(path);
    ui.horizontal(|ui| {
        ui.add_sized(
            vec2(110.0, 20.0),
            egui::Label::new(egui::RichText::new(tr!("ai-properties-access")).color(p.text_tertiary).size(12.0)),
        );
        let label = if explicit {
            access_short(access).to_string()
        } else {
            tr!("ai-access-inherited", access = access_short(access))
        };
        let r = chip_menu(ui, Some(access_icon(access)), &label, explicit);
        let mut acts = Vec::new();
        crate::ui::native::popup::Menu::menu(&r).show(|ui| access_menu(ui, path, access, explicit, &mut acts));
        for a in acts {
            if let PanelAct::FolderAccess(path, access) = a {
                cmds.push(Cmd::Ai(AiCmd::SetFolderAccess { path, access }));
            }
        }
    });
}

/// Esc stops a running task while the panel is open (after the core's pumps each frame),
/// in the window showing the panel.
pub fn escape_stops_task(app: &mut SoyuzApp, ctx: &egui::Context) {
    if panel::escape_stops_task(&app.core)
        && crate::models::owners::here(&app.core, crate::models::owners::Panel::Assistant)
        && !app.gui.dialogs.is_open()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        app.core.ai.stop();
    }
}

/// Draw the panel (right dock) when open.
pub fn show(root: &mut Ui, app: &mut SoyuzApp) {
    if !app.core.ai.panel.open {
        return;
    }
    let ctx = root.ctx().clone();
    let p = palette(&ctx);
    let mut acts: Vec<PanelAct> = Vec::new();
    let resp = Panel::right("soyuz.ai")
        .resizable(true)
        .default_size(app.core.ai.panel.width)
        .size_range(320.0..=760.0)
        .show_separator_line(false)
        .frame(Frame::new().fill(p.bg_window).inner_margin(Margin { left: 0, right: 8, top: 2, bottom: 2 }))
        .show(root, |ui| {
            let card = ui.available_rect_before_wrap();
            ui.painter().rect_filled(card, CornerRadius::same(radius::MODAL), p.bg_surface1);
            ui.painter().rect_stroke(
                card,
                CornerRadius::same(radius::MODAL),
                Stroke::new(1.0, p.border_subtle),
                StrokeKind::Inside,
            );
            ui.allocate_rect(card, Sense::hover());
            let inner = card.shrink2(vec2(12.0, 10.0));
            ui.scope_builder(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)), |ui| {
                header(ui, app, &p, &mut acts);
                ui.add_space(6.0);
                chips(ui, app, &p, &mut acts);
                ui.add_space(8.0);
                tabs(ui, app, &p, &mut acts);
                ui.add_space(6.0);
                match app.core.ai.panel.tab {
                    PanelTab::Chat => chat::show(ui, app, &p, &mut acts),
                    PanelTab::Recipes => pages::recipes(ui, app, &p, &mut acts),
                    PanelTab::Audit => pages::audit(ui, app, &p, &mut acts),
                }
            });
        });
    app.core.ai.panel.width = resp.response.rect.width();
    crate::ui::a11y::panel_splitter(root, "soyuz.ai", tr!("app-a11y-resize-assistant"));
    apply(app, &ctx, acts);
}

fn header(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let running = app.core.ai.running.as_ref();
    ui.painter().circle_filled(pos2(r.left() + 13.0, r.center().y), 13.0, p.accent.gamma_multiply(0.14));
    ui.painter().text(
        pos2(r.left() + 13.0, r.center().y),
        Align2::CENTER_CENTER,
        ph::SPARKLE,
        icons::font_fill(15.0),
        p.accent,
    );
    ui.painter().text(
        pos2(r.left() + 34.0, r.center().y - 7.0),
        Align2::LEFT_CENTER,
        tr!("ai-panel-title"),
        semibold(14.0),
        p.text_primary,
    );
    let sub = match running {
        Some(r) if r.recipe => tr!("ai-status-recipe").to_string(),
        Some(r) => {
            let waiting = app.core.ai.chat.iter().any(
                |c| matches!(c, state::ChatItem::Plan(x) if x.task == r.task && x.state == state::CardState::Pending),
            );
            if waiting {
                tr!("ai-status-waiting").into()
            } else {
                tr!("ai-status-working", secs = r.started.elapsed().as_secs())
            }
        }
        None => match app.core.services.mcp.approvals.lock().pending() {
            0 => tr!("ai-status-idle").into(),
            n => tr!("ai-status-external", count = n),
        },
    };
    let sub_w = (r.width() - 34.0 - 100.0).max(40.0);
    let sg = crate::ui::widgets::trunc_galley(ui.painter(), &sub, FontId::proportional(11.0), p.text_tertiary, sub_w);
    ui.painter().galley(pos2(r.left() + 34.0, r.center().y + 1.0), sg, p.text_tertiary);
    if running.is_some() {
        crate::ui::widgets::spinner(ui, pos2(r.left() + 13.0, r.center().y), 15.5, p.accent);
    }
    let mut x = r.right();
    let btn = |ui: &mut Ui, x: &mut f32, glyph: &str, tip: &str, on: bool| -> bool {
        *x -= 30.0;
        let br = Rect::from_min_size(pos2(*x, r.center().y - 15.0), vec2(30.0, 30.0));
        ui.scope_builder(UiBuilder::new().max_rect(br), |ui| icon_toggle(ui, glyph, tip, true, on)).inner.clicked()
    };
    let close = match crate::keys::chord_label(&app.core.keymap, &Action::ToggleAiPanel) {
        Some(k) => tr!("ai-close-key", key = k),
        None => tr!("ai-close").to_string(),
    };
    if btn(ui, &mut x, ph::X, &close, false) {
        acts.push(PanelAct::Close);
    }
    if btn(ui, &mut x, ph::GEAR_SIX, tr!("ai-settings"), false) {
        acts.push(PanelAct::OpenSettings);
    }
    if app.core.ai.running.is_none() && btn(ui, &mut x, ph::NOTE_PENCIL, tr!("ai-new-chat"), false) {
        acts.push(PanelAct::NewChat);
    }
}

fn chips(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        // Profile / model.
        let route = app.core.ai.panel.route;
        let label = match &app.core.ai.panel.profile_override {
            Some(name) => app.core.config.ai.profile(name).map(|p| (p.name.clone(), p.model.clone())),
            None => model::route_label(&app.core.config.ai, route),
        };
        let text = match &label {
            Some((prof, m)) if m.is_empty() => prof.clone(),
            Some((prof, m)) => format!("{prof} \u{00b7} {m}"),
            None => tr!("ai-no-model").into(),
        };
        let r = chip_menu(ui, Some(ph::CPU), &text, false);
        crate::ui::native::popup::Menu::menu(&r).show(|ui| {
            ui.set_width(240.0);
            crate::ui::widgets::menu_caption(ui, tr!("ai-model-caption"));
            let none = app.core.ai.panel.profile_override.is_none();
            if menu_item(ui, Some(if none { ph::CHECK } else { ph::FLOW_ARROW }), tr!("ai-model-auto"), None, true)
                .clicked()
            {
                acts.push(PanelAct::SetProfile(None));
                ui.close();
            }
            for prof in &app.core.config.ai.profiles {
                let on = app.core.ai.panel.profile_override.as_deref() == Some(prof.name.as_str());
                let icon = if on {
                    ph::CHECK
                } else if prof.kind.is_local() {
                    ph::DESKTOP_TOWER
                } else {
                    ph::CLOUD
                };
                if menu_item(ui, Some(icon), &format!("{} \u{00b7} {}", prof.name, prof.model), None, true).clicked() {
                    acts.push(PanelAct::SetProfile(Some(prof.name.clone())));
                    ui.close();
                }
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::GEAR_SIX), tr!("ai-manage-providers"), None, true).clicked() {
                acts.push(PanelAct::OpenSettings);
                ui.close();
            }
        });
        if app.core.ai.panel.profile_override.is_none() {
            let (icon, l) = match route {
                Route::Fast => (ph::LIGHTNING, tr!("ai-route-fast")),
                Route::Strong => (ph::BRAIN, tr!("ai-route-strong")),
            };
            if chip(ui, Some(icon), l, false).on_hover_text(tr!("ai-route-tip")).clicked() {
                acts.push(PanelAct::SetRoute(if route == Route::Fast { Route::Strong } else { Route::Fast }));
            }
        }
        // Folder access of the current folder.
        if let Location::Dir(dir) = &app.core.ws.pane().location {
            let (access, explicit) = state::access_for(&app.core, dir);
            let r = chip_menu(ui, Some(access_icon(access)), access_short(access), access == AiAccess::Contents)
                .on_hover_text(if explicit {
                    tr!("ai-access-tip", name = soyuz_core::entry::display_name(dir))
                } else {
                    tr!("ai-access-tip-inherited", name = soyuz_core::entry::display_name(dir))
                });
            let dir = dir.clone();
            crate::ui::native::popup::Menu::menu(&r).show(|ui| access_menu(ui, &dir, access, explicit, acts));
        }
        // Meter.
        let (usage, cost) = match &app.core.ai.running {
            Some(r) => (r.usage, r.cost),
            None => (app.core.ai.session_usage, (app.core.ai.session_cost > 0.0).then_some(app.core.ai.session_cost)),
        };
        let month = label.as_ref().and_then(|(prof, _)| {
            let l = app.core.ai.month.as_ref()?;
            let s = l.spend(&soyuz_ai::pricing::current_month(), prof);
            let cap = app.core.config.ai.profile(prof).map_or(0.0, |p| p.monthly_budget_usd);
            Some((s.cost_usd, cap))
        });
        let meter = match cost {
            Some(c) => tr!(
                "ai-meter-tokens-cost",
                tokens = tokens(usage.total()),
                cost = soyuz_core::locale::format_decimal(c, 3)
            ),
            None => tr!("ai-meter-tokens", tokens = tokens(usage.total())),
        };
        let warn = month.is_some_and(|(spent, cap)| {
            cap > 0.0 && spent / cap * 100.0 >= f64::from(app.core.config.ai.budget_warn_percent)
        });
        let r = chip(ui, Some(ph::COINS), &meter, false);
        let tip = match month {
            Some((spent, cap)) if cap > 0.0 => tr!(
                "ai-meter-tip-capped",
                spent = soyuz_core::locale::format_decimal(spent, 2),
                cap = soyuz_core::locale::format_decimal(cap, 2),
                percent = soyuz_core::locale::format_decimal(spent / cap * 100.0, 0)
            ),
            Some((spent, _)) => tr!("ai-meter-tip-month", spent = soyuz_core::locale::format_decimal(spent, 2)),
            None => tr!("ai-meter-tip").into(),
        };
        let r = r.on_hover_text(tip);
        if warn {
            ui.painter().circle_filled(pos2(r.rect.right() - 4.0, r.rect.top() + 4.0), 3.5, p.warning);
        }
    });
}

fn tabs(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, acts: &mut Vec<PanelAct>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (tab, icon, label) in [
            (PanelTab::Chat, ph::CHAT_CIRCLE_DOTS, tr!("ai-tab-chat")),
            (PanelTab::Recipes, ph::BOOK_OPEN, tr!("ai-tab-recipes")),
            (PanelTab::Audit, ph::SCROLL, tr!("ai-tab-audit")),
        ] {
            let on = app.core.ai.panel.tab == tab;
            let g = ui.painter().layout_no_wrap(label.into(), semibold(12.0), Color32::PLACEHOLDER);
            let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 34.0, 28.0), Sense::click());
            if on {
                ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface3);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface2);
            }
            let c = if on { p.text_primary } else { p.text_secondary };
            ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, icon, icons::font(13.0), c);
            ui.painter().galley(pos2(r.left() + 27.0, r.center().y - g.size().y / 2.0), g, c);
            crate::ui::a11y::toggle(&resp, label, on);
            if resp.clicked() {
                acts.push(PanelAct::Tab(tab));
            }
        }
        if let Some(r) = &app.core.ai.running {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let _ = r;
                if button(ui, Some(ph::STOP_CIRCLE), tr!("ai-stop-esc"), Tone::Danger, true).clicked() {
                    acts.push(PanelAct::Stop);
                }
            });
        }
    });
}

/// Apply panel actions (after drawing): the core applies them; "settings" opens the GUI
/// settings window right away (the core would queue `FrontendCmd::OpenSettings`).
pub fn apply(app: &mut SoyuzApp, ctx: &egui::Context, acts: Vec<PanelAct>) {
    let host = crate::host::EguiHost::new(ctx);
    for a in acts {
        match a {
            PanelAct::OpenSettings => crate::ui::settings::open(app, Some(crate::ui::settings::Section::Ai)),
            a => panel::apply(&mut app.core, &host, vec![a]),
        }
    }
}

#[cfg(test)]
mod tests {
    use soyuz_core::Locale;

    use crate::i18n::{FluentArgs, t_args_in, t_in};

    #[test]
    fn panel_messages_render_in_both_languages() {
        let mut a = FluentArgs::new();
        a.set("count", 12);
        a.set("risk", "read");
        assert_eq!(
            t_args_in("ai-tools-builtin", &a, Locale::EnUs),
            "12 built-in tools (files, search, media, jobs, UI) \u{00b7} up to Read risk"
        );
        assert!(t_args_in("ai-tools-builtin", &a, Locale::TrTr).ends_with("risk sınırı: okuma"));
        let mut a = FluentArgs::new();
        a.set("spent", "1.50");
        a.set("cap", "10.00");
        a.set("percent", "15");
        assert_eq!(
            t_args_in("ai-meter-tip-capped", &a, Locale::EnUs),
            "Tokens and cost of this task / session\nThis month: $1.50 of $10.00 (15 %)"
        );
        let mut a = FluentArgs::new();
        a.set("count", 1);
        assert_eq!(t_args_in("ai-status-external", &a, Locale::EnUs), "1 external request waits for you");
        assert_eq!(t_args_in("ai-status-external", &a, Locale::TrTr), "1 dış istek onayınızı bekliyor");
        assert_eq!(t_in("ai-tab-audit", Locale::TrTr), "Denetim günlüğü");
    }
}

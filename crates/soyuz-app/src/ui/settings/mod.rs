//! Settings window (Ctrl+,): a large modal with a sectioned sidebar covering every
//! `config.toml` section. Edits apply live (theme, keys, views …) and are written back
//! to `config.toml` shortly after the last change; external edits to the file (hot
//! reload) refresh the window. Search filters rows across all sections.

mod ai;
mod ai_connections;
mod appearance;
mod density_gallery;
mod editor;
mod fonts;
mod keybindings;
mod lists;
mod pages;
mod system;
mod themes_ui;
pub mod widgets;

use std::path::PathBuf;
use std::time::Duration;

use egui::emath::TSTransform;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Key, LayerId, Layout, Margin, Modal, Modifiers, Order,
    Rect, RichText, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use soyuz_core::config::{self, Config, Keymap};

use crate::app::SoyuzApp;
use crate::theme::icons::ph;
use crate::theme::{Palette, icons, palette};
use crate::ui::dialogs::keybind_record::{RecordOutcome, RecordTarget, Recorder};
use crate::ui::toasts::Toast;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickPurpose {
    Export,
    Import,
    ThemeExport,
    ThemeImport,
}

/// Delay between the last edit and writing `config.toml`.
const NAV_W: f32 = 236.0;

pub use crate::models::settings::Section;

/// Icon of a settings section.
pub fn section_icon(sec: Section) -> &'static str {
    match sec {
        Section::General => ph::GEAR_SIX,
        Section::Appearance => ph::PALETTE,
        Section::Fonts => ph::TEXT_AA,
        Section::Views => ph::LAYOUT,
        Section::Behaviour => ph::CURSOR_CLICK,
        Section::Jobs => ph::COPY,
        Section::Terminal => ph::TERMINAL_WINDOW,
        Section::TerminalUi => ph::TERMINAL,
        Section::Preview => ph::EYE,
        Section::Editor => ph::NOTE_PENCIL,
        Section::Keyboard => ph::KEYBOARD,
        Section::Commands => ph::LIGHTNING,
        Section::Bookmarks => ph::BOOKMARK_SIMPLE,
        Section::Scripts => ph::CODE,
        Section::Automations => ph::ROBOT,
        Section::Ai => ph::SPARKLE,
        Section::Advanced => ph::FILE_CODE,
        Section::About => ph::PLANET,
    }
}

/// Effective key map of the draft, rebuilt only when key-related config changes.
struct KeyCache {
    keys: config::Keys,
    bookmarks: Vec<config::Bookmark>,
    commands: Vec<config::CustomCommand>,
    keymap: Keymap,
    warnings: Vec<String>,
    conflicts: Vec<crate::keys::conflicts::Conflict>,
}

pub struct SettingsState {
    open: bool,
    pub section: Section,
    pub(crate) draft: Config,
    applied: Config,
    query: String,
    opened_at: f64,
    focus_search: bool,
    recorder: Option<Recorder>,
    key_cache: Option<KeyCache>,
    // Page-local UI state.
    keys_filter: String,
    keys_conflicts_only: bool,
    keys_modified_only: bool,
    /// Export / import file picker (drawn on top of the settings window).
    picker: Option<(PickPurpose, crate::ui::path_picker::PathPicker)>,
    scale_temp: Option<f32>,
    args_text: Vec<String>,
    scroll_top: bool,
    /// AI page state (connection tests, credential operations, usage ledger).
    ai: ai::AiPageState,
    /// AI → Connections (MCP) page state.
    connections: ai_connections::ConnectionsState,
}

impl Default for SettingsState {
    fn default() -> Self {
        SettingsState {
            open: false,
            section: Section::General,
            draft: Config::default(),
            applied: Config::default(),
            query: String::new(),
            opened_at: 0.0,
            focus_search: false,
            recorder: None,
            key_cache: None,
            keys_filter: String::new(),
            keys_conflicts_only: false,
            keys_modified_only: false,
            picker: None,
            scale_temp: None,
            args_text: Vec::new(),
            scroll_top: false,
            ai: ai::AiPageState::default(),
            connections: Default::default(),
        }
    }
}

impl SettingsState {
    pub fn is_open(&self) -> bool {
        self.open
    }
}

/// Open the settings (optionally at a section).
pub fn open(app: &mut SoyuzApp, section: Option<Section>) {
    let s = &mut app.gui.settings;
    if !s.open {
        s.draft = app.core.config.clone();
        s.applied = app.core.config.clone();
        s.opened_at = f64::NAN;
        s.query.clear();
        s.focus_search = false;
        s.args_text.clear();
    }
    s.open = true;
    app.core.ui.settings_open = true;
    if let Some(sec) = section {
        s.section = sec;
        s.scroll_top = true;
    }
}

pub fn close(app: &mut SoyuzApp) {
    app.gui.settings.open = false;
    app.core.ui.settings_open = false;
    app.gui.settings.recorder = None;
    flush_save(app);
}

fn flush_save(app: &mut SoyuzApp) {
    crate::models::settings::flush(&mut app.core);
}

fn key_cache(s: &mut SettingsState) -> &KeyCache {
    let stale = match &s.key_cache {
        Some(k) => k.keys != s.draft.keys || k.bookmarks != s.draft.bookmarks || k.commands != s.draft.custom_commands,
        None => true,
    };
    if stale {
        let (keymap, warnings) = config::keymap(&s.draft);
        let conflicts = crate::keys::conflicts::analyze(&keymap);
        s.key_cache = Some(KeyCache {
            keys: s.draft.keys.clone(),
            bookmarks: s.draft.bookmarks.clone(),
            commands: s.draft.custom_commands.clone(),
            keymap,
            warnings,
            conflicts,
        });
    }
    s.key_cache.as_ref().expect("just built")
}

/// Draw the window when open; applies and saves edits.
pub fn show(ctx: &egui::Context, app: &mut SoyuzApp) {
    if !app.gui.settings.open {
        pump_save(ctx, app);
        return;
    }
    // External change (hot reload) while open: start over from the new config.
    if app.core.config != app.gui.settings.applied {
        app.gui.settings.draft = app.core.config.clone();
        app.gui.settings.applied = app.core.config.clone();
    }
    let p = palette(ctx);
    let now = ctx.input(|i| i.time);
    if app.gui.settings.opened_at.is_nan() {
        app.gui.settings.opened_at = now;
    }
    let t = if app.core.ui.reduced_motion {
        1.0
    } else {
        ((now - app.gui.settings.opened_at) / 0.18).clamp(0.0, 1.0) as f32
    };
    let ease = 1.0 - (1.0 - t).powi(3);
    if t < 1.0 {
        ctx.request_repaint();
    }

    if crate::ui::native::active(ctx) {
        show_native(ctx, app, &p);
        return;
    }
    if window_keys(ctx, app) {
        return;
    }
    in_window(ctx, app, &p, ease);
}

/// Recorder / window keys (Esc closes, Ctrl+F searches, Ctrl+PgUp/PgDn switch
/// sections). Returns `true` when the window was closed.
fn window_keys(ctx: &egui::Context, app: &mut SoyuzApp) -> bool {
    // Recorder on top (owns the keyboard while open).
    if app.gui.settings.recorder.is_some() {
        let cmd_is_ctrl = app.gui.settings.draft.keys.cmd_is_ctrl;
        let km = key_cache(&mut app.gui.settings).keymap.clone();
        let outcome = app.gui.settings.recorder.as_mut().map(|r| r.show(ctx, &km, cmd_is_ctrl));
        match outcome {
            Some(RecordOutcome::Save(binding)) => {
                let target = app.gui.settings.recorder.take().map(|r| r.target);
                if let Some(target) = target {
                    apply_recorded(&mut app.gui.settings, target, binding);
                }
            }
            Some(RecordOutcome::Cancel) => app.gui.settings.recorder = None,
            _ => {}
        }
    } else if app.gui.settings.picker.is_none() && !crate::ui::widgets::dropdown::any_open(ctx) {
        // Window keys: Esc closes, Ctrl+F searches, Ctrl+PgUp/PgDn switch sections.
        let (esc, find, prev, next) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::COMMAND, Key::F),
                i.consume_key(Modifiers::COMMAND, Key::PageUp) || i.consume_key(Modifiers::ALT, Key::ArrowUp),
                i.consume_key(Modifiers::COMMAND, Key::PageDown) || i.consume_key(Modifiers::ALT, Key::ArrowDown),
            )
        });
        if esc {
            if app.gui.settings.query.is_empty() {
                close(app);
                return true;
            }
            app.gui.settings.query.clear();
        }
        if find {
            app.gui.settings.focus_search = true;
        }
        if prev || next {
            let i = Section::ALL.iter().position(|s| *s == app.gui.settings.section).unwrap_or(0);
            let n = Section::ALL.len();
            app.gui.settings.section = Section::ALL[if next { (i + 1) % n } else { (i + n - 1) % n }];
            app.gui.settings.scroll_top = true;
        }
    }
    false
}

/// The settings window as its own OS window (`[appearance] native_dialogs`).
fn show_native(ctx: &egui::Context, app: &mut SoyuzApp, p: &Palette) {
    let mut w = crate::ui::native::dialog::DialogWindow::new(
        Id::new("soyuz.settings.native"),
        Id::new("soyuz.settings"),
        tr!("settings-title"),
    );
    w.icon = Some(ph::GEAR_SIX);
    w.resizable = Some(vec2(1000.0, 700.0 + crate::ui::native::dialog::TITLE_H));
    w.margin = 0.0;
    w.fill = p.bg_surface1;
    let shown = w.show(ctx, |ui| {
        let ctx = ui.ctx().clone();
        if window_keys(&ctx, app) {
            return true;
        }
        let rect = ui.max_rect();
        let close_now = body(ui, app, p, rect, Id::new("soyuz.settings"), false);
        show_picker(&ctx, app);
        close_now
    });
    if shown.close || shown.inner == Some(true) {
        if app.gui.settings.open {
            close(app);
        }
        return;
    }
    apply_and_save(ctx, app);
}

/// Live apply + debounced save.
fn apply_and_save(ctx: &egui::Context, app: &mut SoyuzApp) {
    if app.gui.settings.draft != app.gui.settings.applied {
        let draft = app.gui.settings.draft.clone();
        crate::services::config_watch::apply_config(&mut app.core, &crate::host::EguiHost::new(ctx), draft.clone());
        app.gui.settings.applied = draft;
        app.core.config_save.schedule();
    }
    pump_save(ctx, app);
}

/// Navigation column, content and (with `own_close`) the close button inside `rect`.
/// Returns `true` to close.
fn body(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette, rect: Rect, id: Id, own_close: bool) -> bool {
    let mut close_now = false;
    let nav = Rect::from_min_size(rect.min, vec2(NAV_W, rect.height()));
    let content = Rect::from_min_max(pos2(nav.right(), rect.top()), rect.max);
    let r = if own_close { 14 } else { 0 };
    ui.painter().rect_filled(
        nav,
        CornerRadius { nw: r, sw: if own_close { 14 } else { 8 }, ne: 0, se: 0 },
        p.bg_window,
    );
    ui.painter().line_segment(
        [pos2(nav.right(), nav.top()), pos2(nav.right(), nav.bottom())],
        Stroke::new(1.0, p.border_subtle),
    );
    let mut nav_ui =
        ui.new_child(UiBuilder::new().max_rect(nav.shrink2(vec2(12.0, 14.0))).layout(Layout::top_down(Align::Min)));
    if sidebar(&mut nav_ui, app, p) {
        close_now = true;
    }
    let mut c_ui =
        ui.new_child(UiBuilder::new().max_rect(content.shrink2(vec2(28.0, 18.0))).layout(Layout::top_down(Align::Min)));
    content_area(&mut c_ui, app, p);
    if own_close {
        let cr = Rect::from_center_size(pos2(rect.right() - 22.0, rect.top() + 22.0), vec2(26.0, 26.0));
        let cresp = ui.interact(cr, id.with("close"), Sense::click()).on_hover_text(tr!("settings-close-tooltip"));
        crate::ui::a11y::button(&cresp, tr!("settings-close"));
        if cresp.has_focus() {
            crate::ui::widgets::focus_ring(ui, cr);
        }
        if cresp.hovered() {
            ui.painter().rect_filled(cr, CornerRadius::same(6), p.bg_surface3);
        }
        ui.painter().text(cr.center(), Align2::CENTER_CENTER, ph::X, icons::font(14.0), p.text_secondary);
        if cresp.clicked() {
            close_now = true;
        }
    }
    close_now
}

/// The settings window drawn inside the app window (an egui modal).
fn in_window(ctx: &egui::Context, app: &mut SoyuzApp, p: &Palette, ease: f32) {
    let screen = ctx.content_rect();
    let size = vec2((screen.width() - 48.0).clamp(560.0, 1000.0), (screen.height() - 48.0).clamp(400.0, 700.0));
    let id = Id::new("soyuz.settings");
    let frame = Frame::new()
        .fill(p.bg_surface1)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::ZERO)
        .shadow(egui::epaint::Shadow {
            offset: [0, 20],
            blur: 60,
            spread: 0,
            color: Color32::from_black_alpha((p.shadow_alpha * 255.0 * 1.2).min(255.0) as u8),
        });
    let mut close_now = false;
    let resp = Modal::new(id)
        .frame(frame)
        .backdrop_color(Color32::from_black_alpha(((if p.dark { 130.0 } else { 80.0 }) * ease) as u8))
        .show(ctx, |ui| {
            ui.set_opacity(ease);
            crate::ui::a11y::container(ui.ctx(), ui.unique_id(), egui::accesskit::Role::Dialog, tr!("settings-title"));
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            close_now = body(ui, app, p, rect, id, true);
        });
    let s = 0.98 + 0.02 * ease;
    let c = screen.center();
    ctx.set_transform_layer(LayerId::new(Order::Foreground, id), TSTransform::new(c.to_vec2() * (1.0 - s), s));
    if (resp.backdrop_response.clicked() && app.gui.settings.recorder.is_none() && app.gui.settings.picker.is_none())
        || close_now
    {
        close(app);
        return;
    }
    show_picker(ctx, app);
    apply_and_save(ctx, app);
}

fn pump_save(ctx: &egui::Context, app: &mut SoyuzApp) {
    crate::models::settings::pump(&mut app.core, &crate::host::EguiHost::new(ctx));
}

pub use crate::models::settings::update_config;

fn apply_recorded(s: &mut SettingsState, target: RecordTarget, binding: String) {
    match target {
        RecordTarget::Action(a) => {
            let km = &key_cache(s).keymap;
            let mut list = keybindings::current_bindings(km, &a);
            if !list.contains(&binding) {
                list.push(binding);
            }
            keybindings::set_override(&mut s.draft, &a, list);
        }
        RecordTarget::Bookmark(i) => {
            if let Some(b) = s.draft.bookmarks.get_mut(i) {
                b.hotkey = binding;
            }
        }
        RecordTarget::Command(i) => {
            if let Some(c) = s.draft.custom_commands.get_mut(i) {
                c.hotkey = binding;
            }
        }
    }
}

/// Left column: search, sections, file actions. Returns `true` to close.
fn sidebar(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette) -> bool {
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        crate::ui::widgets::icon_label(ui, icons::icon(ph::GEAR_SIX, 18.0).color(p.accent));
        ui.label(RichText::new(tr!("settings-title")).font(crate::theme::fonts::heading(17.0)).color(p.text_primary));
    });
    ui.add_space(12.0);
    let s = &mut app.gui.settings;
    let search_id = Id::new("soyuz.settings.search");
    // Typing anywhere (with nothing focused) starts a search.
    let typing = ui.ctx().memory(|m| m.focused().is_none())
        && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(t) if !t.trim().is_empty())));
    if s.focus_search || typing {
        ui.ctx().memory_mut(|m| m.request_focus(search_id));
        s.focus_search = false;
    }
    let search = ui.add(
        TextEdit::singleline(&mut s.query)
            .id(search_id)
            .hint_text(
                RichText::new(format!("{}  {}", ph::MAGNIFYING_GLASS, tr!("settings-search-hint")))
                    .color(p.text_tertiary),
            )
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(10, 7)),
    );
    crate::ui::a11y::name(&search, tr!("settings-search-hint"));
    ui.add_space(10.0);
    let searching = !s.query.trim().is_empty();
    // The section list scrolls when the window is too short for it, so it never runs
    // under the footer (status + "Edit config.toml").
    let footer_h = 64.0;
    egui::ScrollArea::vertical()
        .id_salt("soyuz.settings.nav")
        .max_height((ui.available_height() - footer_h).max(64.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for sec in Section::ALL {
                let active = !searching && s.section == sec;
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                crate::ui::a11y::tab(&resp, sec.title(), active);
                if resp.has_focus() {
                    crate::ui::widgets::focus_ring(ui, r);
                }
                let hover = ui.ctx().animate_bool_with_time(
                    resp.id.with("h"),
                    resp.hovered(),
                    crate::theme::anim(ui.ctx(), 0.1),
                );
                if active {
                    ui.painter().rect_filled(r, CornerRadius::same(7), p.accent_subtle);
                    ui.painter().rect_filled(
                        Rect::from_min_size(pos2(r.left(), r.center().y - 8.0), vec2(3.0, 16.0)),
                        CornerRadius::same(2),
                        p.accent,
                    );
                } else if hover > 0.0 {
                    ui.painter().rect_filled(r, CornerRadius::same(7), p.bg_surface3.gamma_multiply(hover));
                }
                let fg = if active { p.text_primary } else { p.text_secondary };
                let glyph_font = if active { icons::font_fill(15.0) } else { icons::font(15.0) };
                ui.painter().text(
                    pos2(r.left() + 14.0, r.center().y),
                    Align2::LEFT_CENTER,
                    section_icon(sec),
                    glyph_font,
                    if active { p.accent } else { p.text_tertiary },
                );
                ui.painter().text(
                    pos2(r.left() + 38.0, r.center().y),
                    Align2::LEFT_CENTER,
                    sec.title(),
                    FontId::proportional(13.0),
                    fg,
                );
                if sec == Section::Keyboard {
                    let n = key_cache(s)
                        .conflicts
                        .iter()
                        .filter(|c| c.kind != crate::keys::conflicts::ConflictKind::Shadowed)
                        .count();
                    if n > 0 {
                        let badge = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(20.0, 16.0));
                        ui.painter().rect_filled(badge, CornerRadius::same(8), p.danger.gamma_multiply(0.85));
                        ui.painter().text(
                            badge.center(),
                            Align2::CENTER_CENTER,
                            n.to_string(),
                            FontId::proportional(10.0),
                            Color32::WHITE,
                        );
                    }
                }
                if resp.clicked() {
                    s.section = sec;
                    s.query.clear();
                    s.scroll_top = true;
                }
            }
        });
    // Bottom: save status + config file shortcuts.
    let mut close = false;
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        let status = if app.core.config_save.due.is_some() {
            (ph::CIRCLE_NOTCH, tr!("settings-status-saving"), p.text_tertiary)
        } else if app.core.config_save.flashed.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
            (ph::CHECK_CIRCLE, tr!("settings-status-saved"), p.success)
        } else {
            (ph::CHECK, tr!("settings-status-live"), p.text_tertiary)
        };
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            crate::ui::widgets::icon_label(ui, icons::icon(status.0, 12.0).color(status.2));
            ui.label(RichText::new(status.1).size(11.0).color(status.2));
        });
        ui.add_space(6.0);
        if nav_link(ui, p, ph::FILE_CODE, tr!("settings-edit-config")) {
            app.core.cmds.push(crate::cmd::Cmd::Action(soyuz_core::config::Action::EditConfigFile));
            close = true;
        }
    });
    close
}

fn nav_link(ui: &mut Ui, p: &Palette, glyph: &str, label: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    crate::ui::a11y::button(&resp, label);
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(6), p.bg_surface3);
    }
    if resp.has_focus() {
        crate::ui::widgets::focus_ring(ui, r);
    }
    let fg = if resp.hovered() { p.accent } else { p.text_secondary };
    ui.painter().text(pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, glyph, icons::font(13.0), fg);
    ui.painter().text(pos2(r.left() + 36.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(12.0), fg);
    resp.clicked()
}

fn content_area(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette) {
    let searching = !app.gui.settings.query.trim().is_empty();
    let sec = app.gui.settings.section;
    // Header.
    ui.horizontal(|ui| {
        if searching {
            ui.label(
                RichText::new(tr!("settings-search-results", query = app.gui.settings.query.trim()))
                    .font(crate::theme::fonts::heading(19.0))
                    .color(p.text_primary),
            );
        } else {
            crate::ui::widgets::icon_label(ui, icons::icon(section_icon(sec), 20.0).color(p.accent));
            ui.label(RichText::new(sec.title()).font(crate::theme::fonts::heading(19.0)).color(p.text_primary));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(30.0);
                if sec != Section::Advanced
                    && widgets::small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("settings-reset-section"), true)
                        .on_hover_text(tr!("settings-reset-section-tooltip"))
                        .clicked()
                {
                    sec.reset(&mut app.gui.settings.draft);
                    app.core.ui.toasts.push(Toast::info(tr!("settings-reset-section-done", section = sec.title())));
                }
            });
        }
    });
    if !searching {
        ui.label(RichText::new(sec.description()).size(12.0).color(p.text_tertiary));
    }
    ui.add_space(4.0);
    let scroll_top = std::mem::take(&mut app.gui.settings.scroll_top);
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("soyuz.settings.scroll", sec as u8, searching))
        .auto_shrink([false, false]);
    if scroll_top {
        area = area.vertical_scroll_offset(0.0);
    }
    area.show(ui, |ui| {
        ui.set_width(ui.available_width() - 6.0);
        let q = app.gui.settings.query.trim().to_string();
        if searching {
            let mut any = false;
            for s in Section::ALL {
                any |= page(ui, app, s, &q);
            }
            if !any {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    crate::ui::widgets::icon_label(ui, icons::icon(ph::MAGNIFYING_GLASS, 28.0).color(p.text_tertiary));
                    ui.label(RichText::new(tr!("settings-search-empty")).color(p.text_secondary));
                });
            }
        } else {
            page(ui, app, sec, "");
        }
        ui.add_space(24.0);
    });
}

/// Draw one section's groups (filtered by `q`); returns whether anything was drawn.
fn page(ui: &mut Ui, app: &mut SoyuzApp, sec: Section, q: &str) -> bool {
    match sec {
        Section::General => system::file_manager(ui, app, q) | pages::general(ui, app, q),
        Section::Appearance => appearance::page(ui, app, q),
        Section::Fonts => fonts::page(ui, app, q),
        Section::Views => pages::views(ui, app, q),
        Section::Behaviour => pages::behaviour(ui, app, q),
        Section::Jobs => pages::jobs(ui, app, q),
        Section::Terminal => pages::terminal(ui, app, q),
        Section::TerminalUi => pages::terminal_ui(ui, app, q),
        Section::Preview => pages::preview(ui, app, q),
        Section::Editor => editor::editor(ui, app, q),
        Section::Keyboard => keybindings::page(ui, app, q),
        Section::Commands => lists::commands(ui, app, q),
        Section::Bookmarks => lists::bookmarks(ui, app, q),
        Section::Scripts => crate::ui::automation::scripts_page::page(ui, app, q),
        Section::Automations => crate::ui::automation::rules_page::page(ui, app, q),
        Section::Ai => ai::page(ui, app, q),
        Section::Advanced => pages::advanced(ui, app, q),
        Section::About => system::about(ui, app, q),
    }
}

/// Start recording a shortcut.
fn record(app: &mut SoyuzApp, target: RecordTarget, title: String) {
    app.gui.settings.recorder = Some(Recorder::new(target, title));
}

/// First free `soyuz-config.toml`, `soyuz-config (2).toml`, … in `dir` (an export never
/// overwrites an earlier one).
fn export_path(dir: &std::path::Path) -> PathBuf {
    (1..1000)
        .map(|n| dir.join(if n == 1 { "soyuz-config.toml".to_string() } else { format!("soyuz-config ({n}).toml") }))
        .find(|p| std::fs::symlink_metadata(p).is_err())
        .unwrap_or_else(|| dir.join(format!("soyuz-config-{}.toml", chrono::Local::now().format("%Y%m%d-%H%M%S"))))
}

/// Default folder for export / import: Downloads, else home.
fn transfer_dir() -> PathBuf {
    let dir = soyuz_fs::home().join("Downloads");
    if dir.is_dir() { dir } else { soyuz_fs::home() }
}

/// Open the in-app save picker for exporting the configuration.
pub fn start_export(app: &mut SoyuzApp) {
    use crate::ui::path_picker::{PathPicker, PickMode};
    let dir = transfer_dir();
    let name = export_path(&dir).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let picker = PathPicker::new(tr!("settings-export-title"), PickMode::Save, dir, &name, &["toml"]);
    app.gui.settings.picker = Some((PickPurpose::Export, picker));
}

/// Open the in-app picker for importing a configuration file.
fn start_import(app: &mut SoyuzApp) {
    use crate::ui::path_picker::{PathPicker, PickMode};
    let picker = PathPicker::new(tr!("settings-import-title"), PickMode::Open, transfer_dir(), "", &["toml"]);
    app.gui.settings.picker = Some((PickPurpose::Import, picker));
}

/// Open the picker for importing a theme file (copied into the themes folder).
fn start_theme_import(app: &mut SoyuzApp) {
    use crate::ui::path_picker::{PathPicker, PickMode};
    let picker = PathPicker::new(tr!("settings-theme-import-title"), PickMode::Open, transfer_dir(), "", &["toml"]);
    app.gui.settings.picker = Some((PickPurpose::ThemeImport, picker));
}

/// Open the save picker for exporting the current colors as a theme file.
fn start_theme_export(app: &mut SoyuzApp) {
    use crate::ui::path_picker::{PathPicker, PickMode};
    let dir = crate::theme::library::themes_dir();
    let _ = std::fs::create_dir_all(&dir);
    let base = crate::theme::themes::slug(&app.gui.settings.draft.appearance.color_theme);
    let name = format!("{base}-custom.toml");
    let picker = PathPicker::new(tr!("settings-theme-export-title"), PickMode::Save, dir, &name, &["toml"]);
    app.gui.settings.picker = Some((PickPurpose::ThemeExport, picker));
}

/// Write the draft's resolved palettes (theme + accent + overrides) as a theme file.
fn export_theme(app: &mut SoyuzApp, path: &std::path::Path) {
    let d = &app.gui.settings.draft;
    let (mut dark, mut light, _) = crate::theme::resolve(&d.appearance);
    let _ = dark.apply_overrides(&d.theme.colors);
    let _ = light.apply_overrides(&d.theme.colors);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "custom".into());
    let def = crate::theme::themes::ThemeDef::from_palettes(&stem, &stem, &dark, &light);
    match soyuz_fs::write_atomic(path, def.to_toml().as_bytes()) {
        Ok(()) => {
            app.core.ui.toasts.push(Toast::success(tr!("settings-theme-exported")).body(path.display().to_string()));
        }
        Err(e) => {
            app.core.ui.toasts.push(Toast::error(tr!("settings-theme-export-failed")).body(e.to_string()));
        }
    }
}

/// Validate a theme file, copy it into the themes folder and select it.
fn import_theme(app: &mut SoyuzApp, path: &std::path::Path) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            app.core.ui.toasts.push(Toast::error(tr!("settings-theme-import-failed")).body(e.to_string()));
            return;
        }
    };
    match crate::theme::library::import(&text, &stem) {
        Ok((t, warnings)) => {
            app.gui.settings.draft.appearance.color_theme = t.id.clone();
            app.gui.settings.draft.appearance.high_contrast = false;
            let toast = if warnings.is_empty() {
                Toast::success(tr!("settings-theme-imported", name = &t.name))
            } else {
                Toast::warning(tr!("settings-theme-imported-warnings", name = &t.name)).body(warnings.join("\n"))
            };
            app.core.ui.toasts.push(toast);
        }
        Err(e) => {
            app.core.ui.toasts.push(Toast::error(tr!("settings-theme-invalid")).body(e));
        }
    }
}

/// Draw the export / import picker; act on its result.
fn show_picker(ctx: &egui::Context, app: &mut SoyuzApp) {
    use crate::ui::path_picker::PickResult;
    let Some((purpose, picker)) = app.gui.settings.picker.as_mut() else { return };
    let purpose = *purpose;
    match picker.show(ctx) {
        PickResult::Pending => {}
        PickResult::Cancelled => {
            log::debug!("export / import picker cancelled");
            app.gui.settings.picker = None
        }
        PickResult::Picked(path) => {
            app.gui.settings.picker = None;
            match purpose {
                PickPurpose::Export => {
                    export(app, path);
                }
                PickPurpose::Import => import(app, &path),
                PickPurpose::ThemeExport => export_theme(app, &path),
                PickPurpose::ThemeImport => import_theme(app, &path),
            }
        }
    }
}

/// Write the active config to `path` (keeps that file's comments when it exists).
fn export(app: &mut SoyuzApp, path: PathBuf) -> Option<PathBuf> {
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let text = match config::to_string_preserving(&existing, &app.core.config) {
        Ok(t) => t,
        Err(e) => {
            app.core.ui.toasts.push(Toast::error(tr!("settings-export-failed")).body(e.to_string()));
            return None;
        }
    };
    match soyuz_fs::write_atomic(&path, text.as_bytes()) {
        Ok(()) => {
            app.core.ui.toasts.push(Toast::success(tr!("settings-exported")).body(path.display().to_string()));
            Some(path)
        }
        Err(e) => {
            app.core.ui.toasts.push(Toast::error(tr!("settings-export-failed")).body(e.to_string()));
            None
        }
    }
}

/// Import a config file into the draft (applies live, saved shortly after).
fn import(app: &mut SoyuzApp, path: &std::path::Path) {
    match config::load(path) {
        Ok(loaded) => {
            app.gui.settings.draft = loaded.config;
            let mut t = Toast::success(tr!("settings-imported"));
            if !loaded.warnings.is_empty() {
                t = Toast::warning(tr!("settings-imported-warnings")).body(loaded.warnings.join("\n"));
            }
            app.core.ui.toasts.push(t);
        }
        Err(e) => {
            app.core.ui.toasts.push(Toast::error(tr!("settings-import-failed")).body(e.to_string()));
        }
    }
}

/// A pill showing a chord binding with a hover "×"; returns (clicked, remove clicked).
fn binding_chip(ui: &mut Ui, p: &Palette, binding: &str, conflict: bool, removable: bool) -> (bool, bool) {
    let w = crate::ui::palette::widgets::chips_width(ui, binding) + if removable { 18.0 } else { 6.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    let hover = resp.hovered();
    let fill = if conflict {
        p.danger.gamma_multiply(0.18)
    } else if hover {
        p.bg_surface3
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    if conflict {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(7),
            Stroke::new(1.0, p.danger.gamma_multiply(0.7)),
            StrokeKind::Inside,
        );
    }
    let right = if removable { rect.right() - 16.0 } else { rect.right() - 3.0 };
    crate::ui::palette::widgets::paint_chips(ui, p, binding, right, rect.center().y, false);
    let mut removed = false;
    if removable {
        let xr = Rect::from_center_size(pos2(rect.right() - 9.0, rect.center().y), vec2(14.0, 14.0));
        let xresp = ui.interact(xr, resp.id.with("x"), Sense::click());
        let c = if xresp.hovered() {
            p.danger
        } else if hover {
            p.text_secondary
        } else {
            p.text_tertiary.gamma_multiply(0.6)
        };
        ui.painter().text(xr.center(), Align2::CENTER_CENTER, ph::X, icons::font(10.0), c);
        removed = xresp.clicked();
    }
    (resp.clicked() && !removed, removed)
}

#[cfg(test)]
mod export_tests {
    #[test]
    fn export_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let first = super::export_path(dir.path());
        assert_eq!(first, dir.path().join("soyuz-config.toml"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(super::export_path(dir.path()), dir.path().join("soyuz-config (2).toml"));
    }

    #[test]
    fn every_section_is_translated() {
        for s in super::Section::ALL {
            for l in [soyuz_core::Locale::EnUs, soyuz_core::Locale::TrTr] {
                let k = format!("settings-section-{}", s.id());
                assert!(crate::i18n::has_in(&k, l), "{k}");
                assert!(crate::i18n::has_in(&format!("{k}-desc"), l), "{k}-desc");
            }
        }
        assert_eq!(super::Section::Views.english_title(), "Views & layout");
    }
}

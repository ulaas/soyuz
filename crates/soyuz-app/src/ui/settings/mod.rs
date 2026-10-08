//! Settings window (Ctrl+,): a large modal with a sectioned sidebar covering every
//! `config.toml` section. Edits apply live (theme, keys, views …) and are written back
//! to `config.toml` shortly after the last change; external edits to the file (hot
//! reload) refresh the window. Search filters rows across all sections.

mod appearance;
mod density_gallery;
mod keybindings;
mod lists;
mod pages;
pub mod widgets;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::emath::TSTransform;
use egui::{
    Align, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Id, Key, LayerId, Layout, Margin, Modal,
    Modifiers, Order, Rect, RichText, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2,
};
use soyuz_core::config::{self, Config, Keymap};

use crate::app::SoyuzApp;
use crate::theme::fonts::SEMIBOLD;
use crate::theme::icons::ph;
use crate::theme::{Palette, icons, palette};
use crate::ui::dialogs::keybind_record::{RecordOutcome, RecordTarget, Recorder};
use crate::ui::toasts::Toast;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickPurpose {
    Export,
    Import,
}

/// Delay between the last edit and writing `config.toml`.
const SAVE_DELAY: Duration = Duration::from_millis(700);
const NAV_W: f32 = 216.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    General,
    Appearance,
    Views,
    Behaviour,
    Jobs,
    Terminal,
    Preview,
    Keyboard,
    Commands,
    Bookmarks,
    Advanced,
}

impl Section {
    pub const ALL: [Section; 11] = [
        Section::General,
        Section::Appearance,
        Section::Views,
        Section::Behaviour,
        Section::Jobs,
        Section::Terminal,
        Section::Preview,
        Section::Keyboard,
        Section::Commands,
        Section::Bookmarks,
        Section::Advanced,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Views => "Views & layout",
            Section::Behaviour => "Behaviour",
            Section::Jobs => "File operations",
            Section::Terminal => "Terminal",
            Section::Preview => "Preview & search",
            Section::Keyboard => "Keyboard",
            Section::Commands => "Custom commands",
            Section::Bookmarks => "Bookmarks",
            Section::Advanced => "Config file",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Section::General => ph::GEAR_SIX,
            Section::Appearance => ph::PALETTE,
            Section::Views => ph::LAYOUT,
            Section::Behaviour => ph::CURSOR_CLICK,
            Section::Jobs => ph::COPY,
            Section::Terminal => ph::TERMINAL_WINDOW,
            Section::Preview => ph::EYE,
            Section::Keyboard => ph::KEYBOARD,
            Section::Commands => ph::LIGHTNING,
            Section::Bookmarks => ph::BOOKMARK_SIMPLE,
            Section::Advanced => ph::FILE_CODE,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Section::General => "Startup, hidden files, sizes and dates.",
            Section::Appearance => "Theme, accent color, density and fonts. Changes preview live.",
            Section::Views => "Default view, columns, sorting and window layout.",
            Section::Behaviour => "Clicking, confirmations, drag and drop and typing.",
            Section::Jobs => "Copy / move engine: parallelism, verification, preservation.",
            Section::Terminal => "The integrated terminal panel.",
            Section::Preview => "Preview limits, search defaults and git integration.",
            Section::Keyboard => "Presets and every shortcut. Click a shortcut to change it.",
            Section::Commands => "Your own commands with placeholders, in the palette and context menu.",
            Section::Bookmarks => "Folders you jump to often, with optional hotkeys.",
            Section::Advanced => "Edit, export or import config.toml.",
        }
    }

    /// Reset this section of `c` to defaults.
    fn reset(self, c: &mut Config) {
        let d = Config::default();
        match self {
            Section::General => c.general = d.general,
            Section::Appearance => {
                c.appearance = d.appearance;
                c.theme = d.theme;
            }
            Section::Views => {
                c.columns = d.columns;
                c.layout = d.layout;
                c.sidebar = d.sidebar;
                c.toolbar = d.toolbar;
                c.context_menu = d.context_menu;
                c.general.default_view = d.general.default_view;
                c.general.natural_sort = d.general.natural_sort;
                c.general.case_insensitive_sort = d.general.case_insensitive_sort;
                c.general.folders_first = d.general.folders_first;
            }
            Section::Behaviour => {
                c.behaviour = d.behaviour;
                c.general.single_click_open = d.general.single_click_open;
                c.general.confirm_delete = d.general.confirm_delete;
                c.general.confirm_permanent_delete = d.general.confirm_permanent_delete;
                c.general.double_click_speed_ms = d.general.double_click_speed_ms;
                c.general.open_folders_in = d.general.open_folders_in;
            }
            Section::Jobs => c.jobs = d.jobs,
            Section::Terminal => c.terminal = d.terminal,
            Section::Preview => {
                c.preview = d.preview;
                c.git = d.git;
                let saved = std::mem::take(&mut c.search.saved);
                c.search = d.search;
                c.search.saved = saved;
            }
            Section::Keyboard => c.keys = d.keys,
            Section::Commands => c.custom_commands = d.custom_commands,
            Section::Bookmarks => c.bookmarks = d.bookmarks,
            Section::Advanced => *c = d,
        }
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
    draft: Config,
    applied: Config,
    query: String,
    opened_at: f64,
    focus_search: bool,
    save_due: Option<Instant>,
    saved_flash: Option<Instant>,
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
            save_due: None,
            saved_flash: None,
            recorder: None,
            key_cache: None,
            keys_filter: String::new(),
            keys_conflicts_only: false,
            keys_modified_only: false,
            picker: None,
            scale_temp: None,
            args_text: Vec::new(),
            scroll_top: false,
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
    let s = &mut app.settings;
    if !s.open {
        s.draft = app.config.clone();
        s.applied = app.config.clone();
        s.opened_at = f64::NAN;
        s.query.clear();
        s.focus_search = false;
        s.args_text.clear();
    }
    s.open = true;
    if let Some(sec) = section {
        s.section = sec;
        s.scroll_top = true;
    }
}

pub fn close(app: &mut SoyuzApp) {
    app.settings.open = false;
    app.settings.recorder = None;
    flush_save(app);
}

/// Write `config.toml` now if a save is pending.
fn flush_save(app: &mut SoyuzApp) {
    if app.settings.save_due.take().is_none() {
        return;
    }
    let path = app.config_watch.path().to_path_buf();
    // Merge into the user's file so comments and formatting survive (toml_edit).
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    match config::to_string_preserving(&existing, &app.config) {
        Ok(text) => {
            if let Some(dir) = path.parent() {
                let _ = soyuz_fs::create_dir_all(dir);
            }
            match soyuz_fs::write_atomic(&path, text.as_bytes()) {
                Ok(()) => {
                    app.config_watch.last_text = Some(text);
                    app.settings.saved_flash = Some(Instant::now());
                }
                Err(e) => {
                    app.ui.toasts.push(Toast::error("Couldn't save config.toml").body(e.to_string()));
                }
            }
        }
        Err(e) => {
            app.ui.toasts.push(Toast::error("Couldn't save config.toml").body(e.to_string()));
        }
    }
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
    if !app.settings.open {
        pump_save(ctx, app);
        return;
    }
    // External change (hot reload) while open: start over from the new config.
    if app.config != app.settings.applied {
        app.settings.draft = app.config.clone();
        app.settings.applied = app.config.clone();
    }
    let p = palette(ctx);
    let now = ctx.input(|i| i.time);
    if app.settings.opened_at.is_nan() {
        app.settings.opened_at = now;
    }
    let t = if app.ui.reduced_motion { 1.0 } else { ((now - app.settings.opened_at) / 0.18).clamp(0.0, 1.0) as f32 };
    let ease = 1.0 - (1.0 - t).powi(3);
    if t < 1.0 {
        ctx.request_repaint();
    }

    // Recorder on top (owns the keyboard while open).
    if app.settings.recorder.is_some() {
        let cmd_is_ctrl = app.settings.draft.keys.cmd_is_ctrl;
        let km = key_cache(&mut app.settings).keymap.clone();
        let outcome = app.settings.recorder.as_mut().map(|r| r.show(ctx, &km, cmd_is_ctrl));
        match outcome {
            Some(RecordOutcome::Save(binding)) => {
                let target = app.settings.recorder.take().map(|r| r.target);
                if let Some(target) = target {
                    apply_recorded(&mut app.settings, target, binding);
                }
            }
            Some(RecordOutcome::Cancel) => app.settings.recorder = None,
            _ => {}
        }
    } else if app.settings.picker.is_none() {
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
            if app.settings.query.is_empty() {
                close(app);
                return;
            }
            app.settings.query.clear();
        }
        if find {
            app.settings.focus_search = true;
        }
        if prev || next {
            let i = Section::ALL.iter().position(|s| *s == app.settings.section).unwrap_or(0);
            let n = Section::ALL.len();
            app.settings.section = Section::ALL[if next { (i + 1) % n } else { (i + n - 1) % n }];
            app.settings.scroll_top = true;
        }
    }

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
            crate::ui::a11y::container(ui.ctx(), ui.unique_id(), egui::accesskit::Role::Dialog, "Settings");
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let nav = Rect::from_min_size(rect.min, vec2(NAV_W, rect.height()));
            let content = Rect::from_min_max(pos2(nav.right(), rect.top()), rect.max);
            ui.painter().rect_filled(nav, CornerRadius { nw: 14, sw: 14, ne: 0, se: 0 }, p.bg_window);
            ui.painter().line_segment(
                [pos2(nav.right(), nav.top()), pos2(nav.right(), nav.bottom())],
                Stroke::new(1.0, p.border_subtle),
            );
            let mut nav_ui = ui.new_child(
                UiBuilder::new().max_rect(nav.shrink2(vec2(12.0, 14.0))).layout(Layout::top_down(Align::Min)),
            );
            if sidebar(&mut nav_ui, app, &p) {
                close_now = true;
            }
            let mut c_ui = ui.new_child(
                UiBuilder::new().max_rect(content.shrink2(vec2(28.0, 18.0))).layout(Layout::top_down(Align::Min)),
            );
            content_area(&mut c_ui, app, &p);
            // Close button.
            let cr = Rect::from_center_size(pos2(rect.right() - 22.0, rect.top() + 22.0), vec2(26.0, 26.0));
            let cresp = ui.interact(cr, id.with("close"), Sense::click()).on_hover_text("Close (Esc)");
            crate::ui::a11y::button(&cresp, "Close settings");
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
        });
    let s = 0.98 + 0.02 * ease;
    let c = screen.center();
    ctx.set_transform_layer(LayerId::new(Order::Foreground, id), TSTransform::new(c.to_vec2() * (1.0 - s), s));
    if (resp.backdrop_response.clicked() && app.settings.recorder.is_none() && app.settings.picker.is_none())
        || close_now
    {
        close(app);
        return;
    }
    show_picker(ctx, app);

    // Live apply + debounced save.
    if app.settings.draft != app.settings.applied {
        let draft = app.settings.draft.clone();
        crate::services::config_watch::apply_config(app, ctx, draft.clone());
        app.settings.applied = draft;
        app.settings.save_due = Some(Instant::now() + SAVE_DELAY);
    }
    pump_save(ctx, app);
}

/// Write `config.toml` once the debounce delay has passed.
fn pump_save(ctx: &egui::Context, app: &mut SoyuzApp) {
    if let Some(due) = app.settings.save_due {
        if Instant::now() >= due {
            flush_save(app);
        } else {
            ctx.request_repaint_after(due - Instant::now());
        }
    }
}

/// Change the active configuration from outside the settings window (UI zoom keys,
/// quick toggles): applies live and writes `config.toml` shortly after.
pub fn update_config(app: &mut SoyuzApp, ctx: &egui::Context, edit: impl FnOnce(&mut Config)) {
    let mut c = app.config.clone();
    edit(&mut c);
    if c == app.config {
        return;
    }
    crate::services::config_watch::apply_config(app, ctx, c);
    app.settings.save_due = Some(Instant::now() + SAVE_DELAY);
    ctx.request_repaint_after(SAVE_DELAY);
}

/// Flush a pending save (on exit).
pub fn save_now(app: &mut SoyuzApp) {
    flush_save(app);
}

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
        ui.label(icons::icon(ph::GEAR_SIX, 18.0).color(p.accent));
        ui.label(
            RichText::new("Settings").font(FontId::new(17.0, FontFamily::Name(SEMIBOLD.into()))).color(p.text_primary),
        );
    });
    ui.add_space(12.0);
    let s = &mut app.settings;
    let search_id = Id::new("soyuz.settings.search");
    // Typing anywhere (with nothing focused) starts a search.
    let typing = ui.ctx().memory(|m| m.focused().is_none())
        && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(t) if !t.trim().is_empty())));
    if s.focus_search || typing {
        ui.ctx().memory_mut(|m| m.request_focus(search_id));
        s.focus_search = false;
    }
    ui.add(
        TextEdit::singleline(&mut s.query)
            .id(search_id)
            .hint_text(RichText::new(format!("{}  Search settings", ph::MAGNIFYING_GLASS)).color(p.text_tertiary))
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(10, 7)),
    );
    ui.add_space(10.0);
    let searching = !s.query.trim().is_empty();
    for sec in Section::ALL {
        let active = !searching && s.section == sec;
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
        crate::ui::a11y::tab(&resp, sec.title(), active);
        if resp.has_focus() {
            crate::ui::widgets::focus_ring(ui, r);
        }
        let hover =
            ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), crate::theme::anim(ui.ctx(), 0.1));
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
            sec.icon(),
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
    // Bottom: save status + config file shortcuts.
    let mut close = false;
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        let status = if s.save_due.is_some() {
            (ph::CIRCLE_NOTCH, "Saving…", p.text_tertiary)
        } else if s.saved_flash.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
            (ph::CHECK_CIRCLE, "Saved to config.toml", p.success)
        } else {
            (ph::CHECK, "Changes apply instantly", p.text_tertiary)
        };
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(icons::icon(status.0, 12.0).color(status.2));
            ui.label(RichText::new(status.1).size(11.0).color(status.2));
        });
        ui.add_space(6.0);
        if nav_link(ui, p, ph::FILE_CODE, "Edit config.toml") {
            app.cmds.push(crate::cmd::Cmd::Action(soyuz_core::config::Action::EditConfigFile));
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
    let searching = !app.settings.query.trim().is_empty();
    let sec = app.settings.section;
    // Header.
    ui.horizontal(|ui| {
        if searching {
            ui.label(
                RichText::new(format!("Results for “{}”", app.settings.query.trim()))
                    .font(FontId::new(19.0, FontFamily::Name(SEMIBOLD.into())))
                    .color(p.text_primary),
            );
        } else {
            ui.label(icons::icon(sec.icon(), 20.0).color(p.accent));
            ui.label(
                RichText::new(sec.title())
                    .font(FontId::new(19.0, FontFamily::Name(SEMIBOLD.into())))
                    .color(p.text_primary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(30.0);
                if sec != Section::Advanced
                    && widgets::small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), "Reset section", true)
                        .on_hover_text("Restore the defaults of this section")
                        .clicked()
                {
                    sec.reset(&mut app.settings.draft);
                    app.ui.toasts.push(Toast::info(format!("{} reset to defaults", sec.title())));
                }
            });
        }
    });
    if !searching {
        ui.label(RichText::new(sec.description()).size(12.0).color(p.text_tertiary));
    }
    ui.add_space(4.0);
    let scroll_top = std::mem::take(&mut app.settings.scroll_top);
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("soyuz.settings.scroll", sec as u8, searching))
        .auto_shrink([false, false]);
    if scroll_top {
        area = area.vertical_scroll_offset(0.0);
    }
    area.show(ui, |ui| {
        ui.set_width(ui.available_width() - 6.0);
        let q = app.settings.query.trim().to_string();
        if searching {
            let mut any = false;
            for s in Section::ALL {
                any |= page(ui, app, s, &q);
            }
            if !any {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(icons::icon(ph::MAGNIFYING_GLASS, 28.0).color(p.text_tertiary));
                    ui.label(RichText::new("No settings match").color(p.text_secondary));
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
        Section::General => pages::general(ui, app, q),
        Section::Appearance => appearance::page(ui, app, q),
        Section::Views => pages::views(ui, app, q),
        Section::Behaviour => pages::behaviour(ui, app, q),
        Section::Jobs => pages::jobs(ui, app, q),
        Section::Terminal => pages::terminal(ui, app, q),
        Section::Preview => pages::preview(ui, app, q),
        Section::Keyboard => keybindings::page(ui, app, q),
        Section::Commands => lists::commands(ui, app, q),
        Section::Bookmarks => lists::bookmarks(ui, app, q),
        Section::Advanced => pages::advanced(ui, app, q),
    }
}

/// Start recording a shortcut.
fn record(app: &mut SoyuzApp, target: RecordTarget, title: String) {
    app.settings.recorder = Some(Recorder::new(target, title));
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
    let picker = PathPicker::new("Export settings", PickMode::Save, dir, &name, &["toml"]);
    app.settings.picker = Some((PickPurpose::Export, picker));
}

/// Open the in-app picker for importing a configuration file.
fn start_import(app: &mut SoyuzApp) {
    use crate::ui::path_picker::{PathPicker, PickMode};
    let picker = PathPicker::new("Import settings", PickMode::Open, transfer_dir(), "", &["toml"]);
    app.settings.picker = Some((PickPurpose::Import, picker));
}

/// Draw the export / import picker; act on its result.
fn show_picker(ctx: &egui::Context, app: &mut SoyuzApp) {
    use crate::ui::path_picker::PickResult;
    let Some((purpose, picker)) = app.settings.picker.as_mut() else { return };
    let purpose = *purpose;
    match picker.show(ctx) {
        PickResult::Pending => {}
        PickResult::Cancelled => {
            log::debug!("export / import picker cancelled");
            app.settings.picker = None
        }
        PickResult::Picked(path) => {
            app.settings.picker = None;
            match purpose {
                PickPurpose::Export => {
                    export(app, path);
                }
                PickPurpose::Import => import(app, &path),
            }
        }
    }
}

/// Write the active config to `path` (keeps that file's comments when it exists).
fn export(app: &mut SoyuzApp, path: PathBuf) -> Option<PathBuf> {
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let text = match config::to_string_preserving(&existing, &app.config) {
        Ok(t) => t,
        Err(e) => {
            app.ui.toasts.push(Toast::error("Export failed").body(e.to_string()));
            return None;
        }
    };
    match soyuz_fs::write_atomic(&path, text.as_bytes()) {
        Ok(()) => {
            app.ui.toasts.push(Toast::success("Settings exported").body(path.display().to_string()));
            Some(path)
        }
        Err(e) => {
            app.ui.toasts.push(Toast::error("Export failed").body(e.to_string()));
            None
        }
    }
}

/// Import a config file into the draft (applies live, saved shortly after).
fn import(app: &mut SoyuzApp, path: &std::path::Path) {
    match config::load(path) {
        Ok(loaded) => {
            app.settings.draft = loaded.config;
            let mut t = Toast::success("Settings imported");
            if !loaded.warnings.is_empty() {
                t = Toast::warning("Settings imported with warnings").body(loaded.warnings.join("\n"));
            }
            app.ui.toasts.push(t);
        }
        Err(e) => {
            app.ui.toasts.push(Toast::error("Import failed").body(e.to_string()));
        }
    }
}

/// A pill showing a chord binding with a hover "×"; returns (clicked, remove clicked).
fn binding_chip(ui: &mut Ui, p: &Palette, binding: &str, conflict: bool, removable: bool) -> (bool, bool) {
    let w = crate::palette::widgets::chips_width(ui, binding) + if removable { 18.0 } else { 6.0 };
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
    crate::palette::widgets::paint_chips(ui, p, binding, right, rect.center().y, false);
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
}

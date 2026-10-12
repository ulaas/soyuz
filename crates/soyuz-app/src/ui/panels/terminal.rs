//! Integrated terminal (GUI side): the bottom panel's Terminal tab — the session strip
//! (mini-tabs with new / close / rename), the terminal view, the OSC 52 prompt and the
//! key dispatch while the terminal owns the keyboard. Sessions, focus and the folder
//! following live in [`crate::models::terminal`]. The color theme derives from the app
//! palette.

use std::path::{Path, PathBuf};

use std::time::Duration;

use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Rect, Sense, Stroke, TextEdit, Ui, UiBuilder, pos2, vec2};
use soyuz_core::config::{Action, KeyChord, KeyContext, Keymap};
use soyuz_term::{TermTheme, TerminalView};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::theme::icons::{self, ph};
use crate::theme::{Palette, palette};
use crate::ui::widgets::{menu_item, menu_separator};

pub use crate::models::terminal::*;

const STRIP_H: f32 = 30.0;

/// Key handling while the terminal owns the keyboard: only app-level chords that a
/// shell never needs reach the keymap (Ctrl+`, Ctrl+Tab, Ctrl+Shift+…, Alt+P, F-keys
/// with Ctrl). Returns the actions to run.
pub fn dispatch_keys(ctx: &egui::Context, keymap: &Keymap, cmd_is_ctrl: bool) -> Vec<Action> {
    let mut out = Vec::new();
    ctx.input_mut(|i| {
        i.events.retain(|ev| {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = ev else { return true };
            let Some(name) = crate::ui::keys::key_name(*key) else { return true };
            let chord = KeyChord::new(crate::ui::keys::mods(*modifiers), name).normalized(cmd_is_ctrl);
            if !terminal_passthrough(chord) {
                return true;
            }
            match keymap.lookup(KeyContext::Terminal, chord) {
                Some(a) => {
                    out.push(a.clone());
                    false
                }
                None => true,
            }
        });
    });
    out
}

/// Theme from the palette (`[terminal] theme = auto`), or the built-in dark / light.
pub fn theme_for(p: &Palette, setting: &str) -> TermTheme {
    match setting.trim().to_ascii_lowercase().as_str() {
        "dark" => return TermTheme::dark(),
        "light" => return TermTheme::light(),
        _ => {}
    }
    let mut t = if p.dark { TermTheme::dark() } else { TermTheme::light() };
    t.fg = p.text_primary;
    t.bg = if p.dark { p.bg_sunken } else { p.bg_surface1 };
    t.cursor = p.accent;
    let [r, g, b, _] = p.accent.to_array();
    t.selection = Color32::from_rgba_unmultiplied(r, g, b, if p.dark { 77 } else { 60 });
    t.ansi[1] = p.danger;
    t.ansi[2] = p.success;
    t.ansi[3] = p.warning;
    t.ansi[4] = p.accent;
    t.ansi[5] = p.glyph_video;
    t.ansi[6] = p.info;
    t.ansi[12] = p.accent_hover;
    t
}

/// The frontend carries out `FrontendCmd::ReleaseFocus`: no widget keeps the keyboard.
pub fn release_focus(ctx: &egui::Context) {
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
}

/// The cached terminal theme: `(dark, [terminal] theme)` it was derived for.
type ThemeCache = Option<(bool, String, TermTheme)>;

thread_local! {
    static THEME: std::cell::RefCell<ThemeCache> = const { std::cell::RefCell::new(None) };
}

/// CJK / Hangul fallbacks join every font family (incl. the terminal's), once a terminal
/// is shown.
fn request_cjk_fonts() {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
        use crate::theme::font_fallback::{Script, request};
        request(&[Script::Cjk, Script::Hangul]);
    }
}

/// Non-blocking prompt for a pending OSC 52 write (drawn over everything, bottom
/// centre, every frame while a request waits): Allow once / Always for this session /
/// Deny.
pub fn clipboard_prompt(ctx: &egui::Context, app: &mut SoyuzApp) {
    use crate::ui::dialogs::conflict::{Btn, button};
    let Some(text) = app.core.ui.terminal.clip_request.clone() else { return };
    let p = palette(ctx);
    let screen = ctx.content_rect();
    let bottom = if app.core.ui.status_visible { 36.0 } else { 12.0 };
    let width = 460.0_f32.min(screen.width() - 32.0);
    let mut answer: Option<ClipDecision> = None;
    let mut always = false;
    egui::Area::new(Id::new("soyuz-osc52-prompt"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -bottom))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(p.bg_surface2)
                .stroke(Stroke::new(1.0, p.border_strong))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::same(14))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 18,
                    spread: 0,
                    color: Color32::from_black_alpha(60),
                })
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        ui.label(icons::icon(ph::CLIPBOARD_TEXT, 16.0).color(p.warning));
                        ui.label(
                            egui::RichText::new(tr!("panels-terminal-clip-title"))
                                .font(FontId::proportional(13.0))
                                .color(p.text_primary),
                        );
                    });
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(p.bg_sunken)
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let preview = clipboard_preview(&text);
                            let shown = if preview.is_empty() {
                                tr!("panels-terminal-clip-empty").to_string()
                            } else {
                                preview
                            };
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(shown).font(FontId::monospace(11.5)).color(p.text_secondary),
                                )
                                .truncate(),
                            );
                        });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(tr!("panels-terminal-clip-warning"))
                            .font(FontId::proportional(11.0))
                            .color(p.text_tertiary),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if button(ui, tr!("panels-terminal-clip-deny"), Btn::Ghost).clicked() {
                            answer = Some(ClipDecision::Drop);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if button(ui, tr!("panels-terminal-clip-allow-once"), Btn::Primary).clicked() {
                                answer = Some(ClipDecision::Copy);
                            }
                            if button(ui, tr!("panels-terminal-clip-always"), Btn::Secondary)
                                .on_hover_text(tr!("panels-terminal-clip-always-tip"))
                                .clicked()
                            {
                                answer = Some(ClipDecision::Copy);
                                always = true;
                            }
                        });
                    });
                });
        });
    if let Some(d) = answer {
        answer_clip(&mut app.core, &crate::host::EguiHost::new(ctx), d == ClipDecision::Copy, always);
    }
}

/// Body of the Terminal tab (inside the bottom panel card).
pub fn body(ui: &mut Ui, rect: Rect, app: &mut SoyuzApp) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    request_cjk_fonts();
    ensure_session(&mut app.core, &crate::host::EguiHost::new(&ctx));
    let owner = current_owner(&app.core);
    let ids = app.core.ui.terminal.visible_ids(owner);
    let Some(active) = app.core.ui.terminal.active_id(owner) else { return };

    // ---- session strip ----
    let strip = Rect::from_min_size(rect.min + vec2(8.0, 0.0), vec2(rect.width() - 16.0, STRIP_H));
    let mut x = strip.left();
    let mut to_close = None;
    let mut to_activate = None;
    let mut add_session = false;
    let mut external = false;
    for id in &ids {
        let (label, alive, busy) = {
            let s = app.core.ui.terminal.sessions.iter().find(|s| s.id == *id).expect("visible id");
            (s.label(), s.is_alive(), s.is_busy())
        };
        let is_active = *id == active;
        let renaming = app.core.ui.terminal.renaming.as_ref().filter(|(rid, _)| rid == id).map(|(_, t)| t.clone());
        let font = FontId::proportional(12.0);
        let text_w = ui.painter().layout_no_wrap(label.clone(), font.clone(), p.text_primary).size().x.min(180.0);
        let w = if renaming.is_some() { 160.0 } else { text_w + 52.0 };
        let r = Rect::from_min_size(pos2(x, strip.top() + 3.0), vec2(w, STRIP_H - 6.0));
        x += w + 4.0;
        let resp = ui.interact(r, Id::new(("soyuz.term.tab", *id)), Sense::click());
        let hovered = resp.hovered();
        let fill = if is_active {
            p.bg_surface2
        } else if hovered {
            p.bg_surface2.gamma_multiply(0.55)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(r, CornerRadius::same(6), fill);
        if is_active {
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(r.left() + 8.0, r.bottom() - 2.0), pos2(r.right() - 8.0, r.bottom())),
                CornerRadius::same(1),
                p.accent,
            );
        }
        let dot = if !alive {
            p.danger
        } else if busy {
            p.warning
        } else {
            p.success
        };
        ui.painter().circle_filled(pos2(r.left() + 12.0, r.center().y), 3.0, dot);
        if let Some(mut text) = renaming {
            let er = Rect::from_min_max(pos2(r.left() + 20.0, r.top() + 2.0), pos2(r.right() - 6.0, r.bottom() - 2.0));
            let mut child = ui.new_child(UiBuilder::new().max_rect(er));
            let edit = child.add(
                TextEdit::singleline(&mut text).font(font.clone()).desired_width(er.width()).frame(egui::Frame::NONE),
            );
            edit.request_focus();
            let done = edit.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter));
            if done {
                rename(&mut app.core, *id, &text);
            } else {
                app.core.ui.terminal.renaming = Some((*id, text));
            }
            continue;
        }
        let c = if is_active { p.text_primary } else { p.text_secondary };
        let galley = crate::ui::widgets::trunc_galley(ui.painter(), &label, font, c, 180.0);
        ui.painter().galley(pos2(r.left() + 22.0, r.center().y - galley.size().y / 2.0), galley, c);
        let close_r = Rect::from_center_size(pos2(r.right() - 13.0, r.center().y), vec2(16.0, 16.0));
        let close = ui.interact(close_r, Id::new(("soyuz.term.close", *id)), Sense::click());
        if hovered || close.hovered() || is_active {
            if close.hovered() {
                ui.painter().rect_filled(close_r, CornerRadius::same(4), p.bg_surface3);
            }
            ui.painter().text(close_r.center(), Align2::CENTER_CENTER, ph::X, icons::font(11.0), p.text_tertiary);
        }
        if close.clicked() || resp.middle_clicked() {
            to_close = Some(*id);
        } else if resp.double_clicked() {
            app.core.ui.terminal.renaming = Some((*id, label.clone()));
        } else if resp.clicked() {
            to_activate = Some(*id);
        }
        let mut rename = false;
        crate::ui::native::popup::context_menu(&resp, |ui| {
            ui.set_min_width(200.0);
            if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("common-rename"), None, true).clicked() {
                rename = true;
                ui.close();
            }
            if menu_item(ui, Some(ph::PLUS), tr!("panels-terminal-new"), None, true).clicked() {
                add_session = true;
                ui.close();
            }
            if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("panels-terminal-open-external"), None, true).clicked() {
                external = true;
                ui.close();
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::X), tr!("common-close"), None, true).clicked() {
                to_close = Some(*id);
                ui.close();
            }
        });
        if rename {
            app.core.ui.terminal.renaming = Some((*id, label));
        }
    }
    let plus = Rect::from_min_size(pos2(x + 2.0, strip.top() + 4.0), vec2(22.0, 22.0));
    let presp = ui.interact(plus, Id::new("soyuz.term.new"), Sense::click()).on_hover_text(tr!("panels-terminal-new"));
    if presp.hovered() {
        ui.painter().rect_filled(plus, CornerRadius::same(5), p.bg_surface2);
    }
    ui.painter().text(plus.center(), Align2::CENTER_CENTER, ph::PLUS, icons::font(13.0), p.text_secondary);
    add_session |= presp.clicked();
    // Right side: busy hint + external terminal button.
    let ext = Rect::from_min_size(pos2(strip.right() - 26.0, strip.top() + 4.0), vec2(22.0, 22.0));
    let eresp = ui
        .interact(ext, Id::new("soyuz.term.external"), Sense::click())
        .on_hover_text(tr!("panels-terminal-open-external"));
    if eresp.hovered() {
        ui.painter().rect_filled(ext, CornerRadius::same(5), p.bg_surface2);
    }
    ui.painter().text(ext.center(), Align2::CENTER_CENTER, ph::ARROW_SQUARE_OUT, icons::font(13.0), p.text_secondary);
    external |= eresp.clicked();
    if let Some(s) = app.core.ui.terminal.sessions.iter().find(|s| s.id == active)
        && s.is_busy()
    {
        ui.painter().text(
            pos2(ext.left() - 10.0, strip.center().y),
            Align2::RIGHT_CENTER,
            tr!("panels-terminal-busy"),
            FontId::proportional(11.0),
            p.warning,
        );
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    // ---- terminal ----
    let term_rect = Rect::from_min_max(pos2(rect.left() + 8.0, strip.bottom() + 2.0), rect.max - vec2(8.0, 6.0));
    let theme_key = (p.dark, app.core.config.terminal.theme.clone());
    let theme = THEME.with_borrow_mut(|cache| {
        if cache.as_ref().is_none_or(|(d, s, _)| (*d, s.clone()) != theme_key) {
            let th = theme_for(&p, &app.core.config.terminal.theme);
            *cache = Some((theme_key.0, theme_key.1, th));
        }
        cache.as_ref().map(|(_, _, t)| t.clone()).unwrap_or_default()
    });
    ui.painter().rect_filled(term_rect, CornerRadius::same(6), theme.bg);
    let inner = term_rect.shrink2(vec2(8.0, 6.0));
    // Clicking anywhere else gives the keyboard back to the file view.
    let (pressed, ptr) = ui.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos()));
    if pressed && app.core.ui.terminal.focused && ptr.is_some_and(|pt| !term_rect.contains(pt) && !strip.contains(pt)) {
        app.core.ui.terminal.focused = false;
    }
    let focused = app.core.ui.terminal.focused;
    let font_size = app.core.config.terminal.font_size.clamp(7.0, 32.0);
    let mut link = None;
    let mut link_base: Option<PathBuf> = None;
    let mut esc = false;
    let mut wants = false;
    if let Some(s) = app.core.ui.terminal.sessions.iter().find(|s| s.id == active) {
        match &s.term {
            Some(term) => {
                if crate::ui::a11y::active(ui.ctx()) {
                    // Screen readers: a terminal node named after the session, its visible
                    // text as the value. Added before the view so it never covers it.
                    let node = ui.interact(inner, Id::new(("soyuz.term.a11y", s.id)), Sense::hover());
                    let name = tr!("panels-terminal-a11y", name = s.label());
                    let text = term.screen_text();
                    crate::ui::a11y::with(ui.ctx(), node.id, |n| {
                        n.set_role(egui::accesskit::Role::Terminal);
                        n.set_label(name);
                        n.set_value(text.trim_end());
                    });
                }
                let mut child = ui.new_child(UiBuilder::new().max_rect(inner).id_salt(("soyuz.term", s.id)));
                child.set_clip_rect(inner);
                let r = TerminalView::new(term)
                    .focused(focused)
                    .theme(&theme)
                    .font(crate::theme::fonts::terminal(ui.ctx(), font_size))
                    .show(&mut child);
                wants = r.wants_focus;
                link = r.link_clicked;
                if link.is_some() {
                    link_base = term.cwd().or_else(|| s.last_dir.clone());
                }
                esc = r.escape_on_empty_line;
            }
            None => {
                ui.painter().text(
                    inner.center() - vec2(0.0, 10.0),
                    Align2::CENTER_CENTER,
                    ph::WARNING_CIRCLE,
                    icons::font_fill(26.0),
                    p.danger,
                );
                ui.painter().text(
                    inner.center() + vec2(0.0, 18.0),
                    Align2::CENTER_CENTER,
                    tr!(
                        "panels-terminal-shell-failed",
                        error = s.error.clone().unwrap_or_else(|| tr!("panels-terminal-unknown-error").to_string())
                    ),
                    FontId::proportional(12.0),
                    p.text_secondary,
                );
            }
        }
    }
    if focused {
        ui.painter().rect_stroke(
            term_rect,
            CornerRadius::same(6),
            Stroke::new(1.0, p.accent.gamma_multiply(0.45)),
            egui::StrokeKind::Inside,
        );
    }
    if wants {
        app.core.ui.terminal.focused = true;
    }
    if esc && app.core.config.terminal.escape_returns_focus {
        unfocus(&mut app.core);
    }
    if let Some(url) = link {
        open_link(app, &ctx, &url, link_base.as_deref());
    }

    if let Some(id) = to_activate {
        app.core.ui.terminal.set_active(owner, id);
        app.core.ui.terminal.focused = true;
    }
    if let Some(id) = to_close {
        close_session(&mut app.core, id);
    }
    if add_session {
        new_session(&mut app.core, &crate::host::EguiHost::new(&ctx));
    }
    if external {
        open_external(&mut app.core, active);
    }
}

/// Ctrl+click on a link: URLs go to the browser, paths open in the pane / default app
/// (see [`open_path_link`]).
fn open_link(app: &mut SoyuzApp, ctx: &egui::Context, url: &str, base: Option<&Path>) {
    if is_web_link(url) {
        ctx.open_url(egui::OpenUrl::new_tab(url));
        return;
    }
    open_path_link(&mut app.core, url, base);
}

/// Context-menu rows: integrated + external terminal at `dir`.
pub fn menu_items(ui: &mut Ui, cmds: &mut Vec<Cmd>, dir: &Path) {
    if menu_item(ui, Some(ph::TERMINAL_WINDOW), tr!("panels-terminal-open-here"), Some("Ctrl+`"), true).clicked() {
        cmds.push(Cmd::OpenTerminalPanel(dir.to_path_buf()));
        ui.close();
    }
    if menu_item(ui, Some(ph::ARROW_SQUARE_OUT), tr!("panels-terminal-open-external"), None, true).clicked() {
        cmds.push(Cmd::TerminalAt(dir.to_path_buf()));
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_theme() {
        let t = theme_for(&crate::theme::ORBIT_DARK, "auto");
        assert_eq!(t.bg, crate::theme::ORBIT_DARK.bg_sunken);
        assert_eq!(t.cursor, crate::theme::ORBIT_DARK.accent);
        let l = theme_for(&crate::theme::ORBIT_LIGHT, "auto");
        assert_eq!(l.fg, crate::theme::ORBIT_LIGHT.text_primary);
        assert_eq!(theme_for(&crate::theme::ORBIT_LIGHT, "dark"), TermTheme::dark());
    }
}

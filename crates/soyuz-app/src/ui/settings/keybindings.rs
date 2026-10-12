//! Keyboard page: preset picker, options, and the keybinding editor (search actions,
//! record / remove / reset shortcuts, conflicts highlighted).

use egui::{Align, Align2, CornerRadius, FontId, Layout, Margin, RichText, Sense, TextEdit, Ui, vec2};
use soyuz_core::config::{Action, ActionCategory, Config, KeyPreset, Keymap, format_sequence};

use super::widgets::{choice, dd, group, matches, small_button, switch, upper};
use super::{binding_chip, key_cache, record};
use crate::app::SoyuzApp;
use crate::keys::conflicts::{Conflict, ConflictKind, for_action};
use crate::palette::catalog;
use crate::theme::icons::ph;
use crate::theme::{icons, palette};
use crate::ui::dialogs::keybind_record::RecordTarget;

/// Effective bindings of `a` as strings (chords first, then sequences).
pub fn current_bindings(km: &Keymap, a: &Action) -> Vec<String> {
    let mut v: Vec<String> = km.chords_for(a).iter().map(|c| c.to_string()).collect();
    v.extend(km.sequences_for(a).iter().map(|s| format_sequence(s)));
    v.dedup();
    v
}

/// Replace the `[keys.bindings]` entry of `a` (removing differently spelled keys).
pub fn set_override(c: &mut Config, a: &Action, list: Vec<String>) {
    c.keys.bindings.retain(|k, _| k.parse::<Action>().ok().as_ref() != Some(a));
    c.keys.bindings.insert(a.to_string(), list);
}

/// Drop the override of `a` (back to the preset).
pub fn reset_override(c: &mut Config, a: &Action) {
    c.keys.bindings.retain(|k, _| k.parse::<Action>().ok().as_ref() != Some(a));
}

pub fn is_overridden(c: &Config, a: &Action) -> bool {
    c.keys.bindings.keys().any(|k| k.parse::<Action>().ok().as_ref() == Some(a))
}

fn preset_help(k: KeyPreset) -> &'static str {
    match k {
        KeyPreset::Explorer => tr!("settings-keys-preset-explorer-desc"),
        KeyPreset::Commander => tr!("settings-keys-preset-commander-desc"),
        KeyPreset::Finder => tr!("settings-keys-preset-finder-desc"),
        KeyPreset::Vim => tr!("settings-keys-preset-vim-desc"),
    }
}

pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let mut any = false;
    {
        let d = &mut app.gui.settings.draft;
        any |= group(ui, q, "Keyboard shortcuts keys", tr!("settings-keys-preset"), |r| {
            let help = preset_help(d.keys.preset);
            r.row_kw(tr!("settings-keys-key-preset"), help, "explorer commander finder vim", |ui| {
                let item = |k: KeyPreset, icon| {
                    dd::Item::new(k, crate::i18n::labels::key_preset(k)).icon(icon).keywords(preset_help(k))
                };
                let items = [
                    item(KeyPreset::Explorer, ph::WINDOWS_LOGO),
                    item(KeyPreset::Commander, ph::COLUMNS),
                    item(KeyPreset::Finder, ph::APPLE_LOGO),
                    item(KeyPreset::Vim, ph::TERMINAL_WINDOW),
                ];
                choice(ui, "key_preset", &mut d.keys.preset, &items, 210.0);
            });
            r.row_kw(tr!("settings-keys-cmd-ctrl"), tr!("settings-keys-cmd-ctrl-desc"), "command meta", |ui| {
                switch(ui, &mut d.keys.cmd_is_ctrl);
            });
            r.row_kw(tr!("settings-keys-marks"), tr!("settings-keys-marks-desc"), "ranger bookmark", |ui| {
                switch(ui, &mut d.keys.marks);
            });
        });
    }
    let searching_outside = !q.trim().is_empty();
    // The editor itself is a big block; in global search it lists matching actions only.
    let cache = key_cache(&mut app.gui.settings);
    let km = cache.keymap.clone();
    let warnings = cache.warnings.clone();
    let conflicts = cache.conflicts.clone();
    let filter = if searching_outside { q.to_string() } else { app.gui.settings.keys_filter.clone() };

    let mut actions: Vec<Action> = Action::all();
    let n_bm = app.gui.settings.draft.bookmarks.len();
    actions.retain(|a| {
        !matches!(a, Action::OpenBookmark(n) if *n as usize > n_bm.max(3)) && !crate::actions::unavailable(a)
    });
    actions.extend(
        app.gui
            .settings
            .draft
            .custom_commands
            .iter()
            .filter(|c| !c.name.trim().is_empty())
            .map(|c| Action::CustomCommand(c.name.clone())),
    );
    let rows: Vec<(Action, catalog::Entry, Vec<String>)> = actions
        .into_iter()
        .map(|a| {
            let e = catalog::entry(&a);
            let b = current_bindings(&km, &a);
            (a, e, b)
        })
        .filter(|(a, e, b)| {
            let text_ok = matches(
                &filter,
                &[
                    &e.title,
                    &e.title_en,
                    e.keywords,
                    &a.to_string(),
                    &b.join(" "),
                    crate::i18n::labels::action_category(e.category),
                ],
            );
            let conflict_ok = !app.gui.settings.keys_conflicts_only
                || for_action(&conflicts, a).iter().any(|c| c.kind != ConflictKind::Shadowed);
            let modified_ok = !app.gui.settings.keys_modified_only || is_overridden(&app.gui.settings.draft, a);
            text_ok && conflict_ok && modified_ok
        })
        .collect();
    if searching_outside && rows.is_empty() {
        return any;
    }
    any = true;
    ui.add_space(14.0);
    ui.label(
        RichText::new(upper(tr!("settings-keys-shortcuts"))).font(FontId::proportional(10.5)).color(p.text_tertiary),
    );
    ui.add_space(6.0);
    if !searching_outside {
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut app.gui.settings.keys_filter)
                    .hint_text(
                        RichText::new(format!("{}  {}", ph::MAGNIFYING_GLASS, tr!("settings-keys-search-hint")))
                            .color(p.text_tertiary),
                    )
                    .desired_width(260.0)
                    .margin(Margin::symmetric(10, 6)),
            );
            ui.add_space(8.0);
            ui.checkbox(&mut app.gui.settings.keys_conflicts_only, tr!("settings-keys-conflicts-only"));
            ui.add_space(4.0);
            ui.checkbox(&mut app.gui.settings.keys_modified_only, tr!("settings-keys-changed-only"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let n = app.gui.settings.draft.keys.bindings.len();
                if n > 0
                    && small_button(
                        ui,
                        Some(ph::ARROW_COUNTER_CLOCKWISE),
                        &tr!("settings-keys-reset-changes", count = n),
                        true,
                    )
                    .clicked()
                {
                    app.gui.settings.draft.keys.bindings.clear();
                }
            });
        });
        ui.add_space(8.0);
        // Problems banner.
        let hard: Vec<&Conflict> = conflicts.iter().filter(|c| c.kind != ConflictKind::Shadowed).collect();
        if !hard.is_empty() || !warnings.is_empty() {
            egui::Frame::new()
                .fill(p.danger.gamma_multiply(0.10))
                .stroke(egui::Stroke::new(1.0, p.danger.gamma_multiply(0.5)))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for c in &hard {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(icons::icon(ph::WARNING, 13.0).color(p.danger));
                            ui.label(RichText::new(c.message()).size(12.0).color(p.text_primary));
                        });
                    }
                    for w in &warnings {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(icons::icon(ph::WARNING_CIRCLE, 13.0).color(p.warning));
                            ui.label(RichText::new(w).size(12.0).color(p.text_primary));
                        });
                    }
                });
            ui.add_space(8.0);
        }
    }
    egui::Frame::new()
        .fill(p.bg_surface2.gamma_multiply(if p.dark { 0.7 } else { 1.0 }))
        .stroke(egui::Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if rows.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(tr!("settings-keys-empty")).color(p.text_tertiary)));
                ui.add_space(20.0);
                return;
            }
            let mut last: Option<ActionCategory> = None;
            for (a, e, bindings) in &rows {
                if last != Some(e.category) {
                    ui.add_space(if last.is_none() { 4.0 } else { 10.0 });
                    ui.label(
                        RichText::new(upper(crate::i18n::labels::action_category(e.category)))
                            .font(FontId::proportional(10.0))
                            .color(p.text_tertiary),
                    );
                    ui.add_space(2.0);
                    last = Some(e.category);
                }
                action_row(ui, app, a, e, bindings, &conflicts);
            }
        });
    any
}

fn action_row(
    ui: &mut Ui,
    app: &mut SoyuzApp,
    a: &Action,
    e: &catalog::Entry,
    bindings: &[String],
    conflicts: &[Conflict],
) {
    let p = palette(ui.ctx());
    let mine = for_action(conflicts, a);
    let overridden = is_overridden(&app.gui.settings.draft, a);
    let row_rect = ui.available_rect_before_wrap();
    let row_rect = egui::Rect::from_min_size(row_rect.min, vec2(row_rect.width(), 34.0));
    let hovered = ui.rect_contains_pointer(row_rect);
    if hovered {
        ui.painter().rect_filled(row_rect, CornerRadius::same(6), p.bg_surface3.gamma_multiply(0.7));
    }
    let mut remove: Option<usize> = None;
    let mut add = false;
    let mut reset = false;
    ui.horizontal(|ui| {
        ui.set_min_height(34.0);
        ui.add_space(6.0);
        let (ir, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        ui.painter().text(
            ir.center(),
            Align2::CENTER_CENTER,
            crate::ui::icons::glyph(e.icon),
            icons::font(14.0),
            p.text_secondary,
        );
        ui.add_space(6.0);
        let title = ui.label(RichText::new(&e.title).size(13.0).color(p.text_primary));
        title.on_hover_text(format!("{}\n{}", e.description, a));
        if overridden {
            let (dr, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(dr.center(), 3.0, p.accent);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if overridden
                && small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), "", false)
                    .on_hover_text(tr!("settings-keys-reset-one"))
                    .clicked()
            {
                reset = true;
            }
            let plus =
                small_button(ui, Some(ph::PLUS), if bindings.is_empty() { tr!("settings-add") } else { "" }, false)
                    .on_hover_text(tr!("settings-keys-record"));
            if plus.clicked() {
                add = true;
            }
            for (i, b) in bindings.iter().enumerate().rev() {
                let conflict =
                    mine.iter().any(|c| c.kind != ConflictKind::Shadowed && c.keys.split(" / ").any(|k| k == b));
                let shadow = mine.iter().find(|c| c.kind == ConflictKind::Shadowed && c.keys == *b);
                let (_, removed) = binding_chip(ui, &p, b, conflict, true);
                if removed {
                    remove = Some(i);
                }
                if let Some(s) = shadow {
                    let (sr, sresp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    ui.painter().text(sr.center(), Align2::CENTER_CENTER, ph::INFO, icons::font(11.0), p.warning);
                    sresp.on_hover_text(s.message());
                }
            }
            for c in mine.iter().filter(|c| c.kind != ConflictKind::Shadowed) {
                let (wr, wresp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                ui.painter().text(wr.center(), Align2::CENTER_CENTER, ph::WARNING, icons::font(12.0), p.danger);
                wresp.on_hover_text(c.message());
            }
        });
    });
    if reset {
        reset_override(&mut app.gui.settings.draft, a);
    }
    if let Some(i) = remove {
        let mut list = bindings.to_vec();
        list.remove(i);
        set_override(&mut app.gui.settings.draft, a, list);
    }
    if add {
        record(app, RecordTarget::Action(a.clone()), tr!("settings-keys-record-title", action = &e.title));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::keymap;

    #[test]
    fn override_helpers() {
        let mut c = Config::default();
        let (km, _) = keymap(&c);
        let before = current_bindings(&km, &Action::CommandPalette);
        assert!(before.contains(&"Ctrl+Shift+P".to_string()), "{before:?}");
        c.keys.bindings.insert("Command-Palette".into(), vec!["F1".into()]);
        assert!(is_overridden(&c, &Action::CommandPalette));
        set_override(&mut c, &Action::CommandPalette, vec!["F1".into(), "Ctrl+K Ctrl+P".into()]);
        assert_eq!(c.keys.bindings.len(), 1, "differently spelled key replaced");
        let (km, w) = keymap(&c);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(current_bindings(&km, &Action::CommandPalette), vec!["F1", "Ctrl+K Ctrl+P"]);
        reset_override(&mut c, &Action::CommandPalette);
        assert!(!is_overridden(&c, &Action::CommandPalette));
        // Removing every chord keeps an empty override (= unbound).
        set_override(&mut c, &Action::Refresh, vec![]);
        let (km, _) = keymap(&c);
        assert!(current_bindings(&km, &Action::Refresh).is_empty());
    }
}

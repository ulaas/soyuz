//! Settings → Preview & search → tr!("settings-index"): turn the background index on, choose
//! the indexed folders and exclusions, content indexing, the NTFS change journal,
//! throttling; live status and tr!("settings-index-rebuild").

use egui::{Id, RichText, TextEdit, Ui};
use soyuz_core::config::Config;
use soyuz_index::{Phase, UsnState};

use crate::theme::icons::{self, ph};
use crate::theme::palette;
use crate::ui::settings::widgets::{group, number, small_button, switch};

const CTX: &str = "Preview search index smart folders";

/// Multi-line / list text field bound to a `Vec<String>`: the raw text lives in egui
/// memory while editing, so typing separators doesn't fight the parsing.
fn list_field(ui: &mut Ui, id: &str, list: &mut Vec<String>, sep: &str, multiline: bool, hint: &str) {
    let id = Id::new(("soyuz.settings.index", id));
    let joined = list.join(sep);
    let parse = |t: &str| -> Vec<String> {
        t.split(|c: char| if multiline { c == '\n' } else { c == ',' || c == ';' || c.is_whitespace() })
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let mut text: String = ui.data(|d| d.get_temp::<String>(id)).filter(|t| parse(t) == *list).unwrap_or(joined);
    let edit = if multiline {
        TextEdit::multiline(&mut text).desired_rows(4).font(egui::TextStyle::Monospace)
    } else {
        TextEdit::singleline(&mut text)
    };
    let r = ui.add(edit.hint_text(hint).desired_width(f32::INFINITY));
    if r.changed() {
        *list = parse(&text);
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

fn usn_text(st: &UsnState) -> (&'static str, String) {
    match st {
        UsnState::Active => (ph::CHECK_CIRCLE, tr!("settings-index-usn-active").into()),
        UsnState::NeedsAdmin => (ph::SHIELD_WARNING, tr!("settings-index-usn-admin").into()),
        UsnState::Unsupported(why) => (ph::INFO, tr!("settings-index-usn-unsupported", reason = why.to_string())),
        UsnState::Off => (ph::INFO, tr!("settings-index-usn-off").into()),
    }
}

/// The tr!("settings-index") group. Returns whether anything was drawn.
pub fn index_group(ui: &mut Ui, q: &str, d: &mut Config) -> bool {
    let p = palette(ui.ctx());
    let status = crate::services::index::status();
    group(ui, q, CTX, tr!("settings-index"), |r| {
        let ix = &mut d.search.index;
        r.row_kw(
            tr!("settings-index-enabled"),
            tr!("settings-index-enabled-desc"),
            "fts sqlite everything instant whole disk",
            |ui| {
                switch(ui, &mut ix.enabled);
            },
        );
        r.block(tr!("settings-index-status"), "", |ui| {
            ui.horizontal_wrapped(|ui| {
                match &status {
                    None if ix.enabled => {
                        ui.label(RichText::new(tr!("settings-index-starting")).color(p.text_secondary));
                    }
                    None => {
                        ui.label(RichText::new(tr!("settings-index-off")).color(p.text_secondary));
                    }
                    Some(st) => {
                        let (glyph, color) = match st.phase {
                            Phase::Idle => (ph::CHECK_CIRCLE, p.success),
                            Phase::Failed(_) => (ph::WARNING_CIRCLE, p.danger),
                            _ => (ph::CIRCLE_NOTCH, p.accent),
                        };
                        ui.label(icons::icon(glyph, 14.0).color(color));
                        ui.label(RichText::new(crate::ui::smart::status_line(st)).color(p.text_primary));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_button(ui, Some(ph::ARROWS_CLOCKWISE), tr!("settings-index-rebuild"), false)
                        .on_hover_text(tr!("settings-index-rebuild-tooltip"))
                        .clicked()
                    {
                        crate::services::index::rebuild();
                    }
                });
            });
            if let Some(st) = &status {
                let small = |s: String| RichText::new(s).size(11.5).color(p.text_tertiary);
                if let Some(cur) = &st.current {
                    ui.add(egui::Label::new(small(cur.display().to_string())).truncate());
                }
                if st.content_enabled {
                    ui.label(small(tr!("settings-index-content-files", count = st.content_files)));
                }
                if st.journal_changes > 0 {
                    ui.label(small(tr!("settings-index-journal-changes", count = st.journal_changes)));
                }
                ui.add_space(4.0);
                let (glyph, text) = usn_text(&st.usn);
                ui.horizontal_top(|ui| {
                    ui.label(icons::icon(glyph, 13.0).color(p.text_tertiary));
                    ui.add(egui::Label::new(small(text)).wrap());
                });
                for w in &st.warnings {
                    ui.horizontal_top(|ui| {
                        ui.label(icons::icon(ph::WARNING, 13.0).color(p.warning));
                        ui.add(egui::Label::new(small(w.clone())).wrap());
                    });
                }
                if let Some(t) = st.updated {
                    let ago = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
                    ui.label(small(tr!("settings-index-last-write", seconds = ago)));
                }
            }
            ui.label(
                RichText::new(tr!(
                    "settings-index-database",
                    path = crate::services::index::index_dir().display().to_string()
                ))
                .size(11.0)
                .color(p.text_tertiary),
            );
        });
        r.block(tr!("settings-index-roots"), tr!("settings-index-roots-desc"), |ui| {
            let mut remove = None;
            for (i, root) in ix.roots.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(icons::icon(ph::FOLDER_SIMPLE, 13.0).color(p.text_secondary));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if small_button(ui, Some(ph::TRASH), "", true)
                            .on_hover_text(tr!("settings-index-remove-root"))
                            .clicked()
                        {
                            remove = Some(i);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(root).monospace()).truncate()).on_hover_text(root);
                        });
                    });
                });
            }
            if let Some(i) = remove {
                ix.roots.remove(i);
            }
            let id = Id::new("soyuz.settings.index.newroot");
            let mut new_root: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
            let mut add = false;
            ui.horizontal(|ui| {
                let r = ui.add(
                    TextEdit::singleline(&mut new_root).hint_text(tr!("settings-index-root-hint")).desired_width(260.0),
                );
                add = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                add |= small_button(ui, Some(ph::PLUS), tr!("settings-index-add-root"), false).clicked();
            });
            if add && !new_root.trim().is_empty() {
                if !ix.roots.iter().any(|r| r.eq_ignore_ascii_case(new_root.trim())) {
                    ix.roots.push(new_root.trim().to_string());
                }
                new_root.clear();
            }
            ui.data_mut(|d| d.insert_temp(id, new_root));
        });
        r.block(tr!("settings-skip"), tr!("settings-index-skip-desc"), |ui| {
            list_field(ui, "exclude", &mut ix.exclude, "\n", true, "node_modules");
        });
        r.row_kw(tr!("settings-index-content"), tr!("settings-index-content-desc"), "full text grep", |ui| {
            switch(ui, &mut ix.content);
        });
        r.row(tr!("settings-index-content-limit"), tr!("settings-index-content-limit-desc"), |ui| {
            number(ui, &mut ix.content_max_kib, 16..=65_536, " KiB");
        });
        r.block(tr!("settings-index-content-types"), tr!("settings-index-content-types-desc"), |ui| {
            list_field(ui, "content_ext", &mut ix.content_extensions, ", ", false, "txt, md, rs");
        });
        r.row_kw(tr!("settings-index-usn"), tr!("settings-index-usn-desc"), "usn admin fast startup", |ui| {
            switch(ui, &mut ix.usn_journal);
        });
        r.row_kw(tr!("settings-index-throttle"), tr!("settings-index-throttle-desc"), "throttle", |ui| {
            number(ui, &mut ix.throttle_ms, 0..=200, " ms");
        });
        r.row(tr!("settings-index-smart-refresh"), tr!("settings-index-smart-refresh-desc"), |ui| {
            number(ui, &mut ix.smart_refresh_secs, 1..=120, " s");
        });
        r.row_kw(tr!("settings-index-use-for-search"), tr!("settings-index-use-for-search-desc"), "search bar", |ui| {
            switch(ui, &mut ix.use_for_search);
        });
    })
}

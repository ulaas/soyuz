//! Settings page for `[editor]`: the built-in text, hex and image editors ("Edit in
//! Soyuz", F4).

use egui::Ui;
use soyuz_core::config::EditorDock;

use super::widgets::{group, number, segmented, switch};
use crate::app::SoyuzApp;

pub fn editor(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let d = &mut app.settings.draft.editor;
    let ctx = "Editor edit in soyuz text hex image F4";
    let mut any = false;
    any |= group(ui, q, ctx, tr!("settings-editor-panel"), |r| {
        r.row_kw(tr!("settings-editor-dock"), tr!("settings-editor-dock-desc"), "position layout", |ui| {
            segmented(
                ui,
                &mut d.dock,
                &[
                    (EditorDock::Right, tr!("settings-views-activity-right")),
                    (EditorDock::Bottom, tr!("settings-views-activity-bottom")),
                ],
            );
        });
    });
    any |= group(ui, q, ctx, tr!("settings-editor-text"), |r| {
        r.row_kw(tr!("settings-terminal-font-size"), tr!("settings-editor-font-size-desc"), "zoom monospace", |ui| {
            number(ui, &mut d.font_size, 8.0..=32.0, " pt");
        });
        r.row_kw(tr!("settings-editor-tab-width"), tr!("settings-editor-tab-width-desc"), "indent spaces", |ui| {
            number(ui, &mut d.tab_width, 1..=16, "");
        });
        r.row_kw(tr!("settings-editor-wrap"), tr!("settings-editor-wrap-desc"), "word wrap", |ui| {
            switch(ui, &mut d.wrap);
        });
        r.row_kw(
            tr!("settings-editor-whitespace"),
            tr!("settings-editor-whitespace-desc"),
            "invisible characters",
            |ui| {
                switch(ui, &mut d.show_whitespace);
            },
        );
        r.row_kw(tr!("settings-editor-max-text"), tr!("settings-editor-max-text-desc"), "size limit max", |ui| {
            number(ui, &mut d.max_text_mib, 1..=512, " MiB");
        });
    });
    any |= group(ui, q, ctx, tr!("settings-editor-saving"), |r| {
        r.row_kw(tr!("settings-editor-backup"), tr!("settings-editor-backup-desc"), "backup undo bak", |ui| {
            switch(ui, &mut d.backup_to_trash);
        });
        r.row_kw(tr!("settings-editor-jpeg"), tr!("settings-editor-jpeg-desc"), "image compression", |ui| {
            number(ui, &mut d.jpeg_quality, 50..=100, "");
        });
        r.row_kw(tr!("settings-editor-max-image"), tr!("settings-editor-max-image-desc"), "megapixels limit", |ui| {
            number(ui, &mut d.max_image_mp, 10..=1000, " MP");
        });
    });
    any
}

//! Appearance page: theme, accent, density, fonts, scale, motion, icons and thumbnails,
//! a live preview card, and per-token color overrides (`[theme.colors]`).

use egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_core::SizeStyle;
use soyuz_core::config::{Backdrop, FallbackFonts, IconStyle};

use super::widgets::{color_swatch, group, number, segmented, small_button, switch, to_hex};
use crate::app::SoyuzApp;
use crate::theme::icons::ph;
use crate::theme::tokens::parse_hex;
use crate::theme::{Palette, icons, palette};

/// Token name → getter (same names as `[theme.colors]` / PLAN §1.2).
/// `(token name, label message key, getter)`.
pub type Token = (&'static str, &'static str, fn(&Palette) -> Color32);

pub const TOKENS: [Token; 33] = [
    ("bg.window", "settings-token-bg-window", |p| p.bg_window),
    ("bg.surface1", "settings-token-bg-surface1", |p| p.bg_surface1),
    ("bg.surface2", "settings-token-bg-surface2", |p| p.bg_surface2),
    ("bg.surface3", "settings-token-bg-surface3", |p| p.bg_surface3),
    ("bg.sunken", "settings-token-bg-sunken", |p| p.bg_sunken),
    ("border.subtle", "settings-token-border-subtle", |p| p.border_subtle),
    ("border.strong", "settings-token-border-strong", |p| p.border_strong),
    ("text.primary", "settings-token-text-primary", |p| p.text_primary),
    ("text.secondary", "settings-token-text-secondary", |p| p.text_secondary),
    ("text.tertiary", "settings-token-text-tertiary", |p| p.text_tertiary),
    ("accent", "settings-token-accent", |p| p.accent),
    ("accent.hover", "settings-token-accent-hover", |p| p.accent_hover),
    ("accent.subtle", "settings-token-accent-subtle", |p| p.accent_subtle),
    ("success", "settings-token-success", |p| p.success),
    ("warning", "settings-token-warning", |p| p.warning),
    ("danger", "settings-token-danger", |p| p.danger),
    ("info", "settings-token-info", |p| p.info),
    ("glyph.folder", "settings-token-glyph-folder", |p| p.glyph_folder),
    ("glyph.image", "settings-token-glyph-image", |p| p.glyph_image),
    ("glyph.video", "settings-token-glyph-video", |p| p.glyph_video),
    ("glyph.audio", "settings-token-glyph-audio", |p| p.glyph_audio),
    ("glyph.archive", "settings-token-glyph-archive", |p| p.glyph_archive),
    ("glyph.code", "settings-token-glyph-code", |p| p.glyph_code),
    ("glyph.doc", "settings-token-glyph-doc", |p| p.glyph_doc),
    ("glyph.exe", "settings-token-glyph-exe", |p| p.glyph_exe),
    ("tag.red", "settings-token-tag-red", |p| p.tags[0]),
    ("tag.orange", "settings-token-tag-orange", |p| p.tags[1]),
    ("tag.yellow", "settings-token-tag-yellow", |p| p.tags[2]),
    ("tag.green", "settings-token-tag-green", |p| p.tags[3]),
    ("tag.cyan", "settings-token-tag-cyan", |p| p.tags[4]),
    ("tag.blue", "settings-token-tag-blue", |p| p.tags[5]),
    ("tag.purple", "settings-token-tag-purple", |p| p.tags[6]),
    ("tag.gray", "settings-token-tag-gray", |p| p.tags[7]),
];

pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let s = &mut app.settings;
    let d = &mut s.draft;
    let ctx = "Appearance theme colors";
    let mut any = false;
    let mut theme_action = None;
    let mut go_fonts = false;
    let mut open_packs = false;
    if q.trim().is_empty() {
        preview_card(ui, &p, d.appearance.density.row_height(), d.appearance.icon_style == IconStyle::Colored);
        any = true;
    }
    any |= group(ui, q, ctx, tr!("settings-appearance-theme"), |r| {
        r.row_kw(
            tr!("settings-appearance-color-theme"),
            tr!("settings-appearance-color-theme-desc"),
            "theme palette nord dracula",
            |ui| super::themes_ui::theme_dropdown(ui, d),
        );
        r.block("", "", |ui| {
            if let Some(a) = super::themes_ui::gallery(ui, d, &p) {
                theme_action = Some(a);
            }
        });
        r.row_kw(
            tr!("settings-appearance-mode"),
            tr!("settings-appearance-mode-desc"),
            "dark light mode theme",
            |ui| {
                let mut v = match d.appearance.theme.to_ascii_lowercase().as_str() {
                    "dark" => 1,
                    "light" => 2,
                    _ => 0,
                };
                if segmented(
                    ui,
                    &mut v,
                    &[
                        (0, tr!("settings-appearance-mode-system")),
                        (1, tr!("settings-dark")),
                        (2, tr!("settings-light")),
                    ],
                ) {
                    d.appearance.theme = ["system", "dark", "light"][v].to_string();
                }
            },
        );
        r.row_kw(tr!("settings-appearance-accent"), tr!("settings-appearance-accent-desc"), "color colour", |ui| {
            super::themes_ui::accent_row(ui, d, &p);
        });
        r.row_kw(
            tr!("settings-appearance-reduce-motion"),
            tr!("settings-appearance-reduce-motion-desc"),
            "animation accessibility",
            |ui| {
                switch(ui, &mut d.appearance.reduced_motion);
            },
        );
        r.row_kw(
            tr!("settings-appearance-high-contrast"),
            tr!("settings-appearance-high-contrast-desc"),
            "accessibility a11y contrast vision",
            |ui| {
                switch(ui, &mut d.appearance.high_contrast);
            },
        );
    });
    any |= group(ui, q, ctx, tr!("settings-appearance-density-text"), |r| {
        r.block(tr!("settings-appearance-density"), tr!("settings-appearance-density-desc"), |ui| {
            super::density_gallery::gallery(ui, &mut d.appearance.density);
        });
        r.row_kw(tr!("settings-terminal-font-size"), tr!("settings-appearance-font-size-desc"), "text scale", |ui| {
            number(ui, &mut d.appearance.font_size, 11.0..=17.0, " pt");
        });
        r.row_kw(tr!("settings-appearance-scale"), tr!("settings-appearance-scale-desc"), "ui scale zoom dpi", |ui| {
            let mut v = s.scale_temp.unwrap_or(d.appearance.ui_scale);
            let resp = number(ui, &mut v, 0.5..=3.0, "×");
            if resp.dragged() {
                s.scale_temp = Some(v);
            } else {
                if resp.changed() || s.scale_temp.is_some() {
                    d.appearance.ui_scale = (v * 20.0).round() / 20.0;
                }
                s.scale_temp = None;
            }
        });
        r.row_kw(
            tr!("settings-preview-fonts"),
            tr!("settings-appearance-fonts-desc"),
            "typeface family segoe consolas monospace",
            |ui| {
                if small_button(ui, Some(ph::TEXT_AA), tr!("settings-appearance-fonts-button"), false).clicked() {
                    go_fonts = true;
                }
            },
        );
        r.row_kw(
            tr!("settings-appearance-fallback"),
            tr!("settings-appearance-fallback-desc"),
            "cjk chinese japanese korean emoji tofu glyph unicode font fallback",
            |ui| {
                segmented(
                    ui,
                    &mut d.appearance.fallback_fonts,
                    &[
                        (FallbackFonts::Auto, tr!("settings-automatic")),
                        (FallbackFonts::Startup, tr!("settings-appearance-fallback-startup")),
                        (FallbackFonts::Off, tr!("settings-off")),
                    ],
                );
            },
        );
    });
    any |= group(ui, q, ctx, tr!("settings-appearance-icons"), |r| {
        r.row(tr!("settings-appearance-file-icons"), tr!("settings-appearance-file-icons-desc"), |ui| {
            segmented(
                ui,
                &mut d.appearance.icon_style,
                &[
                    (IconStyle::Colored, tr!("settings-appearance-colored")),
                    (IconStyle::Mono, tr!("settings-appearance-mono")),
                ],
            );
        });
        r.block(tr!("settings-appearance-icon-pack"), tr!("settings-appearance-icon-pack-desc"), |ui| {
            crate::ui::views::icon_pack::scan(ui.ctx());
            let mut packs = crate::ui::views::icon_pack::available();
            if !packs.iter().any(|(id, _)| *id == d.appearance.icon_pack) {
                packs.push((d.appearance.icon_pack.clone(), d.appearance.icon_pack.clone()));
            }
            let options: Vec<(String, &str)> = packs.iter().map(|(id, name)| (id.clone(), name.as_str())).collect();
            ui.horizontal(|ui| {
                super::widgets::dropdown(ui, "soyuz.settings.icon_pack", &mut d.appearance.icon_pack, &options, 220.0);
                if small_button(ui, Some(ph::FOLDER_OPEN), tr!("settings-appearance-icon-pack-folder"), false).clicked()
                {
                    open_packs = true;
                }
            });
            ui.add_space(6.0);
            icon_pack_strip(ui, &p);
        });
        r.row_kw(tr!("settings-appearance-badges"), tr!("settings-appearance-badges-desc"), "icon label type", |ui| {
            switch(ui, &mut d.appearance.extension_badges);
        });
        r.row_kw(tr!("settings-appearance-thumbs"), tr!("settings-appearance-thumbs-desc"), "preview pictures", |ui| {
            switch(ui, &mut d.appearance.show_thumbnails);
        });
        r.row_kw(
            tr!("settings-appearance-folder-thumbs"),
            tr!("settings-appearance-folder-thumbs-desc"),
            "mosaic folder preview",
            |ui| {
                switch(ui, &mut d.appearance.folder_thumbnails);
            },
        );
        let ff = crate::services::thumb_video::ffmpeg_path();
        let video_desc = match &ff {
            Some(p) => tr!("settings-appearance-video-thumbs-desc", path = p.display().to_string()),
            None => tr!("settings-appearance-video-thumbs-missing").to_string(),
        };
        r.row_kw(tr!("settings-appearance-video-thumbs"), &video_desc, "ffmpeg movie mp4 frame", |ui| {
            switch(ui, &mut d.appearance.video_thumbnails);
        });
        r.row_kw(
            tr!("settings-appearance-pdf-thumbs"),
            tr!("settings-appearance-pdf-thumbs-desc"),
            "pdf pdfium page document",
            |ui| {
                switch(ui, &mut d.appearance.pdf_thumbnails);
            },
        );
        if cfg!(windows) {
            r.row_kw(
                tr!("settings-appearance-os-thumbs"),
                tr!("settings-appearance-os-thumbs-desc"),
                "thumbcache explorer windows cache video",
                |ui| {
                    switch(ui, &mut d.appearance.os_thumbnail_cache);
                },
            );
            r.row_kw(
                tr!("settings-appearance-startup-placeholder"),
                tr!("settings-appearance-startup-placeholder-desc"),
                "splash startup launch window fast",
                |ui| {
                    switch(ui, &mut d.appearance.startup_placeholder);
                },
            );
            r.row_kw(
                tr!("settings-appearance-cpu-first-frame"),
                tr!("settings-appearance-cpu-first-frame-desc"),
                "startup launch fast cpu render first frame performance",
                |ui| {
                    switch(ui, &mut d.performance.cpu_first_frame);
                },
            );
        }
        r.row_kw(
            tr!("settings-appearance-gpu-backend"),
            tr!("settings-appearance-gpu-backend-desc"),
            "renderer graphics gpu direct3d dx12 vulkan opengl wgpu driver",
            |ui| {
                use soyuz_core::config::GpuBackend;
                let auto_desc = if cfg!(windows) { "Direct3D 12" } else { "" };
                let items = [
                    super::widgets::dd::Item::new(GpuBackend::Auto, tr!("settings-automatic"))
                        .secondary(auto_desc)
                        .icon(ph::MAGIC_WAND),
                    super::widgets::dd::Item::new(GpuBackend::Dx12, "Direct3D 12")
                        .icon(ph::WINDOWS_LOGO)
                        .disabled((!cfg!(windows)).then(|| tr!("settings-appearance-gpu-windows-only").to_string())),
                    super::widgets::dd::Item::new(GpuBackend::Vulkan, "Vulkan").icon(ph::GRAPHICS_CARD),
                    super::widgets::dd::Item::new(GpuBackend::Gl, "OpenGL")
                        .secondary(tr!("settings-appearance-gpu-compat"))
                        .icon(ph::CUBE),
                ];
                super::widgets::choice(ui, "gpu_backend", &mut d.performance.gpu_backend, &items, 190.0);
            },
        );
        r.row_kw(
            tr!("settings-appearance-thumb-cache"),
            tr!("settings-appearance-thumb-cache-desc"),
            "cache thumbs disk storage",
            |ui| {
                if small_button(ui, Some(ph::TRASH), tr!("settings-clear"), false).clicked() {
                    crate::services::thumb_disk::clear();
                }
                number(ui, &mut d.appearance.thumb_disk_cache_mib, 0..=4096, " MiB");
            },
        );
        r.row_kw(
            tr!("settings-appearance-thumb-size"),
            tr!("settings-appearance-thumb-size-desc"),
            "icon size zoom",
            |ui| {
                number(ui, &mut d.appearance.thumbnail_size, 32..=256, " px");
            },
        );
        r.row(tr!("settings-appearance-sidebar-width"), "", |ui| {
            number(ui, &mut d.appearance.sidebar_width, 160.0..=420.0, " px");
        });
    });
    any |= group(ui, q, ctx, tr!("settings-views-new-window-window"), |r| {
        let desc = if crate::ui::titlebar::Chrome::live_switch() {
            tr!("settings-appearance-titlebar-desc")
        } else {
            tr!("settings-appearance-titlebar-desc-restart")
        };
        r.row_kw(tr!("settings-appearance-titlebar"), desc, "titlebar window frame decorations chrome caption", |ui| {
            switch(ui, &mut d.appearance.custom_titlebar);
        });
        r.row_kw(
            tr!("settings-appearance-backdrop"),
            tr!("settings-appearance-backdrop-desc"),
            "mica acrylic translucent transparent blur material",
            |ui| {
                segmented(
                    ui,
                    &mut d.appearance.backdrop,
                    &[(Backdrop::None, tr!("settings-none")), (Backdrop::Mica, "Mica"), (Backdrop::Acrylic, "Acrylic")],
                );
            },
        );
        r.row_kw(
            tr!("settings-appearance-snap"),
            tr!("settings-appearance-snap-desc"),
            "snap maximize layouts flyout",
            |ui| {
                switch(ui, &mut d.appearance.snap_layouts);
            },
        );
        r.row_kw(
            tr!("settings-appearance-native-dialogs"),
            tr!("settings-appearance-native-dialogs-desc"),
            "native dialogs windows popup menus monitor outside clipped",
            |ui| {
                switch(ui, &mut d.appearance.native_dialogs);
            },
        );
    });
    any |= group(ui, q, ctx, tr!("settings-appearance-theme-colors"), |r| {
        r.block("", tr!("settings-appearance-theme-colors-desc"), |ui| {
            let mut reset_all = false;
            ui.horizontal(|ui| {
                let n = d.theme.colors.len();
                ui.label(
                    RichText::new(tr!("settings-appearance-overrides", count = n)).size(11.5).color(p.text_tertiary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if n > 0
                        && small_button(
                            ui,
                            Some(ph::ARROW_COUNTER_CLOCKWISE),
                            tr!("settings-appearance-reset-colors"),
                            true,
                        )
                        .clicked()
                    {
                        reset_all = true;
                    }
                });
            });
            ui.add_space(6.0);
            let cols = ((ui.available_width() / 250.0).floor() as usize).max(1);
            egui::Grid::new("soyuz.settings.tokens").num_columns(cols).spacing(vec2(18.0, 6.0)).show(ui, |ui| {
                for (i, (name, label, get)) in TOKENS.iter().enumerate() {
                    let overridden = d.theme.colors.contains_key(*name);
                    let mut c = d.theme.colors.get(*name).and_then(|h| parse_hex(h)).unwrap_or_else(|| get(&p));
                    ui.horizontal(|ui| {
                        ui.set_min_width(230.0);
                        if color_swatch(ui, &mut c, overridden, crate::i18n::t_dyn(label)) {
                            d.theme.colors.insert(name.to_string(), to_hex(c));
                        }
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(crate::i18n::t_dyn(label)).size(12.0).color(p.text_primary));
                            ui.label(RichText::new(*name).size(10.5).monospace().color(p.text_tertiary));
                        });
                        if overridden
                            && small_button(ui, Some(ph::X), "", false)
                                .on_hover_text(tr!("settings-appearance-token-reset"))
                                .clicked()
                        {
                            d.theme.colors.remove(*name);
                        }
                    });
                    if (i + 1) % cols == 0 {
                        ui.end_row();
                    }
                }
            });
            if reset_all {
                d.theme.colors.clear();
            }
        });
    });
    match theme_action {
        Some(super::themes_ui::ThemeAction::Import) => super::start_theme_import(app),
        Some(super::themes_ui::ThemeAction::Export) => super::start_theme_export(app),
        Some(super::themes_ui::ThemeAction::OpenFolder) => {
            let dir = crate::theme::library::themes_dir();
            let _ = std::fs::create_dir_all(&dir);
            app.cmds.push(crate::cmd::Cmd::OpenInNewTab { location: soyuz_core::Location::Dir(dir), activate: true });
        }
        None => {}
    }
    if open_packs {
        let dir = crate::ui::views::icon_pack::packs_dir();
        let _ = std::fs::create_dir_all(&dir);
        app.cmds.push(crate::cmd::Cmd::OpenInNewTab { location: soyuz_core::Location::Dir(dir), activate: true });
    }
    if go_fonts {
        app.settings.section = super::Section::Fonts;
        app.settings.scroll_top = true;
    }
    any
}

/// Sample icons of the active icon pack (glyphs where it has none): the live preview.
fn icon_pack_strip(ui: &mut Ui, p: &Palette) {
    use crate::ui::views::icons as vicons;
    use soyuz_core::{EntryInfo, EntryKind};
    let samples = [
        ("Projects", EntryKind::Dir),
        ("photo.jpg", EntryKind::File),
        ("clip.mp4", EntryKind::File),
        ("song.flac", EntryKind::File),
        ("main.rs", EntryKind::File),
        ("report.pdf", EntryKind::File),
        ("notes.docx", EntryKind::File),
        ("budget.xlsx", EntryKind::File),
        ("backup.zip", EntryKind::File),
        ("setup.exe", EntryKind::File),
        ("readme.txt", EntryKind::File),
    ];
    let cell = 46.0;
    let (rect, _) = ui.allocate_exact_size(vec2(cell * samples.len() as f32, cell + 14.0), Sense::hover());
    let painter = ui.painter_at(rect);
    for (i, (name, kind)) in samples.iter().enumerate() {
        let e = EntryInfo::minimal(std::path::PathBuf::from(name), *kind);
        let c = pos2(rect.left() + cell * (i as f32 + 0.5), rect.top() + cell / 2.0);
        if !crate::ui::views::icon_pack::paint(&painter, c, 32.0, &e, p, 1.0) {
            let font = if e.is_dir_like() { icons::font_fill(28.0) } else { icons::font(28.0) };
            painter.text(c, Align2::CENTER_CENTER, vicons::glyph(&e), font, vicons::color(&e, p));
        }
        let ext = name.rsplit_once('.').map_or("", |(_, x)| x);
        painter.text(
            pos2(c.x, rect.bottom() - 6.0),
            Align2::CENTER_CENTER,
            ext,
            FontId::proportional(10.0),
            p.text_tertiary,
        );
    }
}

/// A miniature window rendered with the live palette.
fn preview_card(ui: &mut Ui, p: &Palette, row_h: f32, colored: bool) {
    ui.add_space(14.0);
    let w = ui.available_width();
    let h = 64.0 + row_h * 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let painter = ui.painter();
    let r10 = CornerRadius::same(10);
    painter.rect_filled(rect, r10, p.bg_window);
    painter.rect_stroke(rect, r10, Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
    // Sidebar.
    let side = Rect::from_min_size(rect.min, vec2(150.0, h));
    painter.rect_filled(side, CornerRadius { nw: 10, sw: 10, ne: 0, se: 0 }, p.bg_surface1);
    for (i, (g, label, active)) in [
        (ph::HOUSE, tr!("settings-appearance-preview-home"), false),
        (ph::FOLDER_SIMPLE, tr!("settings-appearance-preview-projects"), true),
        (ph::DOWNLOAD_SIMPLE, tr!("settings-appearance-preview-downloads"), false),
    ]
    .iter()
    .enumerate()
    {
        let rr = Rect::from_min_size(pos2(side.left() + 8.0, side.top() + 14.0 + i as f32 * 30.0), vec2(134.0, 26.0));
        if *active {
            painter.rect_filled(rr, CornerRadius::same(6), p.accent_subtle);
        }
        let gfont = if *active { icons::font_fill(13.0) } else { icons::font(13.0) };
        painter.text(
            pos2(rr.left() + 10.0, rr.center().y),
            Align2::LEFT_CENTER,
            *g,
            gfont,
            if *active { p.accent } else { p.text_secondary },
        );
        painter.text(
            pos2(rr.left() + 30.0, rr.center().y),
            Align2::LEFT_CENTER,
            *label,
            FontId::proportional(12.0),
            p.text_primary,
        );
    }
    // Toolbar + breadcrumb.
    let main = Rect::from_min_max(pos2(side.right(), rect.top()), rect.max);
    let crumb =
        Rect::from_min_size(pos2(main.left() + 12.0, main.top() + 10.0), vec2((main.width() - 24.0).max(40.0), 26.0));
    painter.rect_filled(crumb, CornerRadius::same(6), p.bg_surface2);
    painter.text(
        pos2(crumb.left() + 10.0, crumb.center().y),
        Align2::LEFT_CENTER,
        tr!("settings-appearance-preview-crumb"),
        FontId::proportional(12.0),
        p.text_secondary,
    );
    // Rows.
    let rows = [
        (ph::FOLDER_SIMPLE, "src", p.glyph_folder, "—".to_string(), true),
        (ph::IMAGE, "screenshot.png", p.glyph_image, soyuz_core::format_size(1_468_006, SizeStyle::Binary), false),
        (ph::FILE_CODE, "main.rs", p.glyph_code, soyuz_core::format_size(12 * 1024, SizeStyle::Binary), false),
        (ph::FILE_TEXT, "README.md", p.glyph_doc, soyuz_core::format_size(3 * 1024, SizeStyle::Binary), false),
    ];
    for (i, (g, name, color, size, selected)) in rows.iter().enumerate() {
        let rr = Rect::from_min_size(
            pos2(main.left() + 12.0, main.top() + 46.0 + i as f32 * row_h),
            vec2(main.width() - 24.0, row_h - 2.0),
        );
        if *selected {
            painter.rect_filled(rr, CornerRadius::same(6), p.accent_subtle);
            painter.rect_stroke(
                rr,
                CornerRadius::same(6),
                Stroke::new(1.0, p.accent.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
        }
        let gc = if colored { *color } else { p.text_secondary };
        painter.text(pos2(rr.left() + 12.0, rr.center().y), Align2::LEFT_CENTER, *g, icons::font(15.0), gc);
        painter.text(
            pos2(rr.left() + 36.0, rr.center().y),
            Align2::LEFT_CENTER,
            *name,
            FontId::proportional(13.0),
            p.text_primary,
        );
        painter.text(
            pos2(rr.right() - 12.0, rr.center().y),
            Align2::RIGHT_CENTER,
            size.as_str(),
            FontId::proportional(12.0),
            p.text_secondary,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_theme_overrides() {
        // Every token listed here is accepted by `Palette::apply_overrides`.
        let mut pal = crate::theme::ORBIT_DARK.clone();
        let map = TOKENS.iter().map(|(n, _, _)| (n.to_string(), "#123456".to_string())).collect();
        assert!(pal.apply_overrides(&map).is_empty());
        for (_, key, _) in TOKENS {
            for l in [soyuz_core::Locale::EnUs, soyuz_core::Locale::TrTr] {
                assert!(crate::i18n::has_in(key, l), "{key}");
            }
        }
        for (_, _, get) in TOKENS {
            assert_eq!(get(&pal), Color32::from_rgb(0x12, 0x34, 0x56));
        }
    }
}

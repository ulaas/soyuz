//! Settings → Preview & search → Media: the `[media]` keys the app reads today (the
//! FFmpeg libraries switch, the folder searched first, decoder threads, the frame size and
//! the poster time), the audio decoder and the player keys (player, autoplay, loop,
//! volume, muted, seek step) under a status card that says which libraries are loaded, or
//! where they were looked for and where to get them: the `ffmpeg-runtime-<version>`
//! release, which "Download FFmpeg runtime" fetches, verifies and loads in place (V4c,
//! `services::updates::ffmpeg_runtime`; not offered inside Flatpak / MSIX). Also the tools
//! switch, hardware decoding and GPU colour conversion.

use std::path::Path;

use egui::{Align2, Ui, pos2, vec2};
use soyuz_core::config::{AudioBackend, HwDecode};

use super::widgets::{group, number, segmented, small_button, switch, text};
use crate::app::SoyuzApp;
use crate::services::media::{self, State};
use crate::services::updates::ffmpeg_runtime::{self as runtime_dl, Offer, Phase as DlPhase};
use crate::theme::icons::ph;
use crate::theme::palette;

const CTX: &str = "Preview media ffmpeg libraries video";

pub fn media(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let status = media::status();
    let dl = runtime_dl::phase();
    let offered = runtime_dl::offer() == Offer::Offered;
    let repo = app.core.config.updates.repo.clone();
    let mut retry = false;
    let mut browse = false;
    let mut download = false;
    let mut dl_action = None;
    let d = &mut app.gui.settings.draft;
    let any = group(ui, q, CTX, tr!("settings-media"), |r| {
        let (desc, keywords) = status_desc(&status);
        r.row_kw(tr!("settings-media-status"), &desc, &keywords, |ui| match &status.state {
            State::Loaded(_) => status_pill(ui, ph::CHECK_CIRCLE, tr!("settings-media-status-ok"), true),
            State::Loading => status_pill(ui, ph::CIRCLE_DASHED, tr!("settings-media-status-loading-short"), false),
            State::Disabled => status_pill(ui, ph::PROHIBIT, tr!("settings-off"), false),
            State::Failed { .. } => {
                if offered && !dl.busy() {
                    download = download_button(ui, &repo);
                }
                retry = small_button(ui, Some(ph::ARROWS_CLOCKWISE), tr!("settings-media-retry"), false).clicked();
                status_pill(ui, ph::WARNING_CIRCLE, tr!("settings-media-status-missing-short"), false);
            }
        });
        let loaded = matches!(status.state, State::Loaded(_));
        if !matches!(dl, DlPhase::Idle) && !(loaded && matches!(dl, DlPhase::Installed(_))) {
            r.block("", "", |ui| dl_action = download_status(ui, &dl));
        }
        r.row_kw(tr!("settings-media-ffmpeg"), tr!("settings-media-ffmpeg-desc"), "enable libraries runtime", |ui| {
            switch(ui, &mut d.media.ffmpeg);
        });
        let (dir_desc, dir_hint) = if cfg!(windows) {
            (tr!("settings-media-dir-desc"), tr!("settings-media-dir-hint"))
        } else {
            (tr!("settings-media-dir-desc-unix"), tr!("settings-media-dir-hint-unix"))
        };
        r.row_kw(tr!("settings-media-dir"), dir_desc, "folder path dll so dylib avcodec", |ui| {
            browse = small_button(ui, Some(ph::FOLDER_OPEN), tr!("settings-media-browse"), false).clicked();
            text(ui, &mut d.media.ffmpeg_dir, dir_hint, 200.0);
        });
        r.row_kw(tr!("settings-media-threads"), tr!("settings-media-threads-desc"), "decoder cores parallel", |ui| {
            number(ui, &mut d.media.threads, 0..=64, "");
        });
        r.row_kw(
            tr!("settings-media-max-px"),
            tr!("settings-media-max-px-desc"),
            "resolution poster frame size",
            |ui| {
                number(ui, &mut d.media.max_px, 256..=4096, " px");
            },
        );
        r.row_kw(tr!("settings-media-poster"), tr!("settings-media-poster-desc"), "thumbnail frame seconds", |ui| {
            number(ui, &mut d.media.poster_secs, 0.0..=3600.0, " s");
        });
        r.row_kw(
            tr!("settings-media-audio-backend"),
            tr!("settings-media-audio-backend-desc"),
            "symphonia decoder opus wma ape waveform",
            |ui| {
                segmented(
                    ui,
                    &mut d.media.audio_backend,
                    &[
                        (AudioBackend::Auto, tr!("settings-media-backend-auto")),
                        (AudioBackend::Ffmpeg, "FFmpeg"),
                        (AudioBackend::Symphonia, "Symphonia"),
                    ],
                );
            },
        );
        r.row_kw(tr!("settings-media-player"), tr!("settings-media-player-desc"), "video playback quick look", |ui| {
            switch(ui, &mut d.media.player);
        });
        r.row_kw(
            tr!("settings-media-autoplay"),
            tr!("settings-media-autoplay-desc"),
            "start automatically play",
            |ui| {
                switch(ui, &mut d.media.autoplay);
            },
        );
        r.row_kw(
            tr!("settings-media-ql-space"),
            tr!("settings-media-ql-space-desc"),
            "quick look space bar play pause keyboard",
            |ui| {
                switch(ui, &mut d.media.quicklook_space_plays);
            },
        );
        r.row_kw(tr!("settings-media-loop"), tr!("settings-media-loop-desc"), "repeat restart", |ui| {
            switch(ui, &mut d.media.loop_);
        });
        r.row_kw(tr!("settings-media-volume"), tr!("settings-media-volume-desc"), "loudness percent audio", |ui| {
            number(ui, &mut d.media.volume, 0..=100, " %");
        });
        r.row_kw(tr!("settings-media-muted"), tr!("settings-media-muted-desc"), "mute silent audio", |ui| {
            switch(ui, &mut d.media.muted);
        });
        r.row_kw(
            tr!("settings-media-seek-step"),
            tr!("settings-media-seek-step-desc"),
            "skip arrow keys seconds",
            |ui| {
                number(ui, &mut d.media.seek_step_secs, 0.5..=60.0, " s");
            },
        );
        r.row_kw(
            tr!("settings-media-tui-fps"),
            tr!("settings-media-tui-fps-desc"),
            "terminal tui frames per second video",
            |ui| {
                number(ui, &mut d.media.tui_fps, 1..=30, " fps");
            },
        );
        r.row_kw(
            tr!("settings-media-tools"),
            tr!("settings-media-tools-desc"),
            "convert extract audio trim frame ffmpeg menu",
            |ui| {
                switch(ui, &mut d.media.tools);
            },
        );
        r.row_kw(
            tr!("settings-media-hw-decode"),
            tr!("settings-media-hw-decode-desc"),
            "hardware gpu decoding d3d11va vaapi videotoolbox acceleration",
            |ui| {
                let native = if cfg!(windows) {
                    (HwDecode::D3d11va, "D3D11VA")
                } else if cfg!(target_os = "macos") {
                    (HwDecode::Videotoolbox, "VideoToolbox")
                } else {
                    (HwDecode::Vaapi, "VAAPI")
                };
                segmented(
                    ui,
                    &mut d.media.hw_decode,
                    &[
                        (HwDecode::Auto, tr!("settings-media-backend-auto")),
                        (HwDecode::Off, tr!("settings-off")),
                        native,
                    ],
                );
            },
        );
        r.row_kw(
            tr!("settings-media-gpu-yuv"),
            tr!("settings-media-gpu-yuv-desc"),
            "gpu yuv colour color conversion hdr tone mapping shader",
            |ui| {
                switch(ui, &mut d.media.gpu_yuv);
            },
        );
        r.row_kw(
            tr!("settings-media-faststart"),
            tr!("settings-media-faststart-desc"),
            "faststart moov index mp4 mov m4a streaming web",
            |ui| {
                switch(ui, &mut d.media.faststart);
            },
        );
    });
    if retry {
        media::retry();
    }
    if download && let Err(e) = runtime_dl::start(&repo, crate::host::egui_wake(ui.ctx())) {
        app.core.ui.toasts.push(crate::models::toasts::Toast::warning(tr!("services-ffmpeg-dl-cannot")).body(e));
    }
    match dl_action {
        Some(DlAction::Cancel) => runtime_dl::cancel(),
        Some(DlAction::Dismiss) => runtime_dl::dismiss(),
        None => {}
    }
    if browse {
        start_pick(app);
    }
    any
}

/// The status row's description and search keywords.
fn status_desc(status: &media::Status) -> (String, String) {
    let searched = || {
        status
            .dirs
            .iter()
            .map(|d| {
                if d.as_os_str().is_empty() {
                    tr!("settings-media-loader-path").to_string()
                } else {
                    soyuz_fs::strip_verbatim(d).display().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    match &status.state {
        State::Disabled => (tr!("settings-media-status-off").to_string(), String::new()),
        State::Loading => (tr!("settings-media-status-loading").to_string(), String::new()),
        State::Loaded(i) => {
            let mut desc = tr!(
                "settings-media-status-loaded",
                version = i.version_line(),
                dir = soyuz_fs::strip_verbatim(&i.dir).display().to_string(),
                decoders = i.decoders,
                encoders = i.encoders
            );
            if i.versions.gpl {
                desc.push('\n');
                desc.push_str(tr!("settings-media-status-gpl"));
            }
            (desc, i.version_line())
        }
        State::Failed { reason, .. } => {
            let dir = soyuz_fs::strip_verbatim(&soyuz_core::config::config_dir().join("ffmpeg"));
            let desc = format!(
                "{}\n{}\n{}",
                tr!("settings-media-status-missing", reason = reason.as_str()),
                tr!("settings-media-status-get", tag = media::runtime_release_tag(), dir = dir.display().to_string()),
                tr!("settings-media-status-searched", dirs = searched())
            );
            (desc, "missing not found download".into())
        }
    }
}

/// "Download FFmpeg runtime", disabled without an update repository (the download uses
/// the update check's GitHub repository, and only then). Returns whether it was clicked.
fn download_button(ui: &mut Ui, repo: &str) -> bool {
    let tag = media::runtime_release_tag();
    let name = runtime_dl::asset_name(&tag, std::env::consts::OS, std::env::consts::ARCH).unwrap_or_default();
    let dir = soyuz_fs::strip_verbatim(&soyuz_core::config::config_dir().join(runtime_dl::DIR_NAME));
    let can = !repo.trim().is_empty();
    ui.add_enabled_ui(can, |ui| {
        small_button(ui, Some(ph::DOWNLOAD_SIMPLE), tr!("settings-media-download"), false)
            .on_hover_text(tr!(
                "settings-media-download-tooltip",
                name = name.as_str(),
                tag = tag.as_str(),
                repo = repo.trim(),
                dir = dir.display().to_string()
            ))
            .on_disabled_hover_text(tr!("settings-media-download-no-repo"))
            .clicked()
    })
    .inner
}

enum DlAction {
    Cancel,
    Dismiss,
}

/// The download's progress, result or error under the status row.
fn download_status(ui: &mut Ui, phase: &DlPhase) -> Option<DlAction> {
    let p = palette(ui.ctx());
    let mut action = None;
    let line = |ui: &mut Ui, text: &str, color| {
        ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(color)).wrap());
    };
    let busy = |ui: &mut Ui, text: String, frac: Option<f32>, action: &mut Option<DlAction>| {
        line(ui, &text, p.text_secondary);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 110.0).max(80.0);
            progress_bar(ui, w, frac, &text);
            if small_button(ui, Some(ph::X_CIRCLE), tr!("settings-cancel"), true).clicked() {
                *action = Some(DlAction::Cancel);
            }
        });
    };
    match phase {
        DlPhase::Idle => {}
        DlPhase::Fetching => {
            let text = tr!("settings-media-download-fetching", tag = media::runtime_release_tag());
            busy(ui, text, None, &mut action);
        }
        DlPhase::Downloading { asset, got, total } => {
            let text = tr!(
                "settings-media-download-progress",
                name = asset.as_str(),
                done = mib(*got),
                total = if *total > 0 { mib(*total) } else { "?".into() }
            );
            let frac = (*total > 0).then(|| (*got as f32 / *total as f32).clamp(0.0, 1.0));
            busy(ui, text, frac, &mut action);
        }
        DlPhase::Installing => busy(ui, tr!("settings-media-download-installing").to_string(), None, &mut action),
        DlPhase::Installed(dir) => {
            let dir = soyuz_fs::strip_verbatim(dir).display().to_string();
            line(ui, &tr!("settings-media-download-installed", dir = dir), p.success);
        }
        DlPhase::Failed(e) => {
            line(ui, &tr!("settings-media-download-failed", error = e.as_str()), p.danger);
            ui.add_space(4.0);
            if small_button(ui, Some(ph::X), tr!("settings-media-download-dismiss"), false).clicked() {
                action = Some(DlAction::Dismiss);
            }
        }
    }
    action
}

/// A thin determinate (`Some`) or indeterminate bar in the accent colour, announced as a
/// progress indicator named `label`.
fn progress_bar(ui: &mut Ui, width: f32, frac: Option<f32>, label: &str) {
    let p = palette(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(width, 4.0), egui::Sense::hover());
    ui.painter().rect_filled(r, egui::CornerRadius::same(2), p.border_subtle);
    let fill = match frac {
        Some(f) => egui::Rect::from_min_size(r.min, vec2((r.width() * f).max(3.0), r.height())),
        None => {
            let t = ui.input(|i| i.time) as f32;
            let w = r.width() * 0.3;
            let x = r.left() + (t * 0.8).fract() * (r.width() + w) - w;
            ui.ctx().request_repaint();
            egui::Rect::from_min_max(pos2(x.max(r.left()), r.top()), pos2((x + w).min(r.right()), r.bottom()))
        }
    };
    if fill.width() > 0.0 {
        ui.painter().rect_filled(fill, egui::CornerRadius::same(2), p.accent);
    }
    crate::ui::a11y::progress(ui, ui.scope_id().with("ffmpeg-download"), r, label, frac);
}

fn mib(b: u64) -> String {
    format!("{} MiB", soyuz_core::locale::format_decimal(b as f64 / (1024.0 * 1024.0), 1))
}

/// A small coloured status label (icon + text) in the control column.
fn status_pill(ui: &mut Ui, icon: &str, label: &str, ok: bool) {
    let p = palette(ui.ctx());
    let font = egui::FontId::proportional(12.0);
    let g = ui.painter().layout_no_wrap(label.to_string(), font.clone(), p.text_primary);
    let w = g.size().x + 34.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), egui::Sense::hover());
    let (fill, fg) = if ok { (p.success.gamma_multiply(0.16), p.success) } else { (p.bg_surface3, p.text_secondary) };
    ui.painter().rect_filled(rect, egui::CornerRadius::same(12), fill);
    ui.painter().text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::CENTER_CENTER,
        icon,
        crate::theme::icons::font(13.0),
        fg,
    );
    ui.painter().galley(pos2(rect.left() + 26.0, rect.center().y - g.size().y / 2.0), g, fg);
    crate::ui::a11y::name(&resp, label);
}

/// Open the in-app picker on the library file; its folder becomes `[media] ffmpeg_dir`.
fn start_pick(app: &mut SoyuzApp) {
    use crate::ui::path_picker::{PathPicker, PickMode};
    let configured = app.gui.settings.draft.media.ffmpeg_dir.trim();
    let start = if !configured.is_empty() && Path::new(configured).is_dir() {
        Path::new(configured).to_path_buf()
    } else {
        std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(soyuz_fs::home)
    };
    // The file names of the bundled row: avcodec-63.dll / libavcodec.so.63 / libavcodec.63.dylib.
    let names = media::lib_file_names(media::Majors::BUNDLED);
    let exts: Vec<String> = media::Majors::SUPPORTED
        .iter()
        .flat_map(|m| media::lib_file_names(*m))
        .filter_map(|n| Path::new(&n).extension().map(|e| e.to_string_lossy().into_owned()))
        .collect();
    let exts: Vec<&str> = exts.iter().map(String::as_str).collect();
    let title = tr!("settings-media-pick-title", file = names[3].as_str());
    let picker = PathPicker::new(&title, PickMode::Open, start, "", &exts);
    app.gui.settings.picker = Some((super::PickPurpose::FfmpegDir, picker));
}

/// The picker's result: the folder of the chosen file.
pub(super) fn picked(app: &mut SoyuzApp, path: &Path) {
    let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
    app.gui.settings.draft.media.ffmpeg_dir = soyuz_fs::strip_verbatim(dir).display().to_string();
}

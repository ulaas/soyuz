//! Settings pages for OS integration: "About & updates" (manual update check
//! against GitHub Releases) and the "Default file manager" opt-in at the top of the General page.
//! Registry and network work runs on worker threads; the UI only reads snapshots.

use std::path::PathBuf;

use egui::{RichText, Ui};
use parking_lot::Mutex;
use soyuz_core::Location;
use soyuz_fs::integration::{self, AssocKeys, FileManagerStatus, Signature};

use super::widgets::{group, small_button, switch, text};
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::services::updates::{self, Phase};
use crate::theme::icons::ph;
use crate::theme::palette;
use crate::ui::toasts::Toast;

const CTX_ABOUT: &str = "About updates version";
const CTX_FM: &str = "General default file manager explorer folders drives integration os desktop";

fn mib(b: u64) -> String {
    format!("{} MiB", soyuz_core::locale::format_decimal(b as f64 / (1024.0 * 1024.0), 1))
}

/// Settings → About & updates.
pub fn about(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let mut any = false;
    let mut open_about = false;
    any |= group(ui, q, CTX_ABOUT, "Soyuz", |r| {
        r.row_kw(
            &tr!("settings-about-version", version = updates::CURRENT.to_string()),
            &tr!(
                "settings-about-build",
                kind = if cfg!(debug_assertions) { "debug" } else { "release" },
                os = std::env::consts::OS,
                arch = std::env::consts::ARCH
            ),
            "license build platform",
            |ui| {
                open_about = small_button(ui, Some(ph::INFO), tr!("settings-about-button"), false).clicked();
            },
        );
    });
    if open_about {
        app.cmds.push(Cmd::Action(soyuz_core::config::Action::About));
    }

    let phase = updates::phase();
    let busy = matches!(phase, Phase::Checking | Phase::Downloading { .. });
    let mut check = false;
    let d = &mut app.settings.draft;
    let repo_before = d.updates.repo.clone();
    any |= group(ui, q, CTX_ABOUT, tr!("settings-updates"), |r| {
        r.row_kw(tr!("settings-updates-repo"), tr!("settings-updates-repo-desc"), "github check url repo", |ui| {
            text(ui, &mut d.updates.repo, "owner/soyuz", 220.0);
        });
        r.row_kw(
            tr!("settings-updates-weekly"),
            tr!("settings-updates-weekly-desc"),
            "automatic weekly background",
            |ui| {
                switch(ui, &mut d.updates.weekly_check);
            },
        );
        r.row_kw(
            tr!("settings-updates-prereleases"),
            tr!("settings-updates-prereleases-desc"),
            "beta rc prerelease",
            |ui| {
                switch(ui, &mut d.updates.include_prereleases);
            },
        );
        r.block("", tr!("settings-updates-privacy"), |ui| {
            ui.horizontal(|ui| {
                let can = !d.updates.repo.trim().is_empty() && !busy;
                ui.add_enabled_ui(can, |ui| {
                    check = small_button(ui, Some(ph::ARROW_CLOCKWISE), tr!("settings-updates-check"), false).clicked();
                });
                if d.updates.repo.trim().is_empty() {
                    ui.label(RichText::new(tr!("settings-updates-no-repo")).size(11.5).color(p.text_tertiary));
                } else if let Err(e) = updates::parse_repo(&d.updates.repo) {
                    ui.label(RichText::new(e).size(11.5).color(p.warning));
                }
            });
        });
    });
    if d.updates.repo != repo_before {
        updates::reset();
    }
    if check {
        updates::check(d.updates.repo.clone(), d.updates.include_prereleases, ui.ctx().clone());
    }
    if !matches!(phase, Phase::Idle) {
        let mut reveal: Option<PathBuf> = None;
        any |= group(ui, q, CTX_ABOUT, tr!("settings-updates-status"), |r| {
            r.block("", "", |ui| status(ui, app, &phase, &mut reveal));
        });
        if let Some(file) = reveal
            && let Some(dir) = file.parent()
        {
            let pane = app.ws.pane_mut().id;
            let select = file.file_name().map(|n| n.to_string_lossy().into_owned());
            app.cmds.push(Cmd::Navigate { pane, location: Location::Dir(dir.to_path_buf()), select });
            super::close(app);
        }
    }
    any
}

fn status(ui: &mut Ui, app: &mut SoyuzApp, phase: &Phase, reveal: &mut Option<PathBuf>) {
    let p = palette(ui.ctx());
    match phase {
        Phase::Idle => {}
        Phase::Checking => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr!("settings-updates-checking")).color(p.text_secondary));
            });
        }
        Phase::UpToDate { latest } => {
            ui.label(RichText::new(tr!("settings-updates-up-to-date", latest = latest.to_string())).color(p.success));
        }
        Phase::Failed(e) => {
            ui.label(RichText::new(tr!("settings-updates-failed", error = e.to_string())).color(p.danger));
        }
        Phase::Available(rel) => {
            release_header(ui, rel);
            let asset = updates::asset_for_this_copy(rel);
            ui.add_space(8.0);
            ui.horizontal(|ui| match asset {
                Some(a) => {
                    if small_button(ui, Some(ph::DOWNLOAD_SIMPLE), tr!("settings-updates-download"), false)
                        .on_hover_text(tr!("settings-updates-download-tooltip", name = &a.name))
                        .clicked()
                    {
                        // Resolving the Downloads folder reads the registry: do it on the worker.
                        let rel = rel.clone();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            updates::download(rel, integration::downloads_dir(), ctx);
                        });
                    }
                    ui.label(RichText::new(format!("{} · {}", a.name, mib(a.size))).size(11.5).color(p.text_tertiary));
                }
                None => {
                    ui.label(RichText::new(tr!("settings-updates-no-asset")).size(11.5).color(p.text_tertiary));
                }
            });
            ui.horizontal(|ui| {
                if small_button(ui, Some(ph::CLIPBOARD_TEXT), tr!("settings-updates-copy-link"), false).clicked() {
                    app.cmds.push(Cmd::CopyText(rel.url.clone()));
                }
            });
        }
        Phase::Downloading { asset, got, total, .. } => {
            ui.label(
                RichText::new(tr!("settings-updates-downloading", name = asset.to_string())).color(p.text_secondary),
            );
            let frac = if *total > 0 { (*got as f32 / *total as f32).clamp(0.0, 1.0) } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).text(tr!(
                "settings-updates-progress",
                done = mib(*got),
                total = if *total > 0 { mib(*total) } else { "?".into() }
            )));
            if small_button(ui, Some(ph::X_CIRCLE), tr!("settings-cancel"), true).clicked() {
                updates::cancel();
            }
        }
        Phase::Downloaded { release, path, sha256, signature, detached_sig } => {
            release_header(ui, release);
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr!("settings-updates-saved-to", path = path.display().to_string()))
                    .color(p.text_primary),
            );
            ui.label(
                RichText::new(tr!("settings-updates-sha-ok", hash = sha256.to_string()))
                    .monospace()
                    .size(11.0)
                    .color(p.success),
            );
            let (sig_text, sig_color) = match signature {
                Signature::Valid => (tr!("settings-updates-sig-valid").to_string(), p.success),
                Signature::Unsigned => (tr!("settings-updates-sig-unsigned").to_string(), p.warning),
                Signature::Invalid(hr) => (tr!("settings-updates-sig-invalid", code = format!("0x{hr:08X}")), p.danger),
                Signature::Unknown => (tr!("settings-updates-sig-unknown").to_string(), p.text_tertiary),
            };
            ui.label(RichText::new(sig_text).size(11.5).color(sig_color));
            if *detached_sig {
                ui.label(RichText::new(tr!("settings-updates-detached-sig")).size(11.5).color(p.text_tertiary));
            }
            ui.add_space(6.0);
            let is_installer =
                path.extension().is_some_and(|e| e.eq_ignore_ascii_case("msi") || e.eq_ignore_ascii_case("exe"));
            let confirm_id = egui::Id::new("soyuz.settings.update.confirm_run");
            let mut confirming = ui.ctx().data(|d| d.get_temp::<bool>(confirm_id)).unwrap_or(false);
            ui.horizontal(|ui| {
                if is_installer
                    && !confirming
                    && small_button(ui, Some(ph::ROCKET_LAUNCH), tr!("settings-updates-run"), false).clicked()
                {
                    confirming = true;
                }
                if small_button(ui, Some(ph::FOLDER_OPEN), tr!("settings-updates-reveal"), false).clicked() {
                    *reveal = Some(path.clone());
                }
            });
            if confirming {
                ui.add_space(4.0);
                ui.label(RichText::new(tr!("settings-updates-run-note")).size(11.5).color(p.text_secondary));
                ui.horizontal(|ui| {
                    if small_button(ui, Some(ph::ROCKET_LAUNCH), tr!("settings-updates-run-now"), false).clicked() {
                        confirming = false;
                        match updates::run_installer(path) {
                            Ok(()) => {
                                app.ui.toasts.push(Toast::info(tr!("settings-updates-started")));
                            }
                            Err(e) => {
                                app.ui
                                    .toasts
                                    .push(Toast::error(tr!("settings-updates-start-failed")).body(e.to_string()));
                            }
                        }
                    }
                    if small_button(ui, None, tr!("settings-cancel"), false).clicked() {
                        confirming = false;
                    }
                });
            }
            ui.ctx().data_mut(|d| d.insert_temp(confirm_id, confirming));
        }
    }
}

fn release_header(ui: &mut Ui, rel: &updates::Release) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(crate::theme::icons::icon(ph::SEAL_CHECK, 16.0).color(p.accent));
        ui.label(
            RichText::new(tr!(
                "settings-updates-available",
                version = rel.version.to_string(),
                current = updates::CURRENT.to_string()
            ))
            .size(13.5)
            .color(p.text_primary),
        );
    });
    let mut meta = rel.title.clone();
    if !rel.published.is_empty() {
        meta.push_str(&format!(" · {}", rel.published));
    }
    if rel.prerelease || rel.version.is_prerelease() {
        meta.push_str(&format!(" · {}", tr!("settings-updates-prerelease")));
    }
    ui.label(RichText::new(meta).size(11.5).color(p.text_tertiary));
    if !rel.notes.trim().is_empty() {
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(p.bg_sunken)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("soyuz.settings.update.notes").max_height(200.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // Release notes are untrusted text: shown verbatim, never interpreted.
                    let notes: String = rel.notes.chars().take(20_000).collect();
                    ui.add(
                        egui::Label::new(RichText::new(notes.replace('\r', "")).size(12.0).color(p.text_secondary))
                            .wrap(),
                    );
                });
            });
    }
}

// ---- default file manager --------------------------------------------------------------

#[derive(Default)]
struct FmState {
    status: Option<FileManagerStatus>,
    busy: bool,
    /// Result of the last register / restore to report as a toast.
    outcome: Option<Result<&'static str, String>>,
}

static FM: Mutex<FmState> = Mutex::new(FmState { status: None, busy: false, outcome: None });

fn fm_refresh(ctx: &egui::Context, action: Option<bool>) {
    {
        let mut s = FM.lock();
        if s.busy {
            return;
        }
        s.busy = true;
    }
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let keys = AssocKeys::user();
        let outcome = match action {
            Some(true) => Some(
                std::env::current_exe()
                    .and_then(|exe| integration::register_file_manager(&keys, &exe))
                    .map(|()| tr!("settings-fm-registered"))
                    .map_err(|e| e.to_string()),
            ),
            Some(false) => Some(
                integration::restore_explorer(&keys).map(|()| tr!("settings-fm-restored")).map_err(|e| e.to_string()),
            ),
            None => None,
        };
        let status = integration::file_manager_status(&keys);
        let mut s = FM.lock();
        s.status = Some(status);
        s.busy = false;
        if outcome.is_some() {
            s.outcome = outcome;
        }
        drop(s);
        ctx.request_repaint();
    });
}

fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Settings → General → "Default file manager" (first group on the page).
pub fn file_manager(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let (status, busy, outcome) = {
        let mut s = FM.lock();
        (s.status.clone(), s.busy, s.outcome.take())
    };
    if status.is_none() && !busy {
        fm_refresh(ui.ctx(), None);
    }
    match outcome {
        Some(Ok(msg)) => {
            app.ui.toasts.push(Toast::success(msg));
        }
        Some(Err(e)) => {
            app.ui.toasts.push(Toast::error(tr!("settings-fm-failed")).body(e));
        }
        None => {}
    }
    let mut act: Option<bool> = None;
    let any = group(ui, q, CTX_FM, tr!("settings-fm"), |r| {
        r.block(tr!("settings-fm-open-with"), tr!("settings-fm-desc"), |ui| {
            let (label, color) = match &status {
                None => (tr!("settings-fm-checking").to_string(), p.text_tertiary),
                Some(FileManagerStatus::NotRegistered) => {
                    (tr!("settings-fm-status-explorer").to_string(), p.text_secondary)
                }
                Some(FileManagerStatus::Registered { command }) => {
                    (tr!("settings-fm-status-soyuz", command = command), p.success)
                }
                Some(FileManagerStatus::Modified) => (tr!("settings-fm-status-modified").to_string(), p.warning),
                Some(FileManagerStatus::Unsupported) => {
                    (tr!("settings-fm-status-unsupported").to_string(), p.text_tertiary)
                }
            };
            ui.add(egui::Label::new(RichText::new(label).size(12.0).color(color)).wrap());
            if let Some(exe) = exe_path() {
                let s = exe.to_string_lossy().to_ascii_lowercase();
                if s.contains("\\target\\debug\\") || s.contains("\\target\\release\\") || s.contains("\\temp\\") {
                    ui.label(
                        RichText::new(tr!("settings-fm-temp-copy", path = exe.display().to_string()))
                            .size(11.5)
                            .color(p.warning),
                    );
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let supported = !matches!(status, Some(FileManagerStatus::Unsupported) | None);
                ui.add_enabled_ui(supported && !busy, |ui| {
                    let registered = matches!(status, Some(FileManagerStatus::Registered { .. }));
                    let label =
                        if registered { tr!("settings-fm-reregister") } else { tr!("settings-fm-make-default") };
                    if small_button(ui, Some(ph::FOLDER_OPEN), label, false).clicked() {
                        act = Some(true);
                    }
                    let can_restore = !matches!(status, Some(FileManagerStatus::NotRegistered));
                    ui.add_enabled_ui(can_restore, |ui| {
                        if small_button(ui, Some(ph::ARROW_U_UP_LEFT), tr!("settings-fm-restore"), false).clicked() {
                            act = Some(false);
                        }
                    });
                });
                if busy {
                    ui.spinner();
                }
            });
        });
    });
    if let Some(a) = act {
        fm_refresh(ui.ctx(), Some(a));
    }
    any
}

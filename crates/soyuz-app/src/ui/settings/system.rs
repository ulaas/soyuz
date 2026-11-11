//! Settings pages for OS integration: "About & updates" (the update check against
//! GitHub Releases, "Update and restart" and the automatic-check switch) and the
//! "Default file manager" opt-in at the top of the General page.
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
use crate::ui::widgets::{Btn, pill_button};

const CTX_ABOUT: &str = "About updates version";
const CTX_FM: &str = "General default file manager explorer folders drives integration os desktop mimeapps dbus";

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
        app.core.cmds.push(Cmd::Action(soyuz_core::config::Action::About));
    }

    let phase = updates::phase();
    let busy = phase.busy();
    let mut check = false;
    let d = &mut app.gui.settings.draft;
    let repo_before = d.updates.repo.clone();
    any |= group(ui, q, CTX_ABOUT, tr!("settings-updates"), |r| {
        r.row_kw(tr!("settings-updates-repo"), tr!("settings-updates-repo-desc"), "github check url repo", |ui| {
            text(ui, &mut d.updates.repo, "owner/soyuz", 220.0);
        });
        r.row_kw(
            tr!("settings-updates-auto"),
            tr!("settings-updates-auto-desc"),
            "automatic daily background check notify notification",
            |ui| {
                switch(ui, &mut d.updates.auto_check);
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
        // Authenticode is a Windows matter; the other platforms' text leaves it out.
        let privacy =
            if cfg!(windows) { tr!("settings-updates-privacy") } else { tr!("settings-updates-privacy-linux") };
        r.block("", privacy, |ui| {
            ui.horizontal(|ui| {
                let can = !d.updates.repo.trim().is_empty() && !busy;
                ui.add_enabled_ui(can, |ui| {
                    check = small_button(ui, Some(ph::ARROW_CLOCKWISE), tr!("settings-updates-check"), false).clicked();
                });
                if d.updates.repo.trim().is_empty() {
                    ui.label(
                        RichText::new(tr!("settings-updates-no-repo"))
                            .size(crate::theme::font::px(11.5))
                            .color(p.text_tertiary),
                    );
                } else if let Err(e) = updates::parse_repo(&d.updates.repo) {
                    ui.label(RichText::new(e).size(crate::theme::font::px(11.5)).color(p.warning));
                }
            });
        });
    });
    if d.updates.repo != repo_before {
        updates::reset();
    }
    if check {
        updates::check(d.updates.repo.clone(), d.updates.include_prereleases, false, crate::host::egui_wake(ui.ctx()));
    }
    if !matches!(phase, Phase::Idle) {
        let mut reveal: Option<PathBuf> = None;
        any |= group(ui, q, CTX_ABOUT, tr!("settings-updates-status"), |r| {
            r.block("", "", |ui| status(ui, app, &phase, &mut reveal));
        });
        if let Some(file) = reveal
            && let Some(dir) = file.parent()
        {
            let pane = app.core.ws.pane_mut().id;
            let select = file.file_name().map(|n| n.to_string_lossy().into_owned());
            app.core.cmds.push(Cmd::Navigate { pane, location: Location::Dir(dir.to_path_buf()), select });
            super::close(app);
        }
    }
    any
}

/// A small wrapped note under the buttons.
fn note(ui: &mut Ui, text: &str, color: egui::Color32) {
    ui.add(egui::Label::new(RichText::new(text).size(crate::theme::font::px(11.5)).color(color)).wrap());
}

/// What to do about the newer release `rel`, by how this copy was installed: "Update and
/// restart" where Soyuz installs it itself, how to update a copy something else manages,
/// a download to install by hand otherwise. Always the release page.
fn offer(ui: &mut Ui, app: &mut SoyuzApp, rel: &std::sync::Arc<updates::Release>) {
    let p = palette(ui.ctx());
    let kind = updates::install_kind();
    if kind.managed() {
        // Never downloaded or installed here: the package manager does that.
        if let Some(how) = updates::how_to_update(kind) {
            note(ui, &how, p.text_secondary);
            ui.add_space(6.0);
        }
    } else if let Some(a) = updates::asset_for_this_copy(rel) {
        let own = kind.self_installs();
        ui.horizontal(|ui| {
            if own
                && pill_button(ui, Some(ph::ROCKET_LAUNCH), tr!("settings-updates-install"), Btn::Primary)
                    .on_hover_text(tr!("settings-updates-install-tooltip", name = &a.name))
                    .clicked()
            {
                updates::install(rel.clone(), crate::host::egui_wake(ui.ctx()));
            }
            let label = if own { tr!("settings-updates-download-only") } else { tr!("settings-updates-download") };
            if small_button(ui, Some(ph::DOWNLOAD_SIMPLE), label, false)
                .on_hover_text(tr!("settings-updates-download-tooltip", name = &a.name))
                .clicked()
            {
                // Resolving the Downloads folder reads the registry: do it on the worker.
                let rel = rel.clone();
                let wake = crate::host::egui_wake(ui.ctx());
                std::thread::spawn(move || {
                    updates::download(rel, integration::downloads_dir(), wake);
                });
            }
            ui.label(
                RichText::new(format!("{} · {}", a.name, mib(a.size)))
                    .size(crate::theme::font::px(11.5))
                    .color(p.text_tertiary),
            );
        });
        if matches!(kind, updates::InstallKind::WindowsMsi { .. }) {
            note(ui, tr!("settings-updates-install-msi"), p.text_tertiary);
        } else if let Some(why) = updates::how_to_update(kind) {
            note(ui, &why, p.text_tertiary);
        }
    } else {
        note(ui, tr!("settings-updates-no-asset"), p.text_tertiary);
    }
    ui.horizontal(|ui| {
        if rel.url.starts_with("https://")
            && small_button(ui, Some(ph::ARROW_SQUARE_OUT), tr!("settings-updates-release-page"), false).clicked()
        {
            updates::open_release_page(&mut app.core, &rel.url);
        }
        if small_button(ui, Some(ph::CLIPBOARD_TEXT), tr!("settings-updates-copy-link"), false).clicked() {
            app.core.cmds.push(Cmd::CopyText(rel.url.clone()));
        }
    });
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
            let text = tr!("settings-updates-failed", error = e.to_string());
            ui.add(egui::Label::new(RichText::new(text).color(p.danger)).wrap());
        }
        Phase::Available(rel) => {
            release_header(ui, rel);
            ui.add_space(8.0);
            offer(ui, app, rel);
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
        Phase::Installing { release } => {
            release_header(ui, release);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr!("settings-updates-installing")).color(p.text_secondary));
            });
        }
        Phase::Ready { release, restart } => {
            let version = release.version.to_string();
            let text = if restart.installed() {
                tr!("settings-updates-ready", version = version)
            } else {
                tr!("settings-updates-ready-installer", version = version)
            };
            ui.add(egui::Label::new(RichText::new(text).color(p.success)).wrap());
            ui.add_space(6.0);
            // Soyuz restarts by itself; the button is for a restart that was cancelled
            // (a question about running jobs or unsaved documents).
            if pill_button(ui, Some(ph::ARROW_CLOCKWISE), tr!("settings-updates-restart-now"), Btn::Primary).clicked() {
                updates::restart_now();
                ui.ctx().request_repaint();
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
                    .size(crate::theme::font::px(11.0))
                    .color(p.success),
            );
            let (sig_text, sig_color) = match signature {
                Signature::Valid => (tr!("settings-updates-sig-valid").to_string(), p.success),
                Signature::Unsigned => (tr!("settings-updates-sig-unsigned").to_string(), p.warning),
                Signature::Invalid(hr) => (tr!("settings-updates-sig-invalid", code = format!("0x{hr:08X}")), p.danger),
                Signature::Unknown if cfg!(target_os = "linux") => {
                    (tr!("settings-updates-sig-unknown-linux").to_string(), p.text_tertiary)
                }
                Signature::Unknown => (tr!("settings-updates-sig-unknown").to_string(), p.text_tertiary),
            };
            ui.label(RichText::new(sig_text).size(crate::theme::font::px(11.5)).color(sig_color));
            if *detached_sig {
                note(ui, tr!("settings-updates-detached-sig"), p.text_tertiary);
            }
            if let Some(how) = updates::self_update_note() {
                note(ui, &how, p.text_secondary);
            }
            ui.add_space(6.0);
            let is_installer =
                path.extension().is_some_and(|e| e.eq_ignore_ascii_case("msi") || e.eq_ignore_ascii_case("exe"));
            let confirm_id = egui::Id::unique("soyuz.settings.update.confirm_run");
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
                note(ui, tr!("settings-updates-run-note"), p.text_secondary);
                ui.horizontal(|ui| {
                    if small_button(ui, Some(ph::ROCKET_LAUNCH), tr!("settings-updates-run-now"), false).clicked() {
                        confirming = false;
                        match updates::run_installer(path) {
                            Ok(()) => {
                                app.core.ui.toasts.push(Toast::info(tr!("settings-updates-started")));
                            }
                            Err(e) => {
                                app.core
                                    .ui
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
            .size(crate::theme::font::px(13.5))
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
    ui.label(RichText::new(meta).size(crate::theme::font::px(11.5)).color(p.text_tertiary));
    if !rel.notes.trim().is_empty() {
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(p.bg_sunken)
            .corner_radius(crate::theme::tokens::radius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("soyuz.settings.update.notes").max_height(200.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // Release notes are untrusted text: shown verbatim, never interpreted.
                    let notes: String = rel.notes.chars().take(20_000).collect();
                    ui.add(
                        egui::Label::new(
                            RichText::new(notes.replace('\r', ""))
                                .size(crate::theme::font::px(12.0))
                                .color(p.text_secondary),
                        )
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
    /// Linux: display name of the app that opens folders now (mimeapps.list).
    handler: Option<String>,
    /// Linux: our `org.freedesktop.FileManager1` D-Bus service file is installed.
    service: bool,
}

static FM: Mutex<FmState> =
    Mutex::new(FmState { status: None, busy: false, outcome: None, handler: None, service: false });

/// What a click asks the worker to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FmAction {
    Register,
    Restore,
    /// Linux: install / remove the FileManager1 D-Bus service file.
    RegisterService,
    RestoreService,
}

const LINUX: bool = cfg!(target_os = "linux");

fn fm_refresh(ctx: &egui::Context, action: Option<FmAction>) {
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
        let outcome = action.map(|a| match a {
            FmAction::Register => std::env::current_exe()
                .and_then(|exe| integration::register_file_manager(&keys, &exe))
                .map(|()| tr!("settings-fm-registered"))
                .map_err(|e| e.to_string()),
            FmAction::Restore => integration::restore_explorer(&keys)
                .map(|()| if LINUX { tr!("settings-fm-restored-linux") } else { tr!("settings-fm-restored") })
                .map_err(|e| e.to_string()),
            FmAction::RegisterService => std::env::current_exe()
                .and_then(|exe| integration::register_file_manager1_service(&keys, &exe))
                .map(|()| tr!("settings-fm1-installed"))
                .map_err(|e| e.to_string()),
            FmAction::RestoreService => integration::restore_file_manager1_service(&keys)
                .map(|()| tr!("settings-fm1-removed"))
                .map_err(|e| e.to_string()),
        });
        let status = integration::file_manager_status(&keys);
        let (handler, service) = if LINUX {
            (folder_handler_name(), integration::file_manager1_service_registered(&keys))
        } else {
            (None, false)
        };
        let mut s = FM.lock();
        s.status = Some(status);
        s.handler = handler;
        s.service = service;
        s.busy = false;
        if outcome.is_some() {
            s.outcome = outcome;
        }
        drop(s);
        ctx.request_repaint();
    });
}

/// Linux: the name of the app that opens folders now ("Files", "Dolphin", …), falling
/// back to its desktop file id. Reads desktop entries: worker thread only.
fn folder_handler_name() -> Option<String> {
    let id = integration::folder_handler()?;
    let name = soyuz_fs::launch::default_app_for(&soyuz_fs::home())
        .filter(|a| a.id == id || a.id.trim_end_matches(".desktop") == id.trim_end_matches(".desktop"))
        .map(|a| a.name)
        .filter(|n| !n.trim().is_empty());
    Some(name.unwrap_or_else(|| id.trim_end_matches(".desktop").to_string()))
}

fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// This copy runs from a build folder or a temp folder (registering it is fragile).
fn temp_copy(exe: &std::path::Path) -> bool {
    let s = exe.to_string_lossy().to_ascii_lowercase();
    if cfg!(windows) {
        s.contains("\\target\\debug\\") || s.contains("\\target\\release\\") || s.contains("\\temp\\")
    } else {
        s.contains("/target/debug/") || s.contains("/target/release/") || s.starts_with("/tmp/")
    }
}

/// Settings → General → "Default file manager" (first group on the page).
pub fn file_manager(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let (status, busy, outcome, handler, service) = {
        let mut s = FM.lock();
        (s.status.clone(), s.busy, s.outcome.take(), s.handler.clone(), s.service)
    };
    if status.is_none() && !busy {
        fm_refresh(ui.ctx(), None);
    }
    match outcome {
        Some(Ok(msg)) => {
            app.core.ui.toasts.push(Toast::success(msg));
        }
        Some(Err(e)) => {
            app.core.ui.toasts.push(Toast::error(tr!("settings-fm-failed")).body(e));
        }
        None => {}
    }
    let mut act: Option<FmAction> = None;
    let handler = handler.unwrap_or_else(|| tr!("settings-fm-handler-unknown").to_string());
    let desc =
        if LINUX { tr!("settings-fm-desc-linux", handler = handler.as_str()) } else { tr!("settings-fm-desc").into() };
    let any = group(ui, q, CTX_FM, tr!("settings-fm"), |r| {
        r.block(tr!("settings-fm-open-with"), &desc, |ui| {
            let (label, color) = match &status {
                None => (tr!("settings-fm-checking").to_string(), p.text_tertiary),
                Some(FileManagerStatus::NotRegistered) if LINUX => {
                    (tr!("settings-fm-status-other", handler = handler.as_str()), p.text_secondary)
                }
                Some(FileManagerStatus::NotRegistered) => {
                    (tr!("settings-fm-status-explorer").to_string(), p.text_secondary)
                }
                Some(FileManagerStatus::Registered { command }) => {
                    (tr!("settings-fm-status-soyuz", command = command), p.success)
                }
                Some(FileManagerStatus::Modified) if LINUX => {
                    (tr!("settings-fm-status-modified-linux").to_string(), p.warning)
                }
                Some(FileManagerStatus::Modified) => (tr!("settings-fm-status-modified").to_string(), p.warning),
                Some(FileManagerStatus::Unsupported) => {
                    (tr!("settings-fm-status-unsupported").to_string(), p.text_tertiary)
                }
            };
            ui.add(egui::Label::new(RichText::new(label).size(crate::theme::font::px(12.0)).color(color)).wrap());
            if let Some(exe) = exe_path()
                && temp_copy(&exe)
            {
                let note = if LINUX {
                    tr!("settings-fm-temp-copy-linux", path = exe.display().to_string())
                } else {
                    tr!("settings-fm-temp-copy", path = exe.display().to_string())
                };
                ui.label(RichText::new(note).size(crate::theme::font::px(11.5)).color(p.warning));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let supported = !matches!(status, Some(FileManagerStatus::Unsupported) | None);
                ui.add_enabled_ui(supported && !busy, |ui| {
                    let registered = matches!(status, Some(FileManagerStatus::Registered { .. }));
                    let label =
                        if registered { tr!("settings-fm-reregister") } else { tr!("settings-fm-make-default") };
                    if small_button(ui, Some(ph::FOLDER_OPEN), label, false).clicked() {
                        act = Some(FmAction::Register);
                    }
                    let can_restore = !matches!(status, Some(FileManagerStatus::NotRegistered));
                    ui.add_enabled_ui(can_restore, |ui| {
                        let label = if LINUX { tr!("settings-fm-restore-linux") } else { tr!("settings-fm-restore") };
                        if small_button(ui, Some(ph::ARROW_U_UP_LEFT), label, false).clicked() {
                            act = Some(FmAction::Restore);
                        }
                    });
                });
                if busy {
                    ui.spinner();
                }
            });
        });
        // Linux: the D-Bus activation file that lets other apps' "Show in folder" start
        // Soyuz. Separate from (and independent of) the folder handler.
        if LINUX {
            r.block(tr!("settings-fm1"), tr!("settings-fm1-desc"), |ui| {
                let (label, color) = match (&status, service) {
                    (None, _) => (tr!("settings-fm-checking"), p.text_tertiary),
                    (_, true) => (tr!("settings-fm1-status-on"), p.success),
                    (_, false) => (tr!("settings-fm1-status-off"), p.text_secondary),
                };
                ui.add(egui::Label::new(RichText::new(label).size(crate::theme::font::px(12.0)).color(color)).wrap());
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(status.is_some() && !busy, |ui| {
                        let label = if service { tr!("settings-fm-reregister") } else { tr!("settings-fm1-install") };
                        if small_button(ui, Some(ph::PLUGS_CONNECTED), label, false).clicked() {
                            act = Some(FmAction::RegisterService);
                        }
                        ui.add_enabled_ui(service, |ui| {
                            if small_button(ui, Some(ph::ARROW_U_UP_LEFT), tr!("settings-fm1-remove"), false).clicked()
                            {
                                act = Some(FmAction::RestoreService);
                            }
                        });
                    });
                    if busy {
                        ui.spinner();
                    }
                });
            });
        }
    });
    if let Some(a) = act {
        fm_refresh(ui.ctx(), Some(a));
    }
    any
}

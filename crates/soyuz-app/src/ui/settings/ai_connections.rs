//! Settings → AI assistant → Connections (roadmap A3 + A4):
//! * the Soyuz MCP server (pipe attach for `soyuz --mcp`, optional HTTP on 127.0.0.1),
//! * MCP clients: approve pending ones, folders, risk ceiling, approval, HTTP tokens,
//! * "Add Soyuz to Claude Desktop / Claude Code / VS Code" with a diff preview,
//! * external MCP servers whose tools the assistant may use, with a connection test.
//!
//! All file / credential-store work runs on worker threads.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use egui::{CornerRadius, Frame, Margin, RichText, ScrollArea, Stroke, TextEdit, Ui};
use soyuz_ai::Secret;
use soyuz_core::config::{AiMcpClient, AiMcpServer, AiRisk};
use soyuz_mcp::install::{DiffLine, InstallPreview, Target};
use soyuz_mcp::trust;

use super::widgets::{code, group, matches, number, segmented, small_button, subheading, switch};
use crate::app::SoyuzApp;
use crate::services::ai_service::{self, Wake};
use crate::theme::icons::ph;
use crate::theme::{icons, palette};

const CTX: &str = "AI assistant connections MCP model context protocol clients servers claude";

type Rx<T> = Receiver<T>;

enum Install {
    Loading(Target, Rx<Result<InstallPreview, String>>),
    Ready(InstallPreview),
    Writing(Target, Rx<Result<(), String>>),
}

enum Probe {
    Running(Rx<Result<String, String>>),
    Done(Result<String, String>),
}

/// Page-local state (lives in `SettingsState`).
#[derive(Default)]
pub struct ConnectionsState {
    roots_text: HashMap<String, String>,
    origins_text: Option<String>,
    new_client: String,
    new_server: String,
    /// A freshly generated token, shown once: `(client, token)`.
    shown_token: Option<(String, String)>,
    ops: Vec<(String, Rx<Result<(), String>>)>,
    notice: Option<(String, bool)>,
    install: Option<Install>,
    probes: HashMap<String, Probe>,
    args_text: HashMap<String, String>,
}

fn wake_of(ui: &Ui) -> Wake {
    let c = ui.ctx().clone();
    Arc::new(move || c.request_repaint())
}

fn spawn<T: Send + 'static>(ui: &Ui, f: impl FnOnce() -> T + Send + 'static) -> Rx<T> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let wake = wake_of(ui);
    let _ = std::thread::Builder::new().name("soyuz-settings-mcp".into()).spawn(move || {
        let _ = tx.send(f());
        wake();
    });
    rx
}

fn card(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    Frame::new()
        .fill(p.bg_surface2.gamma_multiply(if p.dark { 0.7 } else { 1.0 }))
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        });
}

fn risk_picker(ui: &mut Ui, r: &mut AiRisk) {
    segmented(
        ui,
        r,
        &[
            (AiRisk::Read, tr!("settings-ai-read-only")),
            (AiRisk::Write, tr!("settings-mcp-risk-write")),
            (AiRisk::Destructive, tr!("settings-mcp-risk-delete")),
        ],
    );
}

fn lines_to_paths(t: &str) -> Vec<PathBuf> {
    t.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("soyuz"))
}

impl ConnectionsState {
    fn poll(&mut self, app_refresh: &mut bool) {
        self.ops.retain(|(what, rx)| match rx.try_recv() {
            Ok(Ok(())) => {
                self.notice = Some((what.clone(), false));
                *app_refresh = true;
                false
            }
            Ok(Err(e)) => {
                self.notice = Some((tr!("settings-ai-op-failed", what = what, error = e), true));
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        match self.install.take() {
            Some(Install::Loading(t, rx)) => match rx.try_recv() {
                Ok(Ok(p)) => self.install = Some(Install::Ready(p)),
                Ok(Err(e)) => self.notice = Some((format!("{}: {e}", t.label()), true)),
                Err(crossbeam_channel::TryRecvError::Empty) => self.install = Some(Install::Loading(t, rx)),
                Err(_) => {}
            },
            Some(Install::Writing(t, rx)) => match rx.try_recv() {
                Ok(Ok(())) => self.notice = Some((tr!("settings-mcp-installed", app = t.label()), false)),
                Ok(Err(e)) => self.notice = Some((format!("{}: {e}", t.label()), true)),
                Err(crossbeam_channel::TryRecvError::Empty) => self.install = Some(Install::Writing(t, rx)),
                Err(_) => {}
            },
            other => self.install = other,
        }
        for pr in self.probes.values_mut() {
            if let Probe::Running(rx) = pr
                && let Ok(r) = rx.try_recv()
            {
                *pr = Probe::Done(r);
            }
        }
    }
}

/// Draw the Connections part of the AI page.
pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let mut refresh = false;
    app.settings.connections.poll(&mut refresh);
    if refresh {
        app.services.mcp.refresh_tokens();
    }
    let status = app.services.mcp.status.clone();
    let searching = !q.trim().is_empty();
    let mut any = false;
    let p = palette(ui.ctx());

    if !searching || matches(q, &["connections mcp", CTX, tr!("settings-mcp-connections")]) {
        ui.add_space(18.0);
        ui.label(
            RichText::new(super::widgets::upper(tr!("settings-mcp-connections"))).size(10.5).color(p.text_tertiary),
        );
        ui.add(egui::Label::new(RichText::new(tr!("settings-mcp-intro")).size(11.5).color(p.text_tertiary)).wrap());
        if let Some((msg, err)) = &app.settings.connections.notice {
            ui.add_space(4.0);
            ui.label(RichText::new(msg).size(11.5).color(if *err { p.danger } else { p.success }));
        }
    }

    // --- server ----------------------------------------------------------------------
    {
        let s = &mut app.settings;
        let (d, st) = (&mut s.draft.ai, &mut s.connections);
        any |= group(ui, q, CTX, tr!("settings-mcp-server"), |r| {
            r.row_kw(tr!("settings-mcp-attach"), tr!("settings-mcp-attach-desc"), "pipe stdio attach", |ui| {
                switch(ui, &mut d.mcp.server_enabled);
            });
            r.row_kw(tr!("settings-index-status"), "", "pipe http running", |ui| {
                let mut parts = Vec::new();
                match &status.pipe {
                    Some(Ok(n)) => parts.push((tr!("settings-mcp-pipe", name = n.to_string()), false)),
                    Some(Err(e)) => parts.push((tr!("settings-mcp-pipe", name = e.to_string()), true)),
                    None => parts.push((tr!("settings-mcp-pipe-off").into(), false)),
                }
                match &status.http {
                    Some(Ok(a)) => parts.push((a.clone(), false)),
                    Some(Err(e)) => parts.push((e.clone(), true)),
                    None => {}
                }
                if status.waiting > 0 {
                    parts.push((tr!("settings-mcp-waiting", count = status.waiting), false));
                }
                for (t, err) in parts.into_iter().rev() {
                    ui.label(RichText::new(t).size(11.5).color(if err { p.warning } else { p.text_secondary }));
                }
            });
            r.row_kw(tr!("settings-mcp-http"), tr!("settings-mcp-http-desc"), "http port network token", |ui| {
                switch(ui, &mut d.mcp.http_enabled);
            });
            if d.mcp.http_enabled {
                r.row(tr!("settings-mcp-port"), tr!("settings-mcp-port-desc"), |ui| {
                    number(ui, &mut d.mcp.http_port, 1024..=65535, "");
                });
                r.block(tr!("settings-mcp-origins"), tr!("settings-mcp-origins-desc"), |ui| {
                    let t = st.origins_text.get_or_insert_with(|| d.mcp.allowed_origins.join("\n"));
                    if ui
                        .add(
                            TextEdit::multiline(t)
                                .desired_rows(1)
                                .desired_width(f32::INFINITY)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed()
                    {
                        d.mcp.allowed_origins =
                            t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
                    }
                });
            }
            r.row_kw(tr!("settings-mcp-register"), tr!("settings-mcp-register-desc"), "pending approve new", |ui| {
                switch(ui, &mut d.mcp.register_unknown);
            });
            r.row(tr!("settings-mcp-approval-timeout"), tr!("settings-mcp-approval-timeout-desc"), |ui| {
                number(ui, &mut d.mcp.approval_timeout_secs, 10..=3600, " s");
            });
            r.row_kw(
                tr!("settings-mcp-approval-ui"),
                tr!("settings-mcp-approval-ui-desc"),
                "approval panel dialog modal plan card",
                |ui| {
                    segmented(
                        ui,
                        &mut d.mcp.approval_ui,
                        &[
                            (soyuz_core::config::AiMcpApprovalUi::Panel, tr!("settings-mcp-approval-panel")),
                            (soyuz_core::config::AiMcpApprovalUi::Dialog, tr!("settings-mcp-approval-modal")),
                        ],
                    );
                },
            );
        });
    }

    any |= clients(ui, app, q);
    any |= install(ui, app, q);
    any |= servers(ui, app, q);
    any
}

fn clients(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    if !q.trim().is_empty()
        && !matches(q, &["mcp clients approve pending token scope risk folders", CTX, tr!("settings-mcp-clients")])
    {
        return false;
    }
    let p = palette(ui.ctx());
    let http = app.settings.draft.ai.mcp.http_enabled;
    let port = app.settings.draft.ai.mcp.http_port;
    let s = &mut app.settings;
    let (d, st) = (&mut s.draft.ai, &mut s.connections);
    ui.add_space(14.0);
    ui.label(RichText::new(super::widgets::upper(tr!("settings-mcp-clients"))).size(10.5).color(p.text_tertiary));
    ui.add_space(6.0);
    let mut remove = None;
    for (i, c) in d.mcp_clients.iter_mut().enumerate() {
        ui.add_space(6.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    icons::icon(if c.pending { ph::USER_CHECK } else { ph::PLUGS_CONNECTED }, 16.0).color(p.accent),
                );
                ui.label(RichText::new(&c.name).strong().color(p.text_primary));
                if c.pending {
                    ui.label(RichText::new(tr!("settings-mcp-pending")).size(11.5).color(p.warning));
                    if small_button(ui, Some(ph::CHECK), tr!("settings-mcp-approve"), false).clicked() {
                        c.pending = false;
                    }
                } else {
                    ui.label(RichText::new(tr!("settings-mcp-approved")).size(11.5).color(p.success));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_button(ui, Some(ph::TRASH), "", true)
                        .on_hover_text(tr!("settings-mcp-remove-client"))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            });
            ui.add_space(6.0);
            egui::Grid::new(("mcp_client", i)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                let lbl = |ui: &mut Ui, t: &str| {
                    ui.label(RichText::new(t).size(12.0).color(p.text_secondary));
                };
                lbl(ui, tr!("settings-mcp-may"));
                ui.horizontal(|ui| risk_picker(ui, &mut c.max_risk));
                ui.end_row();
                lbl(ui, tr!("settings-mcp-ask"));
                ui.horizontal(|ui| {
                    switch(ui, &mut c.require_approval);
                    if !c.require_approval {
                        ui.label(RichText::new(tr!("settings-mcp-no-ask")).size(11.0).color(p.warning));
                    }
                });
                ui.end_row();
                lbl(ui, tr!("settings-views-folders"));
                let t = st.roots_text.entry(c.name.clone()).or_insert_with(|| {
                    c.allowed_roots.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n")
                });
                if ui
                    .add(
                        TextEdit::multiline(t)
                            .desired_rows(1)
                            .desired_width(360.0)
                            .hint_text(tr!("settings-mcp-folders-hint"))
                            .font(egui::TextStyle::Monospace),
                    )
                    .changed()
                {
                    c.allowed_roots = lines_to_paths(t);
                }
                ui.end_row();
                if http {
                    lbl(ui, tr!("settings-mcp-token"));
                    ui.horizontal(|ui| {
                        let has = !c.token.is_empty();
                        if has {
                            ui.label(icons::icon(ph::LOCK_KEY, 13.0).color(p.success));
                        }
                        let label = if has { tr!("settings-mcp-rotate") } else { tr!("settings-mcp-generate") };
                        if small_button(ui, Some(ph::KEY), label, false).clicked() {
                            let token = trust::generate_token();
                            let cred = trust::token_credential(&c.name);
                            c.token = format!("credential:{cred}");
                            st.shown_token = Some((c.name.clone(), token.clone()));
                            st.ops.push((
                                tr!("settings-mcp-token-stored").into(),
                                ai_service::store_secret(cred, Secret::new(token), wake_of(ui)),
                            ));
                        }
                        if has && small_button(ui, Some(ph::X), tr!("settings-mcp-revoke"), true).clicked() {
                            if let Ok(soyuz_ai::credentials::KeyRef::Credential(n)) =
                                soyuz_ai::credentials::parse_ref(&c.token)
                            {
                                st.ops.push((
                                    tr!("settings-mcp-token-revoked").into(),
                                    ai_service::delete_secret(n, wake_of(ui)),
                                ));
                            }
                            c.token.clear();
                        }
                    });
                    ui.end_row();
                }
            });
            if let Some((name, token)) = &st.shown_token
                && *name == c.name
            {
                ui.add_space(6.0);
                ui.label(RichText::new(tr!("settings-mcp-token-once")).size(11.5).color(p.warning));
                let snippet = format!(
                    "{{\n  \"type\": \"http\",\n  \"url\": \"http://127.0.0.1:{port}/mcp\",\n  \"headers\": {{ \"Authorization\": \"Bearer {token}\" }}\n}}"
                );
                let mut shown = snippet.clone();
                ui.add(
                    TextEdit::multiline(&mut shown)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(5),
                );
                ui.horizontal(|ui| {
                    if small_button(ui, Some(ph::COPY), tr!("settings-mcp-copy-token"), false).clicked() {
                        ui.ctx().copy_text(token.clone());
                    }
                    if small_button(ui, Some(ph::COPY), tr!("settings-mcp-copy-config"), false).clicked() {
                        ui.ctx().copy_text(snippet.clone());
                    }
                });
            }
        });
    }
    if let Some(i) = remove {
        let c = d.mcp_clients.remove(i);
        if let Ok(soyuz_ai::credentials::KeyRef::Credential(n)) = soyuz_ai::credentials::parse_ref(&c.token) {
            st.ops
                .push((tr!("settings-mcp-client-removed", name = &c.name), ai_service::delete_secret(n, wake_of(ui))));
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        code(ui, &mut st.new_client, tr!("settings-mcp-client-hint"), 220.0);
        let name = trust::sanitize_name(&st.new_client);
        let taken = d.mcp_clients.iter().any(|c| trust::sanitize_name(&c.name) == name);
        if small_button(ui, Some(ph::PLUS), tr!("settings-mcp-add-client"), false).clicked()
            && !name.is_empty()
            && !taken
        {
            d.mcp_clients.push(AiMcpClient { name: name.clone(), ..Default::default() });
            st.new_client.clear();
        }
        if !name.is_empty() {
            ui.label(
                RichText::new(tr!("settings-mcp-starts-with", command = format!("soyuz --mcp --mcp-client {name}")))
                    .monospace()
                    .size(11.0)
                    .color(p.text_tertiary),
            );
        }
    });
    if d.mcp_clients.is_empty() {
        ui.label(RichText::new(tr!("settings-mcp-no-clients")).size(11.5).color(p.text_tertiary));
    }
    true
}

fn install(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    if !q.trim().is_empty()
        && !matches(q, &["add to claude desktop code vs code config snippet install", CTX, tr!("settings-mcp-add-to")])
    {
        return false;
    }
    let p = palette(ui.ctx());
    let s = &mut app.settings;
    let (d, st) = (&mut s.draft.ai, &mut s.connections);
    ui.add_space(14.0);
    ui.label(RichText::new(super::widgets::upper(tr!("settings-mcp-add-to"))).size(10.5).color(p.text_tertiary));
    ui.add_space(6.0);
    let busy = matches!(st.install, Some(Install::Loading(..) | Install::Writing(..)));
    ui.horizontal_wrapped(|ui| {
        for t in Target::ALL {
            if small_button(ui, Some(ph::PLUS_CIRCLE), t.label(), false).clicked() && !busy {
                let exe = exe();
                st.install = Some(Install::Loading(t, spawn(ui, move || soyuz_mcp::install::preview(t, &exe))));
            }
        }
        if busy {
            ui.add(egui::Spinner::new().size(14.0));
        }
    });
    let mut cancel = false;
    let mut write = None;
    if let Some(Install::Ready(pv)) = &st.install {
        ui.add_space(8.0);
        card(ui, |ui| {
            subheading(ui, &format!("{} — {}", pv.target.label(), pv.path.display()));
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("settings-mcp-snippet")).size(11.5).color(p.text_tertiary));
            let mut snippet = pv.target.snippet(&exe());
            ui.add(
                TextEdit::multiline(&mut snippet)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(6),
            );
            ui.add_space(4.0);
            if !pv.changed() {
                ui.label(RichText::new(tr!("settings-mcp-already")).size(11.5).color(p.success));
            } else {
                ui.label(
                    RichText::new(if pv.old_text.is_some() {
                        tr!("settings-mcp-file-changes")
                    } else {
                        tr!("settings-mcp-new-file")
                    })
                    .size(11.5)
                    .color(p.text_tertiary),
                );
                diff_view(ui, &pv.diff);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if pv.changed()
                    && small_button(ui, Some(ph::CHECK), &tr!("settings-mcp-write-to", app = pv.target.label()), false)
                        .clicked()
                {
                    write = Some(pv.clone());
                }
                if small_button(ui, Some(ph::COPY), tr!("settings-mcp-copy-snippet"), false).clicked() {
                    ui.ctx().copy_text(snippet.clone());
                }
                if small_button(ui, Some(ph::X), tr!("settings-close-button"), false).clicked() {
                    cancel = true;
                }
            });
            ui.label(RichText::new(tr!("settings-mcp-backup")).size(11.0).color(p.text_tertiary));
        });
    }
    if cancel {
        st.install = None;
    }
    if let Some(pv) = write {
        // The user set this client up explicitly: register it as approved.
        let name = pv.target.client_name();
        match d.mcp_clients.iter_mut().find(|c| trust::sanitize_name(&c.name) == name) {
            Some(c) => c.pending = false,
            None => d.mcp_clients.push(AiMcpClient { name: name.into(), ..Default::default() }),
        }
        let t = pv.target;
        st.install = Some(Install::Writing(t, spawn(ui, move || soyuz_mcp::install::apply(&pv))));
    }
    true
}

/// Changed lines with three lines of context.
fn diff_view(ui: &mut Ui, diff: &[DiffLine]) {
    let p = palette(ui.ctx());
    let changed: Vec<bool> = diff.iter().map(|l| !matches!(l, DiffLine::Same(_))).collect();
    let near = |i: usize| (i.saturating_sub(3)..(i + 4).min(diff.len())).any(|j| changed[j]);
    Frame::new().fill(p.bg_window).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
        ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
            let mut gap = false;
            for (i, l) in diff.iter().enumerate() {
                if !near(i) {
                    if !gap {
                        ui.label(RichText::new("⋯").monospace().size(11.0).color(p.text_tertiary));
                        gap = true;
                    }
                    continue;
                }
                gap = false;
                let (sign, text, color) = match l {
                    DiffLine::Same(t) => (" ", t, p.text_tertiary),
                    DiffLine::Added(t) => ("+", t, p.success),
                    DiffLine::Removed(t) => ("-", t, p.danger),
                };
                ui.label(RichText::new(format!("{sign} {text}")).monospace().size(11.0).color(color));
            }
        });
    });
}

fn servers(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    if !q.trim().is_empty() && !matches(q, &["external mcp servers tools mount git", CTX, tr!("settings-mcp-servers")])
    {
        return false;
    }
    let p = palette(ui.ctx());
    let s = &mut app.settings;
    let (d, st) = (&mut s.draft.ai, &mut s.connections);
    ui.add_space(14.0);
    ui.label(RichText::new(super::widgets::upper(tr!("settings-mcp-servers"))).size(10.5).color(p.text_tertiary));
    ui.add(egui::Label::new(RichText::new(tr!("settings-mcp-servers-intro")).size(11.5).color(p.text_tertiary)).wrap());
    let mut remove = None;
    for (i, sv) in d.mcp_servers.iter_mut().enumerate() {
        ui.add_space(6.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(icons::icon(ph::PLUGS, 16.0).color(p.accent));
                ui.label(RichText::new(&sv.name).strong().color(p.text_primary));
                switch(ui, &mut sv.enabled);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_button(ui, Some(ph::TRASH), "", true)
                        .on_hover_text(tr!("settings-mcp-remove-server"))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            });
            ui.add_space(6.0);
            egui::Grid::new(("mcp_server", i)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                let lbl = |ui: &mut Ui, t: &str| {
                    ui.label(RichText::new(t).size(12.0).color(p.text_secondary));
                };
                lbl(ui, tr!("settings-commands-command"));
                code(ui, &mut sv.command, tr!("settings-mcp-command-hint"), 320.0);
                ui.end_row();
                lbl(ui, tr!("settings-terminal-args"));
                let t = st.args_text.entry(sv.name.clone()).or_insert_with(|| sv.args.join("\n"));
                if ui
                    .add(
                        TextEdit::multiline(t)
                            .desired_rows(1)
                            .desired_width(320.0)
                            .hint_text(tr!("settings-mcp-args-hint"))
                            .font(egui::TextStyle::Monospace),
                    )
                    .changed()
                {
                    sv.args = t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
                }
                ui.end_row();
                lbl(ui, tr!("settings-mcp-url"));
                code(ui, &mut sv.url, "https://…/mcp (Streamable HTTP)", 320.0);
                ui.end_row();
                if !sv.url.trim().is_empty() {
                    lbl(ui, tr!("settings-mcp-server-token"));
                    code(ui, &mut sv.token, tr!("settings-ai-key-ref-hint"), 320.0);
                    ui.end_row();
                }
                lbl(ui, tr!("settings-mcp-tools-may"));
                ui.horizontal(|ui| risk_picker(ui, &mut sv.risk));
                ui.end_row();
                lbl(ui, tr!("settings-mcp-ask-run"));
                ui.horizontal(|ui| {
                    switch(ui, &mut sv.require_approval);
                });
                ui.end_row();
                lbl(ui, tr!("settings-mcp-trust-hints"));
                ui.horizontal(|ui| {
                    switch(ui, &mut sv.trust_annotations);
                    ui.label(RichText::new(tr!("settings-mcp-trust-hints-desc")).size(11.0).color(p.text_tertiary));
                });
                ui.end_row();
                lbl(ui, tr!("settings-mcp-timeout"));
                ui.add(egui::DragValue::new(&mut sv.timeout_secs).range(5..=600).suffix(" s"));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let running = matches!(st.probes.get(&sv.name), Some(Probe::Running(_)));
                if running {
                    ui.add(egui::Spinner::new().size(14.0));
                } else if small_button(ui, Some(ph::PLUGS_CONNECTED), tr!("settings-mcp-test"), false).clicked() {
                    let cfg = sv.clone();
                    let rx = spawn(ui, move || soyuz_mcp::client::probe_summary(&cfg));
                    st.probes.insert(sv.name.clone(), Probe::Running(rx));
                }
                if let Some(Probe::Done(r)) = st.probes.get(&sv.name) {
                    let (icon, color, text) = match r {
                        Ok(t) => (ph::CHECK_CIRCLE, p.success, t.clone()),
                        Err(e) => (ph::X_CIRCLE, p.danger, e.clone()),
                    };
                    ui.label(icons::icon(icon, 14.0).color(color));
                    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(color)).wrap());
                }
            });
        });
    }
    if let Some(i) = remove {
        d.mcp_servers.remove(i);
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        code(ui, &mut st.new_server, tr!("settings-mcp-server-hint"), 180.0);
        let name = st.new_server.trim().to_string();
        let taken = d.mcp_servers.iter().any(|s| s.name == name);
        if small_button(ui, Some(ph::PLUS), tr!("settings-mcp-add-server"), false).clicked()
            && !name.is_empty()
            && !taken
        {
            d.mcp_servers.push(AiMcpServer { name, ..Default::default() });
            st.new_server.clear();
        }
    });
    true
}

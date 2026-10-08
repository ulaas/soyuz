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
        &[(AiRisk::Read, "Read only"), (AiRisk::Write, "Read + write"), (AiRisk::Destructive, "Incl. delete")],
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
                self.notice = Some((format!("{what} failed: {e}"), true));
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
                Ok(Ok(())) => {
                    self.notice =
                        Some((format!("Added Soyuz to {}. Restart {} to pick it up.", t.label(), t.label()), false))
                }
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

    if !searching || matches(q, &["connections mcp", CTX]) {
        ui.add_space(18.0);
        ui.label(RichText::new("CONNECTIONS (MCP)").size(10.5).color(p.text_tertiary));
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Let AI apps such as Claude Desktop, Claude Code or VS Code use Soyuz through the Model Context \
                     Protocol. They work only inside the folders you allow; changes show up here as a plan you approve, \
                     then run as undoable jobs. Every call is written to ai-audit.jsonl.",
                )
                .size(11.5)
                .color(p.text_tertiary),
            )
            .wrap(),
        );
        if let Some((msg, err)) = &app.settings.connections.notice {
            ui.add_space(4.0);
            ui.label(RichText::new(msg).size(11.5).color(if *err { p.danger } else { p.success }));
        }
    }

    // --- server ----------------------------------------------------------------------
    {
        let s = &mut app.settings;
        let (d, st) = (&mut s.draft.ai, &mut s.connections);
        any |= group(ui, q, CTX, "Soyuz MCP server", |r| {
            r.row_kw(
                "Let `soyuz --mcp` attach to this window",
                "Tool calls then show in this window and write tools get an approval card. Off = `soyuz --mcp` runs headless.",
                "pipe stdio attach",
                |ui| {
                    switch(ui, &mut d.mcp.server_enabled);
                },
            );
            r.row_kw("Status", "", "pipe http running", |ui| {
                let mut parts = Vec::new();
                match &status.pipe {
                    Some(Ok(n)) => parts.push((format!("pipe {n}"), false)),
                    Some(Err(e)) => parts.push((format!("pipe {e}"), true)),
                    None => parts.push(("pipe off".into(), false)),
                }
                match &status.http {
                    Some(Ok(a)) => parts.push((a.clone(), false)),
                    Some(Err(e)) => parts.push((e.clone(), true)),
                    None => {}
                }
                if status.waiting > 0 {
                    parts.push((format!("{} waiting for approval", status.waiting), false));
                }
                for (t, err) in parts.into_iter().rev() {
                    ui.label(RichText::new(t).size(11.5).color(if err { p.warning } else { p.text_secondary }));
                }
            });
            r.row_kw(
                "HTTP transport",
                "Streamable HTTP on 127.0.0.1 for clients that cannot start programs. Each client needs a token.",
                "http port network token",
                |ui| {
                    switch(ui, &mut d.mcp.http_enabled);
                },
            );
            if d.mcp.http_enabled {
                r.row("HTTP port", "Endpoint: http://127.0.0.1:<port>/mcp", |ui| {
                    number(ui, &mut d.mcp.http_port, 1024..=65535, "");
                });
                r.block(
                    "Allowed browser origins",
                    "One per line. Loopback origins are always allowed; anything else is refused (DNS-rebinding defence).",
                    |ui| {
                        let t = st.origins_text.get_or_insert_with(|| d.mcp.allowed_origins.join("\n"));
                        if ui.add(TextEdit::multiline(t).desired_rows(1).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace)).changed() {
                            d.mcp.allowed_origins = t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
                        }
                    },
                );
            }
            r.row_kw(
                "Register unknown clients",
                "A new client is added as “pending” and can only read until you approve it below. Off = refuse it.",
                "pending approve new",
                |ui| {
                    switch(ui, &mut d.mcp.register_unknown);
                },
            );
            r.row("Approval timeout", "Refuse a change when nobody answers its approval card.", |ui| {
                number(ui, &mut d.mcp.approval_timeout_secs, 10..=3600, " s");
            });
            r.row_kw(
                "Approval cards",
                "Where a client's plan card appears: in the assistant panel (it opens when a request arrives) or as a modal card over the window.",
                "approval panel dialog modal plan card",
                |ui| {
                    segmented(
                        ui,
                        &mut d.mcp.approval_ui,
                        &[
                            (soyuz_core::config::AiMcpApprovalUi::Panel, "Assistant panel"),
                            (soyuz_core::config::AiMcpApprovalUi::Dialog, "Modal card"),
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
    if !q.trim().is_empty() && !matches(q, &["mcp clients approve pending token scope risk folders", CTX]) {
        return false;
    }
    let p = palette(ui.ctx());
    let http = app.settings.draft.ai.mcp.http_enabled;
    let port = app.settings.draft.ai.mcp.http_port;
    let s = &mut app.settings;
    let (d, st) = (&mut s.draft.ai, &mut s.connections);
    ui.add_space(14.0);
    ui.label(RichText::new("MCP CLIENTS").size(10.5).color(p.text_tertiary));
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
                    ui.label(RichText::new("pending · read-only").size(11.5).color(p.warning));
                    if small_button(ui, Some(ph::CHECK), "Approve", false).clicked() {
                        c.pending = false;
                    }
                } else {
                    ui.label(RichText::new("approved").size(11.5).color(p.success));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_button(ui, Some(ph::TRASH), "", true).on_hover_text("Remove client").clicked() {
                        remove = Some(i);
                    }
                });
            });
            ui.add_space(6.0);
            egui::Grid::new(("mcp_client", i)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                let lbl = |ui: &mut Ui, t: &str| {
                    ui.label(RichText::new(t).size(12.0).color(p.text_secondary));
                };
                lbl(ui, "May");
                ui.horizontal(|ui| risk_picker(ui, &mut c.max_risk));
                ui.end_row();
                lbl(ui, "Ask before changes");
                ui.horizontal(|ui| {
                    switch(ui, &mut c.require_approval);
                    if !c.require_approval {
                        ui.label(RichText::new("changes run without asking").size(11.0).color(p.warning));
                    }
                });
                ui.end_row();
                lbl(ui, "Folders");
                let t = st.roots_text.entry(c.name.clone()).or_insert_with(|| {
                    c.allowed_roots.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n")
                });
                if ui
                    .add(
                        TextEdit::multiline(t)
                            .desired_rows(1)
                            .desired_width(360.0)
                            .hint_text("empty = the assistant's allowed folders")
                            .font(egui::TextStyle::Monospace),
                    )
                    .changed()
                {
                    c.allowed_roots = lines_to_paths(t);
                }
                ui.end_row();
                if http {
                    lbl(ui, "HTTP token");
                    ui.horizontal(|ui| {
                        let has = !c.token.is_empty();
                        if has {
                            ui.label(icons::icon(ph::LOCK_KEY, 13.0).color(p.success));
                        }
                        let label = if has { "Rotate token" } else { "Generate token" };
                        if small_button(ui, Some(ph::KEY), label, false).clicked() {
                            let token = trust::generate_token();
                            let cred = trust::token_credential(&c.name);
                            c.token = format!("credential:{cred}");
                            st.shown_token = Some((c.name.clone(), token.clone()));
                            st.ops.push((
                                "Token stored in the credential store".into(),
                                ai_service::store_secret(cred, Secret::new(token), wake_of(ui)),
                            ));
                        }
                        if has && small_button(ui, Some(ph::X), "Revoke", true).clicked() {
                            if let Ok(soyuz_ai::credentials::KeyRef::Credential(n)) =
                                soyuz_ai::credentials::parse_ref(&c.token)
                            {
                                st.ops.push(("Token revoked".into(), ai_service::delete_secret(n, wake_of(ui))));
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
                ui.label(
                    RichText::new("Copy this token now; Soyuz will not show it again.").size(11.5).color(p.warning),
                );
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
                    if small_button(ui, Some(ph::COPY), "Copy token", false).clicked() {
                        ui.ctx().copy_text(token.clone());
                    }
                    if small_button(ui, Some(ph::COPY), "Copy config", false).clicked() {
                        ui.ctx().copy_text(snippet.clone());
                    }
                });
            }
        });
    }
    if let Some(i) = remove {
        let c = d.mcp_clients.remove(i);
        if let Ok(soyuz_ai::credentials::KeyRef::Credential(n)) = soyuz_ai::credentials::parse_ref(&c.token) {
            st.ops.push((format!("Removed “{}” and its token", c.name), ai_service::delete_secret(n, wake_of(ui))));
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        code(ui, &mut st.new_client, "client name, e.g. my-agent", 220.0);
        let name = trust::sanitize_name(&st.new_client);
        let taken = d.mcp_clients.iter().any(|c| trust::sanitize_name(&c.name) == name);
        if small_button(ui, Some(ph::PLUS), "Add client", false).clicked() && !name.is_empty() && !taken {
            d.mcp_clients.push(AiMcpClient { name: name.clone(), ..Default::default() });
            st.new_client.clear();
        }
        if !name.is_empty() {
            ui.label(
                RichText::new(format!("starts with: soyuz --mcp --mcp-client {name}"))
                    .monospace()
                    .size(11.0)
                    .color(p.text_tertiary),
            );
        }
    });
    if d.mcp_clients.is_empty() {
        ui.label(
            RichText::new("No clients yet. Use “Add Soyuz to …” below, or connect one: it appears here as pending.")
                .size(11.5)
                .color(p.text_tertiary),
        );
    }
    true
}

fn install(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    if !q.trim().is_empty() && !matches(q, &["add to claude desktop code vs code config snippet install", CTX]) {
        return false;
    }
    let p = palette(ui.ctx());
    let s = &mut app.settings;
    let (d, st) = (&mut s.draft.ai, &mut s.connections);
    ui.add_space(14.0);
    ui.label(RichText::new("ADD SOYUZ TO …").size(10.5).color(p.text_tertiary));
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
            ui.label(RichText::new("Snippet").size(11.5).color(p.text_tertiary));
            let mut snippet = pv.target.snippet(&exe());
            ui.add(
                TextEdit::multiline(&mut snippet)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(6),
            );
            ui.add_space(4.0);
            if !pv.changed() {
                ui.label(RichText::new("Already set up: the file contains this entry.").size(11.5).color(p.success));
            } else {
                ui.label(
                    RichText::new(if pv.old_text.is_some() { "Changes to the file" } else { "New file" })
                        .size(11.5)
                        .color(p.text_tertiary),
                );
                diff_view(ui, &pv.diff);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if pv.changed()
                    && small_button(ui, Some(ph::CHECK), &format!("Write to {}", pv.target.label()), false).clicked()
                {
                    write = Some(pv.clone());
                }
                if small_button(ui, Some(ph::COPY), "Copy snippet", false).clicked() {
                    ui.ctx().copy_text(snippet.clone());
                }
                if small_button(ui, Some(ph::X), "Close", false).clicked() {
                    cancel = true;
                }
            });
            ui.label(
                RichText::new("The old file is kept as .soyuz-backup next to it.").size(11.0).color(p.text_tertiary),
            );
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
    if !q.trim().is_empty() && !matches(q, &["external mcp servers tools mount git", CTX]) {
        return false;
    }
    let p = palette(ui.ctx());
    let s = &mut app.settings;
    let (d, st) = (&mut s.draft.ai, &mut s.connections);
    ui.add_space(14.0);
    ui.label(RichText::new("EXTERNAL MCP SERVERS").size(10.5).color(p.text_tertiary));
    ui.add(
        egui::Label::new(
            RichText::new(
                "Tools from other MCP servers the in-app assistant may use. Their descriptions and results are treated \
                 as untrusted; tools above “read” ask for approval first.",
            )
            .size(11.5)
            .color(p.text_tertiary),
        )
        .wrap(),
    );
    let mut remove = None;
    for (i, sv) in d.mcp_servers.iter_mut().enumerate() {
        ui.add_space(6.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(icons::icon(ph::PLUGS, 16.0).color(p.accent));
                ui.label(RichText::new(&sv.name).strong().color(p.text_primary));
                switch(ui, &mut sv.enabled);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_button(ui, Some(ph::TRASH), "", true).on_hover_text("Remove server").clicked() {
                        remove = Some(i);
                    }
                });
            });
            ui.add_space(6.0);
            egui::Grid::new(("mcp_server", i)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                let lbl = |ui: &mut Ui, t: &str| {
                    ui.label(RichText::new(t).size(12.0).color(p.text_secondary));
                };
                lbl(ui, "Command");
                code(ui, &mut sv.command, "program (stdio), e.g. uvx", 320.0);
                ui.end_row();
                lbl(ui, "Arguments");
                let t = st.args_text.entry(sv.name.clone()).or_insert_with(|| sv.args.join("\n"));
                if ui
                    .add(
                        TextEdit::multiline(t)
                            .desired_rows(1)
                            .desired_width(320.0)
                            .hint_text("one per line")
                            .font(egui::TextStyle::Monospace),
                    )
                    .changed()
                {
                    sv.args = t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
                }
                ui.end_row();
                lbl(ui, "or URL");
                code(ui, &mut sv.url, "https://…/mcp (Streamable HTTP)", 320.0);
                ui.end_row();
                if !sv.url.trim().is_empty() {
                    lbl(ui, "Token");
                    code(ui, &mut sv.token, "credential:<name> or env:<VAR>", 320.0);
                    ui.end_row();
                }
                lbl(ui, "Its tools may");
                ui.horizontal(|ui| risk_picker(ui, &mut sv.risk));
                ui.end_row();
                lbl(ui, "Ask before running");
                ui.horizontal(|ui| {
                    switch(ui, &mut sv.require_approval);
                });
                ui.end_row();
                lbl(ui, "Trust read-only hints");
                ui.horizontal(|ui| {
                    switch(ui, &mut sv.trust_annotations);
                    ui.label(RichText::new("only for servers you trust").size(11.0).color(p.text_tertiary));
                });
                ui.end_row();
                lbl(ui, "Timeout");
                ui.add(egui::DragValue::new(&mut sv.timeout_secs).range(5..=600).suffix(" s"));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let running = matches!(st.probes.get(&sv.name), Some(Probe::Running(_)));
                if running {
                    ui.add(egui::Spinner::new().size(14.0));
                } else if small_button(ui, Some(ph::PLUGS_CONNECTED), "Test", false).clicked() {
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
        code(ui, &mut st.new_server, "server name", 180.0);
        let name = st.new_server.trim().to_string();
        let taken = d.mcp_servers.iter().any(|s| s.name == name);
        if small_button(ui, Some(ph::PLUS), "Add server", false).clicked() && !name.is_empty() && !taken {
            d.mcp_servers.push(AiMcpServer { name, ..Default::default() });
            st.new_server.clear();
        }
    });
    true
}

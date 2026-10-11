//! Drag-out, the egui side: hands an internal drag that left the window to the OS
//! (Windows OLE loop, Linux X11 XDND / Wayland `wl_data_source` sessions) and feeds egui
//! the synthetic input afterwards. The decisions (trigger, formats, follow-up) are
//! [`crate::ops::fileops::dragout`]; see there for the full behaviour.

#[cfg(windows)]
use soyuz_fs::dragout::DROPEFFECT_LINK;
#[cfg(any(windows, target_os = "linux"))]
use soyuz_fs::dragout::DragReport;
#[cfg(target_os = "linux")]
use soyuz_fs::dragout::Outcome;
#[cfg(windows)]
use soyuz_fs::dragout::{DROPEFFECT_COPY, DROPEFFECT_MOVE};

use egui::{Id, Pos2};

use super::{DragPayload, payload};
use crate::app::SoyuzApp;
#[cfg(windows)]
use crate::ops::fileops::dragout::build_data;
#[cfg(target_os = "linux")]
use crate::ops::fileops::dragout::preferred_effect;
use crate::ops::fileops::dragout::should_hand_off;
#[cfg(any(windows, target_os = "linux"))]
use crate::ops::fileops::dragout::{AfterDrop, after_drop};
use crate::ui::geom;

fn reset_id() -> Id {
    Id::new("soyuz.dragout.reset")
}

/// Whether dragging items out of the window into other apps works here (Windows: OLE;
/// Linux: X11 / XWayland via XDND, Wayland via `wl_data_source` once the data device is
/// up). The Settings UI gates `[behaviour] drag_out` on this.
pub fn supported() -> bool {
    #[cfg(windows)]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        use super::linux_dnd::{WindowSystem, window_system};
        match window_system() {
            WindowSystem::X11(_) => true,
            WindowSystem::Wayland => soyuz_fs::dragout::wayland_drag_ready(),
            WindowSystem::Other => false,
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// End of frame (before the drag badge): hand an internal drag that left the window to
/// the OS. A no-op where [`supported`] is false or with `[behaviour] drag_out = false`.
pub fn end_frame(app: &mut SoyuzApp, ctx: &egui::Context) {
    #[cfg(target_os = "linux")]
    poll_session(app, ctx);
    if !supported() || !app.config.behaviour.drag_out {
        return;
    }
    let Some(payload) = payload(ctx) else { return };
    let (primary, pointer, screen) =
        ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos(), i.viewport_rect()));
    if !should_hand_off(true, primary, pointer.map(geom::point), geom::area(screen)) {
        return;
    }
    egui::DragAndDrop::clear_payload(ctx);
    ctx.request_repaint();
    // The button may have been released outside the window without us seeing it
    // (capture lost): never start an OS drag then, it would drop at once.
    if !soyuz_fs::dragout::primary_button_down() {
        ctx.data_mut(|d| d.insert_temp(reset_id(), true));
        return;
    }
    #[cfg(windows)]
    {
        let paths = payload.paths.clone();
        let data = build_data(&paths, payload.default);
        let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
        // The window the drag started in (main or an in-process secondary window): coming
        // back over it resumes the internal drag.
        let hwnd =
            crate::ui::native::hwnd_of(ctx, ctx.viewport_id()).unwrap_or_else(crate::ui::native_chrome::main_hwnd);
        let report = match soyuz_fs::dragout::do_drag(&data, allowed, hwnd) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("drag-out failed: {e}");
                ctx.data_mut(|d| d.insert_temp(reset_id(), true));
                return;
            }
        };
        log::debug!("drag-out of {} item(s): {report:?}", paths.len());
        finish(app, ctx, &report, &payload, true);
    }
    #[cfg(target_os = "linux")]
    start_session(ctx, &payload);
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = payload;
}

/// Act on a finished OS drag. `reset`: feed egui a button release afterwards (the real
/// one went to the OS drag loop / the compositor).
#[cfg(any(windows, target_os = "linux"))]
fn finish(app: &mut SoyuzApp, ctx: &egui::Context, report: &DragReport, payload: &DragPayload, reset: bool) {
    let paths = &payload.paths;
    let action = after_drop(report, paths, |p| p.symlink_metadata().is_ok());
    if action != AfterDrop::Resume && reset {
        ctx.data_mut(|d| d.insert_temp(reset_id(), true));
    }
    match action {
        AfterDrop::Resume => egui::DragAndDrop::set_payload(ctx, DragPayload::clone(payload)),
        action => crate::ops::fileops::dragout::apply_after_drop(app, action, paths),
    }
}

/// The running non-modal Linux drag-out and what it carries.
#[cfg(target_os = "linux")]
static SESSION: parking_lot::Mutex<Option<(soyuz_fs::dragout::DragSession, DragPayload, bool)>> =
    parking_lot::Mutex::new(None);

/// The payload of the running Linux drag-out (our Wayland drag coming back over the
/// window resumes it as the internal drag).
pub(super) fn session_payload() -> Option<DragPayload> {
    #[cfg(target_os = "linux")]
    {
        SESSION.lock().as_ref().map(|(_, p, _)| p.clone())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn start_session(ctx: &egui::Context, payload: &DragPayload) {
    use super::linux_dnd::{WindowSystem, window_system};
    // X11: the real button release still reaches the window (implicit grab), so no
    // synthetic one; Wayland: the compositor's drag swallows it.
    let (started, reset) = match window_system() {
        WindowSystem::X11(own) => {
            (soyuz_fs::dragout::start_x11_drag(&payload.paths, preferred_effect(payload.default), own), false)
        }
        WindowSystem::Wayland => (soyuz_fs::dragout::start_wayland_drag(&payload.paths), true),
        WindowSystem::Other => return,
    };
    match started {
        Ok(s) => {
            log::debug!("drag-out of {} item(s) started", payload.paths.len());
            *SESSION.lock() = Some((s, DragPayload::clone(payload), reset));
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        Err(e) => {
            log::warn!("drag-out failed: {e}");
            ctx.data_mut(|d| d.insert_temp(reset_id(), true));
        }
    }
}

/// Every frame while a Linux drag-out runs: Esc cancels it, its report is acted on.
#[cfg(target_os = "linux")]
fn poll_session(app: &mut SoyuzApp, ctx: &egui::Context) {
    let mut guard = SESSION.lock();
    let Some((session, _, _)) = guard.as_ref() else { return };
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        session.cancel();
    }
    let Some(report) = session.poll() else {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        return;
    };
    let Some((_, payload, reset)) = guard.take() else { return };
    drop(guard);
    log::debug!("drag-out of {} item(s): {report:?}", payload.paths.len());
    if report.outcome == Outcome::Cancelled {
        // A cancelled Wayland drag may have been resumed over the window (`Own*` events):
        // the synthetic release must not drop it there.
        egui::DragAndDrop::clear_payload(ctx);
    }
    finish(app, ctx, &report, &payload, reset);
}

/// `raw_input_hook`: after an OS drag, tell egui the button is up and the pointer left
/// (the real `WM_LBUTTONUP` was consumed by the OLE loop / the Wayland compositor). Also
/// feeds the Linux Wayland drop target's events ([`super::linux_dnd::raw_input`]).
pub fn raw_input(ctx: &egui::Context, raw: &mut egui::RawInput) {
    super::linux_dnd::raw_input(ctx, raw);
    if ctx.data_mut(|d| d.remove_temp::<bool>(reset_id())).is_none() {
        return;
    }
    let (pos, modifiers) = ctx.input(|i| (i.pointer.latest_pos().or(i.pointer.interact_pos()), i.modifiers));
    let pos = pos.unwrap_or(Pos2::ZERO);
    raw.events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    raw.events.push(egui::Event::PointerGone);
}

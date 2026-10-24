//! `WindowCmd::Raise`: bring a window to the front for a launch another `soyuz` process
//! handed over (`services::instance`). On Windows the launcher granted this process the
//! foreground (`AllowSetForegroundWindow`), so a plain `SetForegroundWindow` on the
//! viewport's window works; egui's `ViewportCommand::Focus` would not do: winit forces
//! the activation by injecting an Alt key press. A window created this frame has no
//! native handle yet, so requests wait (up to [`PATIENCE`]) for [`super::hwnd_of`].
//! Elsewhere it is un-minimize + focus (no synthetic input there; Wayland compositors
//! may only flash the window: the launcher's activation token cannot reach winit).

use std::time::{Duration, Instant};

use egui::{Context, ViewportCommand, ViewportId};
use parking_lot::Mutex;

/// How long a request waits for its window to appear.
const PATIENCE: Duration = Duration::from_secs(3);

static PENDING: Mutex<Vec<(ViewportId, Instant)>> = Mutex::new(Vec::new());

/// Scripted runs (`SOYUZ_WINDOW_POS`, `SOYUZ_INSTANCE_TEST`) never take the foreground.
fn suppressed() -> bool {
    super::quiet() || std::env::var_os("SOYUZ_INSTANCE_TEST").is_some()
}

/// Raise viewport `vp` (now, or once its window exists).
pub fn request(ctx: &Context, vp: ViewportId) {
    if suppressed() {
        log::info!("instance: not raising {vp:?} (scripted run)");
        return;
    }
    if !cfg!(windows) {
        ctx.send_viewport_cmd_to(vp, ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(vp, ViewportCommand::Focus);
        return;
    }
    let mut p = PENDING.lock();
    p.retain(|(v, _)| *v != vp);
    p.push((vp, Instant::now() + PATIENCE));
    ctx.request_repaint();
}

/// End of the main window's frame: raise the windows that have a native handle now.
pub(super) fn poll(ctx: &Context) {
    let pending = std::mem::take(&mut *PENDING.lock());
    if pending.is_empty() {
        return;
    }
    let now = Instant::now();
    let mut keep = Vec::new();
    for (vp, until) in pending {
        match super::hwnd_of(ctx, vp) {
            Some(h) => {
                let ok = soyuz_fs::instance::foreground(h);
                log::info!("instance: raised window {h:#x} ({})", if ok { "foreground" } else { "refused" });
            }
            None if now < until => keep.push((vp, until)),
            None => log::info!("instance: window {vp:?} never appeared; not raised"),
        }
    }
    if !keep.is_empty() {
        PENDING.lock().extend(keep);
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

//! Linux external drag and drop glue.
//!
//! * **Wayland drop target**: winit 0.30 delivers no file drops on Wayland, so
//!   [`init`] starts `soyuz_fs`'s drop target on the window's own `wl_display` /
//!   `wl_surface`, and [`raw_input`] (from the `raw_input_hook`) turns its events into
//!   exactly what winit produces on X11 / Windows: `raw.hovered_files` while a file drag
//!   is over the window, `PointerMoved` for its position (the compositor sends no pointer
//!   events during a drag), and `raw.dropped_files` on drop. Everything downstream (pane
//!   highlight, Portal pinning, shelf, `external_drops`) is unchanged.
//! * **Drag-out**: which window system the window is on ([`window_system`]) decides how
//!   `dragout::end_frame` hands an internal drag to the OS (X11 XDND or Wayland
//!   `wl_data_source`).
//!
//! If anything fails, it is logged once and drag and drop silently stays as before.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Event, HoveredFile, Pos2, RawInput};
use parking_lot::Mutex;
use soyuz_fs::dragout::DropTargetEvent;

/// The window system the main window runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowSystem {
    /// Not Linux, or no window handle (tests).
    Other,
    /// X11 (or XWayland) with the window's id.
    X11(u32),
    /// Native Wayland (drop target started).
    Wayland,
}

struct Shared {
    system: WindowSystem,
    events: Option<crossbeam_channel::Receiver<DropTargetEvent>>,
    injector: DropInjector,
}

static SHARED: Mutex<Shared> =
    Mutex::new(Shared { system: WindowSystem::Other, events: None, injector: DropInjector::new() });

/// The main window's window system (set by [`init`]).
pub fn window_system() -> WindowSystem {
    SHARED.lock().system
}

/// Find the window system from the creation context and, on Wayland, start the drop
/// target (its setup runs on its own thread).
pub fn init(cc: &eframe::CreationContext<'_>) {
    #[cfg(target_os = "linux")]
    {
        use raw_window_handle::{HasDisplayHandle as _, HasWindowHandle as _, RawDisplayHandle, RawWindowHandle};
        let window = cc.window_handle().ok().map(|h| h.as_raw());
        let display = cc.display_handle().ok().map(|h| h.as_raw());
        let system = match (display, window) {
            (Some(RawDisplayHandle::Wayland(d)), Some(RawWindowHandle::Wayland(w))) => {
                let (tx, rx) = crossbeam_channel::unbounded();
                let ctx = cc.egui_ctx.clone();
                let sink = Box::new(move |e: DropTargetEvent| {
                    let _ = tx.send(e);
                    ctx.request_repaint();
                });
                let started = soyuz_fs::dragout::start_wayland_drop_target(
                    d.display.as_ptr() as usize,
                    w.surface.as_ptr() as usize,
                    sink,
                );
                match started {
                    Ok(()) => {
                        SHARED.lock().events = Some(rx);
                        WindowSystem::Wayland
                    }
                    Err(e) => {
                        log::warn!("Wayland drag and drop unavailable: {e}");
                        WindowSystem::Other
                    }
                }
            }
            (_, Some(RawWindowHandle::Xlib(w))) => WindowSystem::X11(w.window as u32),
            (_, Some(RawWindowHandle::Xcb(w))) => WindowSystem::X11(w.window.get()),
            _ => WindowSystem::Other,
        };
        log::debug!("window system for drag and drop: {system:?}");
        SHARED.lock().system = system;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cc;
}

/// Stop the Wayland drop target before the window and its connection go away.
pub fn shutdown() {
    let mut s = SHARED.lock();
    if s.events.take().is_some() {
        soyuz_fs::dragout::stop_wayland_drop_target();
    }
}

/// `raw_input_hook`: feed the Wayland drop target's events to egui (root viewport only;
/// the drop target only watches the main window's surface).
pub fn raw_input(ctx: &egui::Context, raw: &mut RawInput) {
    if raw.viewport_id != egui::ViewportId::ROOT {
        return;
    }
    let change = {
        let mut s = SHARED.lock();
        let Some(rx) = &s.events else { return };
        let events: Vec<DropTargetEvent> = rx.try_iter().collect();
        s.injector.apply(events, ctx.zoom_factor(), ctx.input(|i| i.modifiers), raw)
    };
    match change {
        PayloadChange::Keep => {}
        PayloadChange::Restore => {
            if let Some(p) = super::dragout::session_payload() {
                egui::DragAndDrop::set_payload(ctx, p);
            }
        }
        PayloadChange::Clear => egui::DragAndDrop::clear_payload(ctx),
    }
}

/// What a batch of events does to the internal drag (our own Wayland drag-out coming
/// back over the window resumes it; leaving again suspends it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadChange {
    Keep,
    Restore,
    Clear,
}

/// Turns drop-target events into egui input, keeping the hover state across frames.
#[derive(Debug, Default)]
pub struct DropInjector {
    hovering: bool,
    drop_pos: Option<Pos2>,
}

impl DropInjector {
    pub const fn new() -> Self {
        DropInjector { hovering: false, drop_pos: None }
    }

    /// Apply `events` (surface-local logical pixels; `zoom` = egui zoom factor) to `raw`.
    /// Own-drag events (`Own*`) move egui's pointer and end in a primary-button release
    /// on drop, so egui's own drag-and-drop does the internal drop.
    pub fn apply(
        &mut self,
        events: impl IntoIterator<Item = DropTargetEvent>,
        zoom: f32,
        modifiers: egui::Modifiers,
        raw: &mut RawInput,
    ) -> PayloadChange {
        let zoom = if zoom > 0.0 { zoom } else { 1.0 };
        let pt = |x: f64, y: f64| Pos2::new(x as f32 / zoom, y as f32 / zoom);
        let mut change = PayloadChange::Keep;
        for ev in events {
            match ev {
                DropTargetEvent::OwnEnter { x, y } => {
                    change = PayloadChange::Restore;
                    raw.events.push(Event::PointerMoved(pt(x, y)));
                }
                DropTargetEvent::OwnMotion { x, y } => raw.events.push(Event::PointerMoved(pt(x, y))),
                DropTargetEvent::OwnLeave => {
                    change = PayloadChange::Clear;
                    raw.events.push(Event::PointerGone);
                }
                DropTargetEvent::OwnDrop { x, y } => {
                    let pos = pt(x, y);
                    raw.events.push(Event::PointerMoved(pos));
                    raw.events.push(Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers,
                    });
                }
                DropTargetEvent::Enter { x, y } | DropTargetEvent::Motion { x, y } => {
                    self.hovering = true;
                    raw.events.push(Event::PointerMoved(pt(x, y)));
                }
                DropTargetEvent::Leave => {
                    self.hovering = false;
                    self.drop_pos = None;
                    raw.events.push(Event::PointerGone);
                }
                DropTargetEvent::Drop { x, y } => {
                    // Still "hovering" until the list arrives, so the target stays lit.
                    let p = pt(x, y);
                    self.drop_pos = Some(p);
                    raw.events.push(Event::PointerMoved(p));
                }
                DropTargetEvent::Files(paths) => {
                    self.hovering = false;
                    if let Some(p) = self.drop_pos.take() {
                        raw.events.push(Event::PointerMoved(p));
                    }
                    raw.dropped_files.extend(paths.into_iter().map(dropped));
                }
            }
        }
        if self.hovering && raw.hovered_files.is_empty() {
            raw.hovered_files.push(HoveredFile::default());
        }
        change
    }
}

/// A dropped local file, as egui-winit hands them over.
#[derive(Debug)]
struct LocalDrop(PathBuf);

impl egui::DroppedFile for LocalDrop {
    fn path(&self) -> &Path {
        &self.0
    }

    /// Never read on the UI thread: consumers only take the path and copy through jobs.
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Err("dropped files are transferred by path".into())
    }
}

fn dropped(path: PathBuf) -> egui::DroppedFileHandle {
    Arc::new(LocalDrop(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> RawInput {
        RawInput::default()
    }

    #[test]
    fn hover_then_drop_injects_like_winit() {
        let mut inj = DropInjector::new();
        let mut r = raw();
        inj.apply([DropTargetEvent::Enter { x: 100.0, y: 50.0 }], 2.0, egui::Modifiers::NONE, &mut r);
        assert_eq!(r.hovered_files.len(), 1, "hovering shows as an external drag");
        assert_eq!(r.events, vec![Event::PointerMoved(Pos2::new(50.0, 25.0))], "logical px / zoom");
        // Next frame without events: still hovering.
        let mut r = raw();
        inj.apply([], 1.0, egui::Modifiers::NONE, &mut r);
        assert_eq!(r.hovered_files.len(), 1);
        assert!(r.events.is_empty());
        let mut r = raw();
        inj.apply(
            [DropTargetEvent::Motion { x: 10.0, y: 20.0 }, DropTargetEvent::Drop { x: 12.0, y: 22.0 }],
            1.0,
            egui::Modifiers::NONE,
            &mut r,
        );
        assert_eq!(r.hovered_files.len(), 1, "lit until the list arrives");
        assert!(r.dropped_files.is_empty());
        let mut r = raw();
        let p = PathBuf::from("/tmp/a b.txt");
        inj.apply([DropTargetEvent::Files(vec![p.clone()])], 1.0, egui::Modifiers::NONE, &mut r);
        assert!(r.hovered_files.is_empty());
        assert_eq!(r.dropped_files.len(), 1);
        assert_eq!(r.dropped_files[0].path(), p.as_path());
        assert_eq!(r.events, vec![Event::PointerMoved(Pos2::new(12.0, 22.0))], "drop position wins");
        let mut r = raw();
        inj.apply([], 1.0, egui::Modifiers::NONE, &mut r);
        assert!(r.hovered_files.is_empty() && r.dropped_files.is_empty() && r.events.is_empty());
    }

    #[test]
    fn own_drag_coming_back_is_internal() {
        let mut inj = DropInjector::new();
        let mut r = raw();
        assert_eq!(
            inj.apply([DropTargetEvent::OwnEnter { x: 10.0, y: 10.0 }], 1.0, egui::Modifiers::NONE, &mut r),
            PayloadChange::Restore
        );
        assert!(r.hovered_files.is_empty(), "not an external file drag");
        let mut r = raw();
        assert_eq!(inj.apply([DropTargetEvent::OwnLeave], 1.0, egui::Modifiers::NONE, &mut r), PayloadChange::Clear);
        assert_eq!(r.events, vec![Event::PointerGone]);
        let mut r = raw();
        let ch = inj.apply(
            [DropTargetEvent::OwnEnter { x: 4.0, y: 6.0 }, DropTargetEvent::OwnDrop { x: 4.0, y: 6.0 }],
            2.0,
            egui::Modifiers::NONE,
            &mut r,
        );
        assert_eq!(ch, PayloadChange::Restore);
        assert!(matches!(
            r.events.last(),
            Some(Event::PointerButton { pressed: false, button: egui::PointerButton::Primary, pos, .. })
                if *pos == Pos2::new(2.0, 3.0)
        ));
        assert!(r.dropped_files.is_empty());
    }

    #[test]
    fn leave_clears_the_hover() {
        let mut inj = DropInjector::new();
        let mut r = raw();
        inj.apply(
            [DropTargetEvent::Enter { x: 1.0, y: 1.0 }, DropTargetEvent::Leave],
            1.0,
            egui::Modifiers::NONE,
            &mut r,
        );
        assert!(r.hovered_files.is_empty());
        assert_eq!(r.events.last(), Some(&Event::PointerGone));
        // A failed read ends the hover without dropping anything.
        let mut r = raw();
        inj.apply(
            [
                DropTargetEvent::Enter { x: 1.0, y: 1.0 },
                DropTargetEvent::Drop { x: 1.0, y: 1.0 },
                DropTargetEvent::Files(vec![]),
            ],
            1.0,
            egui::Modifiers::NONE,
            &mut r,
        );
        assert!(r.hovered_files.is_empty() && r.dropped_files.is_empty());
    }

    /// The injected drop reaches the existing consumer: `external_drops` copies into the
    /// pane under the drop position.
    #[test]
    fn injected_drop_feeds_external_drops() {
        let ctx = egui::Context::default();
        let mut inj = DropInjector::new();
        let mut r = raw();
        r.screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)));
        inj.apply(
            [
                DropTargetEvent::Enter { x: 300.0, y: 200.0 },
                DropTargetEvent::Drop { x: 300.0, y: 200.0 },
                DropTargetEvent::Files(vec![PathBuf::from("/tmp/x")]),
            ],
            1.0,
            egui::Modifiers::NONE,
            &mut r,
        );
        let mut seen = None;
        let mut out = ctx.run_ui(r, |ui| {
            seen = Some(ui.ctx().input(|i| {
                (i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect::<Vec<_>>(), i.pointer.latest_pos())
            }));
        });
        out.textures_delta.clear();
        let (paths, pos) = seen.unwrap();
        assert_eq!(paths, vec![PathBuf::from("/tmp/x")]);
        assert_eq!(pos, Some(Pos2::new(300.0, 200.0)));
        let frame = super::super::dnd::DndFrame {
            panes: vec![(
                crate::state::pane::PaneId(1),
                egui::Rect::from_min_size(Pos2::new(200.0, 100.0), egui::vec2(400.0, 400.0)),
                Some(PathBuf::from("/dest")),
            )],
            ..Default::default()
        };
        assert_eq!(
            super::super::dnd::external_target(&frame, pos, crate::state::pane::PaneId(2)),
            Some(PathBuf::from("/dest"))
        );
    }
}

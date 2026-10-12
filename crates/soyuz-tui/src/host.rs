//! [`TuiHost`]: the terminal UI's side of the `soyuz_appcore::host::Host` seam.
//!
//! The host only records requests; the frame loop drains them:
//! * wake-ups / redraws arrive on [`TuiHost::redraw_rx`] (a bounded(1) channel, so any
//!   number of wakes between two frames is one message);
//! * [`TuiHost::take_deadline`] gives the earliest `redraw_after` deadline;
//! * [`TuiHost::take_output`] gives escape sequences (OSC 52 copy, OSC 2 title) to write
//!   after the frame is drawn, so they never land in the middle of a frame;
//! * [`TuiHost::take_close`], [`TuiHost::take_modes_changed`] and [`TuiHost::theme`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use crossterm::Command as _;
use parking_lot::Mutex;
use soyuz_appcore::host::{Host, ThemePref, Wake, WindowCmd, WindowId};
use soyuz_core::config::Config;

/// The terminal UI's host. Shared by reference with the core; `Send + Sync`.
pub struct TuiHost {
    tx: Sender<()>,
    rx: Receiver<()>,
    deadline: Mutex<Option<Instant>>,
    output: Mutex<Vec<u8>>,
    close: AtomicBool,
    modes_changed: AtomicBool,
    theme: Mutex<ThemePref>,
    title: Mutex<Option<String>>,
}

impl Default for TuiHost {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiHost {
    pub fn new() -> TuiHost {
        let (tx, rx) = crossbeam_channel::bounded(1);
        TuiHost {
            tx,
            rx,
            deadline: Mutex::new(None),
            output: Mutex::new(Vec::new()),
            close: AtomicBool::new(false),
            modes_changed: AtomicBool::new(false),
            theme: Mutex::new(ThemePref::System),
            title: Mutex::new(None),
        }
    }

    /// The redraw signal: one `()` per burst of wake-ups (select on it with the input
    /// channel).
    pub fn redraw_rx(&self) -> &Receiver<()> {
        &self.rx
    }

    /// The earliest pending `redraw_after` deadline; clears it.
    pub fn take_deadline(&self) -> Option<Instant> {
        self.deadline.lock().take()
    }

    /// Bytes (escape sequences) to write to the terminal after the frame; clears them.
    pub fn take_output(&self) -> Vec<u8> {
        std::mem::take(&mut *self.output.lock())
    }

    /// A close was requested (and not cancelled) since the last call; clears it.
    pub fn take_close(&self) -> bool {
        self.close.swap(false, Ordering::AcqRel)
    }

    /// `[tui]` changed since the last call: re-apply terminal modes
    /// (`TerminalGuard::reapply`); clears it.
    pub fn take_modes_changed(&self) -> bool {
        self.modes_changed.swap(false, Ordering::AcqRel)
    }

    /// The theme preference last set by the core.
    pub fn theme(&self) -> ThemePref {
        *self.theme.lock()
    }

    fn queue(&self, bytes: &[u8]) {
        self.output.lock().extend_from_slice(bytes);
    }
}

fn signal(tx: &Sender<()>) {
    // Full means a redraw is already pending: coalesce.
    let _ = tx.try_send(());
}

/// OSC 2 (window title), with control characters removed: the title usually holds a
/// folder name, which is untrusted and must not smuggle escape sequences.
pub fn title_bytes(title: &str) -> Vec<u8> {
    let clean: String = title.chars().filter(|c| !c.is_control()).collect();
    format!("\x1b]2;{clean}\x07").into_bytes()
}

/// OSC 52 "copy to the clipboard" for `text`.
pub fn osc52_bytes(text: &str) -> Vec<u8> {
    let mut s = String::new();
    // Writing into a String cannot fail.
    let _ = crossterm::clipboard::CopyToClipboard::to_clipboard_from(text.as_bytes()).write_ansi(&mut s);
    s.into_bytes()
}

impl Host for TuiHost {
    fn wake(&self) -> Wake {
        let tx = self.tx.clone();
        std::sync::Arc::new(move || signal(&tx))
    }

    fn redraw(&self) {
        signal(&self.tx);
    }

    fn redraw_after(&self, after: Duration) {
        let at = Instant::now() + after;
        let mut d = self.deadline.lock();
        if d.is_none_or(|cur| at < cur) {
            *d = Some(at);
        }
    }

    fn copy_text(&self, text: String) {
        // Terminals that support OSC 52 (Windows Terminal, kitty, WezTerm, iTerm2, foot,
        // tmux with set-clipboard …) put the text on the clipboard of the machine the
        // terminal runs on, which also works over SSH.
        self.queue(&osc52_bytes(&text));
        // The classic console host ignores OSC 52: set the local clipboard directly too.
        #[cfg(windows)]
        soyuz_appcore::services::os_clipboard::set_text(&text);
        self.redraw();
    }

    fn set_theme(&self, pref: ThemePref) {
        *self.theme.lock() = pref;
        self.redraw();
    }

    fn window(&self, _id: WindowId, cmd: WindowCmd) {
        match cmd {
            WindowCmd::Title(t) => {
                let mut last = self.title.lock();
                if last.as_deref() != Some(t.as_str()) {
                    self.queue(&title_bytes(&t));
                    *last = Some(t);
                    self.redraw();
                }
            }
            WindowCmd::Close => {
                self.close.store(true, Ordering::Release);
                self.redraw();
            }
            WindowCmd::CancelClose => self.close.store(false, Ordering::Release),
            WindowCmd::Focus
            | WindowCmd::Minimized(_)
            | WindowCmd::Maximized(_)
            | WindowCmd::OuterPosition(_)
            | WindowCmd::InnerSize(_) => {}
        }
    }

    fn config_changed(&self, old: &Config, new: &Config) {
        soyuz_appcore::core::apply_core_settings(new);
        if old.tui != new.tui {
            self.modes_changed.store(true, Ordering::Release);
        }
        self.redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wakes_coalesce() {
        let h = TuiHost::new();
        let w = h.wake();
        w();
        w();
        h.redraw();
        assert!(h.redraw_rx().try_recv().is_ok());
        assert!(h.redraw_rx().try_recv().is_err());
        // From another thread too.
        let w2 = h.wake();
        std::thread::spawn(move || w2()).join().unwrap();
        assert!(h.redraw_rx().try_recv().is_ok());
    }

    #[test]
    fn redraw_after_keeps_the_earliest() {
        let h = TuiHost::new();
        h.redraw_after(Duration::from_secs(60));
        h.redraw_after(Duration::from_millis(10));
        h.redraw_after(Duration::from_secs(30));
        let d = h.take_deadline().unwrap();
        assert!(d <= Instant::now() + Duration::from_secs(1));
        assert!(h.take_deadline().is_none());
    }

    #[test]
    fn close_and_cancel() {
        let h = TuiHost::new();
        h.window(WindowId::Current, WindowCmd::Close);
        h.window(WindowId::Current, WindowCmd::CancelClose);
        assert!(!h.take_close());
        h.window(WindowId::Root, WindowCmd::Close);
        assert!(h.take_close());
        assert!(!h.take_close());
        h.window(WindowId::Current, WindowCmd::Focus);
        assert!(h.take_output().is_empty());
    }

    #[test]
    fn title_is_osc2_and_sanitized() {
        let h = TuiHost::new();
        h.window(WindowId::Current, WindowCmd::Title("docs\x1b]52;c;evil\x07 — Soyuz".into()));
        let out = String::from_utf8(h.take_output()).unwrap();
        assert_eq!(out, "\x1b]2;docs]52;c;evil — Soyuz\x07");
        // The same title again writes nothing.
        h.window(WindowId::Current, WindowCmd::Title("docs\x1b]52;c;evil\x07 — Soyuz".into()));
        assert!(h.take_output().is_empty());
    }

    #[test]
    fn copy_queues_osc52() {
        assert_eq!(String::from_utf8(osc52_bytes("hi")).unwrap(), "\x1b]52;c;aGk=\x1b\\");
    }

    #[test]
    fn theme_is_stored() {
        let h = TuiHost::new();
        assert_eq!(h.theme(), ThemePref::System);
        h.set_theme(ThemePref::Light);
        assert_eq!(h.theme(), ThemePref::Light);
    }

    #[test]
    fn modes_change_only_with_tui_section() {
        let h = TuiHost::new();
        let old = Config::default();
        let mut new = old.clone();
        new.behaviour.auto_refresh = !new.behaviour.auto_refresh;
        h.config_changed(&old, &new);
        assert!(!h.take_modes_changed());
        new.tui.mouse = !new.tui.mouse;
        h.config_changed(&old, &new);
        assert!(h.take_modes_changed());
        assert!(!h.take_modes_changed());
    }
}

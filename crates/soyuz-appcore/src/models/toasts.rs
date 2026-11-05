//! Toast notifications: the queue and its rules (duplicates collapse, timers, dismiss).
//! Each frontend draws them (`ui::toasts` for the GUI).

use soyuz_core::config::Action;
use soyuz_jobs::HistoryId;

use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub kind: ToastKind,
    pub title: String,
    pub body: Option<String>,
    /// Button label and the action it dispatches.
    pub action: Option<(String, Action)>,
    /// The action button undoes this history entry, the operation the toast names, not
    /// whatever finished last.
    pub undoes: Option<HistoryId>,
    /// Button label and the command it runs (GUI only: an Undo that is not in the job
    /// history; the terminal UI has no way to press it).
    pub cmd: Option<Box<(String, Cmd)>>,
    /// Seconds before auto-dismiss (`None` = sticky).
    pub duration: Option<f32>,
}

impl Toast {
    pub fn new(kind: ToastKind, title: impl Into<String>) -> Self {
        let duration = match kind {
            ToastKind::Error => Some(8.0),
            ToastKind::Warning => Some(6.0),
            _ => Some(4.0),
        };
        Toast { kind, title: title.into(), body: None, action: None, undoes: None, cmd: None, duration }
    }

    pub fn info(title: impl Into<String>) -> Self {
        Toast::new(ToastKind::Info, title)
    }

    pub fn success(title: impl Into<String>) -> Self {
        Toast::new(ToastKind::Success, title)
    }

    pub fn warning(title: impl Into<String>) -> Self {
        Toast::new(ToastKind::Warning, title)
    }

    pub fn error(title: impl Into<String>) -> Self {
        Toast::new(ToastKind::Error, title)
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn action(mut self, label: impl Into<String>, action: Action) -> Self {
        self.action = Some((label.into(), action));
        self
    }

    /// A button that runs `cmd`; the toast stays up longer so there is time to press it.
    pub fn cmd(mut self, label: impl Into<String>, cmd: Cmd) -> Self {
        self.cmd = Some(Box::new((label.into(), cmd)));
        self.duration = self.duration.map(|d| d.max(8.0));
        self
    }

    /// An Undo button for the history entry `entry`.
    pub fn undo_button(mut self, label: impl Into<String>, entry: HistoryId) -> Self {
        self.action = Some((label.into(), Action::Undo));
        self.undoes = Some(entry);
        self
    }

    /// The command of the action button.
    pub fn command(&self) -> Option<Cmd> {
        if let Some(button) = &self.cmd {
            return Some(button.1.clone());
        }
        match (self.undoes, &self.action) {
            (Some(h), _) => Some(Cmd::FileOp(FileOpCmd::UndoEntry(h))),
            (None, Some((_, action))) => Some(Cmd::Action(action.clone())),
            (None, None) => None,
        }
    }

    pub fn sticky(mut self) -> Self {
        self.duration = None;
        self
    }
}

/// A toast on screen. `born` / `closing` are frontend timestamps in seconds (`NAN` until
/// the frontend first sees it).
pub struct Live {
    pub id: u64,
    pub toast: Toast,
    /// Seconds of visible (unhovered) time left.
    pub remaining: Option<f32>,
    pub born: f64,
    pub closing: Option<f64>,
}

pub struct Toasts {
    pub live: Vec<Live>,
    next: u64,
    /// `[appearance] toast_time_scale`: multiplies each pushed toast's duration.
    time_scale: f32,
}

impl Default for Toasts {
    fn default() -> Self {
        Toasts { live: Vec::new(), next: 0, time_scale: 1.0 }
    }
}

/// Fraction of the auto-dismiss timer still left (`None` for sticky toasts).
pub fn timer_fraction(remaining: Option<f32>, duration: Option<f32>) -> Option<f32> {
    match (remaining, duration) {
        (Some(r), Some(d)) if d > 0.0 => Some((r / d).clamp(0.0, 1.0)),
        _ => None,
    }
}

impl Toasts {
    /// Toasts pushed from now on stay up `scale` times their own duration.
    pub fn set_time_scale(&mut self, scale: f32) {
        self.time_scale = if scale.is_finite() { scale.clamp(0.1, 5.0) } else { 1.0 };
    }

    pub fn push(&mut self, mut toast: Toast) -> u64 {
        toast.duration = toast.duration.map(|d| d * self.time_scale);
        self.next += 1;
        let id = self.next;
        // Collapse exact duplicates (e.g. repeated "coming soon").
        self.live.retain(|l| !(l.toast.title == toast.title && l.toast.body == toast.body && l.closing.is_none()));
        self.live.push(Live { id, remaining: toast.duration, toast, born: f64::NAN, closing: None });
        id
    }

    /// Push, replacing any live toast with the same title (status-like toasts such as
    /// the interface zoom level).
    pub fn push_replacing(&mut self, toast: Toast) -> u64 {
        self.live.retain(|l| l.toast.title != toast.title);
        self.push(toast)
    }

    pub fn dismiss(&mut self, id: u64) {
        if let Some(l) = self.live.iter_mut().find(|l| l.id == id) {
            l.closing.get_or_insert(f64::NAN);
        }
    }

    /// Close the newest toast still open; `false` when none is.
    pub fn dismiss_newest(&mut self) -> bool {
        match self.live.iter_mut().rev().find(|l| l.closing.is_none()) {
            Some(l) => {
                l.closing = Some(f64::NAN);
                true
            }
            None => false,
        }
    }

    /// Close every toast; `false` when none was open.
    pub fn dismiss_all(&mut self) -> bool {
        let mut any = false;
        for l in self.live.iter_mut().filter(|l| l.closing.is_none()) {
            l.closing = Some(f64::NAN);
            any = true;
        }
        any
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_collapse() {
        let mut t = Toasts::default();
        t.push(Toast::info("Coming soon"));
        t.push(Toast::info("Coming soon"));
        t.push(Toast::error("Failed").body("why"));
        assert_eq!(t.live.len(), 2);
        let id = t.push(Toast::success("Done").action("Undo", Action::Undo).sticky());
        assert!(t.live.last().unwrap().remaining.is_none());
        t.dismiss(id);
        assert!(t.live.last().unwrap().closing.is_some());
    }

    #[test]
    fn the_undo_button_targets_its_entry() {
        let t = Toast::success("Copied").undo_button("Undo", HistoryId(7));
        assert!(matches!(t.command(), Some(Cmd::FileOp(FileOpCmd::UndoEntry(HistoryId(7))))));
        let t = Toast::warning("Failed").action("Details", Action::JobsShowHistory);
        assert!(matches!(t.command(), Some(Cmd::Action(Action::JobsShowHistory))));
        assert!(Toast::info("x").command().is_none());
    }

    #[test]
    fn esc_closes_the_newest_first() {
        let mut t = Toasts::default();
        assert!(!t.dismiss_newest());
        t.push(Toast::info("old"));
        t.push(Toast::info("new"));
        assert!(t.dismiss_newest());
        assert!(t.live[1].closing.is_some() && t.live[0].closing.is_none());
        assert!(t.dismiss_newest());
        assert!(t.live[0].closing.is_some());
        assert!(!t.dismiss_newest() && !t.dismiss_all());
    }

    #[test]
    fn a_command_button_gives_time_to_press_it() {
        let t = Toast::success("Done").cmd("Undo", Cmd::ShowSystemFolders(true));
        assert_eq!(t.duration, Some(8.0));
        assert!(matches!(t.command(), Some(Cmd::ShowSystemFolders(true))));
    }

    #[test]
    fn time_scale_shortens_timers() {
        let mut t = Toasts::default();
        t.set_time_scale(0.5);
        t.push(Toast::info("a"));
        t.push(Toast::error("b"));
        t.push(Toast::info("c").sticky());
        let rem: Vec<_> = t.live.iter().map(|l| (l.remaining, l.toast.duration)).collect();
        assert_eq!(rem, [(Some(2.0), Some(2.0)), (Some(4.0), Some(4.0)), (None, None)]);
        t.set_time_scale(f32::NAN);
        t.push(Toast::info("d"));
        assert_eq!(t.live.last().unwrap().remaining, Some(4.0));
    }

    #[test]
    fn timer_fraction_tracks_remaining_time() {
        assert_eq!(timer_fraction(Some(2.0), Some(4.0)), Some(0.5));
        assert_eq!(timer_fraction(Some(-1.0), Some(4.0)), Some(0.0));
        assert_eq!(timer_fraction(None, None), None);
        assert_eq!(timer_fraction(Some(1.0), Some(0.0)), None);
    }
}

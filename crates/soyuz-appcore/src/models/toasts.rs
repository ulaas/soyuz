//! Toast notifications: the queue and its rules (duplicates collapse, timers, dismiss).
//! Each frontend draws them (`ui::toasts` for the GUI).

use soyuz_core::config::Action;

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
        Toast { kind, title: title.into(), body: None, action: None, duration }
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

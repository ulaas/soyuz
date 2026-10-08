//! Context-aware key dispatch with multi-key sequences (`g g`, `Ctrl+K Ctrl+S`) and
//! ranger-style marks (`m<key>` sets, `'<key>` jumps).
//!
//! A chord that starts a sequence waits up to [`TIMEOUT`] for the next key; when the
//! same chord is also bound on its own, that binding fires on timeout (like Vim's
//! `timeoutlen`). Consumed plain keys also swallow the `Text` event egui emits for them,
//! so type-ahead does not see letters used as commands.

use std::time::{Duration, Instant};

use egui::Event;
use soyuz_core::Location;
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, KeyPreset, Keymap, Mods, SeqMatch};

use crate::app::SoyuzApp;
use crate::ops::{self, NavOpts};
use crate::ui::toasts::Toast;

/// How long a partial sequence waits for its next key.
pub const TIMEOUT: Duration = Duration::from_millis(1000);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkOp {
    Set,
    Jump,
}

/// What one chord did.
#[derive(Clone, Debug, PartialEq)]
pub enum Feed {
    /// Not ours: leave the event for widgets.
    Pass,
    /// Swallowed (sequence in progress).
    Pending,
    Action(Action),
    Mark(MarkOp, char),
    /// A pending sequence was abandoned; its single-chord fallback (if any) fires, and
    /// the new chord is fed again.
    Abandoned(Option<Action>),
}

#[derive(Default)]
pub struct Sequencer {
    pending: Vec<KeyChord>,
    since: Option<Instant>,
    /// Single-chord binding of `pending[0]` (fires on timeout).
    fallback: Option<Action>,
    mark: Option<MarkOp>,
}

impl Sequencer {
    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty() || self.mark.is_some()
    }

    /// "g …" / "m …" while waiting.
    pub fn pending_label(&self) -> Option<String> {
        match self.mark {
            Some(MarkOp::Set) => Some(tr!("actions-keyseq-set-mark").into()),
            Some(MarkOp::Jump) => Some(tr!("actions-keyseq-jump-mark").into()),
            None if !self.pending.is_empty() => {
                Some(format!("{} …", self.pending.iter().map(|c| short_chord(*c)).collect::<Vec<_>>().join(" ")))
            }
            None => None,
        }
    }

    pub fn reset(&mut self) {
        *self = Sequencer::default();
    }

    /// Expire a stale sequence; returns the fallback action to run.
    pub fn tick(&mut self, now: Instant) -> Option<Action> {
        match self.since {
            Some(t) if now.duration_since(t) >= TIMEOUT => {
                let a = self.fallback.take();
                self.reset();
                a
            }
            _ => None,
        }
    }

    /// Feed one chord. `marks`: ranger marks are enabled in this context.
    pub fn feed(&mut self, km: &Keymap, kctx: KeyContext, chord: KeyChord, now: Instant, marks: bool) -> Feed {
        if let Some(op) = self.mark.take() {
            self.since = None;
            if let Some(c) = mark_char(chord) {
                return Feed::Mark(op, c);
            }
            if chord.key == KeyName::Escape {
                return Feed::Pending;
            }
            // Anything else cancels the mark and is handled normally.
        }
        if !self.pending.is_empty() {
            let mut typed = self.pending.clone();
            typed.push(chord);
            match km.sequence_match(kctx, &typed) {
                SeqMatch::Exact(a) => {
                    self.reset();
                    return Feed::Action(a);
                }
                SeqMatch::Prefix => {
                    self.pending = typed;
                    self.since = Some(now);
                    return Feed::Pending;
                }
                SeqMatch::None => {
                    let fb = self.fallback.take();
                    self.reset();
                    return Feed::Abandoned(fb);
                }
            }
        }
        if marks && chord.mods.is_empty() {
            let op = match chord.key {
                KeyName::M => Some(MarkOp::Set),
                KeyName::Quote => Some(MarkOp::Jump),
                _ => None,
            };
            if let Some(op) = op {
                self.mark = Some(op);
                self.since = Some(now);
                return Feed::Pending;
            }
        }
        let single = km.lookup(kctx, chord).cloned();
        if km.sequence_match(kctx, &[chord]) == SeqMatch::Prefix {
            self.pending = vec![chord];
            self.since = Some(now);
            self.fallback = single;
            return Feed::Pending;
        }
        match single {
            Some(a) => Feed::Action(a),
            None => Feed::Pass,
        }
    }
}

/// `g` for a plain letter, `G` for Shift+letter, else the canonical chord text.
fn short_chord(c: KeyChord) -> String {
    let k = c.key.as_str();
    if k.len() == 1 && k.chars().all(|x| x.is_ascii_alphabetic()) {
        if c.mods.is_empty() {
            return k.to_ascii_lowercase();
        }
        if c.mods == Mods::SHIFT {
            return k.to_string();
        }
    }
    c.to_string()
}

/// Letter or digit for a mark (`Shift` gives upper case).
fn mark_char(chord: KeyChord) -> Option<char> {
    if chord.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) {
        return None;
    }
    let s = chord.key.as_str();
    let mut it = s.chars();
    let c = it.next()?;
    if it.next().is_some() || !c.is_ascii_alphanumeric() {
        return None;
    }
    Some(if chord.mods.contains(Mods::SHIFT) { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() })
}

/// Whether marks are active for this config.
pub fn marks_enabled(app: &SoyuzApp) -> bool {
    app.config.keys.marks || app.config.keys.preset == KeyPreset::Vim
}

/// One frame of keyboard dispatch for the main window (replaces the plain
/// `keys::dispatch` loop): actions, sequences and marks.
pub fn dispatch_all(app: &mut SoyuzApp, ctx: &egui::Context, kctx: KeyContext, text_focused: bool) {
    let now = Instant::now();
    let mut out: Vec<Feed> = Vec::new();
    if let Some(a) = app.keyseq.tick(now) {
        out.push(Feed::Action(a));
    }
    let marks = marks_enabled(app) && kctx == KeyContext::FileList && !text_focused;
    let cmd_is_ctrl = app.config.keys.cmd_is_ctrl;
    {
        let km = &app.keymap;
        let seq = &mut app.keyseq;
        ctx.input_mut(|i| {
            let current = i.modifiers;
            let mut swallow_text = false;
            i.events.retain(|ev| {
                if swallow_text && matches!(ev, Event::Text(_)) {
                    swallow_text = false;
                    return false;
                }
                let Some(chord) = super::event_chord(ev, current, cmd_is_ctrl) else { return true };
                if text_focused && super::is_text_editing(chord) {
                    return true;
                }
                let plain = !chord.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD);
                // While a text field is focused, sequences never start.
                let mut f = if text_focused {
                    match km.lookup(kctx, chord) {
                        Some(a) => Feed::Action(a.clone()),
                        None => Feed::Pass,
                    }
                } else {
                    seq.feed(km, kctx, chord, now, marks)
                };
                if let Feed::Abandoned(fb) = f {
                    if let Some(a) = fb {
                        out.push(Feed::Action(a));
                    }
                    f = seq.feed(km, kctx, chord, now, marks);
                    if let Feed::Abandoned(_) = f {
                        f = Feed::Pass;
                    }
                }
                match f {
                    Feed::Pass => true,
                    other => {
                        if plain {
                            swallow_text = true;
                        }
                        if !matches!(other, Feed::Pending) {
                            out.push(other);
                        }
                        false
                    }
                }
            });
        });
    }
    if app.keyseq.is_pending() {
        ctx.request_repaint_after(TIMEOUT);
    }
    for f in out {
        match f {
            Feed::Action(a) => crate::actions::execute(app, a, ctx),
            Feed::Mark(op, c) => mark(app, op, c),
            _ => {}
        }
    }
}

fn mark(app: &mut SoyuzApp, op: MarkOp, c: char) {
    let key = c.to_string();
    match op {
        MarkOp::Set => {
            let Some(dir) = app.ws.pane().location.as_fs_path().map(soyuz_fs::strip_verbatim) else {
                app.ui.toasts.push(Toast::warning(tr!("actions-mark-needs-folder")));
                return;
            };
            app.palette.memory.marks.insert(key, dir.clone());
            app.palette.dirty = true;
            app.ui
                .toasts
                .push(Toast::success(tr!("actions-mark-set", mark = c.to_string())).body(dir.display().to_string()));
        }
        MarkOp::Jump => match app.palette.memory.marks.get(&key).cloned() {
            Some(dir) => {
                let id = app.ws.pane().id;
                ops::navigate(app, id, Location::Dir(dir), NavOpts::user());
            }
            None => {
                app.ui.toasts.push(Toast::info(tr!("actions-mark-missing", mark = c.to_string())));
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::{defaults, parse_sequence};

    fn ch(s: &str) -> KeyChord {
        s.parse().unwrap()
    }

    #[test]
    fn vim_sequences() {
        let km = defaults::preset(KeyPreset::Vim);
        let mut s = Sequencer::default();
        let t = Instant::now();
        let l = KeyContext::FileList;
        assert_eq!(s.feed(&km, l, ch("J"), t, false), Feed::Action(Action::CursorDown));
        assert_eq!(s.feed(&km, l, ch("G"), t, false), Feed::Pending);
        assert_eq!(s.pending_label().as_deref(), Some("g …"));
        assert_eq!(s.feed(&km, l, ch("G"), t, false), Feed::Action(Action::CursorHome));
        assert!(!s.is_pending());
        // Unknown continuation abandons the sequence; the chord is fed again by the caller.
        assert_eq!(s.feed(&km, l, ch("Y"), t, false), Feed::Pending);
        assert_eq!(s.feed(&km, l, ch("J"), t, false), Feed::Abandoned(None));
        assert_eq!(s.feed(&km, l, ch("J"), t, false), Feed::Action(Action::CursorDown));
        // Not bound at all.
        assert_eq!(s.feed(&km, l, ch("Q"), t, false), Feed::Pass);
        // Sequences are context-specific.
        assert_eq!(s.feed(&km, KeyContext::Sidebar, ch("G"), t, false), Feed::Pass);
    }

    #[test]
    fn timeout_fires_single_binding() {
        let mut km = Keymap::default();
        km.bind(KeyContext::Global, Action::CommandPalette, ch("Ctrl+K"));
        km.bind_sequence(KeyContext::Global, Action::Settings, parse_sequence("Ctrl+K Ctrl+S").unwrap());
        let mut s = Sequencer::default();
        let t = Instant::now();
        assert_eq!(s.feed(&km, KeyContext::FileList, ch("Ctrl+K"), t, false), Feed::Pending);
        assert_eq!(s.tick(t + Duration::from_millis(100)), None);
        assert_eq!(s.tick(t + TIMEOUT), Some(Action::CommandPalette));
        assert!(!s.is_pending());
        assert_eq!(s.feed(&km, KeyContext::FileList, ch("Ctrl+K"), t, false), Feed::Pending);
        assert_eq!(s.feed(&km, KeyContext::FileList, ch("Ctrl+S"), t, false), Feed::Action(Action::Settings));
        // A different key after the prefix runs the fallback and then itself.
        assert_eq!(s.feed(&km, KeyContext::FileList, ch("Ctrl+K"), t, false), Feed::Pending);
        assert_eq!(s.feed(&km, KeyContext::FileList, ch("X"), t, false), Feed::Abandoned(Some(Action::CommandPalette)));
    }

    #[test]
    fn marks() {
        let km = Keymap::default();
        let mut s = Sequencer::default();
        let t = Instant::now();
        let l = KeyContext::FileList;
        assert_eq!(s.feed(&km, l, ch("M"), t, false), Feed::Pass, "marks disabled");
        assert_eq!(s.feed(&km, l, ch("M"), t, true), Feed::Pending);
        assert_eq!(s.pending_label().as_deref(), Some("m … set mark"));
        assert_eq!(s.feed(&km, l, ch("A"), t, true), Feed::Mark(MarkOp::Set, 'a'));
        assert_eq!(s.feed(&km, l, ch("'"), t, true), Feed::Pending);
        assert_eq!(s.feed(&km, l, ch("Shift+B"), t, true), Feed::Mark(MarkOp::Jump, 'B'));
        assert_eq!(s.feed(&km, l, ch("'"), t, true), Feed::Pending);
        assert_eq!(s.feed(&km, l, ch("Esc"), t, true), Feed::Pending, "Esc cancels quietly");
        assert!(!s.is_pending());
        assert_eq!(mark_char(ch("Ctrl+A")), None);
        assert_eq!(mark_char(ch("F5")), None);
        assert_eq!(mark_char(ch("7")), Some('7'));
    }
}

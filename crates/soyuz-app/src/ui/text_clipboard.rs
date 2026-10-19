//! The text clipboard of egui's widgets (text fields, selectable labels, `copy_text`).
//!
//! eframe is built without its `clipboard` feature (its arboard backend reaches for shell
//! APIs on Windows), so egui-winit keeps copies in a private in-process string: Ctrl+C in
//! the address bar, the rename field or any selectable text never reached other apps, and
//! Ctrl+V never saw their text. This plugin routes both directions through
//! `services::os_clipboard` (raw user32 on Windows, the native Wayland / X11 selection on
//! Linux), for every viewport:
//! * output: each `OutputCommand::CopyText` (Copy / Cut in a widget, `Host::copy_text`) is
//!   also written to the system clipboard;
//! * input: each `Event::Paste` carries the system clipboard's text instead of egui-winit's
//!   private copy, and is dropped when the clipboard holds no text (only files, a
//!   picture), so the file list's Ctrl+V fallback still pastes those.
//!
//! egui-winit only sends `Event::Paste` while its private string is non-empty, so every
//! pass also leaves a placeholder ([`PRIME`]) there; it never reaches a widget. Where the
//! system text clipboard is unavailable (macOS, a Linux session without a native backend)
//! egui's in-process clipboard is left alone.

use egui::{Event, OutputCommand};

use crate::services::os_clipboard;

/// Keeps egui-winit's private clipboard non-empty, so Ctrl+V always produces a paste event.
const PRIME: &str = "\u{2060}soyuz-clipboard\u{2060}";

/// Install the bridge on the real window's context (not in tests: they must not touch the
/// user's clipboard).
pub fn install(ctx: &egui::Context) {
    ctx.add_plugin(Bridge);
}

struct Bridge;

impl egui::plugin::Plugin for Bridge {
    fn debug_name(&self) -> &'static str {
        "soyuz.text_clipboard"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        pastes(&mut input.events, os_clipboard::text_available(), os_clipboard::get_text);
    }

    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        copies(&mut output.platform_output.commands, os_clipboard::text_available(), |t| {
            os_clipboard::set_text(t);
        });
    }
}

/// Fill this pass's paste events from the system clipboard (`get` reads it). Unavailable:
/// only the placeholder is removed.
fn pastes(events: &mut Vec<Event>, available: bool, get: impl Fn() -> Option<String>) {
    if !events.iter().any(|e| matches!(e, Event::Paste(_))) {
        return;
    }
    if !available {
        events.retain(|e| !matches!(e, Event::Paste(s) if s == PRIME));
        return;
    }
    // One read per pass, even with several paste events.
    let text = get().map(|t| t.replace("\r\n", "\n")).filter(|t| !t.is_empty());
    events.retain_mut(|e| match e {
        Event::Paste(s) => match &text {
            Some(t) => {
                s.clone_from(t);
                true
            }
            None => false,
        },
        _ => true,
    });
}

/// Write this pass's text copies with `set`, then prime egui-winit's private clipboard.
fn copies(commands: &mut Vec<OutputCommand>, available: bool, set: impl Fn(&str)) {
    if !available {
        return;
    }
    for c in commands.iter() {
        if let OutputCommand::CopyText(t) = c {
            set(t);
        }
    }
    commands.push(OutputCommand::CopyText(PRIME.into()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn paste(s: &str) -> Event {
        Event::Paste(s.into())
    }

    #[test]
    fn pastes_carry_the_system_text() {
        let mut ev = vec![paste(PRIME), Event::Text("a".into()), paste("old")];
        pastes(&mut ev, true, || Some("line 1\r\nline 2".into()));
        assert_eq!(ev, vec![paste("line 1\nline 2"), Event::Text("a".into()), paste("line 1\nline 2")]);
    }

    #[test]
    fn pastes_without_system_text_are_dropped() {
        let mut ev = vec![paste(PRIME), Event::Copy];
        pastes(&mut ev, true, || None);
        assert_eq!(ev, vec![Event::Copy]);
        let mut ev = vec![paste(PRIME)];
        pastes(&mut ev, true, || Some(String::new()));
        assert!(ev.is_empty());
    }

    #[test]
    fn unavailable_keeps_in_process_pastes_but_not_the_placeholder() {
        let mut ev = vec![paste(PRIME), paste("internal")];
        pastes(&mut ev, false, || panic!("must not read"));
        assert_eq!(ev, vec![paste("internal")]);
    }

    #[test]
    fn no_paste_no_read() {
        let mut ev = vec![Event::Copy];
        pastes(&mut ev, true, || panic!("must not read"));
        assert_eq!(ev, vec![Event::Copy]);
    }

    #[test]
    fn copies_reach_the_system_clipboard_and_prime() {
        let got = RefCell::new(Vec::new());
        let mut cmds = vec![OutputCommand::CopyText("C:\\a b".into()), OutputCommand::TextSelectionSettled("x".into())];
        copies(&mut cmds, true, |t| got.borrow_mut().push(t.to_string()));
        assert_eq!(*got.borrow(), vec!["C:\\a b".to_string()]);
        assert!(matches!(cmds.last(), Some(OutputCommand::CopyText(t)) if t == PRIME));
    }

    #[test]
    fn unavailable_leaves_copies_to_egui() {
        let mut cmds = vec![OutputCommand::CopyText("a".into())];
        copies(&mut cmds, false, |_| panic!("must not write"));
        assert_eq!(cmds.len(), 1);
    }
}

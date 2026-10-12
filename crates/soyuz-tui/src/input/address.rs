//! Keys while the address field is open (step T2.5): the core first
//! (`KeyContext::Omnibar`, `text_focused`), then a `LineEdit` on `ui.address.text` with
//! the unconsumed events. Stub: the core only (`crate::unsupported::address` closes the
//! field after the input that opened it).

use crossterm::event::Event;

use super::{TuiFocus, core_input, key_context};
use crate::app::App;

/// What the address field keeps between frames.
#[derive(Debug, Default)]
pub struct State {}

pub fn route(app: &mut App, events: &[Event]) {
    let (kctx, text) = key_context(TuiFocus::Address);
    core_input(app, events, kctx, text);
}

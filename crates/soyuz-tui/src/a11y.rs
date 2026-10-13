//! Screen-reader mode (Phase T8): `[tui] screen_reader`, the palette's "Toggle
//! screen-reader mode" (`Action::ToggleScreenReader`) and the run-only `--screen-reader`
//! override (`App::sr_override`, never written to the config).
//!
//! When it is on ([`active`]):
//! - the theme says so (`TuiTheme::screen_reader`) and the views draw their linear,
//!   box-free variants: no borders, splitters, scrollbars, spinners or images; `*` marks
//!   selected rows, folders get a trailing `/`, no icons;
//! - the layout (`Facts::screen_reader`) draws only the focused pane, plus the "Other
//!   pane: <path>" line ([`draw`], `Regions::other_pane`, `ViewModel::other_pane`);
//! - each view records the focused element's cell (`Hits::set_caret`) and `view::draw`
//!   puts the terminal's cursor there;
//! - the status line is one sentence that starts with the focused item
//!   ([`status_sentence`]); when it changes it is rewritten in full after the frame
//!   ([`after_frame`]), so a reader that speaks changed text (Narrator, NVDA) reads the
//!   whole line, not the suffix ratatui's diff wrote;
//! - toasts last at least their unscaled duration ([`toast_dt`]).
//!
//! Wave 0 skeleton: the hooks are wired into `App::frame` and `view::draw` and do nothing
//! yet; Phase T8 P3 / P6 fill them in.

use std::io;

use ratatui::Frame;
use soyuz_core::config::TuiSection;

use crate::app::TermIo;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;
use crate::view::ViewModel;

/// Screen-reader mode is on: the setting, or the run-only `--screen-reader` override.
pub fn active(tui: &TuiSection, run_override: bool) -> bool {
    tui.screen_reader || run_override
}

/// What screen-reader mode keeps between frames (`App::a11y`).
#[derive(Debug, Default)]
pub struct State {
    /// The status sentence written last ([`after_frame`]).
    last_status: String,
}

/// The status line as one sentence, starting with the focused item ("b.txt, file, 1 KB,
/// 2 of 3; 12 items, 2 selected; 12 GB free; Details, by name ascending").
pub fn status_sentence(m: &ViewModel) -> String {
    let _ = m;
    String::new()
}

/// Draw screen-reader mode's own lines (the "Other pane: <path>" line). Does nothing
/// outside screen-reader mode.
pub fn draw(f: &mut Frame<'_>, regions: &Regions, m: &ViewModel, theme: &TuiTheme, hits: &mut Hits) {
    if !theme.screen_reader() {
        return;
    }
    let _ = (f, regions, m, hits);
}

/// After a frame was drawn and flushed: in screen-reader mode, rewrite the status line in
/// full when its sentence changed (through `io.write_raw`: move there, the line, erase to
/// the end, then put the cursor back on `hits.caret()`).
pub fn after_frame(
    state: &mut State,
    on: bool,
    m: &ViewModel,
    regions: &Regions,
    hits: &Hits,
    io: &mut dyn TermIo,
) -> io::Result<()> {
    if !on {
        state.last_status.clear();
        return Ok(());
    }
    let _ = (m, regions, hits, io);
    Ok(())
}

/// The time a frame takes off the toasts' timers. `scale` is `[appearance]
/// toast_time_scale`, already applied to each toast's duration when it was pushed: in
/// screen-reader mode a toast lasts at least its unscaled duration, so a reader can finish
/// speaking it.
pub fn toast_dt(dt: f32, scale: f32, on: bool) -> f32 {
    let _ = (scale, on);
    dt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_or_the_run_override_turns_it_on() {
        let mut tui = TuiSection::default();
        assert!(!active(&tui, false));
        assert!(active(&tui, true));
        tui.screen_reader = true;
        assert!(active(&tui, false));
    }
}

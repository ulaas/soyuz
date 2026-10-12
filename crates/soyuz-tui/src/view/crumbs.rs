//! The location line: the focused pane's path (step T2.5 turns it into breadcrumbs and
//! the address field).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;
use soyuz_appcore::core::AppCore;
use soyuz_core::Location;

use super::text;
use crate::dialog::safe;
use crate::layout::Hits;
use crate::theme::{Role, TuiTheme};

/// The location line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrumbsVm {
    /// The focused pane's location as a path.
    pub path: String,
}

impl CrumbsVm {
    pub fn from_core(core: &AppCore) -> CrumbsVm {
        CrumbsVm { path: path_line(&core.ws.pane().location) }
    }
}

/// The location as a path line.
fn path_line(loc: &Location) -> String {
    match loc {
        Location::Dir(p) => safe(&soyuz_fs::strip_verbatim(p).display().to_string()),
        other => safe(&soyuz_appcore::state::crumbs::location_title(other)),
    }
}

/// Draw the location line into `rect`.
pub fn draw_crumbs(f: &mut Frame<'_>, rect: Rect, m: &CrumbsVm, theme: &TuiTheme, _hits: &mut Hits) {
    if rect.is_empty() {
        return;
    }
    // A long path is cut in the middle: its start and the folder you are in stay.
    let path = text::truncate_middle(&m.path, usize::from(rect.width));
    f.render_widget(Paragraph::new(path).style(theme.style(Role::Title)), rect);
}

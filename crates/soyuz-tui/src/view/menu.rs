//! The open popup menu (context menu: Shift+F10, right click) — Phase T3.
//!
//! The menu's state is `crate::input::menu::State` (a `widgets::Menu`); the model is a
//! copy of it and [`draw_menu`] draws it with `Menu::draw` over everything but dialogs,
//! recording `MenuItem` targets.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::widgets::Clear;

use crate::input::InputState;
use crate::input::menu::MenuCmd;
use crate::layout::{HitTarget, Hits};
use crate::theme::TuiTheme;
use crate::widgets::Menu;

/// The open menu as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuVm {
    pub menu: Option<Menu<MenuCmd>>,
}

impl MenuVm {
    pub fn from_input(input: &InputState) -> MenuVm {
        MenuVm { menu: input.menu.menu.clone() }
    }
}

/// Draw the menu over `area` (the whole screen). The highlighted item of the deepest open
/// level is the caret (`Hits::set_caret`).
///
/// Screen-reader mode (T8): the screen under the menu is cleared first, so no row mixes
/// the menu's text with the panes'.
pub fn draw_menu(f: &mut Frame<'_>, area: Rect, m: &MenuVm, theme: &TuiTheme, hits: &mut Hits) {
    if let Some(menu) = &m.menu {
        if theme.screen_reader() {
            f.render_widget(Clear, area.intersection(f.area()));
        }
        menu.clone().draw(f, area, theme, hits);
        if let Some(at) = caret(menu, hits) {
            hits.set_caret(at);
        }
    }
}

/// The cell of the highlighted item of the deepest level that has one (its drawn target).
fn caret(menu: &Menu<MenuCmd>, hits: &Hits) -> Option<Position> {
    let (mut found, mut depth, mut level) = (None, 0u8, Some(menu));
    while let Some(m) = level {
        if let Some(index) = m.cursor {
            found = Some(HitTarget::MenuItem { depth, index });
        }
        depth = depth.saturating_add(1);
        level = m.sub.as_deref();
    }
    let target = found?;
    hits.iter().filter(|(_, t)| *t == target).last().map(|(r, _)| Position { x: r.x, y: r.y })
}

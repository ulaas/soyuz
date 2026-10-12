//! The open popup menu (context menu: Shift+F10, right click) — Phase T3.
//!
//! The menu's state is `crate::input::menu::State` (a `widgets::Menu`); the model is a
//! copy of it and [`draw_menu`] draws it with `Menu::draw` over everything but dialogs,
//! recording `MenuItem` targets.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::input::InputState;
use crate::input::menu::MenuCmd;
use crate::layout::Hits;
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

/// Draw the menu over `area` (the whole screen).
pub fn draw_menu(f: &mut Frame<'_>, area: Rect, m: &MenuVm, theme: &TuiTheme, hits: &mut Hits) {
    if let Some(menu) = &m.menu {
        menu.clone().draw(f, area, theme, hits);
    }
}

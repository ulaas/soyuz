//! Reusable terminal widgets: the line editor (address, filter, palette, dialog fields),
//! dialog forms, popup menus and scrolling lists. Each keeps only its own state; the
//! data belongs to the caller.

pub mod form;
pub mod line_edit;
pub mod menu;
pub mod select_list;

pub use form::{Button, Field, FieldKind, Form, FormOutcome};
pub use line_edit::{EditOutcome, LineEdit};
pub use menu::{EntryKind, Menu, MenuEntry, MenuItem, MenuOutcome};
pub use select_list::{ListMove, ListRow, SelectList};

//! Pane content views. `flow` renders the details, grid, gallery and list views (all
//! virtualized, painter-only rows with one interaction surface, so 100k entries stay
//! smooth); `miller` composes flow columns; `computer` is the drives page.

pub mod columns;
pub mod computer;
pub mod flow;
pub mod icons;
pub mod layout;
pub mod miller;
pub mod trash;

use soyuz_core::config::{Config, Keymap};
use soyuz_fs::VolumeInfo;

use crate::cmd::Cmd;
use crate::services::Services;

/// `[general] show_extensions` (read by every view when drawing names).
static SHOW_EXTENSIONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Apply the display settings the views read while painting (`show_extensions`,
/// `icon_style`); called at startup and on every config change.
pub fn set_display_options(show_extensions: bool, icon_style: soyuz_core::config::IconStyle) {
    SHOW_EXTENSIONS.store(show_extensions, std::sync::atomic::Ordering::Relaxed);
    icons::set_mono(icon_style == soyuz_core::config::IconStyle::Mono);
}

/// The name as displayed: without the extension for files when `show_extensions` is
/// off (folders, dotfiles like `.bashrc` and names that are only an extension keep it).
pub fn display_name(e: &soyuz_core::EntryInfo) -> std::borrow::Cow<'_, str> {
    if SHOW_EXTENSIONS.load(std::sync::atomic::Ordering::Relaxed) || e.is_dir_like() {
        return std::borrow::Cow::Borrowed(&e.name);
    }
    strip_extension(&e.name)
}

fn strip_extension(name: &str) -> std::borrow::Cow<'_, str> {
    match name.rfind('.') {
        Some(i) if i > 0 => std::borrow::Cow::Borrowed(&name[..i]),
        _ => std::borrow::Cow::Borrowed(name),
    }
}

/// Everything a view needs besides the pane itself.
pub struct ViewCtx<'a> {
    pub config: &'a Config,
    pub keymap: &'a Keymap,
    pub services: &'a mut Services,
    pub cmds: &'a mut Vec<Cmd>,
    /// This pane is the focused pane of the active tab.
    pub focused: bool,
    /// Keyboard focus is in the file list (draws the cursor ring).
    pub list_focus: bool,
    pub checkbox_mode: bool,
    pub show_hidden: bool,
    pub show_system: bool,
    pub volumes: &'a [VolumeInfo],
    /// Dual pane is available (enables "open in other pane").
    pub dual: bool,
    /// Clipboard (cut dimming), inline rename, drag and drop, trash view state.
    pub fileops: &'a mut crate::ops::fileops::FileOps,
}

#[cfg(test)]
mod display_tests {
    #[test]
    fn strips_only_real_extensions() {
        assert_eq!(super::strip_extension("report.final.pdf"), "report.final");
        assert_eq!(super::strip_extension(".bashrc"), ".bashrc");
        assert_eq!(super::strip_extension("Makefile"), "Makefile");
        assert_eq!(super::strip_extension("Ürün.txt"), "Ürün");
    }
}

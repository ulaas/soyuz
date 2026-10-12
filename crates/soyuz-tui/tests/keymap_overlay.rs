//! The legacy-terminal overlay reaches the core's key map through the process-wide hook
//! (its own test binary, because the hook can only be installed once per process).

use soyuz_appcore::keys::{build_keymap, set_keymap_overlay};
use soyuz_core::config::{Action, Config, KeyContext};

#[test]
fn build_keymap_applies_the_installed_overlay() {
    let c = Config::default();
    let alt_t = "Alt+Shift+T".parse().unwrap();
    assert_eq!(build_keymap(&c).0.lookup(KeyContext::Global, alt_t), None);
    set_keymap_overlay(soyuz_tui::overlay::apply);
    let (km, _) = build_keymap(&c);
    assert_eq!(km.lookup(KeyContext::Global, alt_t), Some(&Action::ReopenClosedTab));
    assert_eq!(km.lookup(KeyContext::FileList, "Ctrl+T".parse().unwrap()), Some(&Action::NewTab));
}

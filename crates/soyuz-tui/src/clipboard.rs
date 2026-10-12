//! The terminal UI's text clipboard: OSC 52 (`[tui] osc52`) next to the local clipboard —
//! Phase T3 refines `auto` (for example OSC 52 only over SSH or without a local
//! clipboard) and adds the file clipboard paths it needs.
//!
//! `TuiHost::copy_text` asks [`send_osc52`] whether to write the OSC 52 sequence; the
//! local clipboard is set as before.

use soyuz_core::config::TuiOsc52;

/// Whether a copy writes OSC 52 under `policy`.
pub fn send_osc52(policy: TuiOsc52) -> bool {
    match policy {
        TuiOsc52::Auto | TuiOsc52::Always => true,
        TuiOsc52::Never => false,
    }
}

/// `TuiOsc52` as stored in the host (an atomic byte).
pub fn to_byte(p: TuiOsc52) -> u8 {
    match p {
        TuiOsc52::Auto => 0,
        TuiOsc52::Always => 1,
        TuiOsc52::Never => 2,
    }
}

/// The inverse of [`to_byte`] (unknown bytes are `Auto`).
pub fn from_byte(b: u8) -> TuiOsc52 {
    match b {
        1 => TuiOsc52::Always,
        2 => TuiOsc52::Never,
        _ => TuiOsc52::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies() {
        assert!(send_osc52(TuiOsc52::Auto));
        assert!(send_osc52(TuiOsc52::Always));
        assert!(!send_osc52(TuiOsc52::Never));
        for p in TuiOsc52::ALL {
            assert_eq!(from_byte(to_byte(p)), p);
        }
    }
}

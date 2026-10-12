//! Frontend-neutral colours: an sRGB colour with unmultiplied alpha, `#hex` parsing and
//! formatting. Frontends convert it to their own colour type (the egui GUI:
//! `theme::tokens::color32`); palette-dependent colours are resolved there.

/// An sRGB colour with straight (unmultiplied) alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba8 { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Rgba8 { r, g, b, a }
    }

    /// The same colour, fully opaque.
    pub const fn opaque(self) -> Self {
        Rgba8 { a: 255, ..self }
    }

    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// Parse `#RGB`, `#RRGGBB` or `#RRGGBBAA` (leading `#` optional).
pub fn parse_hex(s: &str) -> Option<Rgba8> {
    let h = s.trim().trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        3 => {
            let n = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|v| v * 17);
            Some(Rgba8::rgb(n(0)?, n(1)?, n(2)?))
        }
        6 => Some(Rgba8::rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Rgba8::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

/// `#RRGGBB`, or `#RRGGBBAA` for translucent colours.
pub fn hex_string(c: Rgba8) -> String {
    let Rgba8 { r, g, b, a } = c;
    if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(parse_hex("#f80"), Some(Rgba8::rgb(0xFF, 0x88, 0x00)));
        assert_eq!(parse_hex("0A0B0C"), Some(Rgba8::rgb(10, 11, 12)));
        assert_eq!(parse_hex(" #01020380 "), Some(Rgba8::rgba(1, 2, 3, 0x80)));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
        assert_eq!(hex_string(Rgba8::rgb(1, 2, 255)), "#0102FF");
        assert_eq!(hex_string(Rgba8::rgba(1, 2, 3, 4)), "#01020304");
        assert_eq!(parse_hex("#00FF0080").map(Rgba8::opaque), Some(Rgba8::rgb(0, 255, 0)));
    }
}

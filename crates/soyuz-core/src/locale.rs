//! Process-wide formatting locale: number grouping / decimal separator, month and
//! weekday names, relative-date words, and locale collation for natural sort (Turkish
//! dotted / dotless i, `ç ğ ö ş ü` in alphabet order).
//!
//! The UI strings themselves are translated in `soyuz-app` (Fluent); this module only
//! holds what GUI-free crates need to format sizes, dates and to sort names. The app sets
//! the current locale with [`set_current`] whenever the language setting changes; every
//! formatter here has an explicit `*_in(locale)` form for tests.

use std::sync::atomic::{AtomicU8, Ordering};

/// A supported formatting locale.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Locale {
    #[default]
    EnUs,
    TrTr,
}

impl Locale {
    /// Every supported locale (in the order the language picker lists them).
    pub const ALL: [Locale; 2] = [Locale::EnUs, Locale::TrTr];

    /// BCP-47 tag (`"en-US"`).
    pub fn tag(self) -> &'static str {
        match self {
            Locale::EnUs => "en-US",
            Locale::TrTr => "tr-TR",
        }
    }

    /// The language's own name ("English", "Türkçe"); never translated.
    pub fn native_name(self) -> &'static str {
        match self {
            Locale::EnUs => "English",
            Locale::TrTr => "Türkçe",
        }
    }

    /// Best supported match for a BCP-47 / POSIX-ish tag (`"tr"`, `"tr-TR"`, `"tr_CY"`
    /// → Turkish); anything unknown → `None`.
    pub fn from_tag(tag: &str) -> Option<Locale> {
        let lang = tag.trim().split(['-', '_', '.']).next().unwrap_or("").to_ascii_lowercase();
        match lang.as_str() {
            "en" => Some(Locale::EnUs),
            "tr" => Some(Locale::TrTr),
            _ => None,
        }
    }

    /// Decimal separator (`.` / `,`).
    pub fn decimal_sep(self) -> char {
        match self {
            Locale::EnUs => '.',
            Locale::TrTr => ',',
        }
    }

    /// Thousands separator (`,` / `.`).
    pub fn group_sep(self) -> char {
        match self {
            Locale::EnUs => ',',
            Locale::TrTr => '.',
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The current formatting locale (default en-US).
pub fn current() -> Locale {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Locale::TrTr,
        _ => Locale::EnUs,
    }
}

/// Switch the process-wide formatting locale (live; next frame picks it up).
pub fn set_current(l: Locale) {
    CURRENT.store(
        match l {
            Locale::EnUs => 0,
            Locale::TrTr => 1,
        },
        Ordering::Relaxed,
    );
}

/// `1234567` → `"1,234,567"` (en) / `"1.234.567"` (tr).
pub fn group_int_in(n: u64, l: Locale) -> String {
    let digits = n.to_string();
    let sep = l.group_sep();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

/// [`group_int_in`] in the current locale.
pub fn group_int(n: u64) -> String {
    group_int_in(n, current())
}

/// A float with `decimals` fraction digits, grouped integer part and the locale's
/// decimal separator: `1234.5` → `"1,234.5"` / `"1.234,5"`.
pub fn format_decimal_in(v: f64, decimals: usize, l: Locale) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    let s = format!("{:.*}", decimals, v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let mut out = String::new();
    if v < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.push('-');
    }
    out.push_str(&group_int_in(int.parse().unwrap_or(0), l));
    if !frac.is_empty() {
        out.push(l.decimal_sep());
        out.push_str(frac);
    }
    out
}

/// [`format_decimal_in`] in the current locale.
pub fn format_decimal(v: f64, decimals: usize) -> String {
    format_decimal_in(v, decimals, current())
}

/// Full month names, January first.
pub fn month_names(l: Locale) -> [&'static str; 12] {
    match l {
        Locale::EnUs => [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ],
        Locale::TrTr => [
            "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım",
            "Aralık",
        ],
    }
}

/// Abbreviated month names, January first.
pub fn month_abbr(l: Locale) -> [&'static str; 12] {
    match l {
        Locale::EnUs => ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
        Locale::TrTr => ["Oca", "Şub", "Mar", "Nis", "May", "Haz", "Tem", "Ağu", "Eyl", "Eki", "Kas", "Ara"],
    }
}

/// Full weekday names, Monday first.
pub fn weekday_names(l: Locale) -> [&'static str; 7] {
    match l {
        Locale::EnUs => ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
        Locale::TrTr => ["Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar"],
    }
}

/// Abbreviated weekday names, Monday first.
pub fn weekday_abbr(l: Locale) -> [&'static str; 7] {
    match l {
        Locale::EnUs => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
        Locale::TrTr => ["Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz"],
    }
}

/// Words used by relative dates, ETAs and speeds.
pub(crate) struct Words {
    pub just_now: &'static str,
    /// `{}` = minutes.
    pub min_ago: &'static str,
    pub today: &'static str,
    pub yesterday: &'static str,
    pub per_sec: &'static str,
    pub sec: &'static str,
    pub min: &'static str,
    pub hour: &'static str,
    pub day: &'static str,
    /// Space between an ETA number and its unit (`"3m"` vs `"3 dk"`).
    pub unit_gap: &'static str,
}

pub(crate) fn words(l: Locale) -> &'static Words {
    match l {
        Locale::EnUs => &Words {
            just_now: "Just now",
            min_ago: "{} min ago",
            today: "Today",
            yesterday: "Yesterday",
            per_sec: "/s",
            sec: "s",
            min: "m",
            hour: "h",
            day: "d",
            unit_gap: "",
        },
        Locale::TrTr => &Words {
            just_now: "Az önce",
            min_ago: "{} dk önce",
            today: "Bugün",
            yesterday: "Dün",
            per_sec: "/sn",
            sec: "sn",
            min: "dk",
            hour: "sa",
            day: "g",
            unit_gap: " ",
        },
    }
}

/// Lower-case one char for case-insensitive comparison in locale `l` (Turkish:
/// `I` → `ı`, `İ` → `i`). Returns the first char of the lower-case mapping.
#[inline]
pub fn lower_char_in(c: char, l: Locale) -> char {
    if c.is_ascii() {
        return match (l, c) {
            (Locale::TrTr, 'I') => 'ı',
            _ => c.to_ascii_lowercase(),
        };
    }
    match (l, c) {
        // In every locale 'İ' folds to a plain 'i' (Unicode maps it to "i\u{307}").
        (_, 'İ') => 'i',
        _ => c.to_lowercase().next().unwrap_or(c),
    }
}

/// Lower-case a whole string with Turkish-aware `I`/`İ` handling in locale `l`.
pub fn to_lower_in(s: &str, l: Locale) -> String {
    s.chars().map(|c| lower_char_in(c, l)).collect()
}

/// Collation weight of one (already case-folded, if wanted) char. Code point order,
/// except that in Turkish the letters `ç ğ ı ö ş ü` (and capitals) sort right after
/// their base letter (`ı` right before `i`), matching the Turkish alphabet.
#[inline]
pub fn collation_weight(c: char, l: Locale) -> u64 {
    let base = |b: char, off: u64| u64::from(b) * 4 + off;
    if l == Locale::TrTr {
        match c {
            'ç' => return base('c', 2),
            'Ç' => return base('C', 2),
            'ğ' => return base('g', 2),
            'Ğ' => return base('G', 2),
            // Dotless ı comes before i: between h and i.
            'ı' => return base('h', 2),
            // Dotless capital I is before İ (the dotted capital).
            'I' => return base('H', 2),
            'İ' => return base('I', 0),
            'ö' => return base('o', 2),
            'Ö' => return base('O', 2),
            'ş' => return base('s', 2),
            'Ş' => return base('S', 2),
            'ü' => return base('u', 2),
            'Ü' => return base('U', 2),
            _ => {}
        }
    }
    u64::from(c) * 4
}

/// Fold a string for case-insensitive *matching* (filters, type-ahead, palette): lower
/// case, and all four Turkish i forms (`I İ ı i`) fold to `i` so that typing "i" finds
/// "IMG", "ılık" and "İstanbul" whichever keyboard layout was used. Locale-independent.
pub fn fold_for_match(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'I' | 'İ' | 'ı' => out.push('i'),
            c if c.is_ascii() => out.push(c.to_ascii_lowercase()),
            c => out.extend(c.to_lowercase().filter(|&m| m != '\u{307}')),
        }
    }
    out
}

/// One char of [`fold_for_match`] (for char-by-char fuzzy matchers).
#[inline]
pub fn fold_char_for_match(c: char) -> char {
    match c {
        'I' | 'İ' | 'ı' => 'i',
        c if c.is_ascii() => c.to_ascii_lowercase(),
        c => c.to_lowercase().next().unwrap_or(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(Locale::from_tag("tr-TR"), Some(Locale::TrTr));
        assert_eq!(Locale::from_tag("tr"), Some(Locale::TrTr));
        assert_eq!(Locale::from_tag("en"), Some(Locale::EnUs));
        assert_eq!(Locale::from_tag("en-GB"), Some(Locale::EnUs));
        assert_eq!(Locale::from_tag("de-DE"), None);
        for l in Locale::ALL {
            assert_eq!(Locale::from_tag(l.tag()), Some(l));
        }
    }

    #[test]
    fn numbers() {
        assert_eq!(group_int_in(1_234_567, Locale::EnUs), "1,234,567");
        assert_eq!(group_int_in(1_234_567, Locale::TrTr), "1.234.567");
        assert_eq!(group_int_in(999, Locale::TrTr), "999");
        assert_eq!(format_decimal_in(1234.5, 1, Locale::EnUs), "1,234.5");
        assert_eq!(format_decimal_in(1234.5, 1, Locale::TrTr), "1.234,5");
        assert_eq!(format_decimal_in(-0.25, 2, Locale::TrTr), "-0,25");
        assert_eq!(format_decimal_in(3.0, 0, Locale::TrTr), "3");
    }

    #[test]
    fn turkish_lower() {
        assert_eq!(to_lower_in("IŞIK", Locale::TrTr), "ışık");
        assert_eq!(to_lower_in("İSTANBUL", Locale::TrTr), "istanbul");
        assert_eq!(to_lower_in("IMAGE", Locale::EnUs), "image");
        assert_eq!(to_lower_in("İzmir", Locale::EnUs), "izmir");
    }

    #[test]
    fn match_folding() {
        assert_eq!(fold_for_match("İSTANBUL"), "istanbul");
        assert_eq!(fold_for_match("ılık"), "ilik");
        assert_eq!(fold_for_match("IMG_001.JPG"), "img_001.jpg");
        assert_eq!(fold_for_match("Çağrı"), "çağri");
        assert_eq!(fold_char_for_match('İ'), 'i');
        assert_eq!(fold_char_for_match('Ş'), 'ş');
    }

    #[test]
    fn turkish_collation_order() {
        let w = |c| collation_weight(c, Locale::TrTr);
        // a b c ç d e f g ğ h ı i j k l m n o ö p r s ş t u ü v y z
        let alphabet = "abcçdefgğhıijklmnoöprsştuüvyz";
        let v: Vec<u64> = alphabet.chars().map(w).collect();
        assert!(v.windows(2).all(|p| p[0] < p[1]), "{v:?}");
        let upper = "ABCÇDEFGĞHIİJKLMNOÖPRSŞTUÜVYZ";
        let v: Vec<u64> = upper.chars().map(w).collect();
        assert!(v.windows(2).all(|p| p[0] < p[1]), "{v:?}");
        // English keeps code point order (ç after z).
        assert!(collation_weight('ç', Locale::EnUs) > collation_weight('z', Locale::EnUs));
    }
}

//! Property tests (ROADMAP R2): natural sort is a total order, name validation rules.

use std::cmp::Ordering;

use proptest::prelude::*;
use soyuz_core::{NameError, natural_cmp, validate_name_with};

/// Names mixing letters (both cases, a few non-ASCII), digit runs with leading zeros,
/// separators and punctuation that sort around digits.
fn name() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        prop::sample::select(vec!["a", "B", "b", "Z", "é", "É", "ß", "_", "-", ".", " ", "!", "~", "ſ", "İ"])
            .prop_map(str::to_string),
        (0u32..3, 0u64..20_000).prop_map(|(zeros, n)| format!("{}{n}", "0".repeat(zeros as usize))),
    ];
    prop::collection::vec(piece, 0..6).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    #[test]
    fn natural_cmp_is_a_total_order(a in name(), b in name(), c in name(), ci in any::<bool>()) {
        let ab = natural_cmp(&a, &b, ci);
        // Antisymmetric, and Equal only for identical strings.
        prop_assert_eq!(ab, natural_cmp(&b, &a, ci).reverse());
        prop_assert_eq!(ab == Ordering::Equal, a == b);
        prop_assert_eq!(natural_cmp(&a, &a, ci), Ordering::Equal);
        // Transitive.
        let bc = natural_cmp(&b, &c, ci);
        if ab != Ordering::Greater && bc != Ordering::Greater {
            prop_assert_ne!(natural_cmp(&a, &c, ci), Ordering::Greater, "{:?} <= {:?} <= {:?}", a, b, c);
        }
    }

    #[test]
    fn natural_sort_is_consistent(mut v in prop::collection::vec(name(), 0..40), ci in any::<bool>()) {
        v.sort_by(|x, y| natural_cmp(x, y, ci));
        for w in v.windows(2) {
            prop_assert_ne!(natural_cmp(&w[0], &w[1], ci), Ordering::Greater);
        }
        let mut again = v.clone();
        again.reverse();
        again.sort_by(|x, y| natural_cmp(x, y, ci));
        prop_assert_eq!(again, v, "sorting is order-independent");
    }

    #[test]
    fn numbers_compare_numerically(x in 0u64..1_000_000, y in 0u64..1_000_000, pre in "[a-z]{0,3}") {
        let a = format!("{pre}{x}");
        let b = format!("{pre}{y}");
        prop_assert_eq!(natural_cmp(&a, &b, false), x.cmp(&y));
    }

    #[test]
    fn validation_never_panics_and_respects_rules(s in any::<String>(), win in any::<bool>()) {
        match validate_name_with(&s, win) {
            Ok(()) => {
                prop_assert!(!s.is_empty() && s != "." && s != "..");
                prop_assert!(!s.contains('/') && !s.contains('\0'));
                if win {
                    prop_assert!(!s.chars().any(|c| r#"<>:"\|?*"#.contains(c) || (c as u32) < 32));
                    prop_assert!(!s.ends_with('.') && !s.ends_with(' '));
                    prop_assert!(s.encode_utf16().count() <= 255);
                } else {
                    prop_assert!(s.len() <= 255);
                }
            }
            Err(NameError::Empty) => prop_assert!(s.is_empty()),
            Err(NameError::InvalidChar(c)) => prop_assert!(s.contains(c)),
            Err(_) => {}
        }
    }

    #[test]
    fn plain_names_are_valid(s in "[A-Za-z0-9_][A-Za-z0-9 _.-]{0,40}[A-Za-z0-9_]") {
        let base = s.split('.').next().unwrap().trim_end().to_uppercase();
        let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
            || ((base.starts_with("COM") || base.starts_with("LPT")) && base.len() == 4
                && base.as_bytes()[3].is_ascii_digit() && base.as_bytes()[3] != b'0');
        prop_assert_eq!(validate_name_with(&s, true).is_ok(), !reserved, "{:?}", s);
        prop_assert!(validate_name_with(&s, false).is_ok());
    }

    #[test]
    fn reserved_device_names_are_rejected_with_any_extension(
        dev in prop::sample::select(vec!["CON", "prn", "Aux", "nul", "COM1", "lpt9", "CONIN$"]),
        ext in r"(\.[a-z]{1,4}){0,2}",
    ) {
        let n = format!("{dev}{ext}");
        prop_assert!(matches!(validate_name_with(&n, true), Err(NameError::Reserved(_))), "{}", n);
        prop_assert!(validate_name_with(&n, false).is_ok());
    }

    #[test]
    fn overlong_names_are_rejected(n in 256usize..400) {
        let s = "x".repeat(n);
        prop_assert_eq!(validate_name_with(&s, true), Err(NameError::TooLong(n)));
    }
}

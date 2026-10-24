//! The Fluent catalog parses only the bundles it needs: en-US alone while English is
//! active, tr-TR (plus en-US only on a miss) while Turkish is, and a switch drops every
//! bundle but the new language's (each bundle is ~3.9 MB of heap, measured with a
//! counting allocator). Its own test binary, since it switches the
//! process-wide language.

use soyuz_appcore::i18n;
use soyuz_core::Locale;

#[test]
fn bundles_are_parsed_lazily_and_dropped_on_switch() {
    assert!(i18n::loaded_locales().is_empty(), "nothing parsed before the first lookup");

    // A tr-TR hit needs no en-US bundle.
    assert_eq!(i18n::t_in("common-cancel", Locale::TrTr), "İptal");
    assert_eq!(i18n::loaded_locales(), [Locale::TrTr]);

    i18n::apply_setting("tr-TR");
    assert_eq!(i18n::t("common-cancel"), "İptal");
    assert_eq!(i18n::loaded_locales(), [Locale::TrTr], "Turkish active: en-US not parsed");

    i18n::apply_setting("en-US");
    assert_eq!(i18n::loaded_locales(), Vec::<Locale>::new(), "tr-TR dropped when leaving Turkish");
    assert_eq!(i18n::t("common-cancel"), "Cancel");
    assert_eq!(i18n::loaded_locales(), [Locale::EnUs], "English active: only en-US parsed");

    // Interned strings outlive their bundle, so look up a key not translated yet.
    i18n::apply_setting("tr-TR");
    assert_ne!(i18n::t("common-ok"), "common-ok");
    assert_eq!(i18n::loaded_locales(), [Locale::TrTr], "en-US dropped when leaving English");

    // A miss parses the en-US fallback again.
    assert_eq!(i18n::t("no-such-key-for-tests"), "no-such-key-for-tests");
    assert_eq!(i18n::loaded_locales(), [Locale::EnUs, Locale::TrTr]);

    i18n::apply_setting("en-US");
    assert_eq!(i18n::loaded_locales(), [Locale::EnUs]);
}

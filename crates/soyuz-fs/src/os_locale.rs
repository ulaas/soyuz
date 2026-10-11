//! The OS user-interface language, for `general.language = "auto"`.
//!
//! Windows: kernel32 `GetUserDefaultUILanguage` (the display language, not the regional
//! format). macOS: the first of `AppleLanguages`, else `AppleLocale`, from the global
//! preferences domain (CoreFoundation), then the variables below.
//! Unix: `LC_ALL` / `LC_MESSAGES` / `LANG` (`tr_TR.UTF-8` → `tr-TR`).

/// BCP-47 name of the UI language (`"en-US"`, `"tr-TR"`), `None` when unknown.
pub fn ui_language() -> Option<String> {
    #[cfg(windows)]
    {
        crate::sys::windows::locale::ui_language()
    }
    #[cfg(target_os = "macos")]
    {
        use crate::sys::macos::cf;
        let first = cf::global_strings("AppleLanguages", false).into_iter().find_map(|l| apple_to_bcp47(&l));
        if let Some(l) = first.or_else(|| cf::global_string("AppleLocale", false).and_then(|l| apple_to_bcp47(&l))) {
            return Some(l);
        }
    }
    #[cfg(not(windows))]
    {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .and_then(|v| posix_to_bcp47(&v))
    }
}

/// macOS language / locale identifiers (`tr-TR`, `en_US@rg=trzzzz`, `zh-Hans-CN`) →
/// BCP-47 (`tr-TR`, `en-US`, `zh-Hans-CN`).
pub fn apple_to_bcp47(v: &str) -> Option<String> {
    posix_to_bcp47(v.trim())
}

/// `tr_TR.UTF-8@euro` → `tr-TR`; `C` / `POSIX` → `None`.
pub fn posix_to_bcp47(v: &str) -> Option<String> {
    let base = v.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    Some(base.replace('_', "-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_names() {
        assert_eq!(posix_to_bcp47("tr_TR.UTF-8").as_deref(), Some("tr-TR"));
        assert_eq!(posix_to_bcp47("de_DE@euro").as_deref(), Some("de-DE"));
        assert_eq!(posix_to_bcp47("C"), None);
        assert_eq!(posix_to_bcp47(""), None);
        assert_eq!(apple_to_bcp47("en_US@rg=trzzzz").as_deref(), Some("en-US"));
        assert_eq!(apple_to_bcp47("zh-Hans-CN").as_deref(), Some("zh-Hans-CN"));
        assert_eq!(apple_to_bcp47("tr").as_deref(), Some("tr"));
    }

    #[test]
    fn ui_language_does_not_panic() {
        if let Some(l) = ui_language() {
            assert!(!l.is_empty());
        }
    }
}

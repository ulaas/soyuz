//! The OS user-interface language, for `general.language = "auto"`.
//!
//! Windows: kernel32 `GetUserDefaultUILanguage` (the display language, not the regional
//! format). Unix: `LC_ALL` / `LC_MESSAGES` / `LANG` (`tr_TR.UTF-8` → `tr-TR`).

/// BCP-47 name of the UI language (`"en-US"`, `"tr-TR"`), `None` when unknown.
pub fn ui_language() -> Option<String> {
    #[cfg(windows)]
    {
        crate::sys::windows::locale::ui_language()
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
    }

    #[test]
    fn ui_language_does_not_panic() {
        if let Some(l) = ui_language() {
            assert!(!l.is_empty());
        }
    }
}

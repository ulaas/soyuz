//! Installed fonts as the OS lists them, for font pickers and family-name lookups.
//!
//! Windows: the `Fonts` registry key of the machine
//! (`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`, file names relative to
//! `%WINDIR%\Fonts`) and of the user (`HKCU\…`, absolute paths into
//! `%LOCALAPPDATA%\Microsoft\Windows\Fonts`). Value names look like
//! `Segoe UI Semibold (TrueType)` or `Cambria & Cambria Math (TrueType)`; they are
//! returned raw, the app parses them. Read-only advapi32; other platforms return nothing.

use std::path::PathBuf;

/// One registry entry: the display name (`Segoe UI Semibold (TrueType)`) and the font
/// file it names (absolute when the registry gives a bare file name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledFont {
    pub name: String,
    pub file: PathBuf,
    /// From the per-user font key.
    pub per_user: bool,
}

/// The system fonts folder (`%WINDIR%\Fonts`).
pub fn system_font_dir() -> PathBuf {
    let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    windir.join("Fonts")
}

/// The per-user fonts folder (`%LOCALAPPDATA%\Microsoft\Windows\Fonts`).
pub fn user_font_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join(r"Microsoft\Windows\Fonts"))
}

/// Every installed font registered with the OS (machine first, then per-user).
pub fn installed() -> Vec<InstalledFont> {
    #[cfg(windows)]
    {
        use crate::sys::windows::registry::{Hive, string_values};
        const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";
        let sys = system_font_dir();
        let user = user_font_dir();
        let mut out = Vec::new();
        for (hive, per_user) in [(Hive::LocalMachine, false), (Hive::CurrentUser, true)] {
            for (name, value) in string_values(hive, KEY) {
                let p = PathBuf::from(value.trim());
                let file = if p.is_absolute() {
                    p
                } else if per_user {
                    user.clone().unwrap_or_else(|| sys.clone()).join(p)
                } else {
                    sys.join(p)
                };
                out.push(InstalledFont { name, file, per_user });
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn segoe_ui_is_registered() {
        let all = super::installed();
        let seg = all.iter().find(|f| f.name.starts_with("Segoe UI (")).expect("Segoe UI registered");
        assert!(seg.file.is_absolute());
        assert!(seg.file.is_file(), "{}", seg.file.display());
    }
}

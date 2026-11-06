//! Programs that start with the session: `Run` / `RunOnce` registry keys and the two Startup
//! folders (known paths from environment variables, plain directory reads), with the state
//! Task Manager keeps under `StartupApproved`. Read-only.

use std::ptr::null_mut;

use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RegGetValueW,
};

use super::super::registry::{self, Hive};
use super::super::wide_str;
use crate::sysmon::StartupItem;

const RUN: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
const RUN_ONCE: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce";
const RUN_32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const RUN_ONCE_32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\RunOnce";
const APPROVED: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";
const STARTUP_FOLDER: &str = r"Microsoft\Windows\Start Menu\Programs\Startup";

fn root(h: Hive) -> HKEY {
    if h == Hive::LocalMachine { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER }
}

/// The first byte of a binary value.
fn first_byte(h: Hive, subkey: &str, value: &str) -> Option<u8> {
    let k = wide_str(subkey);
    let v = wide_str(value);
    let mut data = [0u8; 64];
    let mut bytes = data.len() as u32;
    // SAFETY: predefined root, NUL-terminated names, output buffer of `bytes` bytes.
    let rc = unsafe {
        RegGetValueW(
            root(h),
            k.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_BINARY,
            null_mut(),
            data.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    (rc == 0 && bytes > 0).then_some(data[0])
}

/// Bit 0 of a `StartupApproved` value's first byte marks the entry as turned off (2 and 6 are
/// on, 3 and 7 off). No value means on.
fn approved(first: Option<u8>) -> bool {
    first.is_none_or(|b| b & 1 == 0)
}

fn hive_name(h: Hive) -> &'static str {
    if h == Hive::LocalMachine { "HKLM" } else { "HKCU" }
}

fn read_key(h: Hive, key: &str, approved_key: Option<&str>, out: &mut Vec<StartupItem>) {
    for (name, command) in registry::string_values(h, key) {
        let enabled = approved_key.is_none_or(|a| approved(first_byte(h, &format!(r"{APPROVED}\{a}"), &name)));
        out.push(StartupItem { name, command, source: format!(r"{}\{key}", hive_name(h)), enabled });
    }
}

fn read_folder(dir: &std::path::Path, h: Hive, out: &mut Vec<StartupItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.eq_ignore_ascii_case("desktop.ini") {
            continue;
        }
        let name = match file.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem.to_string(),
            _ => file.clone(),
        };
        out.push(StartupItem {
            name,
            command: entry.path().to_string_lossy().into_owned(),
            source: dir.to_string_lossy().into_owned(),
            enabled: approved(first_byte(h, &format!(r"{APPROVED}\StartupFolder"), &file)),
        });
    }
}

/// Programs that start with the session or machine.
pub fn startup_items() -> Vec<StartupItem> {
    let mut out = Vec::new();
    read_key(Hive::CurrentUser, RUN, Some("Run"), &mut out);
    read_key(Hive::CurrentUser, RUN_ONCE, None, &mut out);
    read_key(Hive::LocalMachine, RUN, Some("Run"), &mut out);
    read_key(Hive::LocalMachine, RUN_ONCE, None, &mut out);
    read_key(Hive::LocalMachine, RUN_32, Some("Run32"), &mut out);
    read_key(Hive::LocalMachine, RUN_ONCE_32, None, &mut out);
    if let Some(dir) = std::env::var_os("APPDATA") {
        read_folder(&std::path::Path::new(&dir).join(STARTUP_FOLDER), Hive::CurrentUser, &mut out);
    }
    if let Some(dir) = std::env::var_os("ProgramData") {
        read_folder(&std::path::Path::new(&dir).join(STARTUP_FOLDER), Hive::LocalMachine, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_bytes() {
        assert!(approved(None));
        assert!(approved(Some(2)));
        assert!(approved(Some(6)));
        assert!(!approved(Some(3)));
        assert!(!approved(Some(7)));
    }

    #[test]
    fn folder_listing_skips_desktop_ini() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("desktop.ini"), "x").unwrap();
        std::fs::write(dir.path().join("Tool.lnk"), "x").unwrap();
        let mut out = Vec::new();
        read_folder(dir.path(), Hive::CurrentUser, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "Tool");
        assert!(out[0].enabled);
        assert!(out[0].command.ends_with("Tool.lnk"));
    }
}

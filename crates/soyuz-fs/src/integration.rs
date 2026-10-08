//! OS integration for installers and updates (roadmap D1):
//!
//! * the opt-in "default file manager" registration: per-user `HKCU\Software\Classes`
//!   verbs for `Directory` and `Drive`, written with advapi32 only (no shell API), with an
//!   exact undo ([`restore_explorer`]) driven by a backup stored under `HKCU\Software\Soyuz`;
//! * the user's Downloads folder (from the registry, no `SHGetKnownFolderPath`);
//! * Authenticode verification of a downloaded installer ([`authenticode`]).
//!
//! `Folder` is deliberately left alone: it also covers shell-namespace folders (This PC,
//! Control Panel, Recycle Bin) whose `%1` is a `::{CLSID}` moniker Soyuz cannot open.

use std::io;
use std::path::{Path, PathBuf};

/// Name of the verb Soyuz adds under `<Class>\shell`.
pub const VERB: &str = "soyuz";

/// Classes whose default "open" verb Soyuz takes over.
pub const CLASSES: [&str; 2] = ["Directory", "Drive"];

/// Where the registration is written (both relative to `HKEY_CURRENT_USER`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssocKeys {
    /// The per-user classes root, normally `Software\Classes`.
    pub classes: String,
    /// Where the backup of what we changed is kept, normally `Software\Soyuz\DefaultFileManager`.
    pub state: String,
}

impl AssocKeys {
    /// The real per-user association keys.
    pub fn user() -> Self {
        AssocKeys { classes: r"Software\Classes".into(), state: r"Software\Soyuz\DefaultFileManager".into() }
    }

    /// A sandbox under `HKCU\<base>` (tests): `<base>\Classes` and `<base>\State`.
    pub fn sandbox(base: &str) -> Self {
        AssocKeys { classes: format!(r"{base}\Classes"), state: format!(r"{base}\State") }
    }
}

/// Whether Soyuz currently opens folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileManagerStatus {
    /// Explorer (or whatever was there before) handles folders.
    NotRegistered,
    /// Registered; folders open with this command line.
    Registered { command: String },
    /// Our backup exists but the verbs were changed by something else since.
    Modified,
    /// Not available on this platform.
    Unsupported,
}

/// Command line written for the verb: `"<exe>" "%1"`.
pub fn open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", crate::strip_verbatim(exe).display())
}

/// Result of checking a file's Authenticode signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signature {
    /// Signed with a valid, trusted signature.
    Valid,
    /// No Authenticode signature.
    Unsigned,
    /// Signed, but the signature is broken or not trusted (`HRESULT`).
    Invalid(i32),
    /// Not checked (not supported on this platform).
    Unknown,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::sys::windows::registry as reg;

    const PREV_SUFFIX: &str = ".prev_default";
    const HAD_SUFFIX: &str = ".had_default";
    const CREATED: &str = "created_keys";
    const COMMAND: &str = "command";

    fn shell_key(keys: &AssocKeys, class: &str) -> String {
        format!(r"{}\{class}\shell", keys.classes)
    }

    pub fn status(keys: &AssocKeys) -> FileManagerStatus {
        if !reg::hkcu_key_exists(&keys.state) {
            return FileManagerStatus::NotRegistered;
        }
        let want = reg::get_string(reg::Hive::CurrentUser, &keys.state, Some(COMMAND)).unwrap_or_default();
        let ok = CLASSES.iter().all(|c| {
            let shell = shell_key(keys, c);
            reg::get_string(reg::Hive::CurrentUser, &shell, None).as_deref() == Some(VERB)
                && reg::get_string(reg::Hive::CurrentUser, &format!(r"{shell}\{VERB}\command"), None).as_deref()
                    == Some(want.as_str())
        });
        if ok { FileManagerStatus::Registered { command: want } } else { FileManagerStatus::Modified }
    }

    pub fn register(keys: &AssocKeys, exe: &Path) -> io::Result<()> {
        if reg::hkcu_key_exists(&keys.state) {
            restore(keys)?;
        }
        let command = open_command(exe);
        // Record the keys we are about to create, parents first, so the undo can remove
        // exactly those (and only when they are empty again).
        let mut created: Vec<String> = Vec::new();
        for class in CLASSES {
            for rel in [class.to_string(), format!(r"{class}\shell")] {
                let full = format!(r"{}\{rel}", keys.classes);
                if !reg::hkcu_key_exists(&full) && !created.contains(&rel) {
                    created.push(rel);
                }
            }
        }
        reg::hkcu_create_key(&keys.state)?;
        reg::hkcu_set_string(&keys.state, Some(CREATED), &created.join("\n"))?;
        reg::hkcu_set_string(&keys.state, Some(COMMAND), &command)?;
        for class in CLASSES {
            let shell = shell_key(keys, class);
            let prev = reg::get_string(reg::Hive::CurrentUser, &shell, None);
            reg::hkcu_set_string(
                &keys.state,
                Some(&format!("{class}{HAD_SUFFIX}")),
                if prev.is_some() { "1" } else { "0" },
            )?;
            reg::hkcu_set_string(
                &keys.state,
                Some(&format!("{class}{PREV_SUFFIX}")),
                prev.as_deref().unwrap_or_default(),
            )?;
        }
        let res = (|| {
            for class in CLASSES {
                let shell = shell_key(keys, class);
                let verb = format!(r"{shell}\{VERB}");
                reg::hkcu_set_string(&verb, None, "Open in Soyuz")?;
                reg::hkcu_set_string(&verb, Some("Icon"), &format!("\"{}\",0", crate::strip_verbatim(exe).display()))?;
                reg::hkcu_set_string(&format!(r"{verb}\command"), None, &command)?;
                reg::hkcu_set_string(&shell, None, VERB)?;
            }
            Ok(())
        })();
        if let Err(e) = res {
            // Leave nothing half-done behind.
            let _ = restore(keys);
            return Err(e);
        }
        Ok(())
    }

    pub fn restore(keys: &AssocKeys) -> io::Result<()> {
        if !reg::hkcu_key_exists(&keys.state) {
            return Ok(());
        }
        let st = |name: &str| reg::get_string(reg::Hive::CurrentUser, &keys.state, Some(name));
        for class in CLASSES {
            let shell = shell_key(keys, class);
            reg::hkcu_delete_tree(&format!(r"{shell}\{VERB}"))?;
            let current = reg::get_string(reg::Hive::CurrentUser, &shell, None);
            // Only touch the default verb if it is still ours.
            if current.as_deref() == Some(VERB) {
                if st(&format!("{class}{HAD_SUFFIX}")).as_deref() == Some("1") {
                    reg::hkcu_set_string(&shell, None, &st(&format!("{class}{PREV_SUFFIX}")).unwrap_or_default())?;
                } else {
                    reg::hkcu_delete_value(&shell, None)?;
                }
            }
        }
        let created = st(CREATED).unwrap_or_default();
        for rel in created.lines().rev().filter(|l| !l.trim().is_empty()) {
            reg::hkcu_delete_key_if_empty(&format!(r"{}\{rel}", keys.classes))?;
        }
        reg::hkcu_delete_tree(&keys.state)?;
        // Drop `Software\Soyuz` (or the sandbox base) too when nothing else lives there.
        if let Some((parent, _)) = keys.state.rsplit_once('\\') {
            let _ = reg::hkcu_delete_key_if_empty(parent);
        }
        Ok(())
    }

    pub fn downloads() -> Option<PathBuf> {
        const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";
        const DOWNLOADS: &str = "{374DE290-123F-4565-9164-39C4925E467B}";
        reg::get_string(reg::Hive::CurrentUser, KEY, Some(DOWNLOADS)).map(PathBuf::from).filter(|p| p.is_absolute())
    }

    pub fn authenticode(path: &Path) -> Signature {
        match crate::sys::windows::wintrust::verify(path) {
            Ok(()) => Signature::Valid,
            Err(None) => Signature::Unsigned,
            Err(Some(hr)) => Signature::Invalid(hr),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "default file manager registration is Windows-only for now")
    }

    pub fn status(_keys: &AssocKeys) -> FileManagerStatus {
        FileManagerStatus::Unsupported
    }

    pub fn register(_keys: &AssocKeys, _exe: &Path) -> io::Result<()> {
        Err(unsupported())
    }

    pub fn restore(_keys: &AssocKeys) -> io::Result<()> {
        Err(unsupported())
    }

    pub fn downloads() -> Option<PathBuf> {
        None
    }

    pub fn authenticode(_path: &Path) -> Signature {
        Signature::Unknown
    }
}

/// Current registration state.
pub fn file_manager_status(keys: &AssocKeys) -> FileManagerStatus {
    imp::status(keys)
}

/// Make `exe` the per-user handler for opening folders and drives. Re-registering first
/// restores the previous state, so the backup always describes the pre-Soyuz setup.
pub fn register_file_manager(keys: &AssocKeys, exe: &Path) -> io::Result<()> {
    imp::register(keys, exe)
}

/// Undo [`register_file_manager`]: remove exactly the keys and values it added and put
/// back any default verb that was there before. A no-op when not registered.
pub fn restore_explorer(keys: &AssocKeys) -> io::Result<()> {
    imp::restore(keys)
}

/// The user's Downloads folder (`~/Downloads` when the OS does not say otherwise).
pub fn downloads_dir() -> PathBuf {
    imp::downloads().unwrap_or_else(|| crate::home().join("Downloads"))
}

/// Authenticode signature check (Windows); [`Signature::Unknown`] elsewhere.
pub fn authenticode(path: &Path) -> Signature {
    imp::authenticode(path)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::sys::windows::registry as reg;

    /// Every test uses its own sandbox under `HKCU\Software\Soyuz\TestAssoc`; the real
    /// associations are never touched.
    fn sandbox(name: &str) -> (AssocKeys, String) {
        let base = format!(r"Software\Soyuz\TestAssoc\{name}-{}", std::process::id());
        let _ = reg::hkcu_delete_tree(&base);
        (AssocKeys::sandbox(&base), base)
    }

    fn cleanup(base: &str) {
        reg::hkcu_delete_tree(base).unwrap();
        let _ = reg::hkcu_delete_key_if_empty(r"Software\Soyuz\TestAssoc");
        let _ = reg::hkcu_delete_key_if_empty(r"Software\Soyuz");
    }

    fn get(key: &str, value: Option<&str>) -> Option<String> {
        reg::get_string(reg::Hive::CurrentUser, key, value)
    }

    #[test]
    fn register_and_restore_on_empty_hive() {
        let (keys, base) = sandbox("empty");
        reg::hkcu_create_key(&keys.classes).unwrap();
        let exe = Path::new(r"C:\Apps\Soyuz\soyuz.exe");
        assert_eq!(file_manager_status(&keys), FileManagerStatus::NotRegistered);
        register_file_manager(&keys, exe).unwrap();
        let cmd = open_command(exe);
        assert_eq!(cmd, r#""C:\Apps\Soyuz\soyuz.exe" "%1""#);
        assert_eq!(file_manager_status(&keys), FileManagerStatus::Registered { command: cmd.clone() });
        for class in CLASSES {
            let shell = format!(r"{}\{class}\shell", keys.classes);
            assert_eq!(get(&shell, None).as_deref(), Some(VERB));
            assert_eq!(get(&format!(r"{shell}\soyuz\command"), None), Some(cmd.clone()));
        }
        // Registering again (e.g. after the exe moved) keeps a single, correct backup.
        let exe2 = Path::new(r"D:\Soyuz\soyuz.exe");
        register_file_manager(&keys, exe2).unwrap();
        assert_eq!(file_manager_status(&keys), FileManagerStatus::Registered { command: open_command(exe2) });

        restore_explorer(&keys).unwrap();
        assert_eq!(file_manager_status(&keys), FileManagerStatus::NotRegistered);
        for class in CLASSES {
            assert!(!reg::hkcu_key_exists(&format!(r"{}\{class}", keys.classes)), "{class} key left behind");
        }
        assert!(!reg::hkcu_key_exists(&keys.state));
        // Idempotent.
        restore_explorer(&keys).unwrap();
        cleanup(&base);
    }

    #[test]
    fn restore_keeps_what_was_there_before() {
        let (keys, base) = sandbox("existing");
        // Pre-existing state: another tool set a default verb for Directory, and Drive has
        // an unrelated verb we must not touch.
        let dir_shell = format!(r"{}\Directory\shell", keys.classes);
        let drive_shell = format!(r"{}\Drive\shell", keys.classes);
        reg::hkcu_set_string(&dir_shell, None, "othertool").unwrap();
        reg::hkcu_set_string(&format!(r"{dir_shell}\othertool\command"), None, "other.exe \"%1\"").unwrap();
        reg::hkcu_set_string(&format!(r"{drive_shell}\scan\command"), None, "scan.exe \"%1\"").unwrap();

        register_file_manager(&keys, Path::new(r"C:\soyuz.exe")).unwrap();
        assert_eq!(get(&dir_shell, None).as_deref(), Some(VERB));
        assert_eq!(get(&drive_shell, None).as_deref(), Some(VERB));

        restore_explorer(&keys).unwrap();
        assert_eq!(get(&dir_shell, None).as_deref(), Some("othertool"));
        assert_eq!(get(&drive_shell, None), None, "Drive had no default verb before");
        assert!(reg::hkcu_key_exists(&drive_shell), "pre-existing Drive\\shell must stay");
        assert_eq!(get(&format!(r"{drive_shell}\scan\command"), None).as_deref(), Some("scan.exe \"%1\""));
        assert_eq!(get(&format!(r"{dir_shell}\othertool\command"), None).as_deref(), Some("other.exe \"%1\""));
        assert!(!reg::hkcu_key_exists(&format!(r"{dir_shell}\soyuz")));
        assert!(!reg::hkcu_key_exists(&format!(r"{drive_shell}\soyuz")));
        cleanup(&base);
    }

    #[test]
    fn modified_by_someone_else_is_detected_and_not_clobbered() {
        let (keys, base) = sandbox("modified");
        reg::hkcu_create_key(&keys.classes).unwrap();
        register_file_manager(&keys, Path::new(r"C:\soyuz.exe")).unwrap();
        let dir_shell = format!(r"{}\Directory\shell", keys.classes);
        reg::hkcu_set_string(&dir_shell, None, "newtool").unwrap();
        assert_eq!(file_manager_status(&keys), FileManagerStatus::Modified);
        restore_explorer(&keys).unwrap();
        assert_eq!(get(&dir_shell, None).as_deref(), Some("newtool"), "a later tool's choice is kept");
        cleanup(&base);
    }

    #[test]
    fn downloads_dir_is_absolute() {
        assert!(downloads_dir().is_absolute());
    }

    #[test]
    fn authenticode_of_unsigned_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.exe");
        std::fs::write(&f, b"MZ not really").unwrap();
        assert!(matches!(authenticode(&f), Signature::Unsigned | Signature::Invalid(_)));
        // A binary with an embedded Microsoft signature, when the Windows SDK is installed.
        let kits = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
        let signed = std::fs::read_dir(kits)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join(r"x64\signtool.exe"))
            .find(|p| p.is_file());
        if let Some(p) = signed {
            assert_eq!(authenticode(&p), Signature::Valid, "{}", p.display());
        }
    }
}

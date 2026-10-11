//! Linux / BSD default-file-manager registration through `mimeapps.list` (see the
//! parent module), the opt-in `FileManager1` D-Bus service file, and the Downloads
//! folder from `user-dirs.dirs`.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{AssocKeys, FileManagerStatus, Signature, open_command};

/// The desktop file id Soyuz registers as.
pub const DESKTOP_ID: &str = "soyuz.desktop";
const MIME: &str = "inode/directory";
const SECTION: &str = "[Default Applications]";
const SERVICE: &str = "org.freedesktop.FileManager1";

/// How `mimeapps.list` was changed (enough to undo exactly that).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// An existing `inode/directory=` line had this raw value.
    Replaced { prev: String },
    /// A line was inserted after the existing section header (a newline was added to a
    /// header that ended the file without one).
    Inserted { header_newline_added: bool },
    /// The section (with this exact text) was appended at the end.
    Appended { suffix: String },
}

/// The backup written next to the registration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub command: String,
    pub change: Change,
    /// `mimeapps.list` did not exist before.
    pub created_list: bool,
    /// Previous content of `soyuz.desktop` (`None` = we created it).
    pub desktop_prev: Option<String>,
    /// The desktop id made the default when it is not our own `soyuz.desktop` (Flatpak:
    /// the exported `<app id>.desktop`; no desktop file is written then).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_id: Option<String>,
    /// What we wrote to `soyuz.desktop` (restore only undoes it while it is unchanged).
    /// Missing in backups from older versions.
    #[serde(default)]
    pub desktop_written: Option<String>,
}

/// Inside Flatpak: the desktop id Flatpak exports for us (`<app id>.desktop`).
fn flatpak_desktop_id() -> Option<String> {
    crate::launch::linux::flatpak_id().map(|id| format!("{id}.desktop"))
}

/// Lines with their terminators.
fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn eol(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

fn is_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// Index of the `key=` line inside `[Default Applications]`, and of the header.
fn find(ls: &[&str], key: &str) -> (Option<usize>, Option<usize>) {
    let mut header = None;
    let mut in_sec = false;
    for (i, l) in ls.iter().enumerate() {
        if is_header(l) {
            in_sec = l.trim() == SECTION;
            if in_sec && header.is_none() {
                header = Some(i);
            }
            continue;
        }
        if in_sec
            && let Some((k, _)) = l.split_once('=')
            && k.trim() == key
        {
            return (Some(i), header);
        }
    }
    (None, header)
}

/// The raw value after `=` of a line (terminator excluded).
fn raw_value(l: &str) -> &str {
    let start = l.find('=').map_or(l.len(), |i| i + 1);
    &l[start..l.len() - eol(l).len()]
}

/// The raw value of `key` in `[Default Applications]`.
pub fn default_value(text: &str, key: &str) -> Option<String> {
    let ls = lines(text);
    let i = find(&ls, key).0?;
    Some(raw_value(ls[i]).to_string())
}

/// First id of a `mimeapps.list` value.
pub fn first_id(value: &str) -> Option<&str> {
    value.split(';').map(str::trim).find(|s| !s.is_empty())
}

/// `ls[..i] + line + ls[i+1..]`.
fn splice(ls: &[&str], i: usize, line: &str) -> String {
    let mut out: String = ls[..i].concat();
    out.push_str(line);
    out.push_str(&ls[i + 1..].concat());
    out
}

/// Set `key=value` in `[Default Applications]`, touching nothing else.
pub fn set_default(text: &str, key: &str, value: &str) -> (String, Change) {
    let ls = lines(text);
    match find(&ls, key) {
        (Some(i), _) => {
            let l = ls[i];
            let eq = l.find('=').unwrap_or(l.len());
            let prev = raw_value(l).to_string();
            (splice(&ls, i, &format!("{}{value}{}", &l[..=eq], eol(l))), Change::Replaced { prev })
        }
        (None, Some(h)) => {
            let mut out: String = ls[..=h].concat();
            let added = eol(ls[h]).is_empty();
            if added {
                out.push('\n');
            }
            out.push_str(&format!("{key}={value}\n"));
            out.push_str(&ls[h + 1..].concat());
            (out, Change::Inserted { header_newline_added: added })
        }
        (None, None) => {
            let mut suffix = String::new();
            if !text.is_empty() {
                if !text.ends_with('\n') {
                    suffix.push('\n');
                }
                suffix.push('\n');
            }
            suffix.push_str(&format!("{SECTION}\n{key}={value}\n"));
            (format!("{text}{suffix}"), Change::Appended { suffix })
        }
    }
}

/// Undo [`set_default`] when the line still holds `ours`; otherwise leave it.
pub fn undo_default(text: &str, key: &str, ours: &str, change: &Change) -> String {
    if let Change::Appended { suffix } = change
        && let Some(stripped) = text.strip_suffix(suffix.as_str())
    {
        return stripped.to_string();
    }
    let ls = lines(text);
    let (Some(i), header) = find(&ls, key) else { return text.to_string() };
    let l = ls[i];
    if raw_value(l).trim() != ours {
        return text.to_string();
    }
    match change {
        Change::Replaced { prev } => {
            let eq = l.find('=').unwrap_or(l.len());
            splice(&ls, i, &format!("{}{prev}{}", &l[..=eq], eol(l)))
        }
        Change::Inserted { header_newline_added } => {
            let mut out = splice(&ls, i, "");
            if *header_newline_added && header == Some(i.wrapping_sub(1)) && i + 1 == ls.len() && out.ends_with('\n') {
                out.pop();
            }
            out
        }
        Change::Appended { .. } => splice(&ls, i, ""),
    }
}

fn config(keys: &AssocKeys) -> PathBuf {
    PathBuf::from(&keys.classes)
}

fn data(keys: &AssocKeys) -> PathBuf {
    PathBuf::from(&keys.state)
}

fn list_path(keys: &AssocKeys) -> PathBuf {
    config(keys).join("mimeapps.list")
}

pub fn desktop_path(keys: &AssocKeys) -> PathBuf {
    data(keys).join("applications").join(DESKTOP_ID)
}

fn state_path(keys: &AssocKeys) -> PathBuf {
    data(keys).join("soyuz").join("default-file-manager.json")
}

fn service_path(keys: &AssocKeys) -> PathBuf {
    data(keys).join("dbus-1/services").join(format!("{SERVICE}.service"))
}

fn service_state_path(keys: &AssocKeys) -> PathBuf {
    data(keys).join("soyuz").join("filemanager1-service.json")
}

/// `soyuz.desktop` for `exe`.
pub fn desktop_file(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Soyuz\nGenericName=File Manager\nComment=Browse and manage files\n\
         Exec={}\nIcon=system-file-manager\nTerminal=false\nStartupNotify=true\n\
         Categories=System;FileTools;FileManager;Utility;\nMimeType=inode/directory;\nX-Soyuz-Managed=true\n",
        open_command(exe)
    )
}

fn write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    crate::write_atomic(path, text.as_bytes()).map_err(io::Error::other)
}

/// Where writes to `path` should go: the final target when `path` is a symlink (a
/// dotfile manager's `mimeapps.list` link, e.g. stow / chezmoi / home-manager, stays a
/// link and its target is updated), else `path` itself.
fn write_target(path: &Path) -> PathBuf {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return path.to_path_buf() };
    if !meta.file_type().is_symlink() {
        return path.to_path_buf();
    }
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    // Dangling: follow the link chain by hand (bounded) to the missing file.
    let mut cur = path.to_path_buf();
    for _ in 0..40 {
        match std::fs::read_link(&cur) {
            Ok(t) => cur = cur.parent().map_or_else(|| t.clone(), |d| d.join(&t)),
            Err(_) => break,
        }
    }
    cur
}

/// [`write`] through a symlink (the link itself is kept).
fn write_through(path: &Path, text: &str) -> io::Result<()> {
    write(&write_target(path), text)
}

/// Whether `soyuz.desktop` still holds what [`register`] wrote.
fn desktop_is_ours(text: &str, state: &State) -> bool {
    match &state.desktop_written {
        Some(w) => text == w,
        // Older backups: our marker and the command we registered.
        None => {
            text.lines().any(|l| l.trim() == "X-Soyuz-Managed=true")
                && crate::launch::linux::desktop::parse_desktop_with(text, None)
                    .and_then(|e| e.exec)
                    .is_some_and(|e| e == state.command)
        }
    }
}

fn read_state(keys: &AssocKeys) -> Option<State> {
    serde_json::from_slice(&std::fs::read(state_path(keys)).ok()?).ok()
}

/// The effective `inode/directory` default in this config folder (desktop-specific
/// lists first, as the resolution does).
fn effective_default(keys: &AssocKeys) -> Option<String> {
    let desktops = crate::launch::linux::desktop::current_desktops();
    crate::launch::linux::mimeapps::list_paths(&[config(keys)], &desktops).iter().find_map(|p| {
        let t = std::fs::read_to_string(p).ok()?;
        default_value(&t, MIME).and_then(|v| first_id(&v).map(str::to_string))
    })
}

pub fn status(keys: &AssocKeys) -> FileManagerStatus {
    let state = read_state(keys);
    let flatpak = flatpak_desktop_id();
    let ours = effective_default(keys).as_deref() == Some(flatpak.as_deref().unwrap_or(DESKTOP_ID));
    let command = match crate::launch::linux::flatpak_id() {
        Some(id) => Some(format!("flatpak run {id} %U")),
        None => std::fs::read_to_string(desktop_path(keys))
            .ok()
            .and_then(|t| crate::launch::linux::desktop::parse_desktop_with(&t, None))
            .and_then(|e| e.exec),
    };
    match (ours, command, state) {
        (true, Some(command), _) => FileManagerStatus::Registered { command },
        (_, _, Some(_)) => FileManagerStatus::Modified,
        _ => FileManagerStatus::NotRegistered,
    }
}

pub fn register(keys: &AssocKeys, exe: &Path) -> io::Result<()> {
    if state_path(keys).exists() {
        restore(keys)?;
    }
    let list = list_path(keys);
    let (text, created_list) = match std::fs::read_to_string(&list) {
        Ok(t) => (t, false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (String::new(), true),
        Err(e) => return Err(e),
    };
    // Inside Flatpak the exported `<app id>.desktop` already runs the sandboxed app; a
    // `soyuz.desktop` pointing at `/app/bin/soyuz` would not work on the host.
    let flatpak = flatpak_desktop_id();
    let desktop_prev = if flatpak.is_none() { std::fs::read_to_string(desktop_path(keys)).ok() } else { None };
    let (new_text, change) = set_default(&text, MIME, flatpak.as_deref().unwrap_or(DESKTOP_ID));
    let desktop_written = flatpak.is_none().then(|| desktop_file(exe));
    let state = State {
        version: 1,
        command: open_command(exe),
        change,
        created_list,
        desktop_prev,
        desktop_id: flatpak,
        desktop_written: desktop_written.clone(),
    };
    // Backup first, then the desktop file, then the association.
    write(&state_path(keys), &serde_json::to_string_pretty(&state).map_err(io::Error::other)?)?;
    let desktop = match &desktop_written {
        Some(text) => write(&desktop_path(keys), text),
        None => Ok(()),
    };
    let res = desktop.and_then(|()| write_through(&list, &new_text));
    if let Err(e) = res {
        let _ = restore(keys);
        return Err(e);
    }
    Ok(())
}

pub fn restore(keys: &AssocKeys) -> io::Result<()> {
    let Some(state) = read_state(keys) else {
        let _ = std::fs::remove_file(state_path(keys));
        return Ok(());
    };
    let list = list_path(keys);
    if let Ok(text) = std::fs::read_to_string(&list) {
        let restored = undo_default(&text, MIME, state.desktop_id.as_deref().unwrap_or(DESKTOP_ID), &state.change);
        let is_link = std::fs::symlink_metadata(&list).is_ok_and(|m| m.file_type().is_symlink());
        if restored.is_empty() && state.created_list && !is_link {
            std::fs::remove_file(&list)?;
        } else if restored != text {
            write_through(&list, &restored)?;
        }
    }
    // Only undo our own desktop file; a later change by someone else is kept.
    let desktop = desktop_path(keys);
    if state.desktop_id.is_none() && std::fs::read_to_string(&desktop).is_ok_and(|t| desktop_is_ours(&t, &state)) {
        match &state.desktop_prev {
            Some(prev) => write(&desktop, prev)?,
            None => match std::fs::remove_file(&desktop) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            },
        }
    }
    std::fs::remove_file(state_path(keys))?;
    let _ = std::fs::remove_dir(data(keys).join("soyuz"));
    Ok(())
}

pub fn downloads() -> Option<PathBuf> {
    crate::xdg_dirs::user_dirs().downloads
}

pub fn authenticode(_path: &Path) -> Signature {
    Signature::Unknown
}

/// Quote one word for a D-Bus `.service` `Exec=` line (shell-style double quotes).
fn dbus_quote(s: &str) -> String {
    let mut q = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            q.push('\\');
        }
        q.push(c);
    }
    q.push('"');
    q
}

pub fn service_file(exe: &Path) -> String {
    service_file_for(exe, crate::launch::linux::flatpak_id().as_deref())
}

/// The service file for `exe`, or inside Flatpak for the app `flatpak_id` (the host's
/// D-Bus daemon starts it with `flatpak run`; `exe` is a sandbox path then).
fn service_file_for(exe: &Path, flatpak_id: Option<&str>) -> String {
    let cmd = match flatpak_id {
        Some(id) => format!("/usr/bin/flatpak run --command=soyuz {}", dbus_quote(id)),
        None => dbus_quote(&exe.to_string_lossy()),
    };
    format!("[D-BUS Service]\nName={SERVICE}\nExec={cmd} --dbus-file-manager\n")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ServiceState {
    prev: Option<String>,
    written: String,
}

fn read_service_state(keys: &AssocKeys) -> Option<ServiceState> {
    serde_json::from_slice(&std::fs::read(service_state_path(keys)).ok()?).ok()
}

pub fn register_service(keys: &AssocKeys, exe: &Path) -> io::Result<()> {
    if service_state_path(keys).exists() {
        restore_service(keys)?;
    }
    let path = service_path(keys);
    let st = ServiceState { prev: std::fs::read_to_string(&path).ok(), written: service_file(exe) };
    write(&service_state_path(keys), &serde_json::to_string_pretty(&st).map_err(io::Error::other)?)?;
    if let Err(e) = write(&path, &st.written) {
        let _ = restore_service(keys);
        return Err(e);
    }
    Ok(())
}

pub fn restore_service(keys: &AssocKeys) -> io::Result<()> {
    let sp = service_state_path(keys);
    let Some(st) = read_service_state(keys) else {
        let _ = std::fs::remove_file(&sp);
        return Ok(());
    };
    let path = service_path(keys);
    // Only undo our own file; a later change by someone else is kept.
    if std::fs::read_to_string(&path).ok().as_deref() == Some(st.written.as_str()) {
        match &st.prev {
            Some(p) => write(&path, p)?,
            None => std::fs::remove_file(&path)?,
        }
    }
    std::fs::remove_file(&sp)?;
    let _ = std::fs::remove_dir(data(keys).join("soyuz"));
    Ok(())
}

pub fn service_registered(keys: &AssocKeys) -> bool {
    read_service_state(keys)
        .is_some_and(|st| std::fs::read_to_string(service_path(keys)).ok().as_deref() == Some(st.written.as_str()))
}

pub fn folder_handler() -> Option<String> {
    crate::launch::linux::default_id_for_mime(MIME)
}

#[cfg(test)]
mod tests {
    use super::super::{
        file_manager_status, file_manager1_service_registered, register_file_manager, register_file_manager1_service,
        restore_explorer, restore_file_manager1_service,
    };
    use super::*;

    fn sandbox() -> (tempfile::TempDir, AssocKeys) {
        let d = tempfile::tempdir().unwrap();
        let keys = AssocKeys::sandbox(&d.path().to_string_lossy());
        (d, keys)
    }

    #[test]
    fn mimeapps_edits_round_trip_byte_for_byte() {
        let cases = [
            "",
            "[Added Associations]\ntext/plain=a.desktop;\n",
            "[Added Associations]\r\ntext/plain=a.desktop;",
            "[Default Applications]\ntext/plain=a.desktop\ninode/directory = org.kde.dolphin.desktop;\n# tail\n",
            "[Default Applications]",
            "[Default Applications]\n\n[Added Associations]\ninode/directory=x.desktop;\n",
        ];
        for text in cases {
            let (new, change) = set_default(text, MIME, DESKTOP_ID);
            assert_eq!(first_id(&default_value(&new, MIME).unwrap()), Some(DESKTOP_ID), "{text:?}");
            assert_eq!(default_value(&new, "text/plain"), default_value(text, "text/plain"), "{text:?}");
            assert_eq!(undo_default(&new, MIME, DESKTOP_ID, &change), text, "{text:?} via {change:?}");
        }
    }

    #[test]
    fn undo_keeps_a_later_choice() {
        let (new, change) = set_default("[Default Applications]\ninode/directory=nautilus.desktop\n", MIME, DESKTOP_ID);
        let changed = new.replace(DESKTOP_ID, "thunar.desktop");
        assert_eq!(undo_default(&changed, MIME, DESKTOP_ID, &change), changed);
    }

    #[test]
    fn register_status_restore_on_a_sandbox() {
        let (_d, keys) = sandbox();
        let cfg = PathBuf::from(&keys.classes);
        std::fs::create_dir_all(&cfg).unwrap();
        let original = "[Default Applications]\ninode/directory=org.gnome.Nautilus.desktop\ntext/plain=ed.desktop\n";
        std::fs::write(cfg.join("mimeapps.list"), original).unwrap();
        assert_eq!(file_manager_status(&keys), FileManagerStatus::NotRegistered);
        let exe = Path::new("/opt/Soyuz Apps/soyuz");
        register_file_manager(&keys, exe).unwrap();
        let cmd = open_command(exe);
        assert_eq!(cmd, r#""/opt/Soyuz Apps/soyuz" %U"#);
        assert_eq!(file_manager_status(&keys), FileManagerStatus::Registered { command: cmd });
        let desk = std::fs::read_to_string(desktop_path(&keys)).unwrap();
        let e = crate::launch::linux::desktop::parse_desktop_with(&desk, None).unwrap();
        assert_eq!(e.mime_types, ["inode/directory"]);
        assert_eq!(e.program().as_deref(), Some("/opt/Soyuz Apps/soyuz"));
        // Re-registering (exe moved) keeps one backup of the original state.
        register_file_manager(&keys, Path::new("/usr/bin/soyuz")).unwrap();
        restore_explorer(&keys).unwrap();
        assert_eq!(std::fs::read_to_string(cfg.join("mimeapps.list")).unwrap(), original);
        assert!(!desktop_path(&keys).exists());
        assert_eq!(file_manager_status(&keys), FileManagerStatus::NotRegistered);
        restore_explorer(&keys).unwrap();
    }

    #[test]
    fn creates_and_removes_a_missing_list() {
        let (_d, keys) = sandbox();
        register_file_manager(&keys, Path::new("/usr/bin/soyuz")).unwrap();
        let list = PathBuf::from(&keys.classes).join("mimeapps.list");
        assert!(list.exists());
        // Someone else takes over folders: reported, and their choice survives the undo.
        let t = std::fs::read_to_string(&list).unwrap().replace(DESKTOP_ID, "thunar.desktop");
        std::fs::write(&list, &t).unwrap();
        assert_eq!(file_manager_status(&keys), FileManagerStatus::Modified);
        restore_explorer(&keys).unwrap();
        assert_eq!(std::fs::read_to_string(&list).unwrap(), t);
        std::fs::remove_file(&list).unwrap();
        register_file_manager(&keys, Path::new("/usr/bin/soyuz")).unwrap();
        restore_explorer(&keys).unwrap();
        assert!(!list.exists(), "a list we created is removed again");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_mimeapps_list_stays_a_link() {
        let (d, keys) = sandbox();
        let cfg = PathBuf::from(&keys.classes);
        std::fs::create_dir_all(&cfg).unwrap();
        let dotfiles = d.path().join("dotfiles");
        std::fs::create_dir_all(&dotfiles).unwrap();
        let real = dotfiles.join("mimeapps.list");
        let original = "[Default Applications]\ninode/directory=org.gnome.Nautilus.desktop\n";
        std::fs::write(&real, original).unwrap();
        let list = cfg.join("mimeapps.list");
        std::os::unix::fs::symlink(&real, &list).unwrap();
        register_file_manager(&keys, Path::new("/usr/bin/soyuz")).unwrap();
        assert!(std::fs::symlink_metadata(&list).unwrap().file_type().is_symlink(), "still a link");
        assert!(std::fs::read_to_string(&real).unwrap().contains(DESKTOP_ID), "target updated");
        restore_explorer(&keys).unwrap();
        assert!(std::fs::symlink_metadata(&list).unwrap().file_type().is_symlink(), "still a link after restore");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), original);
    }

    #[test]
    fn restore_keeps_a_desktop_file_changed_by_someone_else() {
        let (_d, keys) = sandbox();
        register_file_manager(&keys, Path::new("/usr/bin/soyuz")).unwrap();
        let desk = desktop_path(&keys);
        let theirs = "[Desktop Entry]\nType=Application\nName=Mine\nExec=/usr/bin/mine %U\n";
        std::fs::write(&desk, theirs).unwrap();
        restore_explorer(&keys).unwrap();
        assert_eq!(std::fs::read_to_string(&desk).unwrap(), theirs, "not ours any more: kept");
        // Unchanged: removed as before.
        std::fs::remove_file(&desk).unwrap();
        register_file_manager(&keys, Path::new("/usr/bin/soyuz")).unwrap();
        restore_explorer(&keys).unwrap();
        assert!(!desk.exists());
        // A backup from an older version (no `desktop_written`) still matches by marker + command.
        let st = State {
            version: 1,
            command: open_command(Path::new("/usr/bin/soyuz")),
            change: Change::Appended { suffix: String::new() },
            created_list: false,
            desktop_prev: None,
            desktop_id: None,
            desktop_written: None,
        };
        assert!(desktop_is_ours(&desktop_file(Path::new("/usr/bin/soyuz")), &st));
        assert!(!desktop_is_ours(&desktop_file(Path::new("/opt/other")), &st));
        assert!(!desktop_is_ours(theirs, &st));
    }

    #[test]
    fn dbus_service_file_is_opt_in_and_undone() {
        let (_d, keys) = sandbox();
        assert!(!file_manager1_service_registered(&keys));
        let svc = PathBuf::from(&keys.state).join("dbus-1/services/org.freedesktop.FileManager1.service");
        std::fs::create_dir_all(svc.parent().unwrap()).unwrap();
        std::fs::write(
            &svc,
            "[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec=/usr/bin/nautilus --gapplication-service\n",
        )
        .unwrap();
        let before = std::fs::read_to_string(&svc).unwrap();
        register_file_manager1_service(&keys, Path::new("/opt/a b/soyuz")).unwrap();
        assert!(file_manager1_service_registered(&keys));
        assert!(std::fs::read_to_string(&svc).unwrap().contains(r#"Exec="/opt/a b/soyuz" --dbus-file-manager"#));
        restore_file_manager1_service(&keys).unwrap();
        assert_eq!(std::fs::read_to_string(&svc).unwrap(), before);
        assert!(!file_manager1_service_registered(&keys));
    }

    #[test]
    fn flatpak_service_file_runs_the_app() {
        assert_eq!(
            service_file_for(Path::new("/app/bin/soyuz"), Some("io.github.ulaas.Soyuz")),
            "[D-BUS Service]\nName=org.freedesktop.FileManager1\n\
             Exec=/usr/bin/flatpak run --command=soyuz \"io.github.ulaas.Soyuz\" --dbus-file-manager\n"
        );
    }

    #[test]
    fn older_state_files_still_parse() {
        let st: State = serde_json::from_str(
            r#"{"version":1,"command":"soyuz %U","change":{"Inserted":{"header_newline_added":false}},"created_list":false,"desktop_prev":null}"#,
        )
        .unwrap();
        assert_eq!(st.desktop_id, None);
    }
}

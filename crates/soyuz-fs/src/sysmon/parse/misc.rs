//! systemd states, autostart entries, unit files and macOS `KERN_PROCARGS2` blobs.

use crate::sysmon::ServiceState;

/// A unit's `ActiveState` to a service state (`active` with sub state `exited` counts as running).
pub(crate) fn service_state(active: &str) -> ServiceState {
    match active {
        "active" | "reloading" | "refreshing" => ServiceState::Running,
        "inactive" => ServiceState::Stopped,
        "failed" => ServiceState::Failed,
        "activating" => ServiceState::Starting,
        "deactivating" => ServiceState::Stopping,
        _ => ServiceState::Unknown,
    }
}

/// A unit worth listing as a service. Templates (`foo@.service`) cannot run as they stand.
pub(crate) fn is_listable_service(unit: &str) -> bool {
    unit.ends_with(".service") && !unit.ends_with("@.service")
}

/// What a `.desktop` file in an autostart folder says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AutostartEntry {
    pub name: String,
    pub command: String,
    pub enabled: bool,
}

/// `None` when the text has no `[Desktop Entry]` section.
pub(crate) fn parse_autostart(s: &str) -> Option<AutostartEntry> {
    let mut seen = false;
    let mut in_entry = false;
    let mut out = AutostartEntry { enabled: true, ..AutostartEntry::default() };
    for line in s.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            seen |= in_entry;
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "Name" => out.name = v.to_owned(),
            "Exec" => out.command = v.to_owned(),
            "Hidden" if v.eq_ignore_ascii_case("true") => out.enabled = false,
            "X-GNOME-Autostart-enabled" if v.eq_ignore_ascii_case("false") => out.enabled = false,
            _ => {}
        }
    }
    seen.then_some(out)
}

/// `(Description, first ExecStart)` of a systemd unit file; the prefixes that change how the
/// command runs (`-`, `@`, `+`, `!`, `:`) are dropped.
pub(crate) fn parse_unit_file(s: &str) -> (Option<String>, Option<String>) {
    let (mut description, mut exec) = (None, None);
    for line in s.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        match k.trim() {
            "Description" if description.is_none() && !v.is_empty() => description = Some(v.to_owned()),
            "ExecStart" if exec.is_none() && !v.is_empty() => {
                exec = Some(v.trim_start_matches(['-', '@', '+', '!', ':']).to_owned());
            }
            _ => {}
        }
    }
    (description, exec)
}

/// What `sysctl(KERN_PROCARGS2)` returns: `argc` (native i32), the exec path, NUL padding,
/// `argv[..]`, then the environment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct ProcArgs {
    pub exe: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn parse_procargs2(b: &[u8]) -> Option<ProcArgs> {
    let argc = i32::from_ne_bytes(b.get(..4)?.try_into().ok()?);
    let argc = usize::try_from(argc).ok()?;
    let rest = &b[4..];
    let end = rest.iter().position(|c| *c == 0)?;
    let exe = String::from_utf8_lossy(&rest[..end]).into_owned();
    let mut pos = end;
    while rest.get(pos) == Some(&0) {
        pos += 1;
    }
    let mut strings = rest.get(pos..).unwrap_or(&[]).split(|c| *c == 0);
    let mut args = Vec::with_capacity(argc.min(1024));
    for _ in 0..argc {
        match strings.next() {
            Some(a) => args.push(String::from_utf8_lossy(a).into_owned()),
            None => break,
        }
    }
    let mut env = Vec::new();
    for s in strings {
        // Beyond the environment come Apple's key=value "apple" strings after one empty string.
        if s.is_empty() {
            break;
        }
        let s = String::from_utf8_lossy(s);
        if let Some((k, v)) = s.split_once('=').filter(|(k, _)| !k.is_empty()) {
            env.push((k.to_owned(), v.to_owned()));
        }
    }
    env.sort();
    Some(ProcArgs { exe, args, env })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states() {
        assert_eq!(service_state("active"), ServiceState::Running);
        assert_eq!(service_state("reloading"), ServiceState::Running);
        assert_eq!(service_state("inactive"), ServiceState::Stopped);
        assert_eq!(service_state("failed"), ServiceState::Failed);
        assert_eq!(service_state("activating"), ServiceState::Starting);
        assert_eq!(service_state("deactivating"), ServiceState::Stopping);
        assert_eq!(service_state("?"), ServiceState::Unknown);
        assert!(is_listable_service("sshd.service"));
        assert!(!is_listable_service("getty@.service"));
        assert!(!is_listable_service("a.socket"));
    }

    #[test]
    fn autostart() {
        let s = "# c\n[Desktop Entry]\nName=Tool\nName[de]=Werkzeug\nExec=/usr/bin/tool --x\nHidden=false\n\n[Desktop Action a]\nExec=other\nHidden=true\n";
        let e = parse_autostart(s).unwrap();
        assert_eq!(e.name, "Tool");
        assert_eq!(e.command, "/usr/bin/tool --x");
        assert!(e.enabled);
        assert!(!parse_autostart("[Desktop Entry]\nHidden=true\n").unwrap().enabled);
        assert!(!parse_autostart("[Desktop Entry]\nX-GNOME-Autostart-enabled=false\n").unwrap().enabled);
        assert_eq!(parse_autostart("Name=x\n"), None);
        assert_eq!(parse_autostart(""), None);
    }

    #[test]
    fn unit_files() {
        let s =
            "[Unit]\nDescription=My thing\n[Service]\nExecStartPre=/bin/a\nExecStart=-/usr/bin/x --y\nExecStart=/z\n";
        let (d, e) = parse_unit_file(s);
        assert_eq!(d.as_deref(), Some("My thing"));
        assert_eq!(e.as_deref(), Some("/usr/bin/x --y"));
        assert_eq!(parse_unit_file("junk\n=\nExecStart=\n"), (None, None));
    }

    fn blob(argc: i32, tail: &[u8]) -> Vec<u8> {
        let mut v = argc.to_ne_bytes().to_vec();
        v.extend_from_slice(tail);
        v
    }

    #[test]
    fn procargs() {
        let b = blob(2, b"/bin/my app\0\0\0\0/bin/my app\0--flag\0B=2\0A=1\0\0ptr_munge=abc\0");
        let p = parse_procargs2(&b).unwrap();
        assert_eq!(p.exe, "/bin/my app");
        assert_eq!(p.args, ["/bin/my app", "--flag"]);
        assert_eq!(p.env, [("A".to_owned(), "1".to_owned()), ("B".to_owned(), "2".to_owned())]);
    }

    #[test]
    fn procargs_malformed() {
        assert_eq!(parse_procargs2(b""), None);
        assert_eq!(parse_procargs2(b"\x01\x00"), None);
        assert_eq!(parse_procargs2(&blob(-1, b"/x\0a\0")), None);
        assert_eq!(parse_procargs2(&blob(1, b"/no-terminator")), None);
        // A huge argc with little data neither panics nor allocates much.
        let p = parse_procargs2(&blob(i32::MAX, b"/x\0a\0")).unwrap();
        assert_eq!(p.exe, "/x");
        let p = parse_procargs2(&blob(0, b"/x\0")).unwrap();
        assert!(p.args.is_empty());
    }
}

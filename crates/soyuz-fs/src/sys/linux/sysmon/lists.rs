//! On-demand lists: sockets, services (systemd over D-Bus), startup entries, process details.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use zbus::blocking::Connection as Bus;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use super::control::verify;
use crate::launch::linux::bus;
use crate::sysmon::parse::misc as pmisc;
use crate::sysmon::parse::net::{RawSocket, parse_sockets};
use crate::sysmon::parse::pid as ppid;
use crate::sysmon::{
    Connection, ControlError, DetailsWant, OpenFile, ProcDetails, ProcKey, Proto, ServiceInfo, ServiceOp, ServiceState,
    StartupItem, ThreadInfo,
};

pub fn connections() -> Vec<Connection> {
    let mut sockets: Vec<RawSocket> = Vec::new();
    for (file, proto) in [("tcp", Proto::Tcp), ("tcp6", Proto::Tcp6), ("udp", Proto::Udp), ("udp6", Proto::Udp6)] {
        if let Some(text) = super::read_text(&format!("/proc/net/{file}")) {
            sockets.extend(parse_sockets(&text, proto));
        }
    }
    let wanted: HashSet<u64> = sockets.iter().map(|s| s.inode).filter(|i| *i != 0).collect();
    let owners = socket_owners(&wanted);
    sockets
        .into_iter()
        .map(|s| {
            let mut c = s.conn;
            c.pid = owners.get(&s.inode).copied();
            c
        })
        .collect()
}

/// Socket inode to owning PID, over the processes whose `fd` directory can be read.
fn socket_owners(wanted: &HashSet<u64>) -> HashMap<u64, u32> {
    let mut owners = HashMap::with_capacity(wanted.len());
    let Ok(dir) = fs::read_dir("/proc") else { return owners };
    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else { continue };
        for fd in fds.flatten() {
            let Ok(link) = fs::read_link(fd.path()) else { continue };
            if let Some(inode) = link.to_str().and_then(ppid::socket_inode)
                && wanted.contains(&inode)
            {
                owners.entry(inode).or_insert(pid);
            }
        }
    }
    owners
}

const SYSTEMD: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";

type UnitRow = (String, String, String, String, String, String, OwnedObjectPath, u32, String, OwnedObjectPath);

fn list_units(c: &Bus) -> Option<Vec<UnitRow>> {
    let m = c.call_method(Some(SYSTEMD), SYSTEMD_PATH, Some(MANAGER), "ListUnits", &()).ok()?;
    m.body().deserialize().ok()
}

fn unit_main_pid(c: &Bus, path: &OwnedObjectPath) -> Option<u32> {
    let m = c
        .call_method(
            Some(SYSTEMD),
            path.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.systemd1.Service", "MainPID"),
        )
        .ok()?;
    let v: OwnedValue = m.body().deserialize().ok()?;
    u32::try_from(v).ok().filter(|p| *p != 0)
}

/// Installed units with their start modes, running ones joined from the unit list. No system
/// bus or no systemd gives an empty list.
pub fn services() -> Vec<ServiceInfo> {
    let Some(c) = bus::system() else { return Vec::new() };
    let Some(units) = list_units(&c) else { return Vec::new() };
    let mut files: HashMap<String, (String, String)> = HashMap::new();
    if let Ok(m) = c.call_method(Some(SYSTEMD), SYSTEMD_PATH, Some(MANAGER), "ListUnitFiles", &())
        && let Ok(rows) = m.body().deserialize::<Vec<(String, String)>>()
    {
        for (path, state) in rows {
            if let Some(name) = Path::new(&path).file_name().and_then(|n| n.to_str())
                && pmisc::is_listable_service(name)
            {
                files.insert(name.to_owned(), (state, path.clone()));
            }
        }
    }
    let mut out = Vec::new();
    for u in &units {
        if !pmisc::is_listable_service(&u.0) || u.2 == "not-found" {
            continue;
        }
        let state = pmisc::service_state(&u.3);
        out.push(ServiceInfo {
            name: u.0.clone(),
            display_name: u.0.trim_end_matches(".service").to_owned(),
            description: u.1.clone(),
            state,
            start_mode: files.remove(&u.0).map(|f| f.0).unwrap_or_default(),
            pid: if state == ServiceState::Running { unit_main_pid(&c, &u.6) } else { None },
        });
    }
    for (name, (start_mode, path)) in files {
        // Units that are not loaded have no description on the bus: their file has it.
        let description = super::read_text(&path).and_then(|t| pmisc::parse_unit_file(&t).0).unwrap_or_default();
        out.push(ServiceInfo {
            display_name: name.trim_end_matches(".service").to_owned(),
            name,
            description,
            start_mode,
            state: ServiceState::Stopped,
            ..ServiceInfo::default()
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Without authorization (a normal user and no polkit rule) this answers `Denied`; no prompt.
pub fn service_control(name: &str, op: ServiceOp) -> Result<(), ControlError> {
    let c = bus::system().ok_or(ControlError::Unsupported)?;
    let method = match op {
        ServiceOp::Start => "StartUnit",
        ServiceOp::Stop => "StopUnit",
        ServiceOp::Restart => "RestartUnit",
    };
    c.call_method(Some(SYSTEMD), SYSTEMD_PATH, Some(MANAGER), method, &(name, "replace")).map(|_| ()).map_err(|e| {
        let text = e.to_string();
        if text.contains("AccessDenied") || text.contains("InteractiveAuthorizationRequired") {
            ControlError::Denied
        } else if text.contains("ServiceUnknown") {
            ControlError::Unsupported
        } else {
            ControlError::Other(text)
        }
    })
}

pub fn startup_items() -> Vec<StartupItem> {
    let home = crate::path::home();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| home.join(".config"), PathBuf::from);
    let mut out = Vec::new();

    // A file in the user's folder replaces the system file of the same name.
    let mut seen = HashSet::new();
    for (dir, source) in
        [(config.join("autostart"), "~/.config/autostart"), ("/etc/xdg/autostart".into(), "/etc/xdg/autostart")]
    {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "desktop") || !seen.insert(e.file_name()) {
                continue;
            }
            let Some(entry) = fs::read(&path).ok().and_then(|b| pmisc::parse_autostart(&String::from_utf8_lossy(&b)))
            else {
                continue;
            };
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            out.push(StartupItem {
                name: if entry.name.is_empty() { stem } else { entry.name },
                command: entry.command,
                source: source.to_owned(),
                enabled: entry.enabled,
            });
        }
    }

    // User units enabled for login: links in `*.target.wants`.
    let mut units = HashSet::new();
    for (dir, source) in
        [(config.join("systemd/user"), "~/.config/systemd/user"), ("/etc/systemd/user".into(), "/etc/systemd/user")]
    {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for wants in rd.flatten() {
            let wants_name = wants.file_name();
            let target = wants_name.to_string_lossy();
            if !target.ends_with(".target.wants") {
                continue;
            }
            let Ok(links) = fs::read_dir(wants.path()) else { continue };
            for link in links.flatten() {
                let unit = link.file_name().to_string_lossy().into_owned();
                if !unit.ends_with(".service") || !units.insert(unit.clone()) {
                    continue;
                }
                let (description, exec) = fs::read(link.path())
                    .map(|b| pmisc::parse_unit_file(&String::from_utf8_lossy(&b)))
                    .unwrap_or_default();
                out.push(StartupItem {
                    name: description.unwrap_or_else(|| unit.clone()),
                    command: exec.unwrap_or(unit),
                    source: source.to_owned(),
                    enabled: true,
                });
            }
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// The executable, working folder and command line always; the lists only when `want` asks.
pub fn process_details(key: ProcKey, want: DetailsWant) -> Result<ProcDetails, ControlError> {
    let pid = verify(key)?;
    let dir = PathBuf::from(format!("/proc/{pid}"));
    let mut limited = false;
    let mut note = |e: &std::io::Error| limited |= e.kind() == ErrorKind::PermissionDenied;

    let mut read_link = |leaf: &str| match fs::read_link(dir.join(leaf)) {
        Ok(l) => Some(l),
        Err(e) => {
            note(&e);
            None
        }
    };
    let exe = read_link("exe").map(|l| PathBuf::from(ppid::clean_exe_link(&l.to_string_lossy())));
    let cwd = read_link("cwd");

    let cmdline = fs::read(dir.join("cmdline")).ok().and_then(|b| ppid::parse_cmdline(&b));

    let mut threads = Vec::new();
    if want.threads {
        match fs::read_dir(dir.join("task")) {
            Ok(rd) => {
                for t in rd.flatten() {
                    let Some(text) = fs::read(t.path().join("stat")).ok() else { continue };
                    let text = String::from_utf8_lossy(&text);
                    let Some(st) = ppid::parse_stat(&text) else { continue };
                    threads.push(ThreadInfo {
                        tid: st.pid,
                        name: st.comm.to_owned(),
                        state: ppid::state_from_char(st.state),
                        cpu_ms: st.utime.saturating_add(st.stime).saturating_mul(1000) / super::clk_tck(),
                        priority: Some(st.nice),
                    });
                }
                threads.sort_by_key(|t| t.tid);
            }
            Err(e) => limited |= e.kind() == ErrorKind::PermissionDenied,
        }
    }

    let mut modules = Vec::new();
    if want.modules {
        match fs::read(dir.join("maps")) {
            Ok(b) => modules = ppid::parse_maps(&String::from_utf8_lossy(&b)),
            Err(e) => limited |= e.kind() == ErrorKind::PermissionDenied,
        }
    }

    let mut open_files = Vec::new();
    if want.open_files {
        match fs::read_dir(dir.join("fd")) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let Ok(link) = fs::read_link(e.path()) else { continue };
                    open_files.push(OpenFile {
                        fd: e.file_name().to_str().and_then(|n| n.parse().ok()),
                        path: link.to_string_lossy().into_owned(),
                    });
                }
                open_files.sort_by_key(|f| f.fd);
            }
            Err(e) => limited |= e.kind() == ErrorKind::PermissionDenied,
        }
    }

    let mut environment = Vec::new();
    if want.environment {
        match fs::read(dir.join("environ")) {
            Ok(b) => environment = ppid::parse_environ(&b),
            Err(e) => limited |= e.kind() == ErrorKind::PermissionDenied,
        }
    }

    Ok(ProcDetails { exe, cwd, cmdline, threads, modules, open_files, environment, limited })
}

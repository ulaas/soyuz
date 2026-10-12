//! Devices of the sidebar (frontend-agnostic; Linux desktop): GTK bookmarks and UDisks2
//! devices (mount, unlock, unmount, lock, eject) plus `Cmd::Device`. The GUI rows and
//! menus are in `ui::sidebar`.

use std::path::{Path, PathBuf};

use soyuz_core::{Location, SizeStyle, format_size};
use soyuz_fs::VolumeInfo;

use crate::cmd::Cmd;
use crate::host::Host;
use crate::models::frontend::FrontendCmd;
use crate::models::toasts::Toast;
use crate::state::pane::PaneId;

// ---- Linux desktop: GTK bookmarks and UDisks2 devices ------------------------------------
//
// One background thread ("soyuz-desktop-places", Linux only) re-reads the GTK bookmarks
// file and the UDisks2 block devices every `DESKTOP_REFRESH` and whenever the volume list
// changes. Mount / unmount / eject run on their own threads (polkit may prompt); their
// results come back through `Desk::events` and are turned into toasts / navigation on the
// next frame. Nothing here touches D-Bus or the disk on the UI thread.

/// Background refresh of the GTK bookmarks and UDisks2 devices.
const DESKTOP_REFRESH: std::time::Duration = std::time::Duration::from_secs(10);

/// `[sidebar] gtk_bookmarks`: show the GTK bookmarks (`~/.config/gtk-3.0/bookmarks`)
/// under Favorites (Linux; default on).
static SHOW_GTK_BOOKMARKS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Show / hide the GTK bookmarks in the Favorites section.
pub fn set_show_gtk_bookmarks(on: bool) {
    SHOW_GTK_BOOKMARKS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// GTK bookmarks to show under Favorites (empty when off or not on Linux).
pub fn gtk_bookmarks(cfg: &soyuz_core::config::Sidebar) -> Vec<soyuz_fs::xdg_dirs::Bookmark> {
    if !cfg!(target_os = "linux")
        || !cfg.gtk_bookmarks
        || !SHOW_GTK_BOOKMARKS.load(std::sync::atomic::Ordering::Relaxed)
    {
        return Vec::new();
    }
    desk().lock().gtk.clone()
}

/// Every block device UDisks2 knows (empty off Linux or before the first scan).
pub fn block_devices() -> Vec<soyuz_fs::udisks::BlockDevice> {
    if cfg!(target_os = "linux") { desk().lock().devices.clone() } else { Vec::new() }
}

/// UDisks object paths with an operation running, and what it is.
pub fn busy() -> std::collections::BTreeMap<String, Work> {
    desk().lock().busy.clone()
}

/// Result of a device operation, applied on the UI thread.
enum DeskEvent {
    Mounted {
        pane: PaneId,
        path: PathBuf,
    },
    Unmounted {
        name: String,
    },
    Ejected {
        name: String,
    },
    Locked {
        name: String,
    },
    Failed {
        title: String,
        body: String,
    },
    /// NTFS left hibernated / dirty by Windows: offer a read-only mount.
    OfferReadOnly {
        dev: Box<soyuz_fs::udisks::BlockDevice>,
        name: String,
    },
}

/// What a busy device is doing (the row shows it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Mounting,
    Unlocking,
    Unmounting,
    Locking,
}

#[derive(Default)]
struct Desk {
    started: bool,
    /// UDisks2 answered (devices are listed); `false` hides the device extras.
    available: bool,
    /// Every block device UDisks2 knows (mounted or not).
    devices: Vec<soyuz_fs::udisks::BlockDevice>,
    /// Bumped whenever `devices` changes (the sidebar re-classifies then).
    generation: u64,
    gtk: Vec<soyuz_fs::xdg_dirs::Bookmark>,
    /// UDisks object paths with an operation running.
    busy: std::collections::BTreeMap<String, Work>,
    events: Vec<DeskEvent>,
    trigger: Option<crossbeam_channel::Sender<()>>,
    /// Signature of the volume list last seen (a change triggers a device rescan).
    vol_sig: u64,
}

fn desk() -> &'static parking_lot::Mutex<Desk> {
    static D: std::sync::OnceLock<parking_lot::Mutex<Desk>> = std::sync::OnceLock::new();
    D.get_or_init(Default::default)
}

/// Start the refresher once (Linux only; the sidebar calls it while it shows).
pub fn start(host: &dyn Host) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let mut d = desk().lock();
    if d.started {
        return;
    }
    d.started = true;
    let (tx, rx) = crossbeam_channel::bounded::<()>(1);
    d.trigger = Some(tx);
    drop(d);
    let wake = host.wake();
    let spawned = std::thread::Builder::new().name("soyuz-desktop-places".into()).spawn(move || {
        let udisks = soyuz_fs::udisks::available();
        loop {
            let gtk = soyuz_fs::xdg_dirs::gtk_bookmarks();
            let devices = if udisks {
                match soyuz_fs::udisks::block_devices() {
                    Ok(v) => Some(v),
                    Err(e) => {
                        log::debug!("udisks: {e}");
                        None
                    }
                }
            } else {
                None
            };
            let changed = {
                let mut d = desk().lock();
                let mut changed = d.gtk != gtk;
                d.gtk = gtk;
                if let Some(devices) = devices {
                    if d.devices != devices || !d.available {
                        changed = true;
                        d.generation += 1;
                    }
                    d.devices = devices;
                    d.available = true;
                }
                changed
            };
            if changed {
                wake();
            }
            match rx.recv_timeout(DESKTOP_REFRESH) {
                Ok(()) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("cannot spawn the desktop places thread: {e}");
    }
}

/// Ask the refresher for a rescan now (after a device operation, or new volumes).
fn desk_rescan() {
    if let Some(t) = &desk().lock().trigger {
        let _ = t.try_send(());
    }
}

/// The volume list should be re-read now (a device was mounted / unmounted from the
/// sidebar); taken by the frame loop, which asks the places service for a refresh.
static VOLUMES_STALE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A device was mounted / unmounted from the sidebar since the last call.
pub fn take_volumes_stale() -> bool {
    VOLUMES_STALE.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Per frame (from the frame loop, sidebar shown or not): rescan when the volume list
/// changed, turn finished device operations into toasts / navigation.
pub fn pump(app: &mut crate::core::AppCore) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let ws = &app.ws;
    let focused = ws.pane().id;
    desk_pump(&app.ui.volumes, &mut app.cmds, |p| ws.find_pane(p).is_some(), focused);
}

/// The body of [`pump`]: `exists` says whether a pane id is still open (a mount that
/// finishes after its pane was closed opens in `focused` instead).
fn desk_pump(volumes: &[VolumeInfo], cmds: &mut Vec<Cmd>, exists: impl Fn(PaneId) -> bool, focused: PaneId) {
    let sig = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for v in volumes {
            v.mount.hash(&mut h);
        }
        h.finish()
    };
    let events = {
        let mut d = desk().lock();
        if !d.started && d.events.is_empty() {
            return;
        }
        let rescan = d.started && d.vol_sig != sig;
        d.vol_sig = sig;
        let ev = std::mem::take(&mut d.events);
        drop(d);
        if rescan {
            desk_rescan();
        }
        ev
    };
    for ev in events {
        VOLUMES_STALE.store(true, std::sync::atomic::Ordering::Relaxed);
        match ev {
            DeskEvent::Mounted { pane, path } => {
                let pane = if exists(pane) { pane } else { focused };
                cmds.push(Cmd::FocusPane(pane));
                cmds.push(Cmd::Navigate { pane, location: Location::Dir(path), select: None });
            }
            DeskEvent::Unmounted { name } => {
                cmds.push(Cmd::Toast(Toast::info(tr!("app-device-unmounted", name = name))))
            }
            DeskEvent::Ejected { name } => cmds.push(Cmd::Toast(Toast::info(tr!("app-device-ejected", name = name)))),
            DeskEvent::Locked { name } => {
                cmds.push(Cmd::Toast(Toast::info(tr!("app-device-locked-done", name = name))))
            }
            DeskEvent::Failed { title, body } => cmds.push(Cmd::Toast(Toast::error(title).body(body))),
            DeskEvent::OfferReadOnly { dev, name } => cmds.push(Cmd::Device(DeviceCmd::OfferReadOnly { dev, name })),
        }
    }
}

/// A short, human reason for a failed UDisks2 call (`fs_type`: the device's filesystem).
fn udisks_reason(e: &std::io::Error, fs_type: &str) -> String {
    use soyuz_fs::udisks::Failure;
    match soyuz_fs::udisks::classify_error(e, fs_type) {
        Failure::NotAuthorized => tr!("app-device-not-authorized").to_string(),
        Failure::Dismissed => tr!("app-device-auth-dismissed").to_string(),
        Failure::Busy => tr!("app-device-busy").to_string(),
        Failure::WindowsHibernated => tr!("app-device-hibernated").to_string(),
        Failure::UnsupportedFs => tr!("app-device-unsupported-fs", fs = soyuz_fs::udisks::fs_display_name(fs_type)),
        Failure::WrongPassphrase => tr!("app-device-wrong-passphrase").to_string(),
        Failure::NoUdisks => tr!("app-device-no-udisks").to_string(),
        Failure::Other => e.to_string().trim_start_matches("UDisks2: ").to_string(),
    }
}

/// Run a device operation on its own thread; `object` is marked busy meanwhile.
fn desk_op(object: &str, work: Work, op: impl FnOnce() -> DeskEvent + Send + 'static) {
    let object = object.to_string();
    {
        let mut d = desk().lock();
        if d.busy.contains_key(&object) {
            return;
        }
        d.busy.insert(object.clone(), work);
    }
    let key = object.clone();
    let spawned = std::thread::Builder::new().name("soyuz-udisks-op".into()).spawn(move || {
        let ev = op();
        let mut d = desk().lock();
        d.busy.remove(&key);
        d.events.push(ev);
        drop(d);
        desk_rescan();
    });
    if let Err(e) = spawned {
        desk().lock().busy.remove(&object);
        log::warn!("cannot spawn a device thread: {e}");
    }
}

/// Mount `dev` (polkit may ask; the OS picks the driver), then open it in `pane`.
/// `read_only` mounts with `ro` (NTFS that Windows left hibernated).
pub fn mount_device(dev: &soyuz_fs::udisks::BlockDevice, name: String, pane: PaneId, read_only: bool) {
    let d = dev.clone();
    desk_op(&dev.object, Work::Mounting, move || mount_now(&d, name, pane, read_only, false));
}

/// The blocking part of a mount (on a device thread). `fresh`: the device was just
/// unlocked, so UDisks may need a moment to probe its filesystem.
fn mount_now(d: &soyuz_fs::udisks::BlockDevice, name: String, pane: PaneId, read_only: bool, fresh: bool) -> DeskEvent {
    let o = soyuz_fs::udisks::MountOptions {
        interactive: true,
        options: if read_only { "ro".into() } else { String::new() },
    };
    let mut tries = if fresh { 10 } else { 1 };
    loop {
        tries -= 1;
        match soyuz_fs::udisks::mount(&d.object, &o) {
            Ok(path) => return DeskEvent::Mounted { pane, path },
            Err(e) => {
                let text = e.to_string();
                // The cleartext device's Filesystem interface appears after the probe.
                if tries > 0 && (text.contains("Unknown") || text.contains("interface")) {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    continue;
                }
                if !read_only
                    && soyuz_fs::udisks::classify_error(&e, &d.fs_type) == soyuz_fs::udisks::Failure::WindowsHibernated
                {
                    return DeskEvent::OfferReadOnly { dev: Box::new(d.clone()), name };
                }
                return DeskEvent::Failed {
                    title: tr!("app-device-mount-failed", name = name),
                    body: udisks_reason(&e, &d.fs_type),
                };
            }
        }
    }
}

pub fn unmount_device(dev: &soyuz_fs::udisks::BlockDevice, name: String) {
    let object = dev.object.clone();
    let fs = dev.fs_type.clone();
    desk_op(&dev.object, Work::Unmounting, move || match soyuz_fs::udisks::unmount(&object, false, true) {
        Ok(()) => DeskEvent::Unmounted { name },
        Err(e) => {
            DeskEvent::Failed { title: tr!("app-device-unmount-failed", name = name), body: udisks_reason(&e, &fs) }
        }
    });
}

/// Unmount the cleartext filesystem (when mounted), then lock its container.
pub fn lock_device(cleartext: &soyuz_fs::udisks::BlockDevice, container: String, name: String) {
    let object = cleartext.object.clone();
    let mounted = !cleartext.mount_points.is_empty();
    let fs = cleartext.fs_type.clone();
    desk_op(&container.clone(), Work::Locking, move || {
        let failed = |e: std::io::Error| DeskEvent::Failed {
            title: tr!("app-device-lock-failed", name = name.clone()),
            body: udisks_reason(&e, &fs),
        };
        if mounted && let Err(e) = soyuz_fs::udisks::unmount(&object, false, true) {
            return failed(e);
        }
        match soyuz_fs::udisks::lock(&container, true) {
            Ok(()) => DeskEvent::Locked { name: name.clone() },
            Err(e) => failed(e),
        }
    });
}

/// Unlock a BitLocker / LUKS container with `secret` (zeroized afterwards), then mount
/// its filesystem and open it in `pane`. `reply` gets `Ok(())` once unlocked, or the
/// reason it failed (the unlock dialog shows it and stays open).
pub fn unlock_device(
    object: &str,
    name: String,
    mut secret: zeroize::Zeroizing<String>,
    pane: PaneId,
    reply: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let object = object.to_string();
    {
        let mut d = desk().lock();
        if d.busy.contains_key(&object) {
            return;
        }
        d.busy.insert(object.clone(), Work::Unlocking);
    }
    let key = object.clone();
    let target = object.clone();
    let spawned = std::thread::Builder::new().name("soyuz-udisks-unlock".into()).spawn(move || {
        let r = soyuz_fs::udisks::unlock(&target, secret.as_str(), true);
        zeroize::Zeroize::zeroize(&mut *secret);
        drop(secret);
        desk().lock().busy.remove(&key);
        match r {
            Ok(cleartext) => {
                let _ = reply.send(Ok(()));
                let dev = soyuz_fs::udisks::BlockDevice { object: cleartext.clone(), ..Default::default() };
                desk_op(&cleartext, Work::Mounting, move || mount_now(&dev, name, pane, false, true));
            }
            Err(e) => {
                let _ = reply.send(Err(udisks_reason(&e, "")));
                desk_rescan();
            }
        }
    });
    if let Err(e) = spawned {
        desk().lock().busy.remove(&object);
        log::warn!("cannot spawn a device thread: {e}");
    }
}

/// The drive a device lives on: its own, or (an unlocked container's cleartext device,
/// which UDisks gives no drive) the drive of its encrypted container.
fn drive_of<'a>(d: &'a soyuz_fs::udisks::BlockDevice, all: &'a [soyuz_fs::udisks::BlockDevice]) -> Option<&'a str> {
    let mut cur = d;
    // Follow the crypto backing chain (bounded: LUKS in LUKS is the deepest real case).
    for _ in 0..4 {
        if let Some(drive) = cur.drive.as_deref() {
            return Some(drive);
        }
        let backing = cur.crypto_backing.as_deref()?;
        cur = all.iter().find(|b| b.object == backing)?;
    }
    None
}

/// What "Eject" does for a drive: unmount `unmount` (deepest mount first), lock the
/// unlocked containers `lock`, then eject the media (`eject`) or power the drive off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EjectPlan {
    pub drive: String,
    pub unmount: Vec<String>,
    pub lock: Vec<String>,
    pub eject: bool,
}

/// The [`EjectPlan`] for the drive of `dev`, or `None` when ejecting it must not be
/// offered: the drive can neither eject nor power off, it is a system drive that is not
/// removable media, or any file system on it is mounted where the running system lives
/// (`/`, `/boot/efi`, `/home` … — a USB-booted system or a hot-plug disk holding a
/// system mount would otherwise be unmounted and powered off).
pub fn eject_plan(dev: &soyuz_fs::udisks::BlockDevice, all: &[soyuz_fs::udisks::BlockDevice]) -> Option<EjectPlan> {
    let drive = drive_of(dev, all)?.to_string();
    let on_drive: Vec<&soyuz_fs::udisks::BlockDevice> =
        all.iter().filter(|b| drive_of(b, all) == Some(drive.as_str())).collect();
    // The drive's own properties live on the devices UDisks links to it directly.
    let direct: Vec<&soyuz_fs::udisks::BlockDevice> =
        on_drive.iter().copied().filter(|b| b.drive.as_deref() == Some(drive.as_str())).collect();
    let holder = direct.first().copied().unwrap_or(dev);
    let (ejectable, power_off) = (holder.ejectable, holder.can_power_off);
    if !ejectable && !power_off {
        return None;
    }
    if direct.iter().any(|b| b.hint_system && !b.is_removable_media()) {
        return None;
    }
    if on_drive.iter().any(|b| b.mount_points.iter().any(|m| is_system_mount(m))) {
        return None;
    }
    let mut mounted: Vec<(usize, String)> = on_drive
        .iter()
        .filter(|b| !b.mount_points.is_empty())
        .map(|b| {
            let depth = b.mount_points.iter().map(|m| m.components().count()).max().unwrap_or(0);
            (depth, b.object.clone())
        })
        .collect();
    // Nested mounts first (a stick mounted inside another file system of the drive).
    mounted.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    // Containers on the drive with an unlocked cleartext device, innermost first.
    let mut lock: Vec<(usize, String)> = on_drive
        .iter()
        .filter(|b| b.encrypted && b.cleartext.is_some())
        .map(|b| {
            let mut depth = 0;
            let mut cur = *b;
            while let Some(up) = cur.crypto_backing.as_deref().and_then(|o| all.iter().find(|x| x.object == o)) {
                depth += 1;
                cur = up;
                if depth > 4 {
                    break;
                }
            }
            (depth, b.object.clone())
        })
        .collect();
    lock.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Some(EjectPlan {
        drive,
        unmount: mounted.into_iter().map(|(_, o)| o).collect(),
        lock: lock.into_iter().map(|(_, o)| o).collect(),
        eject: ejectable,
    })
}

/// Unmount every mounted file system of the drive, lock its unlocked containers, then
/// eject it (or power it off when it can't eject, e.g. a USB disk). Refused (see
/// [`eject_plan`]) for drives holding the running system.
pub fn eject_device(dev: &soyuz_fs::udisks::BlockDevice, all: &[soyuz_fs::udisks::BlockDevice]) {
    let Some(plan) = eject_plan(dev, all) else { return };
    let name = dev.display_name();
    let fs = dev.fs_type.clone();
    desk_op(&dev.object, Work::Unmounting, move || {
        let failed = |e: std::io::Error| DeskEvent::Failed {
            title: tr!("app-device-eject-failed", name = name.clone()),
            body: udisks_reason(&e, &fs),
        };
        for o in &plan.unmount {
            if let Err(e) = soyuz_fs::udisks::unmount(o, false, true) {
                return failed(e);
            }
        }
        for o in &plan.lock {
            if let Err(e) = soyuz_fs::udisks::lock(o, true) {
                return failed(e);
            }
        }
        let r = if plan.eject {
            soyuz_fs::udisks::eject(&plan.drive, true)
        } else {
            soyuz_fs::udisks::power_off(&plan.drive, true)
        };
        match r {
            Ok(()) => DeskEvent::Ejected { name: name.clone() },
            Err(e) => failed(e),
        }
    });
}

/// Mount points of the running system (`/`, `/boot`, `/home`, `/var` …): never offered
/// for unmounting from the sidebar. Mounts below the home folder or under `/mnt`,
/// `/media`, `/run/media` are the user's.
fn is_system_mount(m: &Path) -> bool {
    const EXACT: [&str; 4] = ["/", "/home", "/mnt", "/media"];
    const TREES: [&str; 13] =
        ["/boot", "/efi", "/usr", "/var", "/opt", "/srv", "/nix", "/etc", "/root", "/sysroot", "/gnu", "/tmp", "/snap"];
    EXACT.iter().any(|e| m == Path::new(e)) || TREES.iter().any(|t| m.starts_with(t))
}

/// The UDisks2 device mounted at `mount`, if it may be unmounted from the sidebar
/// (anything but a mount of the running system).
pub fn device_at(devices: &[soyuz_fs::udisks::BlockDevice], mount: &Path) -> Option<soyuz_fs::udisks::BlockDevice> {
    if is_system_mount(mount) {
        return None;
    }
    devices
        .iter()
        .find(|b| b.mount_points.iter().any(|m| m == mount) && b.mount_points.iter().all(|m| !is_system_mount(m)))
        .cloned()
}

/// Sidebar effects that need the app (dialogs, config): `Cmd::Device`.
#[derive(Clone, Debug)]
pub enum DeviceCmd {
    /// Ask for the passphrase / recovery key of a locked container.
    Unlock { object: String, name: String, kind: String },
    /// A mount failed because Windows left the NTFS volume hibernated: explain and
    /// offer a read-only mount.
    OfferReadOnly { dev: Box<soyuz_fs::udisks::BlockDevice>, name: String },
    /// Mount (read-only or not) into the focused pane.
    Mount { dev: Box<soyuz_fs::udisks::BlockDevice>, name: String, read_only: bool },
    /// "Hide from sidebar": add the key to `[sidebar] hidden_devices`.
    Hide(String),
}

/// Apply a [`DeviceCmd`].
pub fn apply(app: &mut crate::core::AppCore, host: &dyn Host, cmd: DeviceCmd) {
    let pane = app.ws.pane().id;
    match cmd {
        DeviceCmd::Unlock { object, name, kind } => {
            app.frontend.push(FrontendCmd::UnlockDevice { object, name, kind, pane })
        }
        DeviceCmd::OfferReadOnly { dev, name } => app.frontend.push(FrontendCmd::OfferReadOnlyMount { dev, name }),
        DeviceCmd::Mount { dev, name, read_only } => mount_device(&dev, name, pane, read_only),
        DeviceCmd::Hide(key) => {
            if !app.config.sidebar.hidden_devices.contains(&key) {
                crate::models::settings::update_config(app, host, |c| c.sidebar.hidden_devices.push(key));
                app.ui.toasts.push(Toast::info(tr!("app-device-hidden")).body(tr!("app-device-hidden-body")));
            }
        }
    }
}

/// Name of an unmounted device: its label, else "<size> <filesystem> volume".
pub fn candidate_name(c: &soyuz_fs::udisks::Candidate) -> String {
    let d = &c.dev;
    for s in [&d.label, &d.hint_name] {
        if !s.trim().is_empty() {
            return s.trim().to_string();
        }
    }
    let fs = soyuz_fs::udisks::fs_display_name(&d.fs_type);
    if d.size > 0 {
        tr!("app-device-volume-name", size = format_size(d.size, SizeStyle::Decimal), fs = fs)
    } else {
        tr!("app-device-volume-name-nosize", fs = fs)
    }
}

/// Badge text: the filesystem, or the container type for a locked device.
pub fn candidate_badge(c: &soyuz_fs::udisks::Candidate) -> String {
    soyuz_fs::udisks::fs_display_name(&c.dev.fs_type)
}

pub fn candidate_tip(c: &soyuz_fs::udisks::Candidate, name: &str) -> String {
    use soyuz_fs::udisks::CandidateState as S;
    let d = &c.dev;
    let mut fs = soyuz_fs::udisks::fs_display_name(&d.fs_type);
    if !c.container_type.is_empty() {
        fs = format!("{fs} · {}", soyuz_fs::udisks::fs_display_name(&c.container_type));
    }
    let (vendor, model) = (d.drive_vendor.trim(), d.drive_model.trim());
    let drive =
        if vendor.is_empty() || model.starts_with(vendor) { model.to_string() } else { format!("{vendor} {model}") };
    let kind = if d.is_removable_media() { tr!("app-device-removable") } else { tr!("app-device-internal") };
    let hint = match c.state {
        S::Unmounted => tr!("app-device-click-mount"),
        S::Locked => tr!("app-device-click-unlock"),
        S::Unsupported => tr!("app-device-unsupported-hint"),
    };
    tr!(
        "app-device-candidate-tip",
        name = name,
        device = d.device.display().to_string(),
        fs = fs,
        size = if d.size > 0 { format_size(d.size, SizeStyle::Decimal) } else { "?".into() },
        drive = if drive.is_empty() { kind.to_string() } else { format!("{drive} · {kind}") },
        hint = hint
    )
}

/// Unmounted devices for the current config, re-classified only when the device list
/// or the config changed.
#[derive(Default)]
pub struct DevicesCache {
    key: Option<(u64, soyuz_core::config::UnmountedDevices, Vec<String>)>,
    list: Vec<soyuz_fs::udisks::Candidate>,
}

pub fn candidates(cache: &mut DevicesCache, cfg: &soyuz_core::config::Sidebar) -> Vec<soyuz_fs::udisks::Candidate> {
    use soyuz_core::config::UnmountedDevices;
    let d = desk().lock();
    if !d.available || cfg.unmounted_devices == UnmountedDevices::Off {
        return Vec::new();
    }
    let key = (d.generation, cfg.unmounted_devices, cfg.hidden_devices.clone());
    if cache.key.as_ref() != Some(&key) {
        let removable = cfg.unmounted_devices == UnmountedDevices::Removable;
        cache.list = soyuz_fs::udisks::sidebar_candidates(&d.devices, removable, &cfg.hidden_devices);
        cache.key = Some(key);
    }
    cache.list.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_match_volumes_by_mount_point() {
        let dev = |mount: &str, removable: bool, system: bool| soyuz_fs::udisks::BlockDevice {
            object: format!("/org/freedesktop/UDisks2/block_devices/{}", mount.len()),
            mount_points: if mount.is_empty() { vec![] } else { vec![PathBuf::from(mount)] },
            removable,
            hint_system: system,
            has_filesystem: true,
            ..Default::default()
        };
        let all = [
            dev("/run/media/u/STICK", true, true),
            dev("/", false, true),
            dev("/mnt/data", false, false),
            dev("/run/media/u/Games", false, true),
        ];
        assert!(device_at(&all, Path::new("/run/media/u/STICK")).is_some());
        assert!(device_at(&all, Path::new("/run/media/u/Games")).is_some(), "internal disks mounted for the user");
        assert!(device_at(&all, Path::new("/")).is_none(), "system disks are not offered for unmount");
        assert!(device_at(&all, Path::new("/mnt/data")).is_some());
        assert!(device_at(&all, Path::new("/nowhere")).is_none());
    }

    #[test]
    fn udisks_errors_read_well() {
        let e = std::io::Error::other("org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain: Not authorized");
        assert!(udisks_reason(&e, "ntfs").contains("polkit"));
        let e = std::io::Error::other("org.freedesktop.UDisks2.Error.DeviceBusy: target is busy");
        assert!(udisks_reason(&e, "ext4").contains("in use"));
        let e = std::io::Error::new(std::io::ErrorKind::Unsupported, "no bus");
        assert!(udisks_reason(&e, "").contains("UDisks2"));
        assert_eq!(udisks_reason(&std::io::Error::other("UDisks2: odd"), ""), "odd");
        let e = std::io::Error::other("Error mounting /dev/sdb1: Windows is hibernated, refused to mount.");
        assert!(udisks_reason(&e, "ntfs").contains("Fast Startup"));
        let e = std::io::Error::other("Error unlocking /dev/sda1: Failed to activate device: Operation not permitted");
        assert!(udisks_reason(&e, "BitLocker").contains("passphrase"));
        let e = std::io::Error::other("UDisks2: org.freedesktop.UDisks2.Error.NotAuthorizedDismissed: dismissed");
        assert!(udisks_reason(&e, "ntfs").contains("cancelled"));
    }

    fn cand(label: &str, fs: &str, size: u64) -> soyuz_fs::udisks::Candidate {
        soyuz_fs::udisks::Candidate {
            dev: soyuz_fs::udisks::BlockDevice {
                label: label.into(),
                fs_type: fs.into(),
                size,
                device: "/dev/sdb1".into(),
                drive_model: "Samsung SSD 860 EVO 1TB".into(),
                ..Default::default()
            },
            state: soyuz_fs::udisks::CandidateState::Unmounted,
            container: None,
            container_type: String::new(),
        }
    }

    #[test]
    fn device_labels() {
        assert_eq!(candidate_name(&cand("Games", "ntfs", 1_000_202_043_392)), "Games");
        assert_eq!(candidate_name(&cand("  ", "ntfs", 1_000_202_043_392)), "1 TB NTFS volume");
        assert_eq!(candidate_name(&cand("", "exfat", 0)), "exFAT volume");
        assert_eq!(candidate_badge(&cand("", "BitLocker", 1)), "BitLocker");
        let tip = candidate_tip(&cand("Games", "ntfs", 1_000_202_043_392), "Games");
        for part in ["Games", "/dev/sdb1", "NTFS", "1 TB", "Samsung SSD 860 EVO 1TB", "internal"] {
            assert!(tip.contains(part), "{part} missing in {tip}");
        }
    }

    fn blk(name: &str, drive: Option<&str>, mounts: &[&str]) -> soyuz_fs::udisks::BlockDevice {
        soyuz_fs::udisks::BlockDevice {
            object: format!("/org/freedesktop/UDisks2/block_devices/{name}"),
            device: PathBuf::from(format!("/dev/{name}")),
            drive: drive.map(|d| format!("/org/freedesktop/UDisks2/drives/{d}")),
            mount_points: mounts.iter().map(PathBuf::from).collect(),
            has_filesystem: true,
            usage: "filesystem".into(),
            ..Default::default()
        }
    }

    fn usb(name: &str, mounts: &[&str]) -> soyuz_fs::udisks::BlockDevice {
        soyuz_fs::udisks::BlockDevice {
            connection_bus: "usb".into(),
            removable: true,
            can_power_off: true,
            ..blk(name, Some("Stick"), mounts)
        }
    }

    fn obj(name: &str) -> String {
        format!("/org/freedesktop/UDisks2/block_devices/{name}")
    }

    #[test]
    fn eject_unmounts_the_whole_drive_and_powers_it_off() {
        let all = [usb("sdb1", &["/run/media/u/A"]), usb("sdb2", &["/run/media/u/A/inner"]), usb("sdb3", &[])];
        let plan = eject_plan(&all[0], &all).expect("a plain USB stick can be ejected");
        assert_eq!(plan.drive, "/org/freedesktop/UDisks2/drives/Stick");
        assert_eq!(plan.unmount, vec![obj("sdb2"), obj("sdb1")], "nested mount first, unmounted ones skipped");
        assert!(plan.lock.is_empty());
        assert!(!plan.eject, "USB disks are powered off");
    }

    #[test]
    fn eject_refuses_drives_holding_a_system_mount() {
        // A USB-booted system: `/` and `/boot/efi` live on the same stick as the data.
        for sys in ["/", "/boot/efi", "/home", "/var"] {
            let all = [usb("sdb1", &["/run/media/u/Data"]), usb("sdb2", &[sys])];
            assert!(eject_plan(&all[0], &all).is_none(), "{sys} on the drive");
        }
        // A system mount on an unlocked container of the drive counts too.
        let mut luks = usb("sdb2", &[]);
        luks.encrypted = true;
        luks.cleartext = Some(obj("dm0"));
        let mut root = blk("dm0", None, &["/"]);
        root.crypto_backing = Some(obj("sdb2"));
        let all = [usb("sdb1", &["/run/media/u/Data"]), luks, root];
        assert!(eject_plan(&all[0], &all).is_none(), "encrypted root on the stick");
        // Another drive's system mount does not matter.
        let all = [usb("sdb1", &["/run/media/u/Data"]), blk("nvme0n1p2", Some("Nvme"), &["/"])];
        assert!(eject_plan(&all[0], &all).is_some());
    }

    #[test]
    fn eject_refuses_internal_system_drives_and_fixed_disks() {
        let mut internal = blk("sda1", Some("Ssd"), &["/run/media/u/Games"]);
        internal.hint_system = true;
        internal.can_power_off = true;
        assert!(eject_plan(&internal, std::slice::from_ref(&internal)).is_none(), "internal system disk");
        let mut fixed = blk("sda1", Some("Ssd"), &["/run/media/u/Games"]);
        fixed.hint_system = false;
        assert!(eject_plan(&fixed, std::slice::from_ref(&fixed)).is_none(), "cannot eject or power off");
        let mut card = blk("mmcblk0p1", Some("Card"), &["/run/media/u/SD"]);
        card.hint_system = true;
        card.connection_bus = "sdio".into();
        card.ejectable = true;
        let plan = eject_plan(&card, std::slice::from_ref(&card)).expect("removable media flagged system");
        assert!(plan.eject);
    }

    #[test]
    fn eject_locks_unlocked_containers_after_unmounting() {
        let mut luks = usb("sdb1", &[]);
        luks.encrypted = true;
        luks.usage = "crypto".into();
        luks.cleartext = Some(obj("dm0"));
        let mut clear = blk("dm0", None, &["/run/media/u/Secret"]);
        clear.crypto_backing = Some(obj("sdb1"));
        let locked = soyuz_fs::udisks::BlockDevice { encrypted: true, cleartext: None, ..usb("sdb2", &[]) };
        let all = [luks, clear.clone(), locked];
        // Ejecting from the cleartext row (UDisks gives it no drive) finds the stick.
        let plan = eject_plan(&clear, &all).expect("cleartext device of a stick");
        assert_eq!(plan.unmount, vec![obj("dm0")]);
        assert_eq!(plan.lock, vec![obj("sdb1")], "only unlocked containers are locked");
        assert_eq!(drive_of(&clear, &all), Some("/org/freedesktop/UDisks2/drives/Stick"));
    }

    #[test]
    fn finished_mounts_open_in_a_pane_that_still_exists() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let path = PathBuf::from("/run/media/u/STICK");
        desk().lock().events.push(DeskEvent::Mounted { pane: PaneId(9_001), path: path.clone() });
        let mut cmds = Vec::new();
        desk_pump(&[], &mut cmds, |p| p != PaneId(9_001), PaneId(7));
        let nav = cmds.iter().find_map(|c| match c {
            Cmd::Navigate { pane, location: Location::Dir(p), .. } if *p == path => Some(*pane),
            _ => None,
        });
        assert_eq!(nav, Some(PaneId(7)), "the closed pane is replaced by the focused one");
        assert!(take_volumes_stale());
    }

    #[test]
    fn system_mounts_are_not_unmountable() {
        for m in ["/", "/boot", "/boot/efi", "/home", "/var", "/var/lib/x", "/usr", "/mnt"] {
            assert!(is_system_mount(Path::new(m)), "{m}");
        }
        for m in ["/run/media/u/Games", "/mnt/data", "/media/stick", "/home/u/disks/x"] {
            assert!(!is_system_mount(Path::new(m)), "{m}");
        }
    }
}

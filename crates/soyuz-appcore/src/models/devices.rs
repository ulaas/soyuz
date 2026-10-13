//! Devices of the sidebar (frontend-agnostic; Linux desktop): GTK bookmarks and UDisks2
//! devices (mount, unlock, unmount, lock, eject) plus `Cmd::Device`. The GUI rows and
//! menus are in `ui::sidebar`; the terminal UI's in `view::sidebar` / `input::sidebar`
//! with the menus as data ([`device_menu`], [`candidate_menu`]) and the unlock dialog's
//! submit ([`submit_unlock`]). The UDisks2 calls go through a [`DeskBackend`], which
//! tests replace ([`set_desk_backend`]).

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
    // Only the Linux refresher (or a test backend, `set_desk_backend`) fills the list.
    desk().lock().devices.clone()
}

/// The UDisks2 calls behind the device rows: the real `soyuz_fs::udisks` (polkit may
/// ask: every call is interactive), or a test fake installed with [`set_desk_backend`]
/// so tests never reach D-Bus (neither the unlock nor the mount that follows it).
#[doc(hidden)]
pub trait DeskBackend: Send + Sync {
    /// UDisks2 answers (asked once per refresher run).
    fn available(&self) -> bool;
    fn block_devices(&self) -> std::io::Result<Vec<soyuz_fs::udisks::BlockDevice>>;
    fn mount(&self, object: &str, options: &soyuz_fs::udisks::MountOptions) -> std::io::Result<PathBuf>;
    fn unmount(&self, object: &str) -> std::io::Result<()>;
    fn lock(&self, object: &str) -> std::io::Result<()>;
    /// Returns the cleartext device's object.
    fn unlock(&self, object: &str, passphrase: &str) -> std::io::Result<String>;
    fn eject(&self, drive: &str) -> std::io::Result<()>;
    fn power_off(&self, drive: &str) -> std::io::Result<()>;
}

/// The real backend: `soyuz_fs::udisks` (interactive, not forced).
struct UDisks;

impl DeskBackend for UDisks {
    fn available(&self) -> bool {
        soyuz_fs::udisks::available()
    }
    fn block_devices(&self) -> std::io::Result<Vec<soyuz_fs::udisks::BlockDevice>> {
        soyuz_fs::udisks::block_devices()
    }
    fn mount(&self, object: &str, options: &soyuz_fs::udisks::MountOptions) -> std::io::Result<PathBuf> {
        soyuz_fs::udisks::mount(object, options)
    }
    fn unmount(&self, object: &str) -> std::io::Result<()> {
        soyuz_fs::udisks::unmount(object, false, true)
    }
    fn lock(&self, object: &str) -> std::io::Result<()> {
        soyuz_fs::udisks::lock(object, true)
    }
    fn unlock(&self, object: &str, passphrase: &str) -> std::io::Result<String> {
        soyuz_fs::udisks::unlock(object, passphrase, true)
    }
    fn eject(&self, drive: &str) -> std::io::Result<()> {
        soyuz_fs::udisks::eject(drive, true)
    }
    fn power_off(&self, drive: &str) -> std::io::Result<()> {
        soyuz_fs::udisks::power_off(drive, true)
    }
}

static BACKEND: parking_lot::RwLock<Option<std::sync::Arc<dyn DeskBackend>>> = parking_lot::RwLock::new(None);

/// The backend device operations use now.
fn backend() -> std::sync::Arc<dyn DeskBackend> {
    BACKEND.read().clone().unwrap_or_else(|| std::sync::Arc::new(UDisks))
}

/// Tests: replace the UDisks2 calls with `b` for the rest of the process, and list its
/// devices right away (on every platform; the Linux refresher keeps asking `b`). Not a
/// cargo feature: tests of other crates (the terminal UI's headless runs) need it too.
#[doc(hidden)]
pub fn set_desk_backend(b: Box<dyn DeskBackend>) {
    let b: std::sync::Arc<dyn DeskBackend> = std::sync::Arc::from(b);
    *BACKEND.write() = Some(b.clone());
    let devices = b.block_devices().unwrap_or_default();
    {
        let mut d = desk().lock();
        d.available = b.available();
        d.devices = devices;
        d.generation += 1;
    }
    desk_rescan();
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
        let udisks = backend().available();
        loop {
            let gtk = soyuz_fs::xdg_dirs::gtk_bookmarks();
            let devices = if udisks {
                match backend().block_devices() {
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
        match backend().mount(&d.object, &o) {
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
    desk_op(&dev.object, Work::Unmounting, move || match backend().unmount(&object) {
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
        if mounted && let Err(e) = backend().unmount(&object) {
            return failed(e);
        }
        match backend().lock(&container) {
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
        let r = backend().unlock(&target, secret.as_str());
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
            if let Err(e) = backend().unmount(o) {
                return failed(e);
            }
        }
        for o in &plan.lock {
            if let Err(e) = backend().lock(o) {
                return failed(e);
            }
        }
        let r = if plan.eject { backend().eject(&plan.drive) } else { backend().power_off(&plan.drive) };
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
    /// Unmount a mounted device (the menus of [`device_menu`]).
    Unmount { dev: Box<soyuz_fs::udisks::BlockDevice>, name: String },
    /// Unmount `dev` (an unlocked container's cleartext device), then lock `container`.
    Lock { dev: Box<soyuz_fs::udisks::BlockDevice>, container: String, name: String },
    /// Eject the drive of `dev` ([`eject_device`] with the current device list).
    Eject { dev: Box<soyuz_fs::udisks::BlockDevice> },
}

/// Apply a [`DeviceCmd`].
pub fn apply(app: &mut crate::core::AppCore, host: &dyn Host, cmd: DeviceCmd) {
    let pane = app.ws.pane().id;
    match cmd {
        DeviceCmd::Unlock { object, name, kind } => {
            app.frontend.push(FrontendCmd::UnlockDevice { object, name, kind, pane })
        }
        // Both frontends ask with their confirm dialog (Phase T7: a core `Confirm`).
        DeviceCmd::OfferReadOnly { dev, name } => app.frontend.push(offer_read_only(dev, name)),
        DeviceCmd::Mount { dev, name, read_only } => mount_device(&dev, name, pane, read_only),
        DeviceCmd::Hide(key) => {
            if !app.config.sidebar.hidden_devices.contains(&key) {
                crate::models::settings::update_config(app, host, |c| c.sidebar.hidden_devices.push(key));
                app.ui.toasts.push(Toast::info(tr!("app-device-hidden")).body(tr!("app-device-hidden-body")));
            }
        }
        DeviceCmd::Unmount { dev, name } => unmount_device(&dev, name),
        DeviceCmd::Lock { dev, container, name } => lock_device(&dev, container, name),
        DeviceCmd::Eject { dev } => eject_device(&dev, &block_devices()),
    }
}

/// The device entries of a mounted volume's menu (after the place entries): Unmount,
/// Lock (an unlocked container's filesystem), Eject (when [`eject_plan`] allows it) —
/// the GUI's `ui::sidebar` device menu as data.
pub fn device_menu(
    dev: &soyuz_fs::udisks::BlockDevice,
    all: &[soyuz_fs::udisks::BlockDevice],
    name: &str,
) -> Vec<crate::models::context_menu::Entry> {
    use crate::icon::IconId as I;
    use crate::models::context_menu::MenuIcon;
    let boxed = || Box::new(dev.clone());
    let mut out = vec![menu_cmd(
        I::ArrowLineUp.into(),
        tr!("app-device-unmount"),
        true,
        Cmd::Device(DeviceCmd::Unmount { dev: boxed(), name: name.to_string() }),
    )];
    if let Some(container) = &dev.crypto_backing {
        out.push(menu_cmd(
            I::LockSimple.into(),
            tr!("app-device-lock"),
            true,
            Cmd::Device(DeviceCmd::Lock { dev: boxed(), container: container.clone(), name: name.to_string() }),
        ));
    }
    if eject_plan(dev, all).is_some() {
        out.push(menu_cmd(
            MenuIcon::Eject,
            tr!("app-device-eject"),
            true,
            Cmd::Device(DeviceCmd::Eject { dev: boxed() }),
        ));
    }
    out
}

/// The menu of an unmounted / locked device row: Mount, Mount read-only, Lock (an
/// unlocked container), or Unlock…; then Hide from sidebar, Copy device path. `idle`:
/// no operation runs on it (the actions are disabled otherwise). The GUI's
/// `ui::sidebar` candidate menu as data.
pub fn candidate_menu(
    cand: &soyuz_fs::udisks::Candidate,
    name: &str,
    idle: bool,
) -> Vec<crate::models::context_menu::Entry> {
    use crate::icon::IconId as I;
    use crate::models::context_menu::{Entry, MenuIcon};
    use soyuz_fs::udisks::CandidateState as S;
    let dev = || Box::new(cand.dev.clone());
    let mount = |read_only| Cmd::Device(DeviceCmd::Mount { dev: dev(), name: name.to_string(), read_only });
    let mut out = Vec::new();
    match cand.state {
        S::Unmounted => {
            out.push(menu_cmd(I::ArrowLineDown.into(), tr!("app-device-mount"), idle, mount(false)));
            out.push(menu_cmd(I::LockSimple.into(), tr!("app-device-mount-ro"), idle, mount(true)));
            if let Some(container) = &cand.container {
                let lock = DeviceCmd::Lock { dev: dev(), container: container.clone(), name: name.to_string() };
                out.push(menu_cmd(I::LockSimple.into(), tr!("app-device-lock"), idle, Cmd::Device(lock)));
            }
        }
        S::Locked => {
            let unlock = DeviceCmd::Unlock {
                object: cand.dev.object.clone(),
                name: name.to_string(),
                kind: cand.dev.fs_type.clone(),
            };
            out.push(menu_cmd(MenuIcon::LockOpen, tr!("app-device-unlock"), idle, Cmd::Device(unlock)));
        }
        S::Unsupported => {
            out.push(menu_cmd(
                MenuIcon::Prohibit,
                tr!("app-device-mount"),
                false,
                Cmd::Device(DeviceCmd::Hide(String::new())),
            ));
        }
    }
    out.push(Entry::Separator);
    out.push(menu_cmd(I::EyeSlash.into(), tr!("app-device-hide"), true, Cmd::Device(DeviceCmd::Hide(cand.hide_key()))));
    out.push(menu_cmd(
        I::ClipboardText.into(),
        tr!("app-device-copy-path"),
        true,
        Cmd::CopyText(cand.dev.device.display().to_string()),
    ));
    out
}

/// A menu item running `cmd` (disabled items run nothing).
fn menu_cmd(
    icon: crate::models::context_menu::MenuIcon,
    label: &str,
    enabled: bool,
    cmd: Cmd,
) -> crate::models::context_menu::Entry {
    use crate::models::context_menu::{Entry, Item, ItemStyle, Run};
    Entry::Item(Item {
        icon,
        label: label.to_string(),
        chord: None,
        enabled,
        style: ItemStyle::Plain,
        run: enabled.then_some(Run::Cmd(cmd)),
        keep_open: false,
        tip: None,
    })
}

/// The status line of a device row (where mounted volumes show their usage): what runs
/// on it, else its state and size ("Not mounted · 32 GB").
pub fn candidate_status(work: Option<Work>, state: soyuz_fs::udisks::CandidateState, size: u64) -> String {
    use soyuz_fs::udisks::CandidateState as S;
    let size = if size > 0 { format_size(size, SizeStyle::Decimal) } else { String::new() };
    match (work, state) {
        (Some(Work::Mounting), _) => tr!("app-device-status-mounting").to_string(),
        (Some(Work::Unlocking), _) => tr!("app-device-status-unlocking").to_string(),
        (Some(Work::Locking), _) => tr!("app-device-status-locking").to_string(),
        (Some(Work::Unmounting), _) => tr!("app-device-status-unmounting").to_string(),
        (None, S::Unmounted) => tr!("app-device-status-unmounted", size = size),
        (None, S::Locked) => tr!("app-device-status-locked", size = size),
        (None, S::Unsupported) => tr!("app-device-status-unsupported", size = size),
    }
}

/// A BitLocker container (`BitLocker` / `crypto_BITLK`): the unlock dialog offers the
/// recovery key.
pub fn is_bitlocker(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("bitlocker") || kind.eq_ignore_ascii_case("crypto_bitlk")
}

/// A BitLocker recovery key as typed: 48 digits in eight dash-separated groups (spaces
/// and missing dashes are tolerated); `None` when the text is not one.
pub fn recovery_key(text: &str) -> Option<zeroize::Zeroizing<String>> {
    let digits: zeroize::Zeroizing<String> =
        zeroize::Zeroizing::new(text.chars().filter(|c| !c.is_whitespace() && *c != '-').collect());
    if digits.len() != 48 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut key = zeroize::Zeroizing::new(String::with_capacity(55));
    for i in 0..8 {
        if i > 0 {
            key.push('-');
        }
        key.push_str(&digits[i * 6..i * 6 + 6]);
    }
    Some(key)
}

/// The unlock dialogs' submit: a BitLocker recovery key is normalized to its dashed form
/// (the typed text is wiped), then [`unlock_device`] runs. The receiver gets `Ok(())`
/// once unlocked (the mount follows), or the reason it failed; it disconnects without a
/// message when another operation runs on the device.
pub fn submit_unlock(
    object: &str,
    name: String,
    kind: &str,
    mut typed: zeroize::Zeroizing<String>,
    pane: PaneId,
) -> std::sync::mpsc::Receiver<Result<(), String>> {
    let secret = if is_bitlocker(kind)
        && let Some(k) = recovery_key(&typed)
    {
        zeroize::Zeroize::zeroize(&mut *typed);
        k
    } else {
        typed
    };
    let (tx, rx) = std::sync::mpsc::channel();
    unlock_device(object, name, secret, pane, tx);
    rx
}

/// `DeviceCmd::OfferReadOnly`: explain the hibernated NTFS volume and offer a read-only
/// mount, as a `FrontendCmd::Confirm` both frontends show with their confirm dialog.
pub fn offer_read_only(dev: Box<soyuz_fs::udisks::BlockDevice>, name: String) -> FrontendCmd {
    FrontendCmd::Confirm {
        title: tr!("app-device-hibernated-title", name = name.clone()),
        message: tr!("app-device-hibernated-body").into(),
        confirm: tr!("app-device-mount-ro").into(),
        on_confirm: Cmd::Device(DeviceCmd::Mount { dev, name, read_only: true }),
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

impl DevicesCache {
    /// The list of the last [`candidates`] call (the terminal UI builds its rows from it
    /// without `&mut`).
    pub fn list(&self) -> &[soyuz_fs::udisks::Candidate] {
        &self.list
    }
}

pub fn candidates(cache: &mut DevicesCache, cfg: &soyuz_core::config::Sidebar) -> Vec<soyuz_fs::udisks::Candidate> {
    use soyuz_core::config::UnmountedDevices;
    let d = desk().lock();
    if !d.available || cfg.unmounted_devices == UnmountedDevices::Off {
        cache.key = None;
        cache.list.clear();
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
    fn hibernated_offer_is_a_core_confirm() {
        let dev = soyuz_fs::udisks::BlockDevice { object: "/org/x/sdb1".into(), ..Default::default() };
        let FrontendCmd::Confirm { title, on_confirm, .. } = offer_read_only(Box::new(dev.clone()), "Win".into())
        else {
            panic!("not a confirm")
        };
        assert!(title.contains("Win"), "{title}");
        match on_confirm {
            Cmd::Device(DeviceCmd::Mount { dev: d, name, read_only }) => {
                assert!(read_only);
                assert_eq!((*d, name.as_str()), (dev, "Win"));
            }
            other => panic!("{other:?}"),
        }
    }

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

    /// The labels and commands of a menu's enabled items, in order.
    fn items(menu: &[crate::models::context_menu::Entry]) -> Vec<(String, Option<Cmd>)> {
        use crate::models::context_menu::{Entry, Run};
        menu.iter()
            .filter_map(|e| match e {
                Entry::Item(i) => Some((
                    i.label.clone(),
                    match &i.run {
                        Some(Run::Cmd(c)) => Some(c.clone()),
                        _ => None,
                    },
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn mounted_device_menus_unmount_lock_and_eject() {
        // An unlocked LUKS stick: the cleartext device is mounted, its container on the stick.
        let mut luks = usb("sdb1", &[]);
        luks.encrypted = true;
        luks.cleartext = Some(obj("dm0"));
        let mut clear = blk("dm0", None, &["/run/media/u/Secret"]);
        clear.crypto_backing = Some(obj("sdb1"));
        let all = [luks, clear.clone()];
        let menu = items(&device_menu(&clear, &all, "Secret"));
        let labels: Vec<&str> = menu.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Unmount", "Lock", "Eject"]);
        assert!(
            matches!(&menu[0].1, Some(Cmd::Device(DeviceCmd::Unmount { dev, name })) if **dev == clear && name == "Secret")
        );
        assert!(
            matches!(&menu[1].1, Some(Cmd::Device(DeviceCmd::Lock { container, .. })) if *container == obj("sdb1"))
        );
        assert!(matches!(&menu[2].1, Some(Cmd::Device(DeviceCmd::Eject { .. }))));
        // A fixed internal disk: only Unmount.
        let fixed = blk("sda1", Some("Ssd"), &["/run/media/u/Games"]);
        let menu = items(&device_menu(&fixed, std::slice::from_ref(&fixed), "Games"));
        assert_eq!(menu.len(), 1);
    }

    #[test]
    fn candidate_menus_follow_the_state() {
        use soyuz_fs::udisks::CandidateState as S;
        let c = cand("Games", "ntfs", 1_000_000_000);
        let menu = items(&candidate_menu(&c, "Games", true));
        let labels: Vec<&str> = menu.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Mount", "Mount read-only", "Hide from sidebar", "Copy device path"]);
        assert!(matches!(&menu[1].1, Some(Cmd::Device(DeviceCmd::Mount { read_only: true, .. }))));
        assert!(matches!(&menu[2].1, Some(Cmd::Device(DeviceCmd::Hide(k))) if *k == c.hide_key()));
        assert!(matches!(&menu[3].1, Some(Cmd::CopyText(t)) if t.contains("sdb1")));
        // Busy: the actions are disabled, hiding / copying still work.
        let busy = items(&candidate_menu(&c, "Games", false));
        assert!(busy[0].1.is_none() && busy[1].1.is_none() && busy[2].1.is_some());
        // An unlocked container's filesystem can be locked again.
        let opened = soyuz_fs::udisks::Candidate { container: Some(obj("sdb2")), ..c.clone() };
        assert!(items(&candidate_menu(&opened, "Games", true)).iter().any(|(l, _)| l == "Lock"));
        let locked = soyuz_fs::udisks::Candidate { state: S::Locked, ..c.clone() };
        let menu = items(&candidate_menu(&locked, "Games", true));
        assert_eq!(menu[0].0, "Unlock…");
        assert!(
            matches!(&menu[0].1, Some(Cmd::Device(DeviceCmd::Unlock { name, kind, .. })) if name == "Games" && kind == "ntfs")
        );
        let odd = soyuz_fs::udisks::Candidate { state: S::Unsupported, ..c };
        let menu = items(&candidate_menu(&odd, "Games", true));
        assert!(menu[0].1.is_none(), "unsupported filesystems can't be mounted");
    }

    /// The GUI draws these menus too (`ui::sidebar`): icons, enabled state and the
    /// separator are part of the shared definition.
    #[test]
    fn menu_icons_separators_and_enabled_state() {
        use crate::icon::IconId as I;
        use crate::models::context_menu::{Entry, MenuIcon};
        use soyuz_fs::udisks::CandidateState as S;
        let shape = |menu: &[Entry]| -> Vec<Option<(MenuIcon, bool)>> {
            menu.iter()
                .map(|e| match e {
                    Entry::Item(i) => Some((i.icon, i.enabled)),
                    _ => None,
                })
                .collect()
        };
        let mut luks = usb("sdb1", &[]);
        luks.encrypted = true;
        luks.cleartext = Some(obj("dm0"));
        let mut clear = blk("dm0", None, &["/run/media/u/Secret"]);
        clear.crypto_backing = Some(obj("sdb1"));
        let all = [luks, clear.clone()];
        assert_eq!(
            shape(&device_menu(&clear, &all, "Secret")),
            [Some((I::ArrowLineUp.into(), true)), Some((I::LockSimple.into(), true)), Some((MenuIcon::Eject, true))]
        );
        let c = soyuz_fs::udisks::Candidate { container: Some(obj("sdb2")), ..cand("Games", "ntfs", 1) };
        let tail = [None, Some((I::EyeSlash.into(), true)), Some((I::ClipboardText.into(), true))];
        let busy = shape(&candidate_menu(&c, "Games", false));
        let mount = [
            Some((I::ArrowLineDown.into(), false)),
            Some((I::LockSimple.into(), false)),
            Some((I::LockSimple.into(), false)),
        ];
        assert_eq!(busy, [&mount[..], &tail[..]].concat());
        let locked = soyuz_fs::udisks::Candidate { state: S::Locked, ..c.clone() };
        assert_eq!(shape(&candidate_menu(&locked, "Games", true))[0], Some((MenuIcon::LockOpen, true)));
        let odd = soyuz_fs::udisks::Candidate { state: S::Unsupported, ..c };
        assert_eq!(
            shape(&candidate_menu(&odd, "Games", true)),
            [&[Some((MenuIcon::Prohibit, false))][..], &tail[..]].concat()
        );
    }

    #[test]
    fn candidate_status_lines() {
        use soyuz_fs::udisks::CandidateState as S;
        assert_eq!(candidate_status(None, S::Unmounted, 32_000_000_000), "Not mounted · 32 GB");
        assert_eq!(candidate_status(None, S::Locked, 0), "Locked · ");
        assert_eq!(candidate_status(Some(Work::Mounting), S::Locked, 1), "Mounting…");
        assert_eq!(candidate_status(Some(Work::Unlocking), S::Locked, 1), "Unlocking…");
    }

    #[test]
    fn recovery_keys_are_normalized() {
        let k = "123456-234567-345678-456789-567890-678901-789012-890123";
        let key = |t: &str| recovery_key(t).map(|z| z.as_str().to_string());
        assert_eq!(key(k).as_deref(), Some(k));
        assert_eq!(key(&k.replace('-', "")).as_deref(), Some(k));
        assert_eq!(key(&k.replace('-', " ")).as_deref(), Some(k));
        assert_eq!(key("hunter2"), None);
        assert_eq!(key(&"1".repeat(47)), None);
        assert!(is_bitlocker("BitLocker") && is_bitlocker("crypto_BITLK") && !is_bitlocker("crypto_LUKS"));
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

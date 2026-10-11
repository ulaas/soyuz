//! Block devices, mount / unmount / eject through the UDisks2 D-Bus API (Linux only).
//!
//! Lists every block device with a filesystem (mounted or not) or an encrypted container
//! from `org.freedesktop.UDisks2` (`ObjectManager.GetManagedObjects` on the system bus),
//! joined with its drive (model, removable / ejectable). Mounting goes through
//! `Filesystem.Mount`, so polkit decides (usually no prompt for removable media of the
//! active session) and the mount lands where the desktop expects
//! (`/run/media/$USER/<label>`). No `mount` / `udisksctl` process is spawned.
//!
//! Every call blocks (bounded by a timeout) and is meant for background threads. Without
//! UDisks2 (other OSes, containers, no system bus) everything fails with
//! `ErrorKind::Unsupported` and [`available`] is `false`.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

/// One block device as UDisks2 reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockDevice {
    /// UDisks2 object path (`/org/freedesktop/UDisks2/block_devices/sdb1`), the handle
    /// for [`mount`] / [`unmount`] / [`unlock`].
    pub object: String,
    /// Device node (`/dev/sdb1`; the preferred `/dev/mapper/…` name when there is one).
    pub device: PathBuf,
    pub label: String,
    pub uuid: String,
    /// Bytes.
    pub size: u64,
    /// `ext4`, `vfat`, `ntfs`, `crypto_LUKS`, …
    pub fs_type: String,
    /// `filesystem`, `crypto`, `other`, …
    pub usage: String,
    /// Has the `Filesystem` interface (can be mounted).
    pub has_filesystem: bool,
    pub mount_points: Vec<PathBuf>,
    pub read_only: bool,
    /// UDisks says: do not show this device to the user.
    pub hint_ignore: bool,
    /// UDisks says: system device (internal disks, not user-removable).
    pub hint_system: bool,
    /// Display name / icon name hints from udev rules (may be empty).
    pub hint_name: String,
    pub hint_icon: String,
    /// Encrypted container (`Encrypted` interface); `cleartext` is the unlocked device.
    pub encrypted: bool,
    pub cleartext: Option<String>,
    /// For an unlocked device: the encrypted container it comes from.
    pub crypto_backing: Option<String>,
    /// Loop device backing file.
    pub loop_file: Option<PathBuf>,
    /// The drive object (`/org/freedesktop/UDisks2/drives/…`), the handle for
    /// [`eject`] / [`power_off`].
    pub drive: Option<String>,
    pub drive_model: String,
    pub drive_vendor: String,
    /// `usb`, `sdio`, `ieee1394`, … (empty for internal buses).
    pub connection_bus: String,
    pub removable: bool,
    pub ejectable: bool,
    pub can_power_off: bool,
}

impl BlockDevice {
    /// Should appear in a device list: not ignored, something to mount or unlock, and
    /// not a system device unless it is removable or mounted somewhere a user would look
    /// (`/media`, `/run/media`, `/mnt`, the home folder — the GVfs rule; `/`, `/boot`
    /// and friends stay hidden).
    pub fn user_visible(&self) -> bool {
        let home = crate::path::home();
        let under = |m: &PathBuf, root: &std::path::Path| m.starts_with(root) && m != root;
        let user_mount = |m: &PathBuf| {
            ["/media", "/run/media", "/mnt"].iter().any(|r| under(m, std::path::Path::new(r))) || under(m, &home)
        };
        !self.hint_ignore
            && (self.has_filesystem || (self.encrypted && self.cleartext.is_none()))
            && (!self.hint_system || self.removable || self.mount_points.iter().any(user_mount))
    }

    /// The name to show: hint, label, else model + size-free device name.
    pub fn display_name(&self) -> String {
        for s in [&self.hint_name, &self.label] {
            if !s.trim().is_empty() {
                return s.clone();
            }
        }
        let dev = self.device.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if self.drive_model.trim().is_empty() { dev } else { format!("{} ({dev})", self.drive_model.trim()) }
    }
}

/// Mount options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountOptions {
    /// Filesystem-specific options (`ro`, `noexec`, …), comma-separated; empty = defaults.
    pub options: String,
    /// Allow polkit to ask for a password (otherwise fail when one would be needed).
    pub interactive: bool,
}

/// Timeout for listing.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(3);
/// Timeout for operations that may wait for a polkit prompt or a slow device.
pub const OP_TIMEOUT: Duration = Duration::from_secs(120);

fn unsupported(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, why.into())
}

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use zbus::blocking::Connection;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    use super::*;
    use crate::launch::linux::bus;

    const DEST: &str = "org.freedesktop.UDisks2";
    const ROOT: &str = "/org/freedesktop/UDisks2";
    const P: &str = "org.freedesktop.UDisks2.";

    type Props = HashMap<String, OwnedValue>;
    type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

    fn s(p: &Props, k: &str) -> String {
        match p.get(k).map(|v| &**v) {
            Some(Value::Str(s)) => s.to_string(),
            _ => String::new(),
        }
    }

    fn b(p: &Props, k: &str) -> bool {
        matches!(p.get(k).map(|v| &**v), Some(Value::Bool(true)))
    }

    fn u(p: &Props, k: &str) -> u64 {
        match p.get(k).map(|v| &**v) {
            Some(Value::U64(n)) => *n,
            _ => 0,
        }
    }

    fn obj(p: &Props, k: &str) -> Option<String> {
        match p.get(k).map(|v| &**v) {
            Some(Value::ObjectPath(o)) if o.as_str() != "/" => Some(o.to_string()),
            _ => None,
        }
    }

    fn bytes_of(v: &Value<'_>) -> Vec<u8> {
        match v {
            Value::Array(a) => {
                let mut out: Vec<u8> =
                    a.iter().filter_map(|x| if let Value::U8(c) = x { Some(*c) } else { None }).collect();
                while out.last() == Some(&0) {
                    out.pop();
                }
                out
            }
            _ => Vec::new(),
        }
    }

    fn path(p: &Props, k: &str) -> Option<PathBuf> {
        let v = bytes_of(p.get(k)?);
        (!v.is_empty()).then(|| crate::launch::linux::uri::path_from_bytes(v))
    }

    fn paths(p: &Props, k: &str) -> Vec<PathBuf> {
        match p.get(k).map(|v| &**v) {
            Some(Value::Array(a)) => a
                .iter()
                .map(bytes_of)
                .filter(|v| !v.is_empty())
                .map(crate::launch::linux::uri::path_from_bytes)
                .collect(),
            _ => Vec::new(),
        }
    }

    fn conn() -> io::Result<Connection> {
        bus::system().ok_or_else(|| unsupported("no D-Bus system bus"))
    }

    fn dbus_err(e: zbus::Error) -> io::Error {
        let text = e.to_string();
        let kind = if text.contains("ServiceUnknown") || text.contains("UnknownObject") {
            io::ErrorKind::Unsupported
        } else if text.contains("NotAuthorized") || text.contains("AccessDenied") {
            io::ErrorKind::PermissionDenied
        } else if text.contains("Timeout") || text.contains("NoReply") {
            io::ErrorKind::TimedOut
        } else if text.contains("DeviceBusy") {
            io::ErrorKind::ResourceBusy
        } else {
            io::ErrorKind::Other
        };
        io::Error::new(kind, format!("UDisks2: {text}"))
    }

    fn objects(c: &Connection) -> io::Result<Objects> {
        let m = c
            .call_method(Some(DEST), ROOT, Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
            .map_err(dbus_err)?;
        m.body().deserialize::<Objects>().map_err(dbus_err)
    }

    pub fn available() -> bool {
        conn().is_ok_and(|c| bus::has_owner(&c, DEST) || bus::activatable(&c, DEST))
    }

    pub fn block_devices() -> io::Result<Vec<BlockDevice>> {
        let c = conn()?;
        let objs = objects(&c)?;
        let empty = Props::new();
        let mut out = Vec::new();
        for (path_, ifaces) in &objs {
            let Some(blk) = ifaces.get(&format!("{P}Block")) else { continue };
            let fs = ifaces.get(&format!("{P}Filesystem"));
            let enc = ifaces.get(&format!("{P}Encrypted"));
            let drive = obj(blk, "Drive");
            let dp = drive
                .as_ref()
                .and_then(|d| objs.iter().find(|(k, _)| k.as_str() == d))
                .and_then(|(_, i)| i.get(&format!("{P}Drive")))
                .unwrap_or(&empty);
            out.push(BlockDevice {
                object: path_.to_string(),
                device: path(blk, "PreferredDevice").or_else(|| path(blk, "Device")).unwrap_or_default(),
                label: s(blk, "IdLabel"),
                uuid: s(blk, "IdUUID"),
                size: u(blk, "Size"),
                fs_type: s(blk, "IdType"),
                usage: s(blk, "IdUsage"),
                has_filesystem: fs.is_some(),
                mount_points: fs.map(|f| paths(f, "MountPoints")).unwrap_or_default(),
                read_only: b(blk, "ReadOnly"),
                hint_ignore: b(blk, "HintIgnore"),
                hint_system: b(blk, "HintSystem"),
                hint_name: s(blk, "HintName"),
                hint_icon: s(blk, "HintIconName"),
                encrypted: enc.is_some(),
                cleartext: enc.and_then(|e| obj(e, "CleartextDevice")),
                crypto_backing: obj(blk, "CryptoBackingDevice"),
                loop_file: ifaces.get(&format!("{P}Loop")).and_then(|l| path(l, "BackingFile")),
                drive,
                drive_model: s(dp, "Model"),
                drive_vendor: s(dp, "Vendor"),
                connection_bus: s(dp, "ConnectionBus"),
                removable: b(dp, "Removable") || b(dp, "MediaRemovable"),
                ejectable: b(dp, "Ejectable"),
                can_power_off: b(dp, "CanPowerOff"),
            });
        }
        out.sort_by(|a, b| a.device.cmp(&b.device));
        Ok(out)
    }

    fn opts(o: &MountOptions) -> HashMap<&'static str, Value<'static>> {
        let mut m: HashMap<&'static str, Value<'static>> = HashMap::new();
        m.insert("auth.no_user_interaction", Value::from(!o.interactive));
        if !o.options.trim().is_empty() {
            m.insert("options", Value::from(o.options.trim().to_string()));
        }
        m
    }

    /// A dedicated connection with the long timeout (mounts may wait for polkit).
    fn op_conn() -> io::Result<Connection> {
        // Fail fast (cached) when there is no system bus at all.
        let _ = conn()?;
        bus::connect(true, OP_TIMEOUT).ok_or_else(|| unsupported("no D-Bus system bus"))
    }

    fn call<B, R>(object: &str, iface: &str, method: &str, body: &B) -> io::Result<R>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        if !object.starts_with(ROOT) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a UDisks2 object path"));
        }
        let c = op_conn()?;
        let m =
            c.call_method(Some(DEST), object, Some(format!("{P}{iface}").as_str()), method, body).map_err(dbus_err)?;
        m.body().deserialize::<R>().map_err(dbus_err)
    }

    pub fn mount(object: &str, o: &MountOptions) -> io::Result<PathBuf> {
        call::<_, String>(object, "Filesystem", "Mount", &(opts(o),)).map(PathBuf::from)
    }

    pub fn unmount(object: &str, force: bool, interactive: bool) -> io::Result<()> {
        let mut m = opts(&MountOptions { interactive, ..Default::default() });
        if force {
            m.insert("force", Value::from(true));
        }
        call::<_, ()>(object, "Filesystem", "Unmount", &(m,))
    }

    pub fn eject(drive: &str, interactive: bool) -> io::Result<()> {
        call::<_, ()>(drive, "Drive", "Eject", &(opts(&MountOptions { interactive, ..Default::default() }),))
    }

    pub fn power_off(drive: &str, interactive: bool) -> io::Result<()> {
        call::<_, ()>(drive, "Drive", "PowerOff", &(opts(&MountOptions { interactive, ..Default::default() }),))
    }

    pub fn unlock(object: &str, passphrase: &str, interactive: bool) -> io::Result<String> {
        let o = opts(&MountOptions { interactive, ..Default::default() });
        call::<_, OwnedObjectPath>(object, "Encrypted", "Unlock", &(passphrase, o)).map(|p| p.to_string())
    }

    pub fn lock(object: &str, interactive: bool) -> io::Result<()> {
        call::<_, ()>(object, "Encrypted", "Lock", &(opts(&MountOptions { interactive, ..Default::default() }),))
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::path::PathBuf;

    use super::*;

    fn no() -> io::Error {
        unsupported("UDisks2 is only available on Linux")
    }
    pub fn available() -> bool {
        false
    }
    pub fn block_devices() -> io::Result<Vec<BlockDevice>> {
        Err(no())
    }
    pub fn mount(_: &str, _: &MountOptions) -> io::Result<PathBuf> {
        Err(no())
    }
    pub fn unmount(_: &str, _: bool, _: bool) -> io::Result<()> {
        Err(no())
    }
    pub fn eject(_: &str, _: bool) -> io::Result<()> {
        Err(no())
    }
    pub fn power_off(_: &str, _: bool) -> io::Result<()> {
        Err(no())
    }
    pub fn unlock(_: &str, _: &str, _: bool) -> io::Result<String> {
        Err(no())
    }
    pub fn lock(_: &str, _: bool) -> io::Result<()> {
        Err(no())
    }
}

/// Whether UDisks2 is reachable (running or D-Bus activatable).
pub fn available() -> bool {
    imp::available()
}

/// Every block device UDisks2 knows (filter with [`BlockDevice::user_visible`]).
pub fn block_devices() -> io::Result<Vec<BlockDevice>> {
    imp::block_devices()
}

/// Mount the filesystem of `object` (a [`BlockDevice::object`]); returns the mount point.
pub fn mount(object: &str, options: &MountOptions) -> io::Result<PathBuf> {
    imp::mount(object, options)
}

/// Unmount the filesystem of `object` (`force` = lazy unmount when busy).
pub fn unmount(object: &str, force: bool, interactive: bool) -> io::Result<()> {
    imp::unmount(object, force, interactive)
}

/// Eject the media of a drive ([`BlockDevice::drive`]); unmount its filesystems first.
pub fn eject(drive: &str, interactive: bool) -> io::Result<()> {
    imp::eject(drive, interactive)
}

/// Power off a drive (safe removal of USB disks); unmount its filesystems first.
pub fn power_off(drive: &str, interactive: bool) -> io::Result<()> {
    imp::power_off(drive, interactive)
}

/// Unlock an encrypted container; returns the cleartext device's object path.
pub fn unlock(object: &str, passphrase: &str, interactive: bool) -> io::Result<String> {
    imp::unlock(object, passphrase, interactive)
}

/// Lock an unlocked encrypted container (`object` = the container, not the cleartext).
pub fn lock(object: &str, interactive: bool) -> io::Result<()> {
    imp::lock(object, interactive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_rules() {
        let mut d = BlockDevice { has_filesystem: true, device: "/dev/sdb1".into(), ..Default::default() };
        assert!(d.user_visible());
        d.hint_ignore = true;
        assert!(!d.user_visible());
        d.hint_ignore = false;
        d.hint_system = true;
        assert!(!d.user_visible(), "unmounted system partition");
        d.mount_points = vec!["/boot".into()];
        assert!(!d.user_visible(), "system mount");
        d.mount_points = vec!["/run/media/u/DATA".into()];
        assert!(d.user_visible(), "system partition mounted for the user");
        d.mount_points = vec!["/mnt".into()];
        assert!(!d.user_visible());
        let locked = BlockDevice { encrypted: true, ..Default::default() };
        assert!(locked.user_visible());
        let unlocked = BlockDevice { encrypted: true, cleartext: Some("/x".into()), ..Default::default() };
        assert!(!unlocked.user_visible(), "the cleartext device is listed instead");
        assert_eq!(d.display_name(), "sdb1");
        d.drive_model = "Flash Disk".into();
        assert_eq!(d.display_name(), "Flash Disk (sdb1)");
        d.label = "STICK".into();
        assert_eq!(d.display_name(), "STICK");
    }

    #[test]
    fn rejects_foreign_object_paths() {
        if cfg!(target_os = "linux") {
            let e = mount("/org/other/thing", &MountOptions::default()).unwrap_err();
            assert!(matches!(e.kind(), io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported), "{e}");
        } else {
            assert!(!available());
            assert_eq!(block_devices().unwrap_err().kind(), io::ErrorKind::Unsupported);
        }
    }

    /// `SOYUZ_DBUS_TESTS=1`: list this machine's devices (read-only; nothing is
    /// mounted or unmounted).
    #[cfg(target_os = "linux")]
    #[test]
    fn real_listing() {
        if std::env::var_os("SOYUZ_DBUS_TESTS").is_none() {
            return;
        }
        assert!(available(), "UDisks2 not reachable");
        let devs = block_devices().unwrap();
        assert!(!devs.is_empty());
        for d in &devs {
            println!(
                "{} {:?} fs={} label={:?} size={} mounts={:?} visible={} sys={} ign={} drive={:?} model={:?} removable={} ejectable={}",
                d.object,
                d.device,
                d.fs_type,
                d.label,
                d.size,
                d.mount_points,
                d.user_visible(),
                d.hint_system,
                d.hint_ignore,
                d.drive,
                d.drive_model,
                d.removable,
                d.ejectable
            );
        }
        assert!(devs.iter().any(|d| d.mount_points.iter().any(|m| m == std::path::Path::new("/")) || d.has_filesystem));
    }
}

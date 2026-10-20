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
    /// Partition type: a GPT type GUID (lower case) or an MBR type (`0x07`); empty when
    /// the device is not a partition.
    pub partition_type: String,
    /// Partition number (1-based; 0 when not a partition).
    pub partition_number: u32,
    /// The partition table's block object (the whole disk), when a partition.
    pub partition_table: Option<String>,
    /// An MBR extended partition (a container of logical partitions).
    pub is_container: bool,
    /// A loop device (see [`BlockDevice::loop_file`] for its backing file).
    pub is_loop: bool,
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

/// GPT partition types never offered for mounting.
mod part_types {
    pub const EFI_SYSTEM: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
    pub const MS_RESERVED: &str = "e3c9e316-0b5c-4db8-817d-f92df00215ae";
    pub const WIN_RECOVERY: &str = "de94bba4-06d1-4d40-a16a-bfd50179d6ac";
    pub const BIOS_BOOT: &str = "21686148-6449-6e6f-744e-656564454649";
    pub const LINUX_SWAP: &str = "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f";
    pub const LDM_METADATA: &str = "5808c8aa-7e8f-42e0-85d2-e1e90434cfb3";
    /// MBR types: EFI system, Windows recovery (hidden NTFS WinRE), Linux swap, extended.
    pub const MBR_EFI: u8 = 0xef;
    pub const MBR_RECOVERY: u8 = 0x27;
    pub const MBR_SWAP: u8 = 0x82;
    pub const MBR_EXTENDED: [u8; 3] = [0x05, 0x0f, 0x85];
}

/// Filesystems Linux has no (usable) driver for: shown, but not mountable.
const UNSUPPORTED_FS: [&str; 2] = ["apfs", "refs"];

/// Encrypted container types the sidebar offers to unlock (UDisks2 2.11 / libblockdev).
fn is_unlockable(fs_type: &str) -> bool {
    matches!(fs_type.to_ascii_lowercase().as_str(), "crypto_luks" | "bitlocker" | "crypto_bitlk")
}

/// Short display name of a filesystem / container type (`NTFS`, `exFAT`, `BitLocker`).
pub fn fs_display_name(fs_type: &str) -> String {
    let n = match fs_type.to_ascii_lowercase().as_str() {
        "ntfs" | "ntfs3" => "NTFS",
        "exfat" => "exFAT",
        "vfat" | "fat" | "msdos" => "FAT",
        "bitlocker" | "crypto_bitlk" => "BitLocker",
        "crypto_luks" => "LUKS",
        "hfsplus" => "HFS+",
        "hfs" => "HFS",
        "apfs" => "APFS",
        "refs" => "ReFS",
        "btrfs" => "Btrfs",
        "xfs" => "XFS",
        "f2fs" => "F2FS",
        "udf" => "UDF",
        "iso9660" => "ISO 9660",
        "zfs_member" => "ZFS",
        _ => return fs_type.to_string(),
    };
    n.to_string()
}

/// How an unmounted device appears in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateState {
    /// A filesystem that can be mounted.
    Unmounted,
    /// An encrypted container (LUKS, BitLocker) that must be unlocked first.
    Locked,
    /// A filesystem Linux can't mount (APFS, ReFS, …): listed but disabled.
    Unsupported,
}

/// An unmounted device the user can mount or unlock (see [`sidebar_candidates`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The device to mount / unlock. For an unlocked container this is the cleartext
    /// device, with the drive details, label and partition position of its container.
    pub dev: BlockDevice,
    pub state: CandidateState,
    /// For an unlocked (cleartext) device: the container's object (the handle for [`lock`]).
    pub container: Option<String>,
    /// The container's filesystem type for an unlocked device (`BitLocker`, `crypto_LUKS`).
    pub container_type: String,
}

impl Candidate {
    /// The key "Hide from sidebar" stores: the filesystem / container UUID, else the
    /// device node.
    pub fn hide_key(&self) -> String {
        hide_key(&self.dev)
    }
}

/// `[sidebar] hidden_devices` key of a device: its UUID, else its device node.
pub fn hide_key(d: &BlockDevice) -> String {
    if d.uuid.trim().is_empty() { d.device.to_string_lossy().into_owned() } else { d.uuid.clone() }
}

impl BlockDevice {
    /// Removable media: the drive says so, or it sits on a hot-plug bus (USB, SD, FireWire).
    pub fn is_removable_media(&self) -> bool {
        self.removable || self.ejectable || matches!(self.connection_bus.as_str(), "usb" | "sdio" | "ieee1394")
    }

    /// A partition type that holds no user data (EFI system partition, Microsoft
    /// reserved, Windows recovery, BIOS boot, swap, LDM metadata, MBR extended).
    /// The EFI system partition of a removable drive is not excluded.
    pub fn is_system_partition(&self) -> bool {
        use part_types::*;
        let t = self.partition_type.as_str();
        if self.is_container {
            return true;
        }
        if let Some(hex) = t.strip_prefix("0x") {
            let Ok(n) = u8::from_str_radix(hex, 16) else { return false };
            return (n == MBR_EFI && !self.is_removable_media())
                || n == MBR_RECOVERY
                || n == MBR_SWAP
                || MBR_EXTENDED.contains(&n);
        }
        (t == EFI_SYSTEM && !self.is_removable_media())
            || [MS_RESERVED, WIN_RECOVERY, BIOS_BOOT, LINUX_SWAP, LDM_METADATA].contains(&t)
    }

    /// Never a candidate: zram / ram disks, and loop devices without a user backing
    /// file (detached, or snap / flatpak / system images).
    fn is_virtual_noise(&self) -> bool {
        let dev = self.device.to_string_lossy();
        if dev.starts_with("/dev/zram") || dev.starts_with("/dev/ram") {
            return true;
        }
        if self.is_loop || dev.starts_with("/dev/loop") {
            return match &self.loop_file {
                None => true,
                Some(f) => ["/var/lib", "/snap", "/usr", "/var/cache"].iter().any(|r| f.starts_with(r)),
            };
        }
        false
    }

    /// Sort key: the whole disk, then the partition number.
    fn position(&self) -> (String, u32, String) {
        let disk = self.partition_table.clone().unwrap_or_else(|| self.object.clone());
        (disk, self.partition_number, self.object.clone())
    }
}

/// The unmounted devices the sidebar offers to mount or unlock, in drive / partition
/// order. Mounted filesystems are not listed (they are mounted volumes already), nor
/// are system partitions, swap, RAID / LVM members, the other members of a mounted
/// multi-device filesystem (btrfs), devices UDisks marks as ignored, and `hidden` ones
/// (UUIDs or device nodes, see [`hide_key`]). `removable_only` keeps removable media.
pub fn sidebar_candidates(all: &[BlockDevice], removable_only: bool, hidden: &[String]) -> Vec<Candidate> {
    let by_object = |o: &str| all.iter().find(|d| d.object == o);
    // Filesystem UUIDs in use (a btrfs RAID mounted through one member).
    let mounted_uuids: std::collections::HashSet<&str> =
        all.iter().filter(|d| !d.mount_points.is_empty() && !d.uuid.is_empty()).map(|d| d.uuid.as_str()).collect();
    let mut out: Vec<Candidate> = Vec::new();
    for d in all {
        // An unlocked container: judged as its container (partition type, drive, label).
        let backing = d.crypto_backing.as_deref().and_then(by_object);
        let outer = backing.unwrap_or(d);
        if d.hint_ignore || outer.hint_ignore || outer.is_system_partition() || outer.is_virtual_noise() {
            continue;
        }
        if removable_only && !outer.is_removable_media() {
            continue;
        }
        let hide = hidden.iter().any(|h| {
            let h = h.trim();
            !h.is_empty()
                && [d, outer].iter().any(|x| h == x.uuid || std::path::Path::new(h) == x.device || h == x.object)
        });
        if hide {
            continue;
        }
        let state = match d.usage.as_str() {
            "filesystem" => {
                if !d.mount_points.is_empty() || (!d.uuid.is_empty() && mounted_uuids.contains(d.uuid.as_str())) {
                    continue;
                }
                let fs = d.fs_type.to_ascii_lowercase();
                if fs == "zfs_member" || fs.is_empty() {
                    continue;
                }
                if !d.has_filesystem || UNSUPPORTED_FS.contains(&fs.as_str()) {
                    CandidateState::Unsupported
                } else {
                    CandidateState::Unmounted
                }
            }
            "crypto" if d.encrypted && d.cleartext.is_none() && is_unlockable(&d.fs_type) => CandidateState::Locked,
            _ => continue,
        };
        let mut dev = d.clone();
        if let Some(b) = backing {
            if dev.label.trim().is_empty() {
                dev.label = b.label.clone();
            }
            for (field, from) in [
                (&mut dev.drive_model, &b.drive_model),
                (&mut dev.drive_vendor, &b.drive_vendor),
                (&mut dev.connection_bus, &b.connection_bus),
            ] {
                if field.is_empty() {
                    *field = from.clone();
                }
            }
            if dev.drive.is_none() {
                dev.drive = b.drive.clone();
            }
            dev.removable |= b.removable;
            dev.ejectable |= b.ejectable;
            dev.partition_table = b.partition_table.clone();
            dev.partition_number = b.partition_number;
        }
        out.push(Candidate {
            dev,
            state,
            container: backing.map(|b| b.object.clone()),
            container_type: backing.map(|b| b.fs_type.clone()).unwrap_or_default(),
        });
    }
    out.sort_by_key(|c| c.dev.position());
    out
}

/// Why a UDisks2 operation failed, from its error text (see [`classify_error`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// polkit refused (no authentication agent could ask, or the policy says no).
    NotAuthorized,
    /// The authentication prompt was dismissed.
    Dismissed,
    /// The device is in use.
    Busy,
    /// NTFS left in an unsafe state by Windows (hibernated / Fast Startup, or dirty):
    /// only a read-only mount is possible.
    WindowsHibernated,
    /// The kernel has no driver for the filesystem.
    UnsupportedFs,
    /// Unlock: the passphrase / recovery key is wrong.
    WrongPassphrase,
    /// No UDisks2 on this system.
    NoUdisks,
    /// Something else (the text is the best explanation).
    Other,
}

/// Classify a UDisks2 error (`fs_type`: the device's filesystem, for hints that are
/// only meaningful for NTFS).
pub fn classify_error(e: &io::Error, fs_type: &str) -> Failure {
    let m = e.to_string();
    let l = m.to_ascii_lowercase();
    let ntfs = fs_type.eq_ignore_ascii_case("ntfs") || fs_type.eq_ignore_ascii_case("ntfs3");
    if l.contains("notauthorizeddismissed") || l.contains("dismissed") {
        Failure::Dismissed
    } else if l.contains("notauthorized") || e.kind() == io::ErrorKind::PermissionDenied {
        Failure::NotAuthorized
    } else if l.contains("hibernat")
        || l.contains("unclean file system")
        || l.contains("volume is dirty")
        || l.contains("metadata kept in windows cache")
        || l.contains("fast restart")
        || (ntfs && (l.contains("wrong fs type") || l.contains("bad superblock") || l.contains("dirty")))
    {
        Failure::WindowsHibernated
    } else if l.contains("devicebusy") || l.contains("target is busy") || l.contains("is busy") {
        Failure::Busy
    } else if l.contains("incorrect passphrase")
        || l.contains("no key available")
        || l.contains("wrong passphrase")
        || l.contains("failed to activate device: operation not permitted")
        || (l.contains("error unlocking") && l.contains("operation not permitted"))
    {
        Failure::WrongPassphrase
    } else if l.contains("unknown filesystem type") || (l.contains("not supported") && l.contains("filesystem")) {
        Failure::UnsupportedFs
    } else if e.kind() == io::ErrorKind::Unsupported {
        Failure::NoUdisks
    } else {
        Failure::Other
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

    fn u32_(p: &Props, k: &str) -> u32 {
        match p.get(k).map(|v| &**v) {
            Some(Value::U32(n)) => *n,
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
            let part = ifaces.get(&format!("{P}Partition"));
            let lp = ifaces.get(&format!("{P}Loop"));
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
                loop_file: lp.and_then(|l| path(l, "BackingFile")),
                drive,
                drive_model: s(dp, "Model"),
                drive_vendor: s(dp, "Vendor"),
                connection_bus: s(dp, "ConnectionBus"),
                removable: b(dp, "Removable") || b(dp, "MediaRemovable"),
                ejectable: b(dp, "Ejectable"),
                can_power_off: b(dp, "CanPowerOff"),
                partition_type: part.map(|p| s(p, "Type").to_ascii_lowercase()).unwrap_or_default(),
                partition_number: part.map(|p| u32_(p, "Number")).unwrap_or(0),
                partition_table: part.and_then(|p| obj(p, "Table")),
                is_container: part.is_some_and(|p| b(p, "IsContainer")),
                is_loop: lp.is_some(),
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

    /// Any number property as `f64` (UDisks mixes `y`, `q`, `i`, `x`, `t` and `d`).
    fn num_of(v: &Value<'_>) -> Option<f64> {
        Some(match v {
            Value::U8(n) => f64::from(*n),
            Value::U16(n) => f64::from(*n),
            Value::I16(n) => f64::from(*n),
            Value::U32(n) => f64::from(*n),
            Value::I32(n) => f64::from(*n),
            Value::U64(n) => *n as f64,
            Value::I64(n) => *n as f64,
            Value::F64(n) => *n,
            Value::Value(v) => return num_of(v),
            _ => return None,
        })
    }

    fn num(p: &Props, k: &str) -> Option<f64> {
        p.get(k).and_then(|v| num_of(v))
    }

    /// The health of the drive holding the filesystem mounted at `mount` (through an
    /// unlocked LUKS volume too): `Drive.Ata` SMART (cached by udisksd, read as root) or
    /// `NVMe.Controller` plus its `SmartGetAttributes`.
    pub fn drive_health(mount: &std::path::Path) -> Option<crate::health::DiskHealth> {
        use crate::health::{Concern, DiskHealth};
        let c = conn().ok()?;
        let objs = objects(&c).ok()?;
        let iface =
            |o: &str, i: &str| objs.iter().find(|(k, _)| k.as_str() == o).and_then(|(_, m)| m.get(&format!("{P}{i}")));
        let (mut block, _) = objs.iter().find(|(_, i)| {
            i.get(&format!("{P}Filesystem")).is_some_and(|f| paths(f, "MountPoints").iter().any(|m| m == mount))
        })?;
        // An unlocked volume: its drive is the encrypted block's.
        let mut blk = iface(block.as_str(), "Block")?;
        if obj(blk, "Drive").is_none()
            && let Some(backing) = obj(blk, "CryptoBackingDevice")
            && let Some((k, _)) = objs.iter().find(|(k, _)| k.as_str() == backing)
        {
            block = k;
            blk = iface(block.as_str(), "Block")?;
        }
        let drive = obj(blk, "Drive")?;
        let d = iface(&drive, "Drive")?;
        let vendor = s(d, "Vendor");
        let model = s(d, "Model");
        let mut h = DiskHealth {
            model: if vendor.is_empty() || model.starts_with(&vendor) { model } else { format!("{vendor} {model}") },
            ..DiskHealth::default()
        };
        let kelvin = |k: f64| (k > 0.0).then(|| (k - 273.15).round() as i32);
        if let Some(a) = iface(&drive, "Drive.Ata").filter(|a| b(a, "SmartSupported") && b(a, "SmartEnabled")) {
            h.temperature_c = num(a, "SmartTemperature").and_then(kelvin);
            h.power_on_hours = num(a, "SmartPowerOnSeconds").filter(|s| *s > 0.0).map(|s| (s / 3600.0) as u64);
            if b(a, "SmartFailing") {
                h.concerns.push(Concern::PredictsFailure);
            }
            if let Some(bad) = num(a, "SmartNumBadSectors").filter(|n| *n > 0.0) {
                h.concerns.push(Concern::BadSectors(bad as u64));
            }
        } else if let Some(n) = iface(&drive, "NVMe.Controller") {
            h.nvme = true;
            h.temperature_c = num(n, "SmartTemperature").and_then(kelvin);
            h.power_on_hours = num(n, "SmartPowerOnHours").map(|v| v as u64);
            if let Some(Value::Array(w)) = n.get("SmartCriticalWarning").map(|v| &**v) {
                for x in w.iter() {
                    if let Value::Str(x) = x {
                        match x.as_str() {
                            "spare" => h.concerns.push(Concern::SpareLow),
                            "degraded" => h.concerns.push(Concern::Degraded),
                            "readonly" => h.concerns.push(Concern::ReadOnly),
                            _ => {}
                        }
                    }
                }
            }
            // The full log (no authorization needed to read it).
            let m = c.call_method(
                Some(DEST),
                drive.as_str(),
                Some(format!("{P}NVMe.Controller").as_str()),
                "SmartGetAttributes",
                &(HashMap::<&str, Value<'_>>::new(),),
            );
            if let Ok(attrs) = m.and_then(|m| m.body().deserialize::<Props>()) {
                let at = |k: &str| num(&attrs, k);
                h.percent_used = at("percent_used").map(|v| v as u8);
                h.available_spare = at("avail_spare").map(|v| v as u8);
                h.bytes_written = at("total_data_written").map(|v| v as u64);
                h.bytes_read = at("total_data_read").map(|v| v as u64);
                h.power_cycles = at("power_cycles").map(|v| v as u64);
                h.unsafe_shutdowns = at("unsafe_shutdowns").map(|v| v as u64);
                if let Some(u) = h.percent_used.filter(|u| *u >= 90) {
                    h.concerns.push(Concern::WornOut(u));
                }
                if let Some(e) = at("media_errors").filter(|e| *e > 0.0) {
                    h.concerns.push(Concern::MediaErrors(e as u64));
                }
            }
        }
        h.known().then_some(h)
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
/// The health of the drive under a mount point (`crate::health::volume_health`).
#[cfg(target_os = "linux")]
pub fn drive_health(mount: &std::path::Path) -> Option<crate::health::DiskHealth> {
    imp::drive_health(mount)
}

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

    const BD: &str = "/org/freedesktop/UDisks2/block_devices/";
    const GPT_DATA: &str = "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7";
    const GPT_LINUX: &str = "0fc63daf-8483-4772-8e79-3d69d8477de4";

    /// A partition of an internal disk (`HintSystem`), like `udisksctl dump` shows.
    #[allow(clippy::too_many_arguments)]
    fn part(name: &str, disk: &str, n: u32, ptype: &str, fs: &str, usage: &str, label: &str, gb: u64) -> BlockDevice {
        BlockDevice {
            object: format!("{BD}{name}"),
            device: format!("/dev/{name}").into(),
            label: label.into(),
            uuid: format!("uuid-{name}"),
            size: gb * 1_000_000_000,
            fs_type: fs.into(),
            usage: usage.into(),
            has_filesystem: usage == "filesystem",
            hint_system: true,
            encrypted: usage == "crypto",
            drive: Some(format!("/org/freedesktop/UDisks2/drives/{disk}")),
            drive_model: format!("Model {disk}"),
            partition_type: ptype.into(),
            partition_number: n,
            partition_table: Some(format!("{BD}{disk}")),
            ..Default::default()
        }
    }

    /// This machine's layout (lsblk / `udisksctl dump`, 2026-10).
    fn this_machine() -> Vec<BlockDevice> {
        let mut v = vec![
            part("sda1", "sda", 1, "0x07", "BitLocker", "crypto", "RBYODA Source 19.03.2026", 500),
            part("sdb1", "sdb", 1, "0x07", "ntfs", "filesystem", "Games", 1000),
            part("sdc1", "sdc", 1, part_types::MS_RESERVED, "", "", "", 0),
            part("sdc2", "sdc", 2, GPT_DATA, "ntfs", "filesystem", "GameLot", 8000),
            part("sdd1", "sdd", 1, part_types::EFI_SYSTEM, "vfat", "filesystem", "", 1),
            part("sdd2", "sdd", 2, GPT_LINUX, "ext4", "filesystem", "", 2),
            part("sdd3", "sdd", 3, GPT_LINUX, "btrfs", "filesystem", "fedora", 2000),
            part("sde1", "sde", 1, part_types::MS_RESERVED, "", "", "", 0),
            part("sde2", "sde", 2, GPT_DATA, "ntfs", "filesystem", "barn", 2000),
            part("nvme1n1p1", "nvme1n1", 1, part_types::MS_RESERVED, "", "", "", 0),
            part("nvme1n1p2", "nvme1n1", 2, GPT_DATA, "BitLocker", "crypto", "RBYODA Source2 19.03.2026", 2000),
            part("nvme2n1p1", "nvme2n1", 1, "0x07", "ntfs", "filesystem", "GameH", 1000),
            part("nvme0n1p1", "nvme0n1", 1, part_types::EFI_SYSTEM, "vfat", "filesystem", "", 0),
            part("nvme0n1p2", "nvme0n1", 2, part_types::MS_RESERVED, "", "", "", 0),
            part("nvme0n1p3", "nvme0n1", 3, GPT_DATA, "BitLocker", "crypto", "RBYODA C: 19.03.2026", 2000),
            part("nvme0n1p4", "nvme0n1", 4, part_types::WIN_RECOVERY, "ntfs", "filesystem", "", 1),
        ];
        let mount = |v: &mut Vec<BlockDevice>, name: &str, m: &[&str]| {
            let d = v.iter_mut().find(|d| d.object.ends_with(&format!("/{name}"))).unwrap();
            d.mount_points = m.iter().map(PathBuf::from).collect();
        };
        mount(&mut v, "sdd1", &["/boot/efi"]);
        mount(&mut v, "sdd2", &["/boot"]);
        mount(&mut v, "sdd3", &["/home", "/"]);
        // UDisks marks these ignored itself; the classification must not rely on it.
        for d in &mut v {
            d.hint_ignore = false;
        }
        // Whole disks (partition tables) and zram swap.
        for disk in ["sda", "sdb", "sdc", "sdd", "sde", "nvme0n1", "nvme1n1", "nvme2n1"] {
            v.push(BlockDevice {
                object: format!("{BD}{disk}"),
                device: format!("/dev/{disk}").into(),
                hint_system: true,
                ..Default::default()
            });
        }
        v.push(BlockDevice {
            object: format!("{BD}zram0"),
            device: "/dev/zram0".into(),
            fs_type: "swap".into(),
            usage: "other".into(),
            ..Default::default()
        });
        v
    }

    fn names(c: &[Candidate]) -> Vec<String> {
        c.iter().map(|c| c.dev.label.clone()).collect()
    }

    #[test]
    fn classifies_this_machine() {
        let all = this_machine();
        let c = sidebar_candidates(&all, false, &[]);
        let mut unmounted: Vec<String> =
            c.iter().filter(|c| c.state == CandidateState::Unmounted).map(|c| c.dev.label.clone()).collect();
        unmounted.sort();
        assert_eq!(unmounted, ["GameH", "GameLot", "Games", "barn"]);
        let locked: Vec<&Candidate> = c.iter().filter(|c| c.state == CandidateState::Locked).collect();
        assert_eq!(locked.len(), 3, "{:?}", names(&c));
        assert!(locked.iter().all(|c| fs_display_name(&c.dev.fs_type) == "BitLocker"));
        for gone in
            ["sdc1", "sde1", "nvme1n1p1", "nvme0n1p1", "nvme0n1p2", "nvme0n1p4", "sdd1", "sdd2", "sdd3", "zram0"]
        {
            assert!(!c.iter().any(|c| c.dev.object.ends_with(&format!("/{gone}"))), "{gone} must not be offered");
        }
        assert!(!c.iter().any(|c| c.dev.object.ends_with("/sda")), "whole disks are not candidates");
        // Drive then partition order (object path order of the disks).
        let order: Vec<&str> = c.iter().map(|c| c.dev.object.rsplit('/').next().unwrap()).collect();
        assert_eq!(order, ["nvme0n1p3", "nvme1n1p2", "nvme2n1p1", "sda1", "sdb1", "sdc2", "sde2"]);
        // Only removable media: none here.
        assert!(sidebar_candidates(&all, true, &[]).is_empty());
        // Hidden by UUID or by device node.
        let hidden = ["uuid-sdb1".to_string(), "/dev/sde2".to_string()];
        let c = sidebar_candidates(&all, false, &hidden);
        assert!(!names(&c).contains(&"Games".to_string()) && !names(&c).contains(&"barn".to_string()));
        assert_eq!(c.len(), 5);
    }

    #[test]
    fn unlocked_containers_show_their_cleartext() {
        let mut all = this_machine();
        let c3 = all.iter_mut().find(|d| d.object.ends_with("/nvme0n1p3")).unwrap();
        c3.cleartext = Some(format!("{BD}dm_2d0"));
        all.push(BlockDevice {
            object: format!("{BD}dm_2d0"),
            device: "/dev/mapper/bitlk-abc".into(),
            uuid: "ntfs-inside".into(),
            size: 1_999_000_000_000,
            fs_type: "ntfs".into(),
            usage: "filesystem".into(),
            has_filesystem: true,
            hint_system: true,
            crypto_backing: Some(format!("{BD}nvme0n1p3")),
            ..Default::default()
        });
        let c = sidebar_candidates(&all, false, &[]);
        assert_eq!(c.iter().filter(|c| c.state == CandidateState::Locked).count(), 2);
        let open = c.iter().find(|c| c.dev.object.ends_with("/dm_2d0")).unwrap();
        assert_eq!(open.state, CandidateState::Unmounted);
        assert_eq!(open.dev.label, "RBYODA C: 19.03.2026", "label of the container");
        assert_eq!(open.dev.drive_model, "Model nvme0n1");
        assert_eq!(open.container.as_deref(), Some(format!("{BD}nvme0n1p3").as_str()));
        assert_eq!(fs_display_name(&open.container_type), "BitLocker");
        assert_eq!(c[0].dev.object, open.dev.object, "keeps the container's position");
        // Mounted cleartext: listed as a volume, not here.
        all.last_mut().unwrap().mount_points = vec!["/run/media/u/C".into()];
        assert!(!sidebar_candidates(&all, false, &[]).iter().any(|c| c.dev.object.ends_with("/dm_2d0")));
    }

    #[test]
    fn exclusions_and_flags() {
        let mut usb = part("sdx1", "sdx", 1, part_types::EFI_SYSTEM, "vfat", "filesystem", "BOOTSTICK", 8);
        usb.connection_bus = "usb".into();
        usb.hint_system = false;
        let mut mbr_esp = part("sdy1", "sdy", 1, "0xef", "vfat", "filesystem", "", 1);
        mbr_esp.hint_system = true;
        let winre = part("sdy2", "sdy", 2, "0x27", "ntfs", "filesystem", "", 1);
        let ext = BlockDevice { is_container: true, ..part("sdy3", "sdy", 3, "0x05", "", "", "", 0) };
        let swap = part("sdy5", "sdy", 5, part_types::LINUX_SWAP, "swap", "other", "", 8);
        let lvm = part("sdy6", "sdy", 6, GPT_LINUX, "LVM2_member", "raid", "", 100);
        let apfs =
            BlockDevice { has_filesystem: false, ..part("sdz1", "sdz", 1, GPT_DATA, "apfs", "filesystem", "Mac", 500) };
        let refs = part("sdz2", "sdz", 2, GPT_DATA, "refs", "filesystem", "Pool", 500);
        let mut btrfs_a = part("sdw1", "sdw", 1, GPT_LINUX, "btrfs", "filesystem", "raid", 500);
        btrfs_a.uuid = "same".into();
        btrfs_a.mount_points = vec!["/data".into()];
        let mut btrfs_b = part("sdv1", "sdv", 1, GPT_LINUX, "btrfs", "filesystem", "raid", 500);
        btrfs_b.uuid = "same".into();
        let snap = BlockDevice {
            is_loop: true,
            loop_file: Some("/var/lib/snapd/snaps/core.snap".into()),
            ..part("loop0", "loop0", 0, "", "squashfs", "filesystem", "", 1)
        };
        let iso = BlockDevice {
            is_loop: true,
            loop_file: Some("/home/u/Downloads/fedora.iso".into()),
            partition_table: None,
            ..part("loop1", "loop1", 0, "", "iso9660", "filesystem", "Fedora", 2)
        };
        let ignored =
            BlockDevice { hint_ignore: true, ..part("sdu1", "sdu", 1, GPT_DATA, "ntfs", "filesystem", "X", 1) };
        let luks = part("sdt1", "sdt", 1, GPT_LINUX, "crypto_LUKS", "crypto", "", 100);
        let tcrypt = part("sdt2", "sdt", 2, GPT_LINUX, "crypto_TCRYPT", "crypto", "", 100);
        let all = [usb, mbr_esp, winre, ext, swap, lvm, apfs, refs, btrfs_a, btrfs_b, snap, iso, ignored, luks, tcrypt];
        let c = sidebar_candidates(&all, false, &[]);
        let got: Vec<(&str, CandidateState)> =
            c.iter().map(|c| (c.dev.object.rsplit('/').next().unwrap(), c.state)).collect();
        use CandidateState::*;
        assert_eq!(
            got,
            [("loop1", Unmounted), ("sdt1", Locked), ("sdx1", Unmounted), ("sdz1", Unsupported), ("sdz2", Unsupported),]
        );
        let removable = sidebar_candidates(&all, true, &[]);
        assert_eq!(removable.len(), 1);
        assert_eq!(removable[0].dev.label, "BOOTSTICK");
    }

    #[test]
    fn display_names_and_errors() {
        assert_eq!(fs_display_name("ntfs"), "NTFS");
        assert_eq!(fs_display_name("BitLocker"), "BitLocker");
        assert_eq!(fs_display_name("crypto_LUKS"), "LUKS");
        assert_eq!(fs_display_name("vfat"), "FAT");
        assert_eq!(fs_display_name("ext4"), "ext4");
        let d = BlockDevice { device: "/dev/sdb1".into(), ..Default::default() };
        assert_eq!(hide_key(&d), "/dev/sdb1");
        let e = |m: &str| io::Error::other(m.to_string());
        use Failure::*;
        let cases = [
            (
                "UDisks2: org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain: Not authorized to perform operation",
                "ntfs",
                NotAuthorized,
            ),
            (
                "UDisks2: org.freedesktop.UDisks2.Error.NotAuthorizedDismissed: The authentication dialog was dismissed",
                "ntfs",
                Dismissed,
            ),
            (
                "Error mounting /dev/sdb1 at /run/media/u/Games: Windows is hibernated, refused to mount.",
                "ntfs",
                WindowsHibernated,
            ),
            (
                "The disk contains an unclean file system (0, 0). Metadata kept in Windows cache, refused to mount.",
                "ntfs",
                WindowsHibernated,
            ),
            (
                "Error mounting /dev/sdb1: wrong fs type, bad option, bad superblock on /dev/sdb1, missing codepage or helper program, or other error",
                "ntfs",
                WindowsHibernated,
            ),
            ("Error mounting /dev/sdb1: wrong fs type, bad option, bad superblock on /dev/sdb1", "ext4", Other),
            ("org.freedesktop.UDisks2.Error.DeviceBusy: Error unmounting: target is busy", "ext4", Busy),
            (
                "org.freedesktop.UDisks2.Error.Failed: Error unlocking /dev/sda1: Failed to activate device: Operation not permitted",
                "BitLocker",
                WrongPassphrase,
            ),
            (
                "Error unlocking /dev/sdt1: Failed to activate device: Incorrect passphrase.",
                "crypto_LUKS",
                WrongPassphrase,
            ),
            ("Error mounting /dev/sdz1: unknown filesystem type 'apfs'", "apfs", UnsupportedFs),
            ("weird", "ext4", Other),
        ];
        for (m, fs, want) in cases {
            assert_eq!(classify_error(&e(m), fs), want, "{m}");
        }
        assert_eq!(classify_error(&io::Error::new(io::ErrorKind::Unsupported, "no bus"), ""), NoUdisks);
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
        println!("-- sidebar candidates --");
        for c in sidebar_candidates(&devs, false, &[]) {
            println!(
                "{:?} {:?} {} label={:?} size={} part={} {} drive={:?} removable={}",
                c.state,
                c.dev.device,
                fs_display_name(&c.dev.fs_type),
                c.dev.label,
                c.dev.size,
                c.dev.partition_number,
                c.dev.partition_type,
                c.dev.drive_model,
                c.dev.is_removable_media()
            );
        }
    }
}

//! Disk health (SMART / NVMe health log) for the drive cards, read without administrator
//! or root rights:
//!
//! * Windows: `FILE_ANY_ACCESS` IOCTLs on the volume's physical disks
//!   (`sys::windows::health`): the NVMe health log, the temperature, the drive's failure
//!   prediction and the ATA SMART attribute table;
//! * Linux: UDisks2 (`org.freedesktop.UDisks2.Drive.Ata` / `NVMe.Controller`, which the
//!   daemon reads as root and caches) through [`crate::udisks`];
//! * elsewhere: nothing (`None`).
//!
//! [`DiskHealth::verdict`] sums it up: *failing* when the drive predicts its failure or
//! the NVMe critical warning says the spare ran out, reliability degraded or it went
//! read-only; *warning* for worn-out flash (≥ 90 % used), reallocated / pending /
//! uncorrectable sectors, media errors or a temperature over the drive's warning level.

use std::path::Path;

/// What a disk told about itself, before parsing (one per physical disk).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawHealth {
    /// Vendor and product.
    pub model: String,
    /// Windows `STORAGE_BUS_TYPE` (0 when unknown).
    pub bus: u32,
    /// The NVMe SMART / health information log page (page 2).
    pub nvme_log: Option<[u8; 512]>,
    /// The ATA SMART data (attribute table at bytes 2..362).
    pub ata_attributes: Option<[u8; 512]>,
    /// The drive predicts its own failure (ATA SMART status).
    pub predicts_failure: Option<bool>,
    pub temperature_c: Option<i32>,
    pub temperature_warning_c: Option<i32>,
}

/// How a disk is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Good,
    Warning,
    Failing,
}

/// Why a disk is not [`Verdict::Good`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Concern {
    /// The drive predicts its failure (SMART status).
    PredictsFailure,
    /// NVMe: the spare capacity fell under its threshold.
    SpareLow,
    /// NVMe: reliability degraded (media or internal errors).
    Degraded,
    /// NVMe: the drive went read-only.
    ReadOnly,
    /// Flash wear (percent of the rated endurance used).
    WornOut(u8),
    /// Sectors remapped, waiting to be remapped or unreadable.
    BadSectors(u64),
    /// NVMe media and data integrity errors.
    MediaErrors(u64),
    /// Hotter than the drive's warning level.
    Hot(i32),
}

impl Concern {
    fn severity(&self) -> Verdict {
        match self {
            Concern::PredictsFailure | Concern::SpareLow | Concern::Degraded | Concern::ReadOnly => Verdict::Failing,
            _ => Verdict::Warning,
        }
    }
}

/// One physical disk's health.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiskHealth {
    pub model: String,
    pub nvme: bool,
    pub temperature_c: Option<i32>,
    /// Flash wear: percent of the rated endurance used (may pass 100).
    pub percent_used: Option<u8>,
    /// NVMe: spare capacity left (%).
    pub available_spare: Option<u8>,
    pub power_on_hours: Option<u64>,
    pub power_cycles: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    /// Bytes written over the drive's life.
    pub bytes_written: Option<u64>,
    pub bytes_read: Option<u64>,
    pub concerns: Vec<Concern>,
}

impl DiskHealth {
    pub fn verdict(&self) -> Verdict {
        self.concerns.iter().map(Concern::severity).max().unwrap_or(Verdict::Good)
    }

    /// Whether anything at all was learned (else the drive does not report health).
    pub fn known(&self) -> bool {
        self.temperature_c.is_some()
            || self.percent_used.is_some()
            || self.power_on_hours.is_some()
            || !self.concerns.is_empty()
            || self.bytes_written.is_some()
    }
}

fn le(b: &[u8]) -> u128 {
    b.iter().rev().fold(0u128, |a, &x| (a << 8) | u128::from(x))
}

fn sat(v: u128) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

/// Parse the NVMe SMART / health log (NVMe base spec, log page 02h).
pub fn parse_nvme(log: &[u8; 512]) -> DiskHealth {
    let mut h = DiskHealth { nvme: true, ..DiskHealth::default() };
    let warn = log[0];
    let kelvin = u16::from_le_bytes([log[1], log[2]]);
    if kelvin > 0 {
        h.temperature_c = Some(i32::from(kelvin) - 273);
    }
    h.available_spare = Some(log[3]);
    let spare_threshold = log[4];
    h.percent_used = Some(log[5]);
    // Data units are thousands of 512-byte blocks.
    h.bytes_read = Some(sat(le(&log[32..48]).saturating_mul(512_000)));
    h.bytes_written = Some(sat(le(&log[48..64]).saturating_mul(512_000)));
    h.power_cycles = Some(sat(le(&log[112..128])));
    h.power_on_hours = Some(sat(le(&log[128..144])));
    h.unsafe_shutdowns = Some(sat(le(&log[144..160])));
    let media = sat(le(&log[160..176]));
    if warn & 0x01 != 0 || (spare_threshold > 0 && log[3] < spare_threshold) {
        h.concerns.push(Concern::SpareLow);
    }
    if warn & 0x04 != 0 {
        h.concerns.push(Concern::Degraded);
    }
    if warn & 0x08 != 0 {
        h.concerns.push(Concern::ReadOnly);
    }
    if log[5] >= 90 {
        h.concerns.push(Concern::WornOut(log[5]));
    }
    if media > 0 {
        h.concerns.push(Concern::MediaErrors(media));
    }
    h
}

/// Parse the ATA SMART attribute table (30 entries of 12 bytes from byte 2: id, flags,
/// value, worst, 6 raw bytes, reserved).
///
/// `gib_units`: the drive counts attributes 241 / 242 in GiB instead of 512-byte sectors
/// ([`ata_gib_units`]).
pub fn parse_ata(data: &[u8; 512], gib_units: bool) -> DiskHealth {
    let unit: u64 = if gib_units { 1 << 30 } else { 512 };
    let mut h = DiskHealth::default();
    let mut bad = 0u64;
    for e in data[2..362].as_chunks::<12>().0 {
        let id = e[0];
        if id == 0 {
            continue;
        }
        let value = e[3];
        let raw = le(&e[5..11]) as u64;
        let raw16 = u64::from(u16::from_le_bytes([e[5], e[6]]));
        match id {
            // Reallocated, pending, offline uncorrectable sectors.
            5 | 197 | 198 => bad = bad.saturating_add(raw & 0xffff_ffff),
            // Power-on hours (the low 32 bits; some drives pack minutes above).
            9 => h.power_on_hours = Some(raw & 0xffff_ffff),
            12 => h.power_cycles = Some(raw & 0xffff_ffff),
            // Temperature: the low byte of the raw value.
            194 | 190 if h.temperature_c.is_none() => {
                let t = (raw & 0xff) as i32;
                if (1..120).contains(&t) {
                    h.temperature_c = Some(t);
                }
            }
            // SSD life left (normalized: 100 = new).
            231 | 202 if h.percent_used.is_none() && value <= 100 => h.percent_used = Some(100 - value),
            // Wear leveling count (Samsung): normalized life left.
            177 if h.percent_used.is_none() && value <= 100 => h.percent_used = Some(100 - value),
            // Total LBAs written (512-byte sectors on most drives).
            241 => h.bytes_written = Some(raw.saturating_mul(unit)),
            242 => h.bytes_read = Some(raw.saturating_mul(unit)),
            174 => h.unsafe_shutdowns = Some(raw16),
            _ => {}
        }
    }
    if bad > 0 {
        h.concerns.push(Concern::BadSectors(bad));
    }
    if let Some(u) = h.percent_used.filter(|u| *u >= 90) {
        h.concerns.push(Concern::WornOut(u));
    }
    h
}

/// SSDs whose "total LBAs written" counts GiB (SanDisk, WD SATA, Kingston, Transcend), by
/// their model name; others count 512-byte sectors.
pub fn ata_gib_units(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    ["sandisk", "wdc wds", "wd blue sa", "kingston", "transcend"].iter().any(|p| m.starts_with(p))
}

/// Combine what a disk reported.
pub fn from_raw(raw: &RawHealth) -> DiskHealth {
    let mut h = match (&raw.nvme_log, &raw.ata_attributes) {
        (Some(log), _) => parse_nvme(log),
        (None, Some(ata)) => parse_ata(ata, ata_gib_units(&raw.model)),
        (None, None) => DiskHealth::default(),
    };
    h.model = raw.model.clone();
    if raw.temperature_c.is_some() {
        h.temperature_c = raw.temperature_c;
    }
    if raw.predicts_failure == Some(true) {
        h.concerns.insert(0, Concern::PredictsFailure);
    }
    if let (Some(t), Some(w)) = (h.temperature_c, raw.temperature_warning_c)
        && t >= w
    {
        h.concerns.push(Concern::Hot(t));
    }
    h
}

/// The health of the physical disks under the volume mounted at `root` (one for a plain
/// partition, several for a spanned or mirrored volume). `None` when the system can't
/// tell (network, removable through a bridge without SMART, unsupported platform).
/// Blocking (device I/O): call it from a worker.
pub fn volume_health(root: &Path) -> Option<Vec<DiskHealth>> {
    #[cfg(windows)]
    {
        let disks = crate::sys::windows::health::disks_of_volume(root).ok()?;
        let out: Vec<DiskHealth> = disks
            .into_iter()
            .filter_map(|n| crate::sys::windows::health::disk_health(n).ok())
            .map(|r| from_raw(&r))
            .filter(DiskHealth::known)
            .collect();
        (!out.is_empty()).then_some(out)
    }
    #[cfg(target_os = "linux")]
    {
        crate::udisks::drive_health(root).map(|h| vec![h])
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = root;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nvme_log() -> [u8; 512] {
        let mut l = [0u8; 512];
        l[1..3].copy_from_slice(&(273u16 + 41).to_le_bytes());
        l[3] = 100;
        l[4] = 10;
        l[5] = 3;
        l[48..56].copy_from_slice(&20_000_000u64.to_le_bytes()); // 10.24 TB written
        l[112] = 42;
        l[128..136].copy_from_slice(&5000u64.to_le_bytes());
        l[144] = 7;
        l
    }

    #[test]
    fn nvme_health_log() {
        let h = parse_nvme(&nvme_log());
        assert_eq!(h.temperature_c, Some(41));
        assert_eq!(h.percent_used, Some(3));
        assert_eq!(h.available_spare, Some(100));
        assert_eq!(h.bytes_written, Some(10_240_000_000_000));
        assert_eq!((h.power_cycles, h.power_on_hours, h.unsafe_shutdowns), (Some(42), Some(5000), Some(7)));
        assert_eq!(h.verdict(), Verdict::Good);
        let mut l = nvme_log();
        l[0] = 0x04;
        l[5] = 95;
        l[160] = 2;
        let h = parse_nvme(&l);
        assert_eq!(h.concerns, vec![Concern::Degraded, Concern::WornOut(95), Concern::MediaErrors(2)]);
        assert_eq!(h.verdict(), Verdict::Failing);
        let mut l = nvme_log();
        l[3] = 5;
        assert_eq!(parse_nvme(&l).concerns, vec![Concern::SpareLow]);
    }

    fn attr(t: &mut [u8; 512], slot: usize, id: u8, value: u8, raw: u64) {
        let at = 2 + slot * 12;
        t[at] = id;
        t[at + 3] = value;
        t[at + 5..at + 11].copy_from_slice(&raw.to_le_bytes()[..6]);
    }

    #[test]
    fn ata_attributes_and_prediction() {
        let mut t = [0u8; 512];
        attr(&mut t, 0, 9, 99, 12_345);
        attr(&mut t, 1, 194, 70, 0x0028_0012_001e); // 30 °C (min / max packed above)
        attr(&mut t, 2, 241, 99, 1_000_000);
        attr(&mut t, 3, 5, 100, 0);
        attr(&mut t, 4, 231, 97, 0);
        let h = parse_ata(&t, false);
        assert_eq!(h.power_on_hours, Some(12_345));
        assert_eq!(h.temperature_c, Some(30));
        assert_eq!(h.bytes_written, Some(512_000_000));
        assert_eq!(parse_ata(&t, true).bytes_written, Some(1_000_000 << 30));
        assert!(ata_gib_units("SanDisk SSD PLUS 2000GB") && !ata_gib_units("Samsung SSD 860 EVO 1TB"));
        assert_eq!(h.percent_used, Some(3));
        assert_eq!(h.verdict(), Verdict::Good);
        attr(&mut t, 3, 5, 100, 8);
        attr(&mut t, 5, 197, 100, 2);
        let raw = RawHealth {
            model: "Disk".into(),
            ata_attributes: Some(t),
            predicts_failure: Some(true),
            temperature_c: Some(61),
            temperature_warning_c: Some(60),
            ..RawHealth::default()
        };
        let h = from_raw(&raw);
        assert_eq!(h.concerns, vec![Concern::PredictsFailure, Concern::BadSectors(10), Concern::Hot(61)]);
        assert_eq!(h.verdict(), Verdict::Failing);
        assert!(!from_raw(&RawHealth::default()).known());
    }

    /// The health of this machine's drives
    /// (`cargo test -p soyuz-fs --lib health::tests::probe -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn probe() {
        for v in crate::volumes() {
            #[cfg(windows)]
            {
                let disks = crate::sys::windows::health::disks_of_volume(&v.mount);
                println!("{}: disks {disks:?}", v.mount.display());
                for n in disks.unwrap_or_default() {
                    let r = crate::sys::windows::health::disk_health(n);
                    match r {
                        Ok(r) => println!(
                            "  #{n} {} bus {} nvme {} ata {} predict {:?} temp {:?}/{:?}",
                            r.model,
                            r.bus,
                            r.nvme_log.is_some(),
                            r.ata_attributes.is_some(),
                            r.predicts_failure,
                            r.temperature_c,
                            r.temperature_warning_c
                        ),
                        Err(e) => println!("  #{n} {e}"),
                    }
                }
            }
            println!("  => {:?}", volume_health(&v.mount));
        }
    }
}

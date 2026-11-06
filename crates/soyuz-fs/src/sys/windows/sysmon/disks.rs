//! Physical disks: `FILE_ANY_ACCESS` IOCTLs on `\\.\PhysicalDriveN` opened with no access
//! rights, so no administrator rights. The handle is opened per reading: a handle kept open
//! on a device can get in the way of safely removing it.

use std::io;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DISK_PERFORMANCE, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_PERFORMANCE, IOCTL_STORAGE_QUERY_PROPERTY,
    PropertyStandardQuery, STORAGE_PROPERTY_QUERY, StorageDeviceProperty, StorageDeviceSeekPenaltyProperty,
    StorageDeviceTemperatureProperty,
};

use super::super::{Handle, last_err, wide_str};
use super::{Wrap32, read_pod, ticks_to_ms, u32_at};
use crate::sysmon::{DiskRaw, SensorKind, SensorRaw};

/// Disk numbers tried on a scan.
const MAX_DISKS: u32 = 64;
const SCAN_EVERY: Duration = Duration::from_secs(15);
const TEMPERATURE_EVERY: Duration = Duration::from_secs(30);

/// Offset of `DiskSize` in `DISK_GEOMETRY_EX` (after the 24-byte `DISK_GEOMETRY`).
const GEOMETRY_DISK_SIZE: usize = 24;

struct Known {
    number: u32,
    id: String,
    model: String,
    rotational: Option<bool>,
    size: Option<u64>,
    /// The first busy reading, so `busy_ms` counts from the time this reader started.
    busy_base: Option<i64>,
    temperature: Option<Temperature>,
    temperature_at: Option<Instant>,
    read_ops: Wrap32,
    write_ops: Wrap32,
}

#[derive(Clone, Copy)]
struct Temperature {
    celsius: f32,
    warning: Option<f32>,
    critical: Option<f32>,
}

#[derive(Default)]
pub(super) struct DiskReader {
    known: Vec<Known>,
    scanned: Option<Instant>,
}

fn open(number: u32) -> io::Result<Handle> {
    let path = wide_str(format!(r"\\.\PhysicalDrive{number}"));
    // SAFETY: NUL-terminated device path; no security attributes or template.
    Handle::new(unsafe {
        CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut())
    })
}

fn ioctl(h: &Handle, code: u32, input: &[u8], out: &mut [u8]) -> io::Result<usize> {
    let mut got = 0u32;
    // SAFETY: valid handle; buffers with the lengths we pass; synchronous call.
    let ok = unsafe {
        DeviceIoControl(
            h.raw(),
            code,
            if input.is_empty() { null() } else { input.as_ptr().cast() },
            input.len() as u32,
            out.as_mut_ptr().cast(),
            out.len() as u32,
            &mut got,
            null_mut(),
        )
    };
    if ok == 0 { Err(last_err()) } else { Ok((got as usize).min(out.len())) }
}

fn property(h: &Handle, id: i32, out: &mut [u8]) -> io::Result<usize> {
    let q = STORAGE_PROPERTY_QUERY { PropertyId: id, QueryType: PropertyStandardQuery, AdditionalParameters: [0] };
    // SAFETY: a plain `#[repr(C)]` struct of integers viewed as bytes for the IOCTL input.
    let input = unsafe { std::slice::from_raw_parts((&raw const q).cast::<u8>(), size_of::<STORAGE_PROPERTY_QUERY>()) };
    ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, input, out)
}

fn performance(h: &Handle) -> Option<DISK_PERFORMANCE> {
    let mut out = vec![0u8; size_of::<DISK_PERFORMANCE>()];
    let got = ioctl(h, IOCTL_DISK_PERFORMANCE, &[], &mut out).ok()?;
    read_pod::<DISK_PERFORMANCE>(&out[..got], 0)
}

fn model(h: &Handle) -> String {
    let mut out = vec![0u8; 1024];
    let Ok(got) = property(h, StorageDeviceProperty, &mut out) else { return String::new() };
    model_of_descriptor(&out[..got])
}

/// Vendor and product from a `STORAGE_DEVICE_DESCRIPTOR` (offsets at 12 and 16).
fn model_of_descriptor(d: &[u8]) -> String {
    let text = |at: usize| -> String {
        let off = u32_at(d, at).unwrap_or(0) as usize;
        if off == 0 || off >= d.len() {
            return String::new();
        }
        let end = d[off..].iter().position(|&c| c == 0).map_or(d.len(), |e| off + e);
        String::from_utf8_lossy(&d[off..end]).trim().to_string()
    };
    let (vendor, product) = (text(12), text(16));
    if vendor.is_empty() || product.starts_with(&vendor) { product } else { format!("{vendor} {product}") }
}

fn rotational(h: &Handle) -> Option<bool> {
    // DEVICE_SEEK_PENALTY_DESCRIPTOR: Version, Size, IncursSeekPenalty (a byte at 8).
    let mut out = [0u8; 12];
    let got = property(h, StorageDeviceSeekPenaltyProperty, &mut out).ok()?;
    (got >= 9).then_some(out[8] != 0)
}

fn size(h: &Handle) -> Option<u64> {
    let mut out = [0u8; 256];
    let got = ioctl(h, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut out).ok()?;
    read_pod::<u64>(&out[..got], GEOMETRY_DISK_SIZE).filter(|&s| s > 0)
}

/// `STORAGE_TEMPERATURE_DATA_DESCRIPTOR`: critical at 8, warning at 10, info count at 12,
/// then `STORAGE_TEMPERATURE_INFO[]` at 24 with the temperature 2 bytes in.
fn parse_temperature(d: &[u8]) -> Option<Temperature> {
    let at = |off| read_pod::<i16>(d, off);
    if at(12).unwrap_or(0) <= 0 {
        return None;
    }
    let plausible = |t: &i16| (1..200).contains(t);
    Some(Temperature {
        celsius: f32::from(at(26).filter(|t| (-60..200).contains(t))?),
        warning: at(10).filter(plausible).map(f32::from),
        critical: at(8).filter(plausible).map(f32::from),
    })
}

fn temperature(h: &Handle) -> Option<Temperature> {
    let mut out = [0u8; 512];
    let got = property(h, StorageDeviceTemperatureProperty, &mut out).ok()?;
    parse_temperature(&out[..got])
}

impl DiskReader {
    fn scan(&mut self) {
        let mut known = Vec::new();
        for number in 0..MAX_DISKS {
            let Ok(h) = open(number) else { continue };
            // Counters and caches of a disk seen before carry over.
            if let Some(i) = self.known.iter().position(|k| k.number == number) {
                known.push(self.known.swap_remove(i));
                continue;
            }
            let size = size(&h);
            // A card reader without a card: no size and no counters.
            if size.is_none() && performance(&h).is_none() {
                continue;
            }
            known.push(Known {
                number,
                id: format!("PhysicalDrive{number}"),
                model: model(&h),
                rotational: rotational(&h),
                size,
                busy_base: None,
                temperature: None,
                temperature_at: None,
                read_ops: Wrap32::default(),
                write_ops: Wrap32::default(),
            });
        }
        self.known = known;
        self.scanned = Some(Instant::now());
    }

    pub(super) fn read(&mut self, hardware: bool) -> Vec<DiskRaw> {
        if self.scanned.is_none_or(|t| t.elapsed() >= SCAN_EVERY) {
            self.scan();
        }
        let mut out = Vec::with_capacity(self.known.len());
        let mut lost = false;
        for k in &mut self.known {
            let Ok(h) = open(k.number) else {
                lost = true;
                continue;
            };
            let mut d = DiskRaw {
                id: k.id.clone(),
                model: k.model.clone(),
                rotational: k.rotational,
                size: k.size,
                ..DiskRaw::default()
            };
            if let Some(p) = performance(&h) {
                d.read_bytes = p.BytesRead.max(0) as u64;
                d.write_bytes = p.BytesWritten.max(0) as u64;
                d.read_ops = k.read_ops.push(p.ReadCount);
                d.write_ops = k.write_ops.push(p.WriteCount);
                d.queue = Some(p.QueueDepth as f32);
                // Time not idle: both clocks tick in 100 ns.
                let busy = p.QueryTime.saturating_sub(p.IdleTime);
                let base = *k.busy_base.get_or_insert(busy);
                d.busy_ms = Some(ticks_to_ms(busy.saturating_sub(base)));
            }
            // Solid-state only: asking a sleeping spinning disk could wake it.
            if hardware
                && k.rotational == Some(false)
                && k.temperature_at.is_none_or(|t| t.elapsed() >= TEMPERATURE_EVERY)
            {
                k.temperature = temperature(&h);
                k.temperature_at = Some(Instant::now());
            }
            out.push(d);
        }
        if lost {
            self.scanned = None;
        }
        out
    }

    pub(super) fn sensors(&self) -> Vec<SensorRaw> {
        self.known
            .iter()
            .filter_map(|k| {
                let t = k.temperature?;
                Some(SensorRaw {
                    id: format!("disk:{}:temp", k.id),
                    group: if k.model.is_empty() { k.id.clone() } else { k.model.clone() },
                    label: "Temperature".to_string(),
                    kind: SensorKind::Temperature,
                    value: t.celsius,
                    max: t.warning,
                    critical: t.critical,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_text_is_bounds_checked() {
        let mut d = vec![0u8; 64];
        d[12] = 40;
        d[16] = 48;
        d[40..44].copy_from_slice(b"ACME");
        d[48..54].copy_from_slice(b"SSD 1 ");
        assert_eq!(model_of_descriptor(&d), "ACME SSD 1");
        d[12] = 0;
        assert_eq!(model_of_descriptor(&d), "SSD 1");
        d[16] = 200;
        assert_eq!(model_of_descriptor(&d), "");
        assert_eq!(model_of_descriptor(&[]), "");
    }

    #[test]
    fn temperature_descriptor() {
        let mut d = vec![0u8; 40];
        assert!(parse_temperature(&d).is_none(), "no entries");
        d[8] = 85;
        d[10] = 70;
        d[12] = 1;
        d[26] = 42;
        let t = parse_temperature(&d).unwrap();
        assert_eq!((t.celsius, t.warning, t.critical), (42.0, Some(70.0), Some(85.0)));
        assert!(parse_temperature(&d[..20]).is_none(), "truncated");
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn performance_struct_is_as_documented() {
        assert_eq!(size_of::<DISK_PERFORMANCE>(), 88);
    }
}

//! Disk health without administrator rights: every query here is a `FILE_ANY_ACCESS`
//! IOCTL on a handle opened with no access rights (`\\.\C:` → its physical disks via
//! `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`, then `\\.\PhysicalDriveN`):
//!
//! * `IOCTL_STORAGE_QUERY_PROPERTY` / `StorageDeviceProperty`: model and bus;
//! * … / `StorageDeviceProtocolSpecificProperty`: the NVMe SMART / health log (page 2);
//! * … / `StorageDeviceTemperatureProperty`: the temperature and its warning level;
//! * `IOCTL_STORAGE_PREDICT_FAILURE`: the drive's own failure prediction, with the ATA
//!   SMART attribute table in its vendor bytes.
//!
//! Each part fails on its own (a USB bridge, a RAID, a virtual disk answer some or none);
//! parsing is portable (`crate::health`).

use std::io;
use std::mem::size_of;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    IOCTL_STORAGE_PREDICT_FAILURE, IOCTL_STORAGE_QUERY_PROPERTY, NVMeDataTypeLogPage, PropertyStandardQuery,
    ProtocolTypeNvme, STORAGE_PREDICT_FAILURE, STORAGE_PROPERTY_QUERY, STORAGE_PROTOCOL_DATA_DESCRIPTOR,
    STORAGE_PROTOCOL_SPECIFIC_DATA, StorageDeviceProperty, StorageDeviceProtocolSpecificProperty,
    StorageDeviceTemperatureProperty,
};

use super::{Handle, last_err, wide_str};
use crate::health::RawHealth;

/// Open a device for IOCTLs only (no read or write access, so no admin rights).
fn open_device(path: &[u16]) -> io::Result<Handle> {
    // SAFETY: NUL-terminated device path; no security attributes or template.
    Handle::new(unsafe {
        CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut())
    })
}

/// `DeviceIoControl` with byte buffers; returns the bytes written.
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
    if ok == 0 { Err(last_err()) } else { Ok(got as usize) }
}

/// The bytes of a plain-old-data struct.
fn bytes_of<T: Copy>(v: &T) -> Vec<u8> {
    // SAFETY: `T` is a `Copy` `#[repr(C)]` Windows struct; reading its bytes is fine.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), size_of::<T>()) }.to_vec()
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn i16_at(b: &[u8], at: usize) -> Option<i16> {
    b.get(at..at + 2).map(|s| i16::from_le_bytes([s[0], s[1]]))
}

/// The physical disk numbers under a volume root (`C:\` or a mount folder).
pub(crate) fn disks_of_volume(root: &Path) -> io::Result<Vec<u32>> {
    let h = open_device(&super::usn::device_path(root)?)?;
    let mut out = vec![0u8; 8 + 24 * 32];
    let n = ioctl(&h, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], &mut out)?;
    // VOLUME_DISK_EXTENTS: count (u32), padding, then DISK_EXTENT { u32 disk, pad, i64, i64 }.
    let count = u32_at(&out, 0) as usize;
    let mut disks: Vec<u32> =
        (0..count).map(|i| 8 + i * 24).filter(|at| at + 4 <= n).map(|at| u32_at(&out, at)).collect();
    disks.dedup();
    Ok(disks)
}

fn query(prop: i32) -> STORAGE_PROPERTY_QUERY {
    STORAGE_PROPERTY_QUERY { PropertyId: prop, QueryType: PropertyStandardQuery, AdditionalParameters: [0] }
}

/// What disk `n` tells about itself.
pub(crate) fn disk_health(n: u32) -> io::Result<RawHealth> {
    let h = open_device(&wide_str(format!(r"\\.\PhysicalDrive{n}")))?;
    let mut raw = RawHealth::default();
    // Model and bus.
    let mut out = vec![0u8; 1024];
    if let Ok(got) = ioctl(&h, IOCTL_STORAGE_QUERY_PROPERTY, &bytes_of(&query(StorageDeviceProperty)), &mut out) {
        let d = &out[..got];
        // STORAGE_DEVICE_DESCRIPTOR: … VendorIdOffset @12, ProductIdOffset @16, BusType @28.
        let text = |off: u32| -> String {
            let off = off as usize;
            if off == 0 || off >= d.len() {
                return String::new();
            }
            let end = d[off..].iter().position(|&c| c == 0).map_or(d.len(), |e| off + e);
            String::from_utf8_lossy(&d[off..end]).trim().to_string()
        };
        let vendor = text(u32_at(d, 12));
        let product = text(u32_at(d, 16));
        raw.model =
            if vendor.is_empty() || product.starts_with(&vendor) { product } else { format!("{vendor} {product}") };
        raw.bus = u32_at(d, 28);
    }
    // NVMe health log (page 2, 512 bytes).
    // The specific data replaces `AdditionalParameters` (offset 8), the page follows it.
    let header = 8 + size_of::<STORAGE_PROTOCOL_SPECIFIC_DATA>();
    let mut input = bytes_of(&query(StorageDeviceProtocolSpecificProperty));
    input.truncate(8);
    let spec = STORAGE_PROTOCOL_SPECIFIC_DATA {
        ProtocolType: ProtocolTypeNvme,
        DataType: NVMeDataTypeLogPage as u32,
        ProtocolDataRequestValue: 2,
        ProtocolDataOffset: size_of::<STORAGE_PROTOCOL_SPECIFIC_DATA>() as u32,
        ProtocolDataLength: 512,
        ..Default::default()
    };
    input.extend(bytes_of(&spec));
    input.resize(header + 512, 0);
    let mut out = vec![0u8; header + 512];
    if let Ok(got) = ioctl(&h, IOCTL_STORAGE_QUERY_PROPERTY, &input, &mut out) {
        // STORAGE_PROTOCOL_DATA_DESCRIPTOR: Version, Size, then the specific data whose
        // offset is relative to that struct.
        let base = 8;
        let off = u32_at(&out, base + 16) as usize;
        let len = u32_at(&out, base + 20) as usize;
        let at = base + off;
        if len >= 512 && at + 512 <= got && size_of::<STORAGE_PROTOCOL_DATA_DESCRIPTOR>() <= got {
            let mut page = [0u8; 512];
            page.copy_from_slice(&out[at..at + 512]);
            raw.nvme_log = Some(page);
        }
    }
    // Temperature.
    let mut out = vec![0u8; 512];
    if let Ok(got) =
        ioctl(&h, IOCTL_STORAGE_QUERY_PROPERTY, &bytes_of(&query(StorageDeviceTemperatureProperty)), &mut out)
    {
        // STORAGE_TEMPERATURE_DATA_DESCRIPTOR: Critical @8, Warning @10, InfoCount @12,
        // then STORAGE_TEMPERATURE_INFO[] @24 (Temperature @+2).
        let d = &out[..got];
        let count = i16_at(d, 12).unwrap_or(0);
        if count > 0 {
            raw.temperature_c = i16_at(d, 26).filter(|t| (-60..200).contains(t)).map(i32::from);
            raw.temperature_warning_c = i16_at(d, 10).filter(|t| (1..200).contains(t)).map(i32::from);
        }
    }
    // Failure prediction (+ the ATA attribute table).
    let mut out = vec![0u8; size_of::<STORAGE_PREDICT_FAILURE>()];
    if let Ok(got) = ioctl(&h, IOCTL_STORAGE_PREDICT_FAILURE, &[], &mut out)
        && got >= 4
    {
        raw.predicts_failure = Some(u32_at(&out, 0) != 0);
        if got >= 4 + 362 && out[4..].iter().any(|&b| b != 0) {
            let mut table = [0u8; 512];
            let n = (got - 4).min(512);
            table[..n].copy_from_slice(&out[4..4 + n]);
            raw.ata_attributes = Some(table);
        }
    }
    Ok(raw)
}

//! Disks, GPUs and batteries from IOKit registry properties. IOKit is declared by hand, as
//! no crate in the workspace wraps it.

use std::ffi::{CString, c_char, c_void};

use core_foundation::base::{CFType, TCFType, kCFAllocatorDefault};
use core_foundation::boolean::CFBoolean;
use core_foundation::data::CFData;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef, CFMutableDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;

use crate::sysmon::{BatteryRaw, DiskRaw, GpuEngine, GpuRaw, SensorKind, SensorRaw};

type IoObject = u32;

const MAIN_PORT_DEFAULT: u32 = 0;
const SERVICE_PLANE: &std::ffi::CStr = c"IOService";

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(main_port: u32, matching: CFDictionaryRef, existing: *mut IoObject) -> i32;
    fn IOIteratorNext(iterator: IoObject) -> IoObject;
    fn IOObjectRelease(object: IoObject) -> i32;
    fn IORegistryEntryCreateCFProperties(
        entry: IoObject,
        properties: *mut CFMutableDictionaryRef,
        allocator: *const c_void,
        options: u32,
    ) -> i32;
    fn IORegistryEntryGetParentEntry(entry: IoObject, plane: *const c_char, parent: *mut IoObject) -> i32;
    fn IORegistryEntryGetChildEntry(entry: IoObject, plane: *const c_char, child: *mut IoObject) -> i32;
}

/// An owned registry object; released on drop.
struct Entry(IoObject);

impl Drop for Entry {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns one reference to the object.
        unsafe { IOObjectRelease(self.0) };
    }
}

impl Entry {
    fn parent(&self) -> Option<Entry> {
        let mut o: IoObject = 0;
        // SAFETY: `o` is a valid out-pointer; the plane name is NUL-terminated.
        let rc = unsafe { IORegistryEntryGetParentEntry(self.0, SERVICE_PLANE.as_ptr(), &raw mut o) };
        (rc == 0 && o != 0).then_some(Entry(o))
    }

    fn child(&self) -> Option<Entry> {
        let mut o: IoObject = 0;
        // SAFETY: `o` is a valid out-pointer; the plane name is NUL-terminated.
        let rc = unsafe { IORegistryEntryGetChildEntry(self.0, SERVICE_PLANE.as_ptr(), &raw mut o) };
        (rc == 0 && o != 0).then_some(Entry(o))
    }

    fn properties(&self) -> Option<Dict> {
        let mut d: CFMutableDictionaryRef = std::ptr::null_mut();
        // SAFETY: `d` is a valid out-pointer; on success the caller owns the dictionary.
        let rc = unsafe { IORegistryEntryCreateCFProperties(self.0, &raw mut d, kCFAllocatorDefault.cast(), 0) };
        if rc != 0 || d.is_null() {
            return None;
        }
        // SAFETY: created above under the create rule, and wrapped exactly once.
        Some(Dict(unsafe { CFDictionary::wrap_under_create_rule(d.cast_const()) }))
    }
}

/// The registry objects of an IOKit class.
fn services(class: &str) -> Vec<Entry> {
    let Ok(name) = CString::new(class) else { return Vec::new() };
    let mut it: IoObject = 0;
    // SAFETY: `name` is NUL-terminated. `IOServiceGetMatchingServices` consumes the one
    // reference `IOServiceMatching` returned, also on failure; a null dictionary is rejected.
    let rc = unsafe {
        let matching = IOServiceMatching(name.as_ptr());
        if matching.is_null() {
            return Vec::new();
        }
        IOServiceGetMatchingServices(MAIN_PORT_DEFAULT, matching.cast_const(), &raw mut it)
    };
    if rc != 0 || it == 0 {
        return Vec::new();
    }
    let iterator = Entry(it);
    let mut out = Vec::new();
    loop {
        // SAFETY: `iterator` is alive; each returned object is owned by the new `Entry`.
        let o = unsafe { IOIteratorNext(iterator.0) };
        if o == 0 {
            return out;
        }
        out.push(Entry(o));
    }
}

/// A property dictionary with typed getters; a missing or differently typed value is `None`.
struct Dict(CFDictionary);

impl Dict {
    fn value(&self, key: &str) -> Option<CFType> {
        let k = CFString::new(key);
        let p = *self.0.find(k.as_concrete_TypeRef().cast::<c_void>())?;
        // SAFETY: values of a live CF dictionary are valid CF objects; the get rule retains it.
        (!p.is_null()).then(|| unsafe { CFType::wrap_under_get_rule(p) })
    }

    fn int(&self, key: &str) -> Option<i64> {
        self.value(key)?.downcast::<CFNumber>()?.to_i64()
    }

    fn bool(&self, key: &str) -> Option<bool> {
        self.value(key)?.downcast::<CFBoolean>().map(bool::from)
    }

    fn string(&self, key: &str) -> Option<String> {
        let v = self.value(key)?;
        if let Some(s) = v.downcast::<CFString>() {
            return Some(s.to_string());
        }
        // Some model names are C strings in a data blob.
        let d = v.downcast::<CFData>()?;
        let bytes = d.bytes();
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }

    fn dict(&self, key: &str) -> Option<Dict> {
        self.value(key)?.downcast::<CFDictionary>().map(Dict)
    }
}

fn to_u64(v: Option<i64>) -> u64 {
    v.and_then(|v| u64::try_from(v).ok()).unwrap_or(0)
}

pub(super) fn disks() -> Vec<DiskRaw> {
    let mut out = Vec::new();
    for driver in services("IOBlockStorageDriver") {
        let Some(props) = driver.properties() else { continue };
        let Some(stats) = props.dict("Statistics") else { continue };
        // The driver's child is the whole-disk media, which has the BSD name.
        let media = driver.child().and_then(|c| c.properties());
        let Some(id) = media.as_ref().and_then(|m| m.string("BSD Name")) else { continue };
        let chars = driver.parent().and_then(|p| p.properties()).and_then(|p| p.dict("Device Characteristics"));
        let busy_ns = to_u64(stats.int("Total Time (Read)")).saturating_add(to_u64(stats.int("Total Time (Write)")));
        out.push(DiskRaw {
            id,
            model: chars.as_ref().and_then(|c| c.string("Product Name")).unwrap_or_default().trim().to_owned(),
            read_bytes: to_u64(stats.int("Bytes (Read)")),
            write_bytes: to_u64(stats.int("Bytes (Write)")),
            read_ops: to_u64(stats.int("Operations (Read)")),
            write_ops: to_u64(stats.int("Operations (Write)")),
            busy_ms: Some(busy_ns / 1_000_000),
            busy_percent: None,
            queue: None,
            rotational: chars.as_ref().and_then(|c| c.string("Medium Type")).map(|m| m == "Rotational"),
            size: media.as_ref().and_then(|m| m.int("Size")).and_then(|s| u64::try_from(s).ok()),
        });
    }
    out
}

pub(super) fn gpus() -> Vec<GpuRaw> {
    let mut out = Vec::new();
    for (i, gpu) in services("IOAccelerator").iter().enumerate() {
        let Some(props) = gpu.properties() else { continue };
        let stats = props.dict("PerformanceStatistics");
        let pct = |key: &str| stats.as_ref().and_then(|s| s.int(key)).map(|v| v.clamp(0, 100) as f32);
        let mut engines = Vec::new();
        for (key, name) in [("Renderer Utilization %", "Renderer"), ("Tiler Utilization %", "Tiler")] {
            if let Some(percent) = pct(key) {
                engines.push(GpuEngine { name: name.to_owned(), percent });
            }
        }
        // The built-in GPU has no separate name: the chip's is what a person knows.
        let name = props
            .string("model")
            .or_else(|| gpu.parent().and_then(|p| p.properties()).and_then(|p| p.string("model")))
            .or_else(|| super::sysctl_str(c"machdep.cpu.brand_string"))
            .unwrap_or_else(|| "GPU".to_owned());
        let mem = |key: &str| stats.as_ref().and_then(|s| s.int(key)).and_then(|v| u64::try_from(v).ok());
        out.push(GpuRaw {
            id: format!("gpu{i}"),
            vendor: if name.starts_with("Apple") { "Apple".to_owned() } else { String::new() },
            name,
            percent: pct("Device Utilization %"),
            engines,
            mem_used: mem("In use system memory").or_else(|| mem("vramUsedBytes")),
            ..GpuRaw::default()
        });
    }
    out
}

/// Batteries, and the sensors they carry: the only ones reachable without private SMC calls.
pub(super) fn batteries() -> (Vec<BatteryRaw>, Vec<SensorRaw>) {
    let (mut out, mut sensors) = (Vec::new(), Vec::new());
    for (i, battery) in services("AppleSmartBattery").iter().enumerate() {
        let Some(p) = battery.properties() else { continue };
        if p.bool("BatteryInstalled") == Some(false) {
            continue;
        }
        let id = format!("battery{i}");
        // Percent on Apple silicon (maximum 100), mAh on Intel: the ratio is right either way.
        let (Some(now), Some(max)) = (p.int("CurrentCapacity"), p.int("MaxCapacity").filter(|m| *m > 0)) else {
            continue;
        };
        let charging = p.bool("IsCharging").unwrap_or(false);
        let volts = p.int("Voltage").map(|mv| mv as f32 / 1000.0);
        let amps = p.int("InstantAmperage").or_else(|| p.int("Amperage")).map(|ma| ma as f32 / 1000.0);
        // 65535 means "still calculating".
        let time_key = if charging { "AvgTimeToFull" } else { "AvgTimeToEmpty" };
        let minutes = p.int(time_key).filter(|m| (1..65535).contains(m));
        let raw_max = p.int("AppleRawMaxCapacity").or_else(|| p.int("NominalChargeCapacity"));
        let health = raw_max.zip(p.int("DesignCapacity").filter(|d| *d > 0)).map(|(m, d)| m as f32 / d as f32 * 100.0);
        out.push(BatteryRaw {
            id: id.clone(),
            name: p.string("DeviceName").filter(|n| !n.is_empty()).unwrap_or_else(|| "Battery".to_owned()),
            percent: (now as f32 / max as f32 * 100.0).clamp(0.0, 100.0),
            charging,
            on_ac: p.bool("ExternalConnected").unwrap_or(charging),
            seconds_left: minutes.map(|m| m as u64 * 60),
            power_w: volts.zip(amps).map(|(v, a)| v * a),
            health_percent: health,
            cycles: p.int("CycleCount").and_then(|c| u32::try_from(c).ok()),
        });
        let mut sensor = |key: &str, kind, value: Option<f32>| {
            if let Some(value) = value {
                sensors.push(SensorRaw {
                    id: format!("{id}:{key}"),
                    group: "Battery".to_owned(),
                    label: key.to_owned(),
                    kind,
                    value,
                    max: None,
                    critical: None,
                });
            }
        };
        // Hundredths of a degree.
        sensor("Temperature", SensorKind::Temperature, p.int("Temperature").map(|t| t as f32 / 100.0));
        sensor("Voltage", SensorKind::Voltage, volts);
        sensor("Current", SensorKind::Current, amps);
    }
    (out, sensors)
}

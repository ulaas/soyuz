//! GPU adapters: the kernel graphics interface in gdi32 (names, memory size, temperature, fan,
//! clocks) and performance counters (engine load, memory use, per-process use).

use std::collections::HashMap;
use std::mem::size_of;
use std::time::{Duration, Instant};

use windows_sys::Wdk::Graphics::Direct3D::{
    D3DKMT_ADAPTER_PERFDATA, D3DKMT_ADAPTER_PERFDATACAPS, D3DKMT_ADAPTERINFO, D3DKMT_ADAPTERREGISTRYINFO,
    D3DKMT_CLOSEADAPTER, D3DKMT_ENUMADAPTERS2, D3DKMT_NODE_PERFDATA, D3DKMT_QUERYADAPTERINFO, D3DKMT_SEGMENTSIZEINFO,
    D3DKMTCloseAdapter, D3DKMTEnumAdapters2, D3DKMTQueryAdapterInfo, KMTQAITYPE_ADAPTERPERFDATA,
    KMTQAITYPE_ADAPTERPERFDATA_CAPS, KMTQAITYPE_ADAPTERREGISTRYINFO, KMTQAITYPE_ADAPTERTYPE, KMTQAITYPE_GETSEGMENTSIZE,
    KMTQAITYPE_NODEPERFDATA, KMTQUERYADAPTERINFOTYPE,
};

use super::super::from_wide_nul;
use super::pdh::Query;
use crate::sysmon::{GpuEngine, GpuRaw};

/// `D3DKMT_ADAPTERTYPE` bits.
const TYPE_RENDER: u32 = 1 << 0;
const TYPE_SOFTWARE: u32 = 1 << 2;

/// How often per-process use is read; rows carry the last reading in between.
const PROCESS_CADENCE: Duration = Duration::from_millis(2500);
/// An engine reading slower than this also moves adapter load to that cadence.
const SLOW_ENGINE_READ: Duration = Duration::from_millis(10);

/// An adapter's LUID as the counters print it: `(high, low)`.
type Luid = (u32, u32);

/// Per-process GPU use. A `None` map: the counter could not be read at all.
#[derive(Default)]
pub(super) struct ProcGpu {
    pub percent: Option<HashMap<u32, f32>>,
    pub mem: Option<HashMap<u32, u64>>,
}

#[derive(Clone)]
struct AdapterStatic {
    name: String,
    vendor: String,
    usable: bool,
    mem_total: Option<u64>,
    has_fan: bool,
    temp_max: Option<f32>,
    temp_warning: Option<f32>,
}

/// Open adapter handles, closed on drop.
struct Adapters(Vec<D3DKMT_ADAPTERINFO>);

impl Adapters {
    fn enumerate() -> Adapters {
        let mut e = D3DKMT_ENUMADAPTERS2 { NumAdapters: 0, pAdapters: std::ptr::null_mut() };
        // SAFETY: a null array asks for the adapter count only.
        if unsafe { D3DKMTEnumAdapters2(&mut e) } != 0 {
            return Adapters(Vec::new());
        }
        // Room for an adapter that appears between the two calls.
        let mut list = vec![D3DKMT_ADAPTERINFO::default(); (e.NumAdapters as usize + 2).min(64)];
        e.NumAdapters = list.len() as u32;
        e.pAdapters = list.as_mut_ptr();
        // SAFETY: `list` holds `NumAdapters` writable entries.
        if unsafe { D3DKMTEnumAdapters2(&mut e) } != 0 {
            return Adapters(Vec::new());
        }
        list.truncate(e.NumAdapters as usize);
        Adapters(list)
    }
}

impl Drop for Adapters {
    fn drop(&mut self) {
        for a in &self.0 {
            let close = D3DKMT_CLOSEADAPTER { hAdapter: a.hAdapter };
            // SAFETY: handle opened by the enumeration, closed once.
            unsafe {
                D3DKMTCloseAdapter(&close);
            }
        }
    }
}

fn query<T>(adapter: u32, kind: KMTQUERYADAPTERINFOTYPE, out: &mut T) -> bool {
    let mut q = D3DKMT_QUERYADAPTERINFO {
        hAdapter: adapter,
        Type: kind,
        pPrivateDriverData: (out as *mut T).cast(),
        PrivateDriverDataSize: size_of::<T>() as u32,
    };
    // SAFETY: open adapter handle; the data pointer and size describe `out`, a struct of the
    // type this query kind expects (the callers pair them).
    unsafe { D3DKMTQueryAdapterInfo(&mut q) == 0 }
}

fn read_static(handle: u32) -> AdapterStatic {
    let mut bits = 0u32;
    let has_type = query(handle, KMTQAITYPE_ADAPTERTYPE, &mut bits);
    let usable = !has_type || (bits & TYPE_RENDER != 0 && bits & TYPE_SOFTWARE == 0);

    let mut reg: D3DKMT_ADAPTERREGISTRYINFO = D3DKMT_ADAPTERREGISTRYINFO {
        AdapterString: [0; 260],
        BiosString: [0; 260],
        DacType: [0; 260],
        ChipType: [0; 260],
    };
    let mut name = String::new();
    if query(handle, KMTQAITYPE_ADAPTERREGISTRYINFO, &mut reg) {
        name = from_wide_nul(&reg.AdapterString).to_string_lossy().trim().to_string();
        if name.is_empty() {
            name = from_wide_nul(&reg.ChipType).to_string_lossy().trim().to_string();
        }
    }
    let mut seg = D3DKMT_SEGMENTSIZEINFO::default();
    let mem_total = (query(handle, KMTQAITYPE_GETSEGMENTSIZE, &mut seg) && seg.DedicatedVideoMemorySize > 0)
        .then_some(seg.DedicatedVideoMemorySize);

    let mut caps = D3DKMT_ADAPTER_PERFDATACAPS::default();
    let has_caps = query(handle, KMTQAITYPE_ADAPTERPERFDATA_CAPS, &mut caps);
    let deci = |v: u32| (has_caps && v > 0).then_some(v as f32 / 10.0);
    AdapterStatic {
        vendor: vendor_of(&name),
        name,
        usable,
        mem_total,
        has_fan: has_caps && caps.MaxFanRPM > 0,
        temp_max: deci(caps.TemperatureMax),
        temp_warning: deci(caps.TemperatureWarning),
    }
}

fn vendor_of(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for (needle, vendor) in [
        ("nvidia", "NVIDIA"),
        ("geforce", "NVIDIA"),
        ("quadro", "NVIDIA"),
        ("amd", "AMD"),
        ("radeon", "AMD"),
        ("intel", "Intel"),
        ("qualcomm", "Qualcomm"),
        ("adreno", "Qualcomm"),
        ("microsoft", "Microsoft"),
    ] {
        if lower.contains(needle) {
            return vendor.to_string();
        }
    }
    String::new()
}

/// One `\GPU Engine` instance:
/// `pid_1234_luid_0x00000000_0x0000CBEF_phys_0_eng_3_engtype_VideoDecode`.
#[derive(Debug, PartialEq, Eq)]
struct EngineInstance<'a> {
    pid: u32,
    luid: Luid,
    engine: u32,
    kind: &'a str,
}

fn parse_hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s.strip_prefix("0x")?, 16).ok()
}

/// `luid_0xHIGH_0xLOW` at the start of `s`, and what follows it.
fn parse_luid(s: &str) -> Option<(Luid, &str)> {
    let rest = s.strip_prefix("luid_")?;
    let (high, rest) = rest.split_once('_')?;
    let (low, rest) = rest.split_once('_').unwrap_or((rest, ""));
    Some(((parse_hex(high)?, parse_hex(low)?), rest))
}

fn parse_engine(s: &str) -> Option<EngineInstance<'_>> {
    let rest = s.strip_prefix("pid_")?;
    let (pid, rest) = rest.split_once('_')?;
    let (luid, rest) = parse_luid(rest)?;
    let rest = rest.strip_prefix("phys_")?;
    let (_, rest) = rest.split_once('_')?;
    let rest = rest.strip_prefix("eng_")?;
    let (engine, rest) = rest.split_once('_')?;
    let kind = rest.strip_prefix("engtype_")?;
    Some(EngineInstance { pid: pid.parse().ok()?, luid, engine: engine.parse().ok()?, kind })
}

/// `pid_1234_luid_0x0_0x1_phys_0`.
fn parse_process_memory(s: &str) -> Option<(u32, Luid)> {
    let rest = s.strip_prefix("pid_")?;
    let (pid, rest) = rest.split_once('_')?;
    Some((pid.parse().ok()?, parse_luid(rest)?.0))
}

/// `VideoDecode` to `Video Decode`, `JPEG_Decode_1` to `JPEG Decode 1`; a blank kind gets the
/// engine's number.
fn engine_label(kind: &str, engine: u32) -> String {
    if kind.is_empty() {
        return format!("Engine {engine}");
    }
    let mut out = String::with_capacity(kind.len() + 2);
    let mut prev_lower = false;
    for c in kind.chars() {
        if c.is_ascii_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_ascii_lowercase();
        out.push(if c == '_' { ' ' } else { c });
    }
    out
}

fn luid_id(l: Luid) -> String {
    format!("{:08X}_{:08X}", l.0, l.1)
}

#[derive(Default)]
pub(super) struct GpuReader {
    engines: Option<Query>,
    engines_tried: bool,
    memory: Option<Query>,
    memory_tried: bool,
    process_memory: Option<Query>,
    process_memory_tried: bool,
    adapters: HashMap<Luid, AdapterStatic>,
    limits: HashMap<String, (Option<f32>, Option<f32>)>,
    /// Load per (adapter, engine number) over processes, with the engine's kind.
    loads: HashMap<(Luid, u32), (f64, String)>,
    engine_cost: Duration,
    engine_at: Option<Instant>,
    process_use: Option<ProcGpu>,
    process_at: Option<Instant>,
}

impl GpuReader {
    /// `(warning-or-max, critical)` temperature limits of an adapter by id.
    pub(super) fn temperature_limits(&self, id: &str) -> (Option<f32>, Option<f32>) {
        self.limits.get(id).copied().unwrap_or((None, None))
    }

    /// The last per-process reading (see [`PROCESS_CADENCE`]).
    pub(super) fn process_use(&self) -> Option<&ProcGpu> {
        self.process_use.as_ref()
    }

    pub(super) fn read(&mut self, hardware: bool, per_process: bool) -> Vec<GpuRaw> {
        if !per_process {
            self.process_use = None;
            self.process_at = None;
        }
        if !hardware && !per_process {
            return Vec::new();
        }
        if !self.engines_tried {
            self.engines_tried = true;
            self.engines = Query::with(&[r"\GPU Engine(*)\Utilization Percentage"]);
        }
        let due = |at: Option<Instant>| at.is_none_or(|t| t.elapsed() >= PROCESS_CADENCE);
        let process_due = per_process && due(self.process_at);
        let load_due = hardware && (self.engine_cost < SLOW_ENGINE_READ || due(self.engine_at));
        if process_due || load_due {
            let began = Instant::now();
            let process_load = self.read_engines(process_due);
            self.engine_cost = began.elapsed();
            self.engine_at = Some(began);
            if process_due {
                // Rate counters have no data on their first reading: ask again next sample.
                if process_load.is_some() || self.engines.is_none() {
                    self.process_at = Some(began);
                }
                self.process_use = Some(ProcGpu { percent: process_load, mem: self.read_process_memory() });
            }
        }
        if hardware { self.read_adapters() } else { Vec::new() }
    }

    /// New engine loads into `self.loads`, and per-process load when asked (`None`: no data).
    fn read_engines(&mut self, per_process: bool) -> Option<HashMap<u32, f32>> {
        let q = self.engines.as_mut()?;
        if !q.collect() {
            return None;
        }
        let loads = &mut self.loads;
        loads.clear();
        let mut per_pid: HashMap<(u32, Luid, u32), f64> = HashMap::new();
        let mut line = String::new();
        q.values(0, |name, v| {
            line.clear();
            line.extend(char::decode_utf16(name.iter().copied()).map(|c| c.unwrap_or('\u{fffd}')));
            let Some(i) = parse_engine(&line) else { return };
            let slot = loads.entry((i.luid, i.engine)).or_insert_with(|| (0.0, i.kind.to_string()));
            slot.0 += v;
            if per_process {
                *per_pid.entry((i.pid, i.luid, i.engine)).or_insert(0.0) += v;
            }
        });
        if per_pid.is_empty() {
            return None;
        }
        // A process's load: its busiest engine, as Task Manager counts it.
        let mut map: HashMap<u32, f32> = HashMap::new();
        for ((pid, ..), v) in per_pid {
            let e = map.entry(pid).or_insert(0.0);
            *e = e.max((v as f32).clamp(0.0, 100.0));
        }
        Some(map)
    }

    fn read_process_memory(&mut self) -> Option<HashMap<u32, u64>> {
        if !self.process_memory_tried {
            self.process_memory_tried = true;
            self.process_memory = Query::with(&[r"\GPU Process Memory(*)\Dedicated Usage"]);
        }
        let q = self.process_memory.as_mut()?;
        if !q.collect() {
            return None;
        }
        let mut map = HashMap::new();
        q.values(0, |name, v| {
            if let Some((pid, _)) = parse_process_memory(&String::from_utf16_lossy(name)) {
                *map.entry(pid).or_insert(0u64) += v.max(0.0) as u64;
            }
        });
        Some(map)
    }

    fn read_adapters(&mut self) -> Vec<GpuRaw> {
        if !self.memory_tried {
            self.memory_tried = true;
            self.memory =
                Query::with(&[r"\GPU Adapter Memory(*)\Dedicated Usage", r"\GPU Adapter Memory(*)\Shared Usage"]);
        }
        let mut dedicated: HashMap<Luid, u64> = HashMap::new();
        let mut shared: HashMap<Luid, u64> = HashMap::new();
        let mut memory_ok = false;
        if let Some(q) = &mut self.memory
            && q.collect()
        {
            memory_ok = true;
            for (index, map) in [(0, &mut dedicated), (1, &mut shared)] {
                q.values(index, |name, v| {
                    if let Some((luid, _)) = parse_luid(&String::from_utf16_lossy(name)) {
                        *map.entry(luid).or_insert(0) += v.max(0.0) as u64;
                    }
                });
            }
        }

        let list = Adapters::enumerate();
        let mut out = Vec::new();
        for a in &list.0 {
            let luid: Luid = (a.AdapterLuid.HighPart as u32, a.AdapterLuid.LowPart);
            let info = self.adapters.entry(luid).or_insert_with(|| read_static(a.hAdapter)).clone();
            if !info.usable {
                continue;
            }
            let mut gpu = GpuRaw {
                id: luid_id(luid),
                name: info.name,
                vendor: info.vendor,
                mem_total: info.mem_total,
                ..GpuRaw::default()
            };
            self.limits.insert(gpu.id.clone(), (info.temp_warning, info.temp_max));

            // Engines of one kind (several copy engines) show as their busiest.
            let mut by_kind: Vec<GpuEngine> = Vec::new();
            for ((l, engine), (v, kind)) in &self.loads {
                if *l != luid {
                    continue;
                }
                let name = engine_label(kind, *engine);
                let percent = (*v as f32).clamp(0.0, 100.0);
                match by_kind.iter_mut().find(|e| e.name == name) {
                    Some(e) => e.percent = e.percent.max(percent),
                    None => by_kind.push(GpuEngine { name, percent }),
                }
            }
            by_kind.sort_by(|a, b| a.name.cmp(&b.name));
            // No engines yet (the counters' first reading) is not the same as an idle GPU.
            if !by_kind.is_empty() {
                gpu.percent = Some(by_kind.iter().fold(0.0, |m: f32, e| m.max(e.percent)));
            }
            gpu.engines = by_kind;
            if memory_ok {
                gpu.mem_used = Some(dedicated.get(&luid).copied().unwrap_or(0));
                gpu.shared_used = Some(shared.get(&luid).copied().unwrap_or(0));
            }

            let mut perf = D3DKMT_ADAPTER_PERFDATA::default();
            if query(a.hAdapter, KMTQAITYPE_ADAPTERPERFDATA, &mut perf) {
                // Deci-degrees Celsius; 0 means the driver does not report it.
                gpu.temperature_c = (perf.Temperature > 0).then_some(perf.Temperature as f32 / 10.0);
                gpu.fan_rpm = info.has_fan.then_some(perf.FanRPM);
                gpu.mem_clock_mhz = (perf.MemoryFrequency > 0).then_some((perf.MemoryFrequency / 1_000_000) as u32);
                // `Power` is a share of the board limit, not watts: `power_w` stays `None`.
            }
            let mut node = D3DKMT_NODE_PERFDATA::default();
            if query(a.hAdapter, KMTQAITYPE_NODEPERFDATA, &mut node) {
                gpu.clock_mhz = (node.Frequency > 0).then_some((node.Frequency / 1_000_000) as u32);
            }
            out.push(gpu);
        }
        out.sort_by(|a, b| b.mem_total.cmp(&a.mem_total).then_with(|| a.id.cmp(&b.id)));
        out
    }
}

/// For `sysmon_part_timings`.
#[cfg(test)]
pub(super) fn part_timings(time: &mut dyn FnMut(&str, &mut dyn FnMut())) {
    time("kmt enumerate and close", &mut || drop(Adapters::enumerate()));
    let list = Adapters::enumerate();
    for a in &list.0 {
        let h = a.hAdapter;
        time("kmt adapter perf", &mut || {
            let mut perf = D3DKMT_ADAPTER_PERFDATA::default();
            query(h, KMTQAITYPE_ADAPTERPERFDATA, &mut perf);
        });
        time("kmt node perf", &mut || {
            let mut node = D3DKMT_NODE_PERFDATA::default();
            query(h, KMTQAITYPE_NODEPERFDATA, &mut node);
        });
    }
    let mut r = GpuReader::default();
    time("whole gpu read", &mut || drop(r.read(true, true)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_instances_parse() {
        let s = "pid_1234_luid_0x00000000_0x0000CBEF_phys_0_eng_3_engtype_VideoDecode";
        assert_eq!(
            parse_engine(s),
            Some(EngineInstance { pid: 1234, luid: (0, 0xCBEF), engine: 3, kind: "VideoDecode" })
        );
        assert_eq!(parse_engine("pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_").map(|i| i.kind), Some(""));
        for bad in
            ["", "_Total", "pid_x_luid_0x0_0x1_phys_0_eng_0_engtype_3D", "pid_1_luid_12_0x1_phys_0_eng_0_engtype_3D"]
        {
            assert_eq!(parse_engine(bad), None, "{bad}");
        }
    }

    #[test]
    fn memory_instances_parse() {
        assert_eq!(parse_luid("luid_0x00000000_0x0000CBEF_phys_0").map(|x| x.0), Some((0, 0xCBEF)));
        assert_eq!(parse_process_memory("pid_77_luid_0x00000001_0x00000002_phys_0"), Some((77, (1, 2))));
        assert_eq!(parse_process_memory("luid_0x1_0x2_phys_0"), None);
    }

    #[test]
    fn labels_and_vendors() {
        assert_eq!(engine_label("VideoDecode", 0), "Video Decode");
        assert_eq!(engine_label("3D", 0), "3D");
        assert_eq!(engine_label("JPEG_Decode_1", 0), "JPEG Decode 1");
        assert_eq!(engine_label("Copy", 2), "Copy");
        assert_eq!(engine_label("", 7), "Engine 7");
        assert_eq!(vendor_of("NVIDIA GeForce RTX 4090"), "NVIDIA");
        assert_eq!(vendor_of("AMD Radeon(TM) Graphics"), "AMD");
        assert_eq!(vendor_of("Intel(R) UHD Graphics"), "Intel");
        assert_eq!(vendor_of("Something"), "");
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn query_struct_sizes() {
        assert_eq!(size_of::<D3DKMT_QUERYADAPTERINFO>(), 24);
        assert_eq!(size_of::<D3DKMT_ADAPTERINFO>(), 20);
    }
}

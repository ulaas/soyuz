//! NVIDIA GPUs through NVML, loaded with `dlopen`: no link-time dependency, and a machine
//! without the library simply has no NVML. Layouts follow `nvml.h`.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};

use crate::sysmon::{GpuEngine, GpuRaw};

type Device = *mut c_void;
type Ret = c_int;

const NVML_SUCCESS: Ret = 0;
const NVML_TEMPERATURE_GPU: c_uint = 0;
const NVML_CLOCK_GRAPHICS: c_uint = 0;
const NVML_CLOCK_MEM: c_uint = 2;
/// `NVML_VALUE_NOT_AVAILABLE`.
const NOT_AVAILABLE: u64 = u64::MAX;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Utilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Memory {
    total: u64,
    free: u64,
    used: u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ProcessSample {
    pid: c_uint,
    time_stamp: u64,
    sm_util: c_uint,
    mem_util: c_uint,
    enc_util: c_uint,
    dec_util: c_uint,
}

/// `nvmlProcessInfo_t` (versions 2 and 3 have the same layout).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ProcessInfo {
    pid: c_uint,
    used_gpu_memory: u64,
    gpu_instance_id: c_uint,
    compute_instance_id: c_uint,
}

/// `nvmlFanSpeedInfo_t` (version 1).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FanSpeedInfo {
    version: c_uint,
    fan: c_uint,
    speed: c_uint,
}

/// GPU use of one process, summed over devices.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ProcUse {
    pub percent: f32,
    pub memory: u64,
}

struct Api {
    name: unsafe extern "C" fn(Device, *mut c_char, c_uint) -> Ret,
    uuid: unsafe extern "C" fn(Device, *mut c_char, c_uint) -> Ret,
    utilization: unsafe extern "C" fn(Device, *mut Utilization) -> Ret,
    memory: unsafe extern "C" fn(Device, *mut Memory) -> Ret,
    temperature: unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> Ret,
    power: unsafe extern "C" fn(Device, *mut c_uint) -> Ret,
    clock: unsafe extern "C" fn(Device, c_uint, *mut c_uint) -> Ret,
    encoder: Option<unsafe extern "C" fn(Device, *mut c_uint, *mut c_uint) -> Ret>,
    decoder: Option<unsafe extern "C" fn(Device, *mut c_uint, *mut c_uint) -> Ret>,
    fan_rpm: Option<unsafe extern "C" fn(Device, *mut FanSpeedInfo) -> Ret>,
    process_util: Option<unsafe extern "C" fn(Device, *mut ProcessSample, *mut c_uint, u64) -> Ret>,
    compute_procs: Option<unsafe extern "C" fn(Device, *mut c_uint, *mut ProcessInfo) -> Ret>,
    graphics_procs: Option<unsafe extern "C" fn(Device, *mut c_uint, *mut ProcessInfo) -> Ret>,
}

struct Dev {
    handle: Device,
    id: String,
    name: String,
    last_seen: u64,
}

pub(super) struct Nvml {
    api: Api,
    devices: Vec<Dev>,
}

// SAFETY: NVML is documented thread-safe and device handles are process-wide identifiers;
// `Nvml` holds no thread-bound state and is only ever used by the thread that owns the `Probe`.
unsafe impl Send for Nvml {}

/// The symbol `name` of `lib` as a function pointer of type `F`, when present.
///
/// # Safety
/// `F` must be a function pointer type matching the symbol's real signature.
unsafe fn sym<F: Copy>(lib: *mut c_void, name: &CStr) -> Option<F> {
    // SAFETY: `lib` is a live handle from `dlopen` and `name` is NUL-terminated.
    let p = unsafe { libc::dlsym(lib, name.as_ptr()) };
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller promises `F` is the matching function pointer type, which has the
    // size of a data pointer on every Linux target; `p` is non-null.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
}

/// Turns a C string buffer's content into text.
fn c_text(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

impl Nvml {
    /// `None` when the library, its symbols or a driver are missing. Never an error to the user.
    pub(super) fn load() -> Option<Nvml> {
        // SAFETY: the name is NUL-terminated. The handle stays open for the life of the process,
        // since the function pointers below point into the library.
        let lib = unsafe { libc::dlopen(c"libnvidia-ml.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if lib.is_null() {
            return None;
        }
        // SAFETY: every type below is the signature `nvml.h` declares for that symbol.
        unsafe {
            let init: unsafe extern "C" fn() -> Ret = sym(lib, c"nvmlInit_v2")?;
            let count: unsafe extern "C" fn(*mut c_uint) -> Ret = sym(lib, c"nvmlDeviceGetCount_v2")?;
            let by_index: unsafe extern "C" fn(c_uint, *mut Device) -> Ret =
                sym(lib, c"nvmlDeviceGetHandleByIndex_v2")?;
            let api = Api {
                name: sym(lib, c"nvmlDeviceGetName")?,
                uuid: sym(lib, c"nvmlDeviceGetUUID")?,
                utilization: sym(lib, c"nvmlDeviceGetUtilizationRates")?,
                memory: sym(lib, c"nvmlDeviceGetMemoryInfo")?,
                temperature: sym(lib, c"nvmlDeviceGetTemperature")?,
                power: sym(lib, c"nvmlDeviceGetPowerUsage")?,
                clock: sym(lib, c"nvmlDeviceGetClockInfo")?,
                encoder: sym(lib, c"nvmlDeviceGetEncoderUtilization"),
                decoder: sym(lib, c"nvmlDeviceGetDecoderUtilization"),
                fan_rpm: sym(lib, c"nvmlDeviceGetFanSpeedRPM"),
                process_util: sym(lib, c"nvmlDeviceGetProcessUtilization"),
                compute_procs: sym(lib, c"nvmlDeviceGetComputeRunningProcesses_v3")
                    .or_else(|| sym(lib, c"nvmlDeviceGetComputeRunningProcesses_v2")),
                graphics_procs: sym(lib, c"nvmlDeviceGetGraphicsRunningProcesses_v3")
                    .or_else(|| sym(lib, c"nvmlDeviceGetGraphicsRunningProcesses_v2")),
            };
            if init() != NVML_SUCCESS {
                return None;
            }
            let mut n: c_uint = 0;
            if count(&raw mut n) != NVML_SUCCESS {
                return None;
            }
            let mut devices = Vec::new();
            for i in 0..n {
                let mut handle: Device = std::ptr::null_mut();
                if by_index(i, &raw mut handle) != NVML_SUCCESS {
                    continue;
                }
                let mut buf = [0 as c_char; 96];
                let name = if (api.name)(handle, buf.as_mut_ptr(), buf.len() as c_uint) == NVML_SUCCESS {
                    c_text(&buf)
                } else {
                    format!("NVIDIA GPU {i}")
                };
                buf = [0; 96];
                let id = if (api.uuid)(handle, buf.as_mut_ptr(), buf.len() as c_uint) == NVML_SUCCESS {
                    c_text(&buf)
                } else {
                    format!("nvidia{i}")
                };
                devices.push(Dev { handle, id, name, last_seen: 0 });
            }
            Some(Nvml { api, devices })
        }
    }

    pub(super) fn gpus(&self) -> Vec<GpuRaw> {
        self.devices.iter().map(|d| self.gpu(d)).collect()
    }

    fn gpu(&self, d: &Dev) -> GpuRaw {
        let a = &self.api;
        let mut g = GpuRaw { id: d.id.clone(), name: d.name.clone(), vendor: "NVIDIA".to_owned(), ..GpuRaw::default() };
        // SAFETY: each call receives a live handle and valid out-pointers of the declared types.
        unsafe {
            let mut u = Utilization::default();
            if (a.utilization)(d.handle, &raw mut u) == NVML_SUCCESS {
                g.percent = Some(u.gpu.min(100) as f32);
            }
            let mut m = Memory::default();
            if (a.memory)(d.handle, &raw mut m) == NVML_SUCCESS {
                g.mem_used = Some(m.used);
                g.mem_total = Some(m.total);
            }
            let mut v: c_uint = 0;
            if (a.temperature)(d.handle, NVML_TEMPERATURE_GPU, &raw mut v) == NVML_SUCCESS {
                g.temperature_c = Some(v as f32);
            }
            if (a.power)(d.handle, &raw mut v) == NVML_SUCCESS {
                g.power_w = Some(v as f32 / 1000.0);
            }
            if (a.clock)(d.handle, NVML_CLOCK_GRAPHICS, &raw mut v) == NVML_SUCCESS {
                g.clock_mhz = Some(v);
            }
            if (a.clock)(d.handle, NVML_CLOCK_MEM, &raw mut v) == NVML_SUCCESS {
                g.mem_clock_mhz = Some(v);
            }
            if let Some(f) = a.fan_rpm {
                let mut info = FanSpeedInfo {
                    version: (size_of::<FanSpeedInfo>() as c_uint) | (1 << 24),
                    ..FanSpeedInfo::default()
                };
                if f(d.handle, &raw mut info) == NVML_SUCCESS {
                    g.fan_rpm = Some(info.speed);
                }
            }
            let mut period: c_uint = 0;
            if let Some(f) = a.encoder
                && f(d.handle, &raw mut v, &raw mut period) == NVML_SUCCESS
            {
                g.engines.push(GpuEngine { name: "Encoder".to_owned(), percent: v.min(100) as f32 });
            }
            if let Some(f) = a.decoder
                && f(d.handle, &raw mut v, &raw mut period) == NVML_SUCCESS
            {
                g.engines.push(GpuEngine { name: "Decoder".to_owned(), percent: v.min(100) as f32 });
            }
        }
        g
    }

    /// Per-process load and memory over every device. Failing calls leave those values out.
    pub(super) fn process_usage(&mut self) -> HashMap<u32, ProcUse> {
        let mut out: HashMap<u32, ProcUse> = HashMap::new();
        for d in &mut self.devices {
            if let Some(f) = self.api.process_util {
                let mut samples = [ProcessSample::default(); 128];
                let mut n = samples.len() as c_uint;
                // SAFETY: `samples` holds `n` elements, and the handle is live.
                let rc = unsafe { f(d.handle, samples.as_mut_ptr(), &raw mut n, d.last_seen) };
                if rc == NVML_SUCCESS {
                    for s in &samples[..(n as usize).min(samples.len())] {
                        let e = out.entry(s.pid).or_default();
                        e.percent = e.percent.max(s.sm_util.min(100) as f32);
                        d.last_seen = d.last_seen.max(s.time_stamp);
                    }
                }
            }
            // A process with compute and graphics work is in both lists, each with its total.
            let mut on_device: HashMap<u32, u64> = HashMap::new();
            for f in [self.api.compute_procs, self.api.graphics_procs].into_iter().flatten() {
                let mut infos = [ProcessInfo::default(); 128];
                let mut n = infos.len() as c_uint;
                // SAFETY: `infos` holds `n` elements, and the handle is live.
                let rc = unsafe { f(d.handle, &raw mut n, infos.as_mut_ptr()) };
                if rc == NVML_SUCCESS {
                    for i in &infos[..(n as usize).min(infos.len())] {
                        if i.used_gpu_memory != NOT_AVAILABLE {
                            let m = on_device.entry(i.pid).or_default();
                            *m = (*m).max(i.used_gpu_memory);
                        }
                    }
                }
            }
            for (pid, memory) in on_device {
                let e = out.entry(pid).or_default();
                e.memory = e.memory.saturating_add(memory);
            }
        }
        out
    }
}

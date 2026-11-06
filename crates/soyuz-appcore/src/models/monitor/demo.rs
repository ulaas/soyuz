//! A made-up machine for the website's screenshots (dev script `monitor:demo`,
//! `scripts/screenshots.ps1`): the pictures must not show this computer's processes, user or
//! host name.

use std::path::PathBuf;
use std::sync::Arc;

use soyuz_fs::sysmon::{GpuEngine, SensorKind, SensorRaw};
use soyuz_sysmon::{
    CoreFrame, Counts, CpuFrame, DiskFrame, Frame, GpuRaw, MemFrame, MemoryRaw, NetFrame, ProcKey, ProcRaw, ProcRow,
    ProcState, SystemInfo,
};

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;
const SECONDS: i64 = 180;
const CORES: usize = 16;

/// `(parent index, name, user, CPU where 100 = one CPU, resident MiB, read KiB/s, write KiB/s, GPU %)`.
type Row = (Option<usize>, &'static str, &'static str, f32, u64, f64, f64, f32);

const ROWS: &[Row] = &[
    (None, "systemd", "root", 0.1, 18, 0.0, 0.0, 0.0),
    (Some(0), "soyuz", "ann", 1.8, 142, 64.0, 12.0, 0.4),
    (Some(0), "compositor", "ann", 3.2, 310, 0.0, 0.0, 11.0),
    (Some(0), "pipewire", "ann", 0.6, 36, 0.0, 0.0, 0.0),
    (Some(0), "postgres", "postgres", 4.4, 884, 2_400.0, 1_800.0, 0.0),
    (Some(4), "postgres: writer", "postgres", 1.1, 96, 0.0, 3_200.0, 0.0),
    (Some(4), "postgres: autovacuum", "postgres", 0.4, 64, 520.0, 120.0, 0.0),
    (Some(0), "browser", "ann", 9.5, 1_420, 340.0, 220.0, 6.0),
    (Some(7), "browser: renderer", "ann", 22.0, 980, 12.0, 6.0, 14.0),
    (Some(7), "browser: renderer", "ann", 6.5, 612, 4.0, 2.0, 2.0),
    (Some(7), "browser: video", "ann", 12.0, 448, 180.0, 0.0, 9.0),
    (Some(7), "browser: network", "ann", 1.2, 128, 0.0, 44.0, 0.0),
    (Some(0), "editor", "ann", 5.2, 1_180, 96.0, 48.0, 0.6),
    (Some(12), "language-server", "ann", 38.0, 2_240, 420.0, 64.0, 0.0),
    (Some(12), "terminal", "ann", 0.8, 88, 0.0, 0.0, 0.2),
    (Some(14), "cargo", "ann", 1.4, 210, 860.0, 320.0, 0.0),
    (Some(15), "rustc", "ann", 96.0, 1_840, 1_200.0, 900.0, 0.0),
    (Some(15), "rustc", "ann", 88.0, 1_620, 980.0, 760.0, 0.0),
    (Some(15), "rustc", "ann", 74.0, 1_310, 640.0, 520.0, 0.0),
    (Some(15), "linker", "ann", 41.0, 2_680, 3_100.0, 2_400.0, 0.0),
    (Some(0), "render-node", "ann", 62.0, 3_260, 90.0, 40.0, 48.0),
    (Some(0), "backup-agent", "root", 2.1, 164, 18_400.0, 0.0, 0.0),
    (Some(0), "indexer", "ann", 0.9, 240, 2_100.0, 180.0, 0.0),
    (Some(0), "music-player", "ann", 1.0, 196, 180.0, 0.0, 0.3),
    (Some(0), "sshd", "root", 0.0, 12, 0.0, 0.0, 0.0),
    (Some(0), "chat", "ann", 1.6, 540, 8.0, 12.0, 0.8),
    (Some(0), "print-service", "root", 0.0, 22, 0.0, 0.0, 0.0),
    (Some(0), "network-manager", "root", 0.2, 34, 0.0, 0.0, 0.0),
];

/// A slow, uneven wave in `0.0..=1.0`.
fn wave(t: f32, speed: f32, phase: f32) -> f32 {
    let a = (t * speed + phase).sin();
    let b = (t * speed * 2.7 + phase * 1.9).sin();
    let c = (t * speed * 6.1 + phase * 0.6).sin();
    (0.5 + 0.28 * a + 0.15 * b + 0.07 * c).clamp(0.0, 1.0)
}

fn procs(t: f32, now_ms: i64) -> Arc<[ProcRow]> {
    let pid = |i: usize| 1_200 + i as u32 * 37;
    ROWS.iter()
        .enumerate()
        .map(|(i, r)| {
            let (parent, name, user, cpu, mem, read, write, gpu) = *r;
            let swing = 0.6 + 0.8 * wave(t, 0.09, i as f32);
            let cpu = cpu * swing;
            ProcRow {
                raw: ProcRaw {
                    key: ProcKey {
                        pid: pid(i),
                        start_ms: now_ms - 3_600_000 + i as i64 * 60_000,
                        start_raw: 1 + i as u64,
                    },
                    parent: parent.map(pid),
                    name: name.into(),
                    exe: Some(PathBuf::from(format!("/usr/bin/{name}")).into()),
                    cmdline: Some(format!("/usr/bin/{name}").into()),
                    user: Some(user.into()),
                    state: if cpu > 8.0 { ProcState::Running } else { ProcState::Sleeping },
                    cpu_user_ms: (cpu * 4_000.0) as u64,
                    cpu_system_ms: (cpu * 900.0) as u64,
                    rss: mem * MIB,
                    threads: 2 + (mem / 90) as u32,
                    gpu_percent: Some(gpu * swing),
                    gpu_mem: Some(mem * MIB / 4),
                    ..ProcRaw::default()
                },
                cpu_percent: cpu,
                read_rate: Some(read * 1024.0 * f64::from(swing)),
                write_rate: Some(write * 1024.0 * f64::from(swing)),
                mem_percent: (mem * MIB) as f32 / (64 * GIB) as f32 * 100.0,
            }
        })
        .collect::<Vec<_>>()
        .into()
}

fn frame(i: i64, now_ms: i64) -> Frame {
    let t = i as f32;
    let cpu = 22.0 + 46.0 * wave(t, 0.07, 0.4);
    let cores = (0..CORES)
        .map(|c| {
            let percent = (cpu * (0.45 + 1.1 * wave(t, 0.11, c as f32 * 1.3))).clamp(1.0, 100.0);
            CoreFrame {
                percent,
                user: percent * 0.76,
                system: percent * 0.24,
                mhz: Some(3_400 + (1_500.0 * wave(t, 0.13, c as f32)) as u32),
            }
        })
        .collect();
    let used = 28.0 + 7.0 * wave(t, 0.015, 1.0);
    let used_bytes = (f64::from(used) * GIB as f64) as u64;
    let read = 40e6 + 310e6 * f64::from(wave(t, 0.1, 2.0).powi(2));
    let write = 12e6 + 120e6 * f64::from(wave(t, 0.08, 4.0).powi(3));
    let rx = 2e6 + 38e6 * f64::from(wave(t, 0.12, 5.0).powi(2));
    let tx = 4e5 + 6e6 * f64::from(wave(t, 0.17, 0.7).powi(2));
    let gpu = 24.0 + 46.0 * wave(t, 0.09, 3.0);
    let temp = 54.0 + 14.0 * wave(t, 0.03, 1.5);
    let sensor = |id: &str, group: &str, label: &str, kind, value, critical| SensorRaw {
        id: id.into(),
        group: group.into(),
        label: label.into(),
        kind,
        value,
        max: None,
        critical,
    };
    Frame {
        at_ms: now_ms - (SECONDS - 1 - i) * 1000,
        uptime_secs: 3 * 86_400 + 14_400 + i as u64,
        load: Some([cpu / 6.0, cpu / 7.0, cpu / 8.0]),
        elapsed_ms: 1000,
        cpu: CpuFrame {
            percent: cpu,
            user: cpu * 0.76,
            system: cpu * 0.2,
            iowait: cpu * 0.04,
            cores,
            mhz_avg: Some(4_450),
            temp_c: Some(temp),
            ctx_per_sec: Some(48_000.0),
            irq_per_sec: Some(31_000.0),
            ..CpuFrame::default()
        },
        memory: MemFrame {
            raw: MemoryRaw {
                total: 64 * GIB,
                available: 64 * GIB - used_bytes,
                cached: Some(19 * GIB),
                buffers: Some(GIB),
                kernel: Some(2 * GIB),
                swap_total: 16 * GIB,
                swap_used: GIB / 2,
                ..MemoryRaw::default()
            },
            used: used_bytes,
            percent: used / 64.0 * 100.0,
            swap_percent: 3.1,
            page_in_rate: Some(read / 50.0),
            page_out_rate: Some(write / 80.0),
        },
        disks: vec![
            DiskFrame {
                id: "nvme0".into(),
                model: "NVMe SSD 2 TB".into(),
                read_rate: read,
                write_rate: write,
                read_iops: read / 60_000.0,
                write_iops: write / 40_000.0,
                busy_percent: Some(((read + write) / 6e6).min(100.0) as f32),
                queue: Some(1.4),
                rotational: Some(false),
                size: Some(2_000 * 1_000_000_000),
                read_bytes: 900 * GIB,
                write_bytes: 410 * GIB,
            },
            DiskFrame {
                id: "sda".into(),
                model: "Hard disk 8 TB".into(),
                read_rate: read / 9.0,
                write_rate: 0.0,
                read_iops: 38.0,
                write_iops: 0.0,
                busy_percent: Some(22.0),
                queue: Some(0.3),
                rotational: Some(true),
                size: Some(8_000 * 1_000_000_000),
                read_bytes: 2_100 * GIB,
                write_bytes: 640 * GIB,
            },
        ],
        nets: vec![NetFrame {
            id: "eth0".into(),
            name: "Ethernet".into(),
            description: "2.5 GbE".into(),
            rx_rate: rx,
            tx_rate: tx,
            rx_packets_rate: rx / 1_400.0,
            tx_packets_rate: tx / 1_000.0,
            rx_bytes: 148 * GIB,
            tx_bytes: 31 * GIB,
            up: true,
            link_bps: Some(2_500_000_000),
            addresses: vec!["192.0.2.24".into()],
            mac: Some("00:00:5e:00:53:24".into()),
            ..NetFrame::default()
        }],
        gpus: vec![GpuRaw {
            id: "gpu0".into(),
            name: "Graphics adapter 16 GB".into(),
            vendor: "GPU".into(),
            percent: Some(gpu),
            engines: vec![
                GpuEngine { name: "3D".into(), percent: gpu },
                GpuEngine { name: "Video decode".into(), percent: gpu / 4.0 },
                GpuEngine { name: "Copy".into(), percent: gpu / 7.0 },
            ],
            mem_used: Some(7 * GIB),
            mem_total: Some(16 * GIB),
            temperature_c: Some(temp - 4.0),
            power_w: Some(70.0 + gpu * 1.6),
            fan_rpm: Some(1_150),
            clock_mhz: Some(2_450),
            mem_clock_mhz: Some(9_500),
            ..GpuRaw::default()
        }],
        sensors: vec![
            sensor("cpu", "CPU", "Package", SensorKind::Temperature, temp, Some(100.0)),
            sensor("ssd", "NVMe SSD", "Composite", SensorKind::Temperature, 41.0, Some(84.0)),
            sensor("fan", "Board", "CPU fan", SensorKind::Fan, 980.0, None),
            sensor("pwr", "CPU", "Package power", SensorKind::Power, 40.0 + cpu, None),
        ],
        procs: procs(t, now_ms),
        counts: Counts { processes: ROWS.len() as u32, threads: 412, handles: None, running: 6 },
        ..Frame::default()
    }
}

/// Three minutes of the made-up machine ending now, and its description.
pub fn frames() -> (Vec<Frame>, SystemInfo) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let info = SystemInfo {
        hostname: "orbit".into(),
        os: "Linux".into(),
        kernel: "6.18".into(),
        arch: "x86_64".into(),
        cpu_model: "16-thread processor".into(),
        physical_cores: 8,
        logical_cores: CORES as u32,
        sockets: 1,
        cpu_max_mhz: Some(4_900),
        cache_l3_kb: Some(32_768),
        memory_total: 64 * GIB,
        ..SystemInfo::default()
    };
    ((0..SECONDS).map(|i| frame(i, now_ms)).collect(), info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_machine_is_consistent() {
        let (frames, info) = frames();
        assert_eq!(frames.len() as i64, SECONDS);
        let last = frames.last().unwrap();
        assert_eq!(last.cpu.cores.len() as u32, info.logical_cores);
        assert!(frames.windows(2).all(|w| w[0].at_ms < w[1].at_ms));
        // Every parent is a process of the table.
        let pids: Vec<u32> = last.procs.iter().map(|p| p.raw.key.pid).collect();
        assert!(last.procs.iter().filter_map(|p| p.raw.parent).all(|p| pids.contains(&p)));
    }
}

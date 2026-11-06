//! Sensors (hwmon, thermal zones), GPUs (DRM sysfs, NVML) and batteries.

use std::collections::HashMap;
use std::fs;

use super::procs::EngineTotals;
use super::{nvml, read_text, read_trimmed};
use crate::sysmon::parse::hw as phw;
use crate::sysmon::{BatteryRaw, GpuEngine, GpuRaw, SensorKind, SensorRaw};

/// Sensors and cards are listed again this often (in samples); values are read every time.
const RESCAN_TICKS: u64 = 60;

/// A sensor's fixed description; only `input` is read per sample.
struct SensorSpec {
    input: String,
    id: String,
    group: String,
    label: String,
    kind: SensorKind,
    max: Option<f32>,
    critical: Option<f32>,
}

struct Card {
    id: String,
    pdev: Option<String>,
    device: String,
    name: String,
    vendor: String,
    hwmon: Option<String>,
    /// Intel clock files, whichever exists.
    clock_file: Option<String>,
}

pub(super) struct Hardware {
    tick: u64,
    specs: Vec<SensorSpec>,
    cards: Vec<Card>,
    /// `Some(None)`: tried, no NVML here.
    nvml: Option<Option<nvml::Nvml>>,
}

impl Hardware {
    pub(super) fn new() -> Self {
        Hardware { tick: 0, specs: Vec::new(), cards: Vec::new(), nvml: None }
    }

    fn nvml(&mut self) -> Option<&mut nvml::Nvml> {
        self.nvml.get_or_insert_with(nvml::Nvml::load).as_mut()
    }

    pub(super) fn nvml_process_usage(&mut self) -> HashMap<u32, nvml::ProcUse> {
        self.nvml().map(|n| n.process_usage()).unwrap_or_default()
    }

    pub(super) fn sensors(&mut self) -> Vec<SensorRaw> {
        if self.tick.is_multiple_of(RESCAN_TICKS) {
            self.specs = scan_sensors();
            self.cards = scan_cards(std::mem::take(&mut self.cards));
        }
        self.tick = self.tick.wrapping_add(1);
        self.specs
            .iter()
            .filter_map(|s| {
                // Some chips answer with an error while asleep: no value, no record.
                let value = phw::hwmon_value(s.kind, &read_text(&s.input)?)?;
                Some(SensorRaw {
                    id: s.id.clone(),
                    group: s.group.clone(),
                    label: s.label.clone(),
                    kind: s.kind,
                    value,
                    max: s.max,
                    critical: s.critical,
                })
            })
            .collect()
    }

    pub(super) fn gpus(&mut self, totals: &EngineTotals) -> Vec<GpuRaw> {
        let mut out = self.nvml().map(|n| n.gpus()).unwrap_or_default();
        let have_nvml = !out.is_empty();
        for card in &self.cards {
            // NVML already reported this adapter, and better.
            if have_nvml && card.vendor == "NVIDIA" {
                continue;
            }
            out.push(read_card(card, totals));
        }
        out
    }
}

fn read_num(path: &str) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

fn read_card(card: &Card, totals: &EngineTotals) -> GpuRaw {
    let dev = &card.device;
    let mut engines: Vec<GpuEngine> = card
        .pdev
        .as_ref()
        .map(|pdev| {
            totals
                .iter()
                .filter(|((d, _), _)| d == pdev)
                .map(|((_, e), pct)| GpuEngine { name: e.clone(), percent: pct.min(100.0) })
                .collect()
        })
        .unwrap_or_default();
    engines.sort_by(|a, b| a.name.cmp(&b.name));
    let busy = read_num(&format!("{dev}/gpu_busy_percent")).map(|v| v.min(100) as f32);
    let percent = busy.or_else(|| engines.iter().map(|e| e.percent).reduce(f32::max));
    let mut g = GpuRaw {
        id: card.id.clone(),
        name: card.name.clone(),
        vendor: card.vendor.clone(),
        percent,
        engines,
        mem_used: read_num(&format!("{dev}/mem_info_vram_used")),
        mem_total: read_num(&format!("{dev}/mem_info_vram_total")),
        shared_used: read_num(&format!("{dev}/mem_info_gtt_used")),
        ..GpuRaw::default()
    };
    if let Some(h) = &card.hwmon {
        g.temperature_c =
            read_text(&format!("{h}/temp1_input")).and_then(|s| phw::hwmon_value(SensorKind::Temperature, &s));
        g.power_w = read_text(&format!("{h}/power1_input"))
            .or_else(|| read_text(&format!("{h}/power1_average")))
            .and_then(|s| phw::hwmon_value(SensorKind::Power, &s));
        g.fan_rpm = read_num(&format!("{h}/fan1_input")).map(|v| v as u32);
        // hwmon reports these in hertz.
        g.clock_mhz = read_num(&format!("{h}/freq1_input")).map(|v| (v / 1_000_000) as u32);
        g.mem_clock_mhz = read_num(&format!("{h}/freq2_input")).map(|v| (v / 1_000_000) as u32);
    }
    if g.clock_mhz.is_none() {
        g.clock_mhz = card.clock_file.as_ref().and_then(|f| read_num(f)).map(|v| v as u32);
    }
    g
}

fn dir_names(dir: &str) -> Vec<String> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<String> = rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
    v.sort();
    v
}

fn link_name(path: &str) -> Option<String> {
    Some(fs::read_link(path).ok()?.file_name()?.to_string_lossy().into_owned())
}

fn scan_sensors() -> Vec<SensorSpec> {
    let mut out = Vec::new();
    let chips: Vec<(String, String)> = dir_names("/sys/class/hwmon")
        .into_iter()
        .map(|d| {
            let dir = format!("/sys/class/hwmon/{d}");
            let chip = read_trimmed(&format!("{dir}/name")).unwrap_or_else(|| d.clone());
            (dir, chip)
        })
        .collect();
    for (dir, chip) in &chips {
        let device = link_name(&format!("{dir}/device")).unwrap_or_default();
        // Two drives of one kind print identical labels: name the device then.
        let twins = chips.iter().filter(|(_, c)| c == chip).count() > 1;
        let names = dir_names(dir);
        for ch in phw::hwmon_channels(names.iter().map(String::as_str)) {
            let extra = |suffix: &str| {
                read_text(&format!("{dir}/{}_{suffix}", ch.prefix))
                    .and_then(|s| phw::hwmon_value(ch.kind, &s))
                    .filter(|v| *v > 0.0)
            };
            let base = read_trimmed(&format!("{dir}/{}_label", ch.prefix)).unwrap_or_else(|| ch.prefix.clone());
            out.push(SensorSpec {
                input: format!("{dir}/{}", ch.input),
                id: format!("hwmon:{chip}:{device}:{}", ch.prefix),
                group: chip.clone(),
                label: if twins && !device.is_empty() { format!("{base} ({device})") } else { base },
                kind: ch.kind,
                max: extra("max"),
                critical: extra("crit"),
            });
        }
    }
    for zone in dir_names("/sys/class/thermal") {
        if !zone.starts_with("thermal_zone") {
            continue;
        }
        let dir = format!("/sys/class/thermal/{zone}");
        out.push(SensorSpec {
            input: format!("{dir}/temp"),
            id: format!("thermal:{zone}"),
            group: read_trimmed(&format!("{dir}/type")).unwrap_or_else(|| "thermal".to_owned()),
            label: zone,
            kind: SensorKind::Temperature,
            max: None,
            critical: None,
        });
    }
    out
}

fn scan_cards(old: Vec<Card>) -> Vec<Card> {
    let mut names: HashMap<String, String> = old.into_iter().map(|c| (c.id, c.name)).collect();
    let mut out = Vec::new();
    for card in dir_names("/sys/class/drm") {
        if !phw::is_drm_card(&card) {
            continue;
        }
        let base = format!("/sys/class/drm/{card}");
        let device = format!("{base}/device");
        let Some(uevent) = read_text(&format!("{device}/uevent")) else { continue };
        let driver = phw::uevent_value(&uevent, "DRIVER").unwrap_or("").to_owned();
        // Software-only DRM devices are no GPU a person would want to monitor.
        if matches!(driver.as_str(), "vgem" | "vkms" | "faux_driver") {
            continue;
        }
        let pdev = phw::uevent_value(&uevent, "PCI_SLOT_NAME").map(str::to_owned);
        let pci_id = phw::uevent_value(&uevent, "PCI_ID");
        let vendor_id = pci_id.and_then(|p| p.split(':').next()).map(str::to_ascii_lowercase);
        let device_id = pci_id.and_then(|p| p.split(':').nth(1)).map(str::to_ascii_lowercase);
        let vendor = vendor_id.as_deref().map(phw::pci_vendor_name).unwrap_or("").to_owned();
        let id = pdev.clone().unwrap_or_else(|| card.clone());
        let name = names.remove(&id).unwrap_or_else(|| {
            read_trimmed(&format!("{device}/product_name"))
                .or_else(|| pci_name(vendor_id.as_deref()?, device_id.as_deref()?))
                .unwrap_or_else(|| if vendor.is_empty() { driver.clone() } else { format!("{vendor} {driver}") })
        });
        let hwmon = dir_names(&format!("{device}/hwmon")).into_iter().next().map(|h| format!("{device}/hwmon/{h}"));
        let clock_file = [
            format!("{base}/gt_cur_freq_mhz"),
            format!("{base}/gt/gt0/rps_cur_freq_mhz"),
            format!("{device}/tile0/gt0/freq0/act_freq"),
        ]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists());
        out.push(Card { id, pdev, device, name, vendor, hwmon, clock_file });
    }
    out
}

fn pci_name(vendor: &str, device: &str) -> Option<String> {
    let text = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids"].iter().find_map(|p| read_text(p))?;
    phw::pci_ids_lookup(&text, vendor, device)
}

pub(super) fn batteries() -> Vec<BatteryRaw> {
    let names = dir_names("/sys/class/power_supply");
    let events: Vec<(String, String)> = names
        .into_iter()
        .filter_map(|n| {
            let t = read_text(&format!("/sys/class/power_supply/{n}/uevent"))?;
            Some((n, t))
        })
        .collect();
    let on_ac = events.iter().any(|(_, t)| phw::uevent_is_mains_online(t));
    events.iter().filter_map(|(n, t)| phw::parse_battery_uevent(n, t, on_ac)).collect()
}

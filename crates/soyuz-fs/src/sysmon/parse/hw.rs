//! hwmon, thermal, DRM and power-supply files of `/sys`.

use crate::sysmon::{BatteryRaw, SensorKind, SensorRaw};

/// One sensor channel of a hwmon directory (`temp1`, `fan2`, …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HwmonChannel {
    pub kind: SensorKind,
    /// `temp1`.
    pub prefix: String,
    /// `temp1_input`, or `power1_average`.
    pub input: String,
    pub number: u32,
}

/// The channels among a hwmon directory's file names, sorted by kind and number.
pub(crate) fn hwmon_channels<'a>(names: impl Iterator<Item = &'a str>) -> Vec<HwmonChannel> {
    let mut out: Vec<HwmonChannel> = Vec::new();
    for name in names {
        let Some((prefix, suffix)) = name.split_once('_') else { continue };
        let (kind_name, number) = prefix.split_at(prefix.find(|c: char| c.is_ascii_digit()).unwrap_or(prefix.len()));
        let Ok(number) = number.parse() else { continue };
        let kind = match kind_name {
            "temp" => SensorKind::Temperature,
            "fan" => SensorKind::Fan,
            "power" => SensorKind::Power,
            "in" => SensorKind::Voltage,
            "curr" => SensorKind::Current,
            _ => continue,
        };
        let value_file = suffix == "input" || (kind == SensorKind::Power && suffix == "average");
        if !value_file {
            continue;
        }
        match out.iter_mut().find(|c| c.prefix == prefix) {
            // `powerN_input` wins over `powerN_average`: it is the current value.
            Some(c) => {
                if suffix == "input" {
                    c.input = name.to_owned();
                }
            }
            None => out.push(HwmonChannel { kind, prefix: prefix.to_owned(), input: name.to_owned(), number }),
        }
    }
    let rank = |k: SensorKind| match k {
        SensorKind::Temperature => 0,
        SensorKind::Fan => 1,
        SensorKind::Power => 2,
        SensorKind::Voltage => 3,
        SensorKind::Current => 4,
    };
    out.sort_by_key(|c| (rank(c.kind), c.number));
    out
}

/// A hwmon file's text to °C (from m°C), RPM, W (µW), V (mV) or A (mA).
pub(crate) fn hwmon_value(kind: SensorKind, s: &str) -> Option<f32> {
    let raw: f64 = s.trim().parse().ok()?;
    if !raw.is_finite() {
        return None;
    }
    let scaled = match kind {
        SensorKind::Temperature | SensorKind::Voltage | SensorKind::Current => raw / 1000.0,
        SensorKind::Fan => raw,
        SensorKind::Power => raw / 1_000_000.0,
    };
    Some(scaled as f32)
}

/// The package temperature among the sensors, in order of trust.
pub(crate) fn pick_cpu_temp(sensors: &[SensorRaw]) -> Option<f32> {
    let temp = |group: &str, label: Option<&str>| {
        sensors
            .iter()
            .filter(|s| s.kind == SensorKind::Temperature && s.group == group)
            .find(|s| label.is_none_or(|l| s.label.starts_with(l)))
            .map(|s| s.value)
    };
    temp("coretemp", Some("Package"))
        .or_else(|| temp("k10temp", Some("Tctl")))
        .or_else(|| temp("k10temp", Some("Tdie")))
        .or_else(|| temp("zenpower", Some("Tdie")))
        .or_else(|| temp("x86_pkg_temp", None))
        .or_else(|| temp("cpu_thermal", None))
        .or_else(|| temp("cpu-thermal", None))
        .or_else(|| temp("coretemp", None))
        .or_else(|| temp("k10temp", None))
}

/// `card0` yes; `card0-DP-1`, `renderD128`, `version` no.
pub(crate) fn is_drm_card(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The value of `KEY=value` in a uevent file.
pub(crate) fn uevent_value<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    s.lines().find_map(|l| l.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

/// A PCI vendor id (`0x10de`, `10DE`) to the vendor's name; empty for an unknown one.
pub(crate) fn pci_vendor_name(id: &str) -> &'static str {
    let id = id.trim().trim_start_matches("0x").to_ascii_lowercase();
    match id.as_str() {
        "10de" => "NVIDIA",
        "1002" | "1022" => "AMD",
        "8086" => "Intel",
        "1af4" => "Red Hat VirtIO",
        "15ad" => "VMware",
        "1414" => "Microsoft",
        "5143" => "Qualcomm",
        "1234" => "QEMU",
        _ => "",
    }
}

/// The device's name in a `pci.ids` file; ids are four lower-case hex digits.
pub(crate) fn pci_ids_lookup(text: &str, vendor: &str, device: &str) -> Option<String> {
    let mut in_vendor = false;
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with("C ") {
            break; // Device classes follow the vendor list.
        }
        if let Some(rest) = line.strip_prefix('\t') {
            if in_vendor
                && !rest.starts_with('\t')
                && let Some(name) = rest.strip_prefix(device).filter(|n| n.starts_with(' '))
            {
                return Some(name.trim().to_owned());
            }
        } else if in_vendor {
            return None; // Past the vendor's block.
        } else {
            in_vendor = line.strip_prefix(vendor).is_some_and(|r| r.starts_with(' '));
        }
    }
    None
}

/// `/sys/class/power_supply/<name>/uevent` of a system battery; `None` for anything else
/// (AC adapters, batteries of mice and headsets, which report scope `Device`).
pub(crate) fn parse_battery_uevent(name: &str, s: &str, on_ac: bool) -> Option<BatteryRaw> {
    let get = |k: &str| uevent_value(s, k);
    let num = |k: &str| get(k).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
    if get("POWER_SUPPLY_TYPE") != Some("Battery") || get("POWER_SUPPLY_SCOPE") == Some("Device") {
        return None;
    }
    if get("POWER_SUPPLY_PRESENT") == Some("0") {
        return None;
    }
    let status = get("POWER_SUPPLY_STATUS").unwrap_or("");
    let charging = status.eq_ignore_ascii_case("charging");
    let discharging = status.eq_ignore_ascii_case("discharging");

    // Watt-hours (ENERGY_*, µWh) or amp-hours (CHARGE_*, µAh); volts turn the latter into watts.
    let volts = num("POWER_SUPPLY_VOLTAGE_NOW").map(|v| v / 1e6).filter(|v| *v > 0.0);
    let watts = num("POWER_SUPPLY_POWER_NOW")
        .map(|p| p.abs() / 1e6)
        .or_else(|| Some(num("POWER_SUPPLY_CURRENT_NOW")?.abs() / 1e6 * volts?));
    let now_wh = num("POWER_SUPPLY_ENERGY_NOW")
        .map(|e| e / 1e6)
        .or_else(|| Some(num("POWER_SUPPLY_CHARGE_NOW")? / 1e6 * volts?));
    let full_wh = num("POWER_SUPPLY_ENERGY_FULL")
        .map(|e| e / 1e6)
        .or_else(|| Some(num("POWER_SUPPLY_CHARGE_FULL")? / 1e6 * volts?));

    let percent = num("POWER_SUPPLY_CAPACITY").or_else(|| {
        let (a, b) = (now_wh?, full_wh?);
        (b > 0.0).then(|| a / b * 100.0)
    })?;
    let design = num("POWER_SUPPLY_ENERGY_FULL_DESIGN")
        .zip(num("POWER_SUPPLY_ENERGY_FULL"))
        .or_else(|| num("POWER_SUPPLY_CHARGE_FULL_DESIGN").zip(num("POWER_SUPPLY_CHARGE_FULL")));
    let health_percent = design.and_then(|(d, f)| (d > 0.0).then_some((f / d * 100.0) as f32));

    let seconds_left = watts.filter(|w| *w > 0.01).and_then(|w| {
        let wh = if discharging {
            now_wh?
        } else if charging {
            (full_wh? - now_wh?).max(0.0)
        } else {
            return None;
        };
        Some((wh / w * 3600.0) as u64)
    });
    let power_w = watts.map(|w| if discharging { -w as f32 } else { w as f32 });
    let cycles = num("POWER_SUPPLY_CYCLE_COUNT").filter(|c| *c > 0.0).map(|c| c as u32);
    let model = [get("POWER_SUPPLY_MANUFACTURER"), get("POWER_SUPPLY_MODEL_NAME")]
        .iter()
        .flatten()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(BatteryRaw {
        id: name.to_owned(),
        name: if model.is_empty() { name.to_owned() } else { model },
        percent: percent.clamp(0.0, 100.0) as f32,
        charging,
        on_ac: on_ac || charging,
        seconds_left,
        power_w,
        health_percent,
        cycles,
    })
}

/// A mains adapter's uevent says it is plugged in.
pub(crate) fn uevent_is_mains_online(s: &str) -> bool {
    let kind = uevent_value(s, "POWER_SUPPLY_TYPE").unwrap_or("");
    let mains = matches!(kind, "Mains" | "USB" | "USB_C" | "USB_PD" | "USB_PD_DRP" | "BrickID");
    mains && uevent_value(s, "POWER_SUPPLY_ONLINE") == Some("1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels() {
        let names = [
            "name",
            "temp2_input",
            "temp1_input",
            "temp1_label",
            "temp1_crit",
            "fan1_input",
            "fan1_min",
            "power1_average",
            "power1_cap",
            "in0_input",
            "curr1_input",
            "freq1_input",
            "uevent",
            "device",
            "pwm1",
            "temp_input",
            "tempx_input",
        ];
        let c = hwmon_channels(names.iter().copied());
        let got: Vec<&str> = c.iter().map(|c| c.input.as_str()).collect();
        assert_eq!(got, ["temp1_input", "temp2_input", "fan1_input", "power1_average", "in0_input", "curr1_input"]);
        assert_eq!(c[0].prefix, "temp1");
        let both = hwmon_channels(["power1_average", "power1_input"].iter().copied());
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].input, "power1_input");
    }

    #[test]
    fn values() {
        assert_eq!(hwmon_value(SensorKind::Temperature, "45500\n"), Some(45.5));
        assert_eq!(hwmon_value(SensorKind::Fan, "1200\n"), Some(1200.0));
        assert_eq!(hwmon_value(SensorKind::Power, "25000000\n"), Some(25.0));
        assert_eq!(hwmon_value(SensorKind::Voltage, "1200\n"), Some(1.2));
        assert_eq!(hwmon_value(SensorKind::Current, "500\n"), Some(0.5));
        assert_eq!(hwmon_value(SensorKind::Fan, "junk"), None);
        assert_eq!(hwmon_value(SensorKind::Fan, ""), None);
        assert_eq!(hwmon_value(SensorKind::Fan, "inf"), None);
    }

    fn sensor(group: &str, label: &str, value: f32) -> SensorRaw {
        SensorRaw { group: group.into(), label: label.into(), value, ..SensorRaw::default() }
    }

    #[test]
    fn cpu_temperature_choice() {
        let v = [
            sensor("nvme", "Composite", 30.0),
            sensor("coretemp", "Core 0", 40.0),
            sensor("coretemp", "Package id 0", 50.0),
        ];
        assert_eq!(pick_cpu_temp(&v), Some(50.0));
        let v = [sensor("k10temp", "Tccd1", 40.0), sensor("k10temp", "Tctl", 60.0)];
        assert_eq!(pick_cpu_temp(&v), Some(60.0));
        assert_eq!(pick_cpu_temp(&[sensor("cpu_thermal", "temp1", 44.0)]), Some(44.0));
        assert_eq!(pick_cpu_temp(&[sensor("nvme", "Composite", 30.0)]), None);
        let fan = SensorRaw { group: "coretemp".into(), kind: SensorKind::Fan, ..SensorRaw::default() };
        assert_eq!(pick_cpu_temp(&[fan]), None);
    }

    #[test]
    fn drm_names_and_uevent() {
        assert!(is_drm_card("card0"));
        assert!(is_drm_card("card12"));
        assert!(!is_drm_card("card0-DP-1"));
        assert!(!is_drm_card("card"));
        assert!(!is_drm_card("renderD128"));
        let u = "DRIVER=amdgpu\nPCI_ID=1002:73BF\nPCI_SLOT_NAME=0000:03:00.0\nbad\n";
        assert_eq!(uevent_value(u, "DRIVER"), Some("amdgpu"));
        assert_eq!(uevent_value(u, "PCI_ID"), Some("1002:73BF"));
        assert_eq!(uevent_value(u, "NOPE"), None);
        assert_eq!(pci_vendor_name("0x10de\n"), "NVIDIA");
        assert_eq!(pci_vendor_name("1002"), "AMD");
        assert_eq!(pci_vendor_name("ffff"), "");
    }

    #[test]
    fn pci_ids() {
        let t = "# comment\n\n1000  Broadcom\n\t0001  Thing\n1002  Advanced Micro Devices\n\t1234  Not it\n\t\t1111 2222  Sub\n\
                 \t73bf  Navi 21 [Radeon RX 6900 XT]\n1003  Next\n\t73bf  Other\nC 00  Class\n\t73bf  Wrong\n";
        assert_eq!(pci_ids_lookup(t, "1002", "73bf").as_deref(), Some("Navi 21 [Radeon RX 6900 XT]"));
        assert_eq!(pci_ids_lookup(t, "1002", "ffff"), None);
        assert_eq!(pci_ids_lookup(t, "9999", "73bf"), None);
        assert_eq!(pci_ids_lookup("", "1002", "73bf"), None);
    }

    #[test]
    fn battery_energy_discharging() {
        let s = "POWER_SUPPLY_NAME=BAT0\nPOWER_SUPPLY_TYPE=Battery\nPOWER_SUPPLY_STATUS=Discharging\nPOWER_SUPPLY_PRESENT=1\n\
                 POWER_SUPPLY_CYCLE_COUNT=120\nPOWER_SUPPLY_VOLTAGE_NOW=12000000\nPOWER_SUPPLY_POWER_NOW=10000000\n\
                 POWER_SUPPLY_ENERGY_FULL_DESIGN=50000000\nPOWER_SUPPLY_ENERGY_FULL=40000000\nPOWER_SUPPLY_ENERGY_NOW=20000000\n\
                 POWER_SUPPLY_CAPACITY=50\nPOWER_SUPPLY_MODEL_NAME=DELL X\nPOWER_SUPPLY_MANUFACTURER=SMP\n";
        let b = parse_battery_uevent("BAT0", s, false).unwrap();
        assert_eq!(b.id, "BAT0");
        assert_eq!(b.name, "SMP DELL X");
        assert_eq!(b.percent, 50.0);
        assert!(!b.charging);
        assert!(!b.on_ac);
        assert_eq!(b.power_w, Some(-10.0));
        assert_eq!(b.seconds_left, Some(7200));
        assert_eq!(b.health_percent, Some(80.0));
        assert_eq!(b.cycles, Some(120));
    }

    #[test]
    fn battery_charge_charging() {
        let s = "POWER_SUPPLY_TYPE=Battery\nPOWER_SUPPLY_STATUS=Charging\nPOWER_SUPPLY_VOLTAGE_NOW=10000000\nPOWER_SUPPLY_CURRENT_NOW=2000000\n\
                 POWER_SUPPLY_CHARGE_FULL=4000000\nPOWER_SUPPLY_CHARGE_NOW=2000000\nPOWER_SUPPLY_CHARGE_FULL_DESIGN=5000000\nPOWER_SUPPLY_CYCLE_COUNT=0\n";
        let b = parse_battery_uevent("BAT1", s, false).unwrap();
        assert_eq!(b.name, "BAT1");
        assert_eq!(b.percent, 50.0);
        assert!(b.charging && b.on_ac);
        assert_eq!(b.power_w, Some(20.0));
        assert_eq!(b.seconds_left, Some(3600));
        assert_eq!(b.health_percent, Some(80.0));
        assert_eq!(b.cycles, None);
    }

    #[test]
    fn battery_rejects() {
        assert!(parse_battery_uevent("AC", "POWER_SUPPLY_TYPE=Mains\nPOWER_SUPPLY_ONLINE=1\n", false).is_none());
        let mouse = "POWER_SUPPLY_TYPE=Battery\nPOWER_SUPPLY_SCOPE=Device\nPOWER_SUPPLY_CAPACITY=20\n";
        assert!(parse_battery_uevent("hid", mouse, false).is_none());
        let absent = "POWER_SUPPLY_TYPE=Battery\nPOWER_SUPPLY_PRESENT=0\nPOWER_SUPPLY_CAPACITY=0\n";
        assert!(parse_battery_uevent("BAT0", absent, false).is_none());
        assert!(parse_battery_uevent("BAT0", "POWER_SUPPLY_TYPE=Battery\n", false).is_none());
        assert!(parse_battery_uevent("BAT0", "", false).is_none());
        let junk = "POWER_SUPPLY_TYPE=Battery\nPOWER_SUPPLY_CAPACITY=500\nPOWER_SUPPLY_POWER_NOW=x\n";
        assert_eq!(parse_battery_uevent("BAT0", junk, true).unwrap().percent, 100.0);
    }

    #[test]
    fn mains() {
        assert!(uevent_is_mains_online("POWER_SUPPLY_TYPE=Mains\nPOWER_SUPPLY_ONLINE=1\n"));
        assert!(!uevent_is_mains_online("POWER_SUPPLY_TYPE=Mains\nPOWER_SUPPLY_ONLINE=0\n"));
        assert!(!uevent_is_mains_online("POWER_SUPPLY_TYPE=Battery\nPOWER_SUPPLY_ONLINE=1\n"));
        assert!(!uevent_is_mains_online(""));
    }
}

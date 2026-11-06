//! Thermal zones and power meters (performance counters), batteries (kernel32, powrprof).

use std::mem::size_of;
use std::ptr::null;

use windows_sys::Win32::System::Power::{
    CallNtPowerInformation, GetSystemPowerStatus, SYSTEM_BATTERY_STATE, SYSTEM_POWER_STATUS, SystemBatteryState,
};

use super::pdh::Query;
use crate::sysmon::{BatteryRaw, SensorKind, SensorRaw};

/// `BatteryFlag`: no system battery; status not known.
const FLAG_NO_BATTERY: u8 = 128;
const FLAG_UNKNOWN: u8 = 255;
/// `BatteryFlag`: charging.
const FLAG_CHARGING: u8 = 8;

/// `(query, group, id prefix)` of a meter whose counter reads milliwatts.
struct Meter {
    query: Query,
    group: &'static str,
    prefix: &'static str,
}

#[derive(Default)]
pub(super) struct SensorReader {
    tried: bool,
    thermal: Option<Query>,
    meters: Vec<Meter>,
}

impl SensorReader {
    pub(super) fn read(&mut self) -> Vec<SensorRaw> {
        if !self.tried {
            self.tried = true;
            self.thermal = Query::with(&[r"\Thermal Zone Information(*)\Temperature"]);
            let mut meter = |path, group, prefix| {
                if let Some(query) = Query::with(&[path]) {
                    self.meters.push(Meter { query, group, prefix });
                }
            };
            meter(r"\Power Meter(*)\Power", "Power Meter", "power");
            meter(r"\Energy Meter(*)\Power", "Energy Meter", "energy");
        }
        let mut out = Vec::new();
        if let Some(q) = &mut self.thermal
            && q.collect()
        {
            q.values(0, |name, kelvin| {
                let label = String::from_utf16_lossy(name);
                if let Some(value) = kelvin_to_celsius(kelvin) {
                    out.push(SensorRaw {
                        id: format!("thermal:{label}"),
                        group: "ACPI Thermal Zone".to_string(),
                        label,
                        kind: SensorKind::Temperature,
                        value,
                        max: None,
                        critical: None,
                    });
                }
            });
        }
        for m in &mut self.meters {
            if !m.query.collect() {
                continue;
            }
            let (group, prefix) = (m.group, m.prefix);
            m.query.values(0, |name, milliwatts| {
                let label = String::from_utf16_lossy(name);
                if label != "_Total" && milliwatts >= 0.0 {
                    out.push(SensorRaw {
                        id: format!("{prefix}:{label}"),
                        group: group.to_string(),
                        label,
                        kind: SensorKind::Power,
                        value: (milliwatts / 1000.0) as f32,
                        max: None,
                        critical: None,
                    });
                }
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

/// A plausible reading only: an unsupported zone reads 0 K.
fn kelvin_to_celsius(k: f64) -> Option<f32> {
    let c = k - 273.15;
    (k > 0.0 && (-60.0..200.0).contains(&c)).then_some(c as f32)
}

/// The system battery, as one record (the OS adds several packs together here).
pub(super) fn batteries() -> Vec<BatteryRaw> {
    let mut s = SYSTEM_POWER_STATUS {
        ACLineStatus: 0,
        BatteryFlag: 0,
        BatteryLifePercent: 0,
        SystemStatusFlag: 0,
        BatteryLifeTime: 0,
        BatteryFullLifeTime: 0,
    };
    // SAFETY: `s` is writable.
    if unsafe { GetSystemPowerStatus(&mut s) } == 0 {
        return Vec::new();
    }
    if s.BatteryFlag == FLAG_UNKNOWN || s.BatteryFlag & FLAG_NO_BATTERY != 0 {
        return Vec::new();
    }
    let mut state = SYSTEM_BATTERY_STATE::default();
    // SAFETY: no input; `state` is writable for the size we pass.
    let st = unsafe {
        CallNtPowerInformation(
            SystemBatteryState,
            null(),
            0,
            (&raw mut state).cast(),
            size_of::<SYSTEM_BATTERY_STATE>() as u32,
        )
    };
    vec![battery_of(&s, (st == 0).then_some(&state))]
}

fn battery_of(s: &SYSTEM_POWER_STATUS, state: Option<&SYSTEM_BATTERY_STATE>) -> BatteryRaw {
    let mut b = BatteryRaw {
        id: "battery".to_string(),
        name: "Battery".to_string(),
        percent: if s.BatteryLifePercent <= 100 { f32::from(s.BatteryLifePercent) } else { 0.0 },
        charging: s.BatteryFlag & FLAG_CHARGING != 0,
        on_ac: s.ACLineStatus == 1,
        seconds_left: (s.BatteryLifeTime != u32::MAX).then_some(u64::from(s.BatteryLifeTime)),
        ..BatteryRaw::default()
    };
    if let Some(state) = state {
        // Signed milliwatts in an unsigned field; `0x80000000` means not known.
        let rate = state.Rate as i32;
        if rate != 0 && rate != i32::MIN {
            b.power_w = Some(rate as f32 / 1000.0);
        }
        if s.BatteryLifePercent > 100 && state.MaxCapacity > 0 {
            b.percent = (f64::from(state.RemainingCapacity) * 100.0 / f64::from(state.MaxCapacity)) as f32;
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(flag: u8, percent: u8, time: u32) -> SYSTEM_POWER_STATUS {
        SYSTEM_POWER_STATUS {
            ACLineStatus: 0,
            BatteryFlag: flag,
            BatteryLifePercent: percent,
            SystemStatusFlag: 0,
            BatteryLifeTime: time,
            BatteryFullLifeTime: 0,
        }
    }

    #[test]
    fn battery_values() {
        let state = SYSTEM_BATTERY_STATE {
            Rate: (-7500i32) as u32,
            MaxCapacity: 50_000,
            RemainingCapacity: 25_000,
            ..SYSTEM_BATTERY_STATE::default()
        };
        let b = battery_of(&status(FLAG_CHARGING, 80, 600), Some(&state));
        assert_eq!(b.percent, 80.0);
        assert!(b.charging);
        assert_eq!(b.seconds_left, Some(600));
        assert_eq!(b.power_w, Some(-7.5));
        // Percent not known: derived from the capacities. Time not known stays `None`.
        let b = battery_of(&status(1, 255, u32::MAX), Some(&state));
        assert_eq!(b.percent, 50.0);
        assert_eq!(b.seconds_left, None);
        // Rate not known.
        let state = SYSTEM_BATTERY_STATE { Rate: 0x8000_0000, ..SYSTEM_BATTERY_STATE::default() };
        assert_eq!(battery_of(&status(1, 50, 0), Some(&state)).power_w, None);
    }

    #[test]
    fn thermal_values() {
        assert_eq!(kelvin_to_celsius(0.0), None);
        assert_eq!(kelvin_to_celsius(1000.0), None);
        assert!((kelvin_to_celsius(300.0).unwrap() - 26.85).abs() < 0.01);
    }
}

//! The per-OS API with nothing behind it. `crate::sys::<os>::sysmon` modules provide the
//! same names.

use super::{
    Connection, ControlError, DetailsWant, ProcDetails, ProcKey, ProcPriority, ProcSignal, RawSample, ServiceInfo,
    ServiceOp, StartupItem, SystemInfo, Want,
};

pub struct Probe;

impl Probe {
    pub fn new() -> Self {
        Probe
    }

    pub fn sample(&mut self, _want: Want) -> RawSample {
        RawSample::default()
    }
}

pub fn system_info() -> SystemInfo {
    SystemInfo::default()
}

pub fn connections() -> Vec<Connection> {
    Vec::new()
}

pub fn services() -> Vec<ServiceInfo> {
    Vec::new()
}

pub fn service_control(_name: &str, _op: ServiceOp) -> Result<(), ControlError> {
    Err(ControlError::Unsupported)
}

pub fn startup_items() -> Vec<StartupItem> {
    Vec::new()
}

pub fn process_details(_key: ProcKey, _want: DetailsWant) -> Result<ProcDetails, ControlError> {
    Err(ControlError::Unsupported)
}

pub fn signal(_key: ProcKey, _sig: ProcSignal) -> Result<(), ControlError> {
    Err(ControlError::Unsupported)
}

pub fn set_priority(_key: ProcKey, _p: ProcPriority) -> Result<(), ControlError> {
    Err(ControlError::Unsupported)
}

pub fn affinity(_key: ProcKey) -> Result<Vec<u32>, ControlError> {
    Err(ControlError::Unsupported)
}

pub fn set_affinity(_key: ProcKey, _cpus: &[u32]) -> Result<(), ControlError> {
    Err(ControlError::Unsupported)
}

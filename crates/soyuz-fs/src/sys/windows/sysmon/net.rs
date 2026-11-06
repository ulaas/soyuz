//! Network interfaces (`GetIfTable2`, `GetAdaptersAddresses`) and sockets
//! (`GetExtendedTcpTable`, `GetExtendedUdpTable`).

use std::collections::HashMap;
use std::mem::size_of;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses,
    GetExtendedTcpTable, GetExtendedUdpTable, GetIfTable2, IP_ADAPTER_ADDRESSES_LH, MIB_IF_ROW2, MIB_IF_TABLE2,
    MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, MIB_UDP6ROW_OWNER_PID, MIB_UDPROW_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    UDP_TABLE_OWNER_PID,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6};

use super::super::from_wide_nul;
use super::{as_bytes, read_pod, u32_at};
use crate::sysmon::{Connection, NetRaw, Proto};

const ADDRESSES_EVERY: Duration = Duration::from_secs(10);
const MAX_TABLE_BYTES: usize = 64 << 20;

// `InterfaceAndOperStatusFlags` bits.
const FLAG_HARDWARE: u8 = 1 << 0;
const FLAG_FILTER: u8 = 1 << 1;
/// `NET_IF_ADMIN_STATUS_UP` and `IF_TYPE_SOFTWARE_LOOPBACK`.
const NET_IF_ADMIN_STATUS_UP: i32 = 1;
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;

#[derive(Default)]
pub(super) struct NetReader {
    addresses: HashMap<u64, Vec<String>>,
    addresses_at: Option<Instant>,
}

impl NetReader {
    pub(super) fn read(&mut self) -> Vec<NetRaw> {
        if self.addresses_at.is_none_or(|t| t.elapsed() >= ADDRESSES_EVERY) {
            self.addresses = adapter_addresses();
            self.addresses_at = Some(Instant::now());
        }
        let mut table: *mut MIB_IF_TABLE2 = null_mut();
        // SAFETY: valid out-pointer; on success the table is freed below.
        if unsafe { GetIfTable2(&mut table) } != NO_ERROR || table.is_null() {
            return Vec::new();
        }
        // SAFETY: on success `table` is a live MIB_IF_TABLE2 with `NumEntries` rows following
        // in place; it is freed only after the rows were copied out.
        let rows = unsafe {
            let n = (*table).NumEntries as usize;
            std::slice::from_raw_parts((&raw const (*table).Table).cast::<MIB_IF_ROW2>(), n)
        };
        let out = rows.iter().filter_map(|r| self.row(r)).collect();
        // SAFETY: table from GetIfTable2, freed once; `rows` is not used after this.
        unsafe { FreeMibTable(table.cast()) };
        out
    }

    fn row(&self, r: &MIB_IF_ROW2) -> Option<NetRaw> {
        let flags = r.InterfaceAndOperStatusFlags._bitfield;
        let up = r.OperStatus == IfOperStatusUp;
        // SAFETY: NET_LUID_LH is a union of plain integers; `Value` covers all of it.
        let luid = unsafe { r.InterfaceLuid.Value };
        let addresses = self.addresses.get(&luid).cloned().unwrap_or_default();
        let shape = Shape {
            hardware: flags & FLAG_HARDWARE != 0,
            filter: flags & FLAG_FILTER != 0,
            enabled: r.AdminStatus == NET_IF_ADMIN_STATUS_UP,
            up,
            loopback: r.Type == IF_TYPE_SOFTWARE_LOOPBACK,
            in_use: r.InOctets != 0 || r.OutOctets != 0 || !addresses.is_empty(),
        };
        if !keep_interface(&shape) {
            return None;
        }
        let mac_len = (r.PhysicalAddressLength as usize).min(r.PhysicalAddress.len());
        let mac = &r.PhysicalAddress[..mac_len];
        Some(NetRaw {
            id: luid.to_string(),
            name: from_wide_nul(&r.Alias).to_string_lossy().into_owned(),
            description: from_wide_nul(&r.Description).to_string_lossy().into_owned(),
            rx_bytes: r.InOctets,
            tx_bytes: r.OutOctets,
            rx_packets: r.InUcastPkts.saturating_add(r.InNUcastPkts),
            tx_packets: r.OutUcastPkts.saturating_add(r.OutNUcastPkts),
            rx_errors: r.InErrors,
            tx_errors: r.OutErrors,
            up,
            virtual_if: !shape.hardware,
            link_bps: link_speed(r.ReceiveLinkSpeed.max(r.TransmitLinkSpeed)),
            addresses,
            mac: format_mac(mac),
        })
    }
}

/// What decides whether an interface is worth listing.
#[derive(Clone, Copy)]
struct Shape {
    hardware: bool,
    filter: bool,
    enabled: bool,
    up: bool,
    loopback: bool,
    in_use: bool,
}

/// Filter-driver pseudo interfaces repeat the real one. Adapters turned off are not listed.
/// Software interfaces only count when up and in use (no WAN miniports, no unused tunnels);
/// loopback traffic never reaches its counters, so it counts anyway.
fn keep_interface(i: &Shape) -> bool {
    !i.filter && i.enabled && (i.hardware || (i.up && (i.in_use || i.loopback)))
}

/// `u64::MAX` and 0 both stand for "not known".
fn link_speed(bps: u64) -> Option<u64> {
    (bps != 0 && bps != u64::MAX).then_some(bps)
}

fn format_mac(mac: &[u8]) -> Option<String> {
    if mac.is_empty() || mac.iter().all(|&b| b == 0) {
        return None;
    }
    Some(mac.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"))
}

fn adapter_addresses() -> HashMap<u64, Vec<String>> {
    let mut out = HashMap::new();
    let mut buf = vec![0u64; (32 << 10) / 8];
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    for _ in 0..4 {
        let mut bytes = (buf.len() * 8) as u32;
        // SAFETY: `buf` is 8-aligned and writable for `bytes` bytes; valid out-pointer.
        let rc = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC),
                flags,
                null(),
                buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
                &mut bytes,
            )
        };
        if rc == ERROR_BUFFER_OVERFLOW && (bytes as usize) < MAX_TABLE_BYTES {
            buf.resize((bytes as usize).div_ceil(8) + 1, 0);
            continue;
        }
        if rc != NO_ERROR {
            return out;
        }
        let mut adapter = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while !adapter.is_null() {
            // SAFETY: the list lives in `buf`, linked by the call; `buf` is not changed here.
            let a = unsafe { &*adapter };
            // SAFETY: NET_LUID_LH is a union of plain integers; `Value` covers all of it.
            let luid = unsafe { a.Luid.Value };
            let mut list = Vec::new();
            let mut unicast = a.FirstUnicastAddress;
            while !unicast.is_null() {
                // SAFETY: same list; each socket address is valid for its family.
                unsafe {
                    let u = &*unicast;
                    if let Some(text) = sockaddr_text(u.Address.lpSockaddr.cast::<u8>().cast_const()) {
                        list.push(text);
                    }
                    unicast = u.Next;
                }
            }
            out.insert(luid, list);
            adapter = a.Next;
        }
        return out;
    }
    out
}

/// # Safety
/// `p` is null or points to a socket address that is valid for its family.
unsafe fn sockaddr_text(p: *const u8) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: per this function's contract; the family is the first field of each form.
    unsafe {
        match std::ptr::read_unaligned(p.cast::<u16>()) {
            AF_INET => {
                let a = std::ptr::read_unaligned(p.cast::<SOCKADDR_IN>());
                Some(Ipv4Addr::from(a.sin_addr.S_un.S_addr.to_ne_bytes()).to_string())
            }
            AF_INET6 => {
                let a = std::ptr::read_unaligned(p.cast::<SOCKADDR_IN6>());
                Some(Ipv6Addr::from(a.sin6_addr.u.Byte).to_string())
            }
            _ => None,
        }
    }
}

/// Fill a table through `call(buffer, &mut size)`: a bounded grow loop.
fn table(mut call: impl FnMut(*mut core::ffi::c_void, &mut u32) -> u32) -> Option<(Vec<u64>, usize)> {
    let mut buf = vec![0u64; (64 << 10) / 8];
    for _ in 0..6 {
        let mut bytes = (buf.len() * 8) as u32;
        let rc = call(buf.as_mut_ptr().cast(), &mut bytes);
        if rc == ERROR_INSUFFICIENT_BUFFER {
            // Sockets come and go between calls: leave room.
            let want = bytes as usize + (16 << 10);
            if want > MAX_TABLE_BYTES {
                return None;
            }
            buf.resize(want.div_ceil(8), 0);
            continue;
        }
        if rc != NO_ERROR {
            return None;
        }
        let len = (bytes as usize).min(buf.len() * 8);
        return Some((buf, len));
    }
    None
}

fn port(v: u32) -> u16 {
    u16::from_be(v as u16)
}

fn tcp_state(s: u32, remote_port: u32) -> &'static str {
    match s {
        // Some Windows builds report listeners as 0; a socket with no remote end is one.
        0 if port(remote_port) == 0 => "LISTEN",
        1 => "CLOSED",
        2 => "LISTEN",
        3 => "SYN_SENT",
        4 => "SYN_RCVD",
        5 => "ESTABLISHED",
        6 => "FIN_WAIT1",
        7 => "FIN_WAIT2",
        8 => "CLOSE_WAIT",
        9 => "CLOSING",
        10 => "LAST_ACK",
        11 => "TIME_WAIT",
        12 => "DELETE_TCB",
        _ => "",
    }
}

fn ipv4(v: u32) -> String {
    Ipv4Addr::from(v.to_ne_bytes()).to_string()
}

fn ipv6(b: [u8; 16]) -> String {
    Ipv6Addr::from(b).to_string()
}

fn owner(pid: u32) -> Option<u32> {
    (pid != 0).then_some(pid)
}

/// The rows of a `MIB_*TABLE_OWNER_PID`: a count, then rows from offset 4 (rows hold 32-bit
/// fields and byte arrays, so no padding comes before them).
fn rows_of<T: Copy>(b: &[u8]) -> impl Iterator<Item = T> + '_ {
    let n = u32_at(b, 0).unwrap_or(0) as usize;
    (0..n).map_while(move |i| read_pod::<T>(b, 4 + i * size_of::<T>()))
}

fn parse_tcp4(b: &[u8], out: &mut Vec<Connection>) {
    out.extend(rows_of::<MIB_TCPROW_OWNER_PID>(b).map(|r| Connection {
        proto: Proto::Tcp,
        local_addr: ipv4(r.dwLocalAddr),
        local_port: port(r.dwLocalPort),
        remote_addr: ipv4(r.dwRemoteAddr),
        remote_port: port(r.dwRemotePort),
        state: tcp_state(r.dwState, r.dwRemotePort).to_string(),
        pid: owner(r.dwOwningPid),
    }));
}

fn parse_tcp6(b: &[u8], out: &mut Vec<Connection>) {
    out.extend(rows_of::<MIB_TCP6ROW_OWNER_PID>(b).map(|r| Connection {
        proto: Proto::Tcp6,
        local_addr: ipv6(r.ucLocalAddr),
        local_port: port(r.dwLocalPort),
        remote_addr: ipv6(r.ucRemoteAddr),
        remote_port: port(r.dwRemotePort),
        state: tcp_state(r.dwState, r.dwRemotePort).to_string(),
        pid: owner(r.dwOwningPid),
    }));
}

fn parse_udp4(b: &[u8], out: &mut Vec<Connection>) {
    out.extend(rows_of::<MIB_UDPROW_OWNER_PID>(b).map(|r| Connection {
        proto: Proto::Udp,
        local_addr: ipv4(r.dwLocalAddr),
        local_port: port(r.dwLocalPort),
        pid: owner(r.dwOwningPid),
        ..Connection::default()
    }));
}

fn parse_udp6(b: &[u8], out: &mut Vec<Connection>) {
    out.extend(rows_of::<MIB_UDP6ROW_OWNER_PID>(b).map(|r| Connection {
        proto: Proto::Udp6,
        local_addr: ipv6(r.ucLocalAddr),
        local_port: port(r.dwLocalPort),
        pid: owner(r.dwOwningPid),
        ..Connection::default()
    }));
}

/// Open sockets with their owning process.
pub fn connections() -> Vec<Connection> {
    let mut out = Vec::new();
    let tcp = |family: u16| {
        table(|buf, size| {
            // SAFETY: `buf` is writable for `*size` bytes; `size` is a valid in/out pointer.
            unsafe { GetExtendedTcpTable(buf, size, 0, u32::from(family), TCP_TABLE_OWNER_PID_ALL, 0) }
        })
    };
    let udp = |family: u16| {
        table(|buf, size| {
            // SAFETY: `buf` is writable for `*size` bytes; `size` is a valid in/out pointer.
            unsafe { GetExtendedUdpTable(buf, size, 0, u32::from(family), UDP_TABLE_OWNER_PID, 0) }
        })
    };
    if let Some((b, n)) = tcp(AF_INET) {
        parse_tcp4(as_bytes(&b, n), &mut out);
    }
    if let Some((b, n)) = tcp(AF_INET6) {
        parse_tcp6(as_bytes(&b, n), &mut out);
    }
    if let Some((b, n)) = udp(AF_INET) {
        parse_udp4(as_bytes(&b, n), &mut out);
    }
    if let Some((b, n)) = udp(AF_INET6) {
        parse_udp6(as_bytes(&b, n), &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_layouts_match_the_documented_ones() {
        assert_eq!(size_of::<MIB_TCPROW_OWNER_PID>(), 24);
        assert_eq!(size_of::<MIB_TCP6ROW_OWNER_PID>(), 56);
        assert_eq!(size_of::<MIB_UDPROW_OWNER_PID>(), 12);
        assert_eq!(size_of::<MIB_UDP6ROW_OWNER_PID>(), 28);
        assert_eq!(size_of::<MIB_IF_ROW2>(), 1352);
    }

    #[test]
    fn tcp_table_parses_and_a_short_table_stops_early() {
        let mut b = vec![0u8; 4 + 24];
        b[0] = 2; // Claims two rows, holds one.
        b[4] = 5; // ESTABLISHED
        b[8..12].copy_from_slice(&[127, 0, 0, 1]);
        b[12..14].copy_from_slice(&443u16.to_be_bytes());
        b[16..20].copy_from_slice(&[10, 0, 0, 2]);
        b[20..22].copy_from_slice(&8080u16.to_be_bytes());
        let mut out = Vec::new();
        parse_tcp4(&b, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].local_addr, "127.0.0.1");
        assert_eq!(out[0].local_port, 443);
        assert_eq!(out[0].remote_addr, "10.0.0.2");
        assert_eq!(out[0].remote_port, 8080);
        assert_eq!(out[0].state, "ESTABLISHED");
        assert_eq!(out[0].pid, None);
        out.clear();
        parse_tcp4(&[], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn tcp_states() {
        assert_eq!(tcp_state(5, 80u16.to_be().into()), "ESTABLISHED");
        assert_eq!(tcp_state(0, 0), "LISTEN");
        assert_eq!(tcp_state(0, 80u16.to_be().into()), "");
        assert_eq!(tcp_state(99, 0), "");
    }

    #[test]
    fn interface_filter_and_speed() {
        let nic = Shape { hardware: true, filter: false, enabled: true, up: false, loopback: false, in_use: false };
        assert!(keep_interface(&nic), "a NIC with no cable stays listed");
        assert!(!keep_interface(&Shape { filter: true, ..nic }));
        assert!(!keep_interface(&Shape { enabled: false, ..nic }));
        let soft = Shape { hardware: false, filter: false, enabled: true, up: true, loopback: false, in_use: true };
        assert!(keep_interface(&soft));
        let idle = Shape { in_use: false, ..soft };
        assert!(!keep_interface(&idle), "an idle WAN miniport");
        assert!(keep_interface(&Shape { loopback: true, ..idle }));
        assert_eq!(link_speed(u64::MAX), None);
        assert_eq!(link_speed(0), None);
        assert_eq!(link_speed(1000), Some(1000));
        assert_eq!(format_mac(&[0xAB, 1]).as_deref(), Some("AB:01"));
        assert_eq!(format_mac(&[0, 0]), None);
        assert_eq!(format_mac(&[]), None);
    }
}

//! Network interfaces: 64-bit counters from `sysctl(NET_RT_IFLIST2)` (the `if_data` of
//! `getifaddrs` is 32-bit and wraps at 4 GiB), addresses and MACs from `getifaddrs`.

use std::collections::HashMap;
use std::ffi::{CStr, c_char};
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::sysmon::NetRaw;

#[derive(Default)]
struct Addresses {
    addresses: Vec<String>,
    mac: Option<String>,
}

pub(super) fn interfaces() -> Vec<NetRaw> {
    let mut addrs = addresses();
    let Some(buf) = iflist2() else { return Vec::new() };
    let mut out = Vec::new();
    let mut pos = 0usize;
    let header = size_of::<libc::if_msghdr2>();
    // Messages of several kinds follow each other; each starts with its u16 length.
    while let Some(len_bytes) = buf.get(pos..pos + 2) {
        let len = usize::from(u16::from_ne_bytes([len_bytes[0], len_bytes[1]]));
        if len == 0 {
            break;
        }
        let msg = buf.get(pos..pos + len);
        pos += len;
        let Some(msg) = msg.filter(|m| m.len() >= header) else { continue };
        // `ifm_type` is the fourth byte of every routing message.
        if i32::from(msg[3]) != libc::RTM_IFINFO2 {
            continue;
        }
        // SAFETY: `msg` holds at least a whole `if_msghdr2`; the unaligned read copies it out, and
        // the struct is plain integers.
        let m: libc::if_msghdr2 = unsafe { msg.as_ptr().cast::<libc::if_msghdr2>().read_unaligned() };
        let mut name_buf = [0 as c_char; libc::IF_NAMESIZE];
        // SAFETY: the buffer has the `IF_NAMESIZE` bytes the call requires.
        let named = unsafe { libc::if_indextoname(u32::from(m.ifm_index), name_buf.as_mut_ptr()) };
        if named.is_null() {
            continue;
        }
        // SAFETY: on success `if_indextoname` stored a NUL-terminated name in `name_buf`.
        let name = unsafe { CStr::from_ptr(name_buf.as_ptr()) }.to_string_lossy().into_owned();
        let d = m.ifm_data;
        let (baud, ib, ob, ip, op, ie, oe) =
            (d.ifi_baudrate, d.ifi_ibytes, d.ifi_obytes, d.ifi_ipackets, d.ifi_opackets, d.ifi_ierrors, d.ifi_oerrors);
        let flags = m.ifm_flags;
        let info = addrs.remove(&name).unwrap_or_default();
        out.push(NetRaw {
            id: name.clone(),
            virtual_if: is_virtual(&name),
            name,
            description: String::new(),
            rx_bytes: ib,
            tx_bytes: ob,
            rx_packets: ip,
            tx_packets: op,
            rx_errors: ie,
            tx_errors: oe,
            up: flags & libc::IFF_UP != 0 && flags & libc::IFF_RUNNING != 0,
            link_bps: (baud > 0).then_some(baud),
            addresses: info.addresses,
            mac: info.mac,
        });
    }
    out
}

fn is_virtual(name: &str) -> bool {
    ["lo", "utun", "awdl", "llw", "bridge", "gif", "stf", "anpi", "ap", "vmenet", "vlan"]
        .iter()
        .any(|p| name.starts_with(p))
}

fn iflist2() -> Option<Vec<u8>> {
    let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
    let mut len = 0usize;
    // SAFETY: a null buffer asks only for the size; `mib` holds the 6 names passed.
    let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), 6, std::ptr::null_mut(), &raw mut len, std::ptr::null_mut(), 0) };
    if rc != 0 || len == 0 {
        return None;
    }
    // An interface can appear between the two calls: room to spare, and one more try.
    for _ in 0..2 {
        let mut buf = vec![0u8; len + len / 4];
        let mut got = buf.len();
        // SAFETY: `buf` is writable for `got` bytes.
        let rc = unsafe {
            libc::sysctl(mib.as_mut_ptr(), 6, buf.as_mut_ptr().cast(), &raw mut got, std::ptr::null_mut(), 0)
        };
        if rc == 0 {
            buf.truncate(got);
            return Some(buf);
        }
        len += len / 2;
    }
    None
}

fn addresses() -> HashMap<String, Addresses> {
    let mut out: HashMap<String, Addresses> = HashMap::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `head` is a valid out-pointer; on success the list is freed below.
    if unsafe { libc::getifaddrs(&raw mut head) } != 0 {
        return out;
    }
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: `cur` is a node of the list `getifaddrs` returned, alive until `freeifaddrs`.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_name.is_null() || ifa.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: `ifa_name` is a NUL-terminated string owned by the list.
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
        let e = out.entry(name).or_default();
        // SAFETY: a non-null `ifa_addr` points to a sockaddr of at least the family field.
        let family = i32::from(unsafe { (*ifa.ifa_addr).sa_family });
        if family == libc::AF_INET {
            // SAFETY: family AF_INET means the address is a `sockaddr_in`.
            let a = unsafe { (*ifa.ifa_addr.cast::<libc::sockaddr_in>()).sin_addr.s_addr };
            e.addresses.push(Ipv4Addr::from(u32::from_be(a)).to_string());
        } else if family == libc::AF_INET6 {
            // SAFETY: family AF_INET6 means the address is a `sockaddr_in6`.
            let a = unsafe { (*ifa.ifa_addr.cast::<libc::sockaddr_in6>()).sin6_addr.s6_addr };
            e.addresses.push(Ipv6Addr::from(a).to_string());
        } else if family == libc::AF_LINK {
            let dl = ifa.ifa_addr.cast::<libc::sockaddr_dl>();
            // SAFETY: family AF_LINK means a `sockaddr_dl`, which holds `sdl_nlen + sdl_alen` bytes
            // of name and address from `sdl_data` on (they may run past the declared array).
            let mac = unsafe {
                let (nlen, alen) = (usize::from((*dl).sdl_nlen), usize::from((*dl).sdl_alen));
                let data = (&raw const (*dl).sdl_data).cast::<u8>().add(nlen);
                std::slice::from_raw_parts(data, alen).to_vec()
            };
            if mac.len() == 6 && mac.iter().any(|b| *b != 0) {
                e.mac = Some(mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"));
            }
        }
    }
    // SAFETY: `head` came from a successful `getifaddrs` and is freed exactly once; no reference
    // into the list outlives this point.
    unsafe { libc::freeifaddrs(head) };
    out
}

//! `/proc/net/dev` and `/proc/net/{tcp,tcp6,udp,udp6}`.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::sysmon::{Connection, Proto};

/// One interface's counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NetDev<'a> {
    pub name: &'a str,
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub rx_errors: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
    pub tx_errors: u64,
}

pub(crate) fn parse_net_dev_line(line: &str) -> Option<NetDev<'_>> {
    // Old kernels print no space after the colon.
    let (name, rest) = line.rsplit_once(':')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut it = rest.split_ascii_whitespace();
    let mut n = || it.next()?.parse::<u64>().ok();
    let rx_bytes = n()?;
    let rx_packets = n()?;
    let rx_errors = n()?;
    for _ in 0..5 {
        n()?; // drop fifo frame compressed multicast
    }
    let tx_bytes = n()?;
    let tx_packets = n()?;
    let tx_errors = n()?;
    Some(NetDev { name, rx_bytes, rx_packets, rx_errors, tx_bytes, tx_packets, tx_errors })
}

/// `speed` of `/sys/class/net/<if>` is Mbit/s, or -1 / an error when unknown.
pub(crate) fn parse_link_speed(s: &str) -> Option<u64> {
    let mbps: i64 = s.trim().parse().ok()?;
    (mbps > 0).then(|| (mbps as u64).saturating_mul(1_000_000))
}

/// A MAC address worth showing (not empty, not all zeros).
pub(crate) fn parse_mac(s: &str) -> Option<String> {
    let s = s.trim();
    let real = !s.is_empty() && s.bytes().any(|b| b != b'0' && b != b':');
    real.then(|| s.to_owned())
}

/// A socket line with its inode (the key to the owning process).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawSocket {
    pub conn: Connection,
    pub inode: u64,
}

/// A whole `/proc/net/<proto>` file. Malformed lines are skipped.
pub(crate) fn parse_sockets(s: &str, proto: Proto) -> Vec<RawSocket> {
    s.lines().skip(1).filter_map(|l| parse_socket_line(l, proto)).collect()
}

fn parse_socket_line(line: &str, proto: Proto) -> Option<RawSocket> {
    let mut it = line.split_ascii_whitespace();
    let _sl = it.next()?;
    let (local_addr, local_port) = endpoint(it.next()?)?;
    let (remote_addr, remote_port) = endpoint(it.next()?)?;
    let state = u8::from_str_radix(it.next()?, 16).ok()?;
    let _queues = it.next()?;
    let _timer = it.next()?;
    let _retrans = it.next()?;
    let _uid = it.next()?;
    let _timeout = it.next()?;
    let inode = it.next()?.parse().ok()?;
    let state = match proto {
        Proto::Tcp | Proto::Tcp6 => tcp_state(state),
        Proto::Udp | Proto::Udp6 => "",
    };
    Some(RawSocket {
        conn: Connection {
            proto,
            local_addr,
            local_port,
            remote_addr,
            remote_port,
            state: state.to_owned(),
            pid: None,
        },
        inode,
    })
}

/// `0100007F:0050` to `("127.0.0.1", 80)`. The address is printed as native-endian u32 groups.
fn endpoint(s: &str) -> Option<(String, u16)> {
    let (addr, port) = s.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let group = |h: &str| u32::from_str_radix(h, 16).ok().map(u32::to_ne_bytes);
    let addr = match addr.len() {
        8 => Ipv4Addr::from(group(addr)?).to_string(),
        32 if addr.is_ascii() => {
            let mut b = [0u8; 16];
            for i in 0..4 {
                b[i * 4..i * 4 + 4].copy_from_slice(&group(&addr[i * 8..i * 8 + 8])?);
            }
            Ipv6Addr::from(b).to_string()
        }
        _ => return None,
    };
    Some((addr, port))
}

fn tcp_state(n: u8) -> &'static str {
    match n {
        1 => "ESTABLISHED",
        2 => "SYN_SENT",
        3 => "SYN_RECV",
        4 => "FIN_WAIT1",
        5 => "FIN_WAIT2",
        6 => "TIME_WAIT",
        7 => "CLOSE",
        8 => "CLOSE_WAIT",
        9 => "LAST_ACK",
        10 => "LISTEN",
        11 => "CLOSING",
        12 => "NEW_SYN_RECV",
        _ => "UNKNOWN",
    }
}

/// `socket_info.soi_proto` of macOS (`in_sockinfo`, followed for TCP by `tcpsi_state`) to a
/// connection: ports at 0 (remote) and 4 (local), `insi_vflag` at 24, foreign and
/// local addresses at 32 and 48, the IPv4 form being the last four bytes of the sixteen.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn decode_in_sockinfo(b: &[u8], tcp: bool) -> Option<Connection> {
    let addr = |at: usize, v6: bool| -> Option<String> {
        let raw: [u8; 16] = b.get(at..at + 16)?.try_into().ok()?;
        Some(if v6 {
            Ipv6Addr::from(raw).to_string()
        } else {
            Ipv4Addr::new(raw[12], raw[13], raw[14], raw[15]).to_string()
        })
    };
    let v6 = match *b.get(24)? {
        1 => false,
        2 => true,
        _ => return None,
    };
    // `insi_fport` and `insi_lport` are ints holding a network-order u16 in their low bytes.
    let low_port = |at: usize| {
        let p = i32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?);
        Some(u16::from_be((p as u32 & 0xffff) as u16))
    };
    let state = if tcp {
        let s = i32::from_ne_bytes(b.get(80..84)?.try_into().ok()?);
        bsd_tcp_state(s).to_owned()
    } else {
        String::new()
    };
    let proto = match (tcp, v6) {
        (true, false) => Proto::Tcp,
        (true, true) => Proto::Tcp6,
        (false, false) => Proto::Udp,
        (false, true) => Proto::Udp6,
    };
    Some(Connection {
        proto,
        local_addr: addr(48, v6)?,
        local_port: low_port(4)?,
        remote_addr: addr(32, v6)?,
        remote_port: low_port(0)?,
        state,
        pid: None,
    })
}

/// `TSI_S_*` of macOS `sys/proc_info.h`.
fn bsd_tcp_state(n: i32) -> &'static str {
    match n {
        0 => "CLOSED",
        1 => "LISTEN",
        2 => "SYN_SENT",
        3 => "SYN_RECEIVED",
        4 => "ESTABLISHED",
        5 => "CLOSE_WAIT",
        6 => "FIN_WAIT_1",
        7 => "CLOSING",
        8 => "LAST_ACK",
        9 => "FIN_WAIT_2",
        10 => "TIME_WAIT",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_in_sockinfo() {
        let mut b = vec![0u8; 84];
        // Remote port 443, local port 8080, both network order inside a native int.
        b[0..4].copy_from_slice(&i32::from(u16::from_be(443)).to_ne_bytes());
        b[4..8].copy_from_slice(&i32::from(u16::from_be(8080)).to_ne_bytes());
        b[24] = 1;
        b[44..48].copy_from_slice(&[93, 184, 216, 34]);
        b[60..64].copy_from_slice(&[192, 168, 0, 2]);
        b[80..84].copy_from_slice(&4i32.to_ne_bytes());
        let c = decode_in_sockinfo(&b, true).unwrap();
        assert_eq!(c.proto, Proto::Tcp);
        assert_eq!((c.local_addr.as_str(), c.local_port), ("192.168.0.2", 8080));
        assert_eq!((c.remote_addr.as_str(), c.remote_port), ("93.184.216.34", 443));
        assert_eq!(c.state, "ESTABLISHED");

        b[24] = 2;
        b[48..64].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
        let c = decode_in_sockinfo(&b, false).unwrap();
        assert_eq!(c.proto, Proto::Udp6);
        assert_eq!(c.local_addr, "::1");
        assert_eq!(c.state, "");

        b[24] = 0;
        assert!(decode_in_sockinfo(&b, true).is_none());
        assert!(decode_in_sockinfo(&b[..10], true).is_none());
        b[24] = 1;
        assert!(decode_in_sockinfo(&b[..70], true).is_none());
        assert_eq!(bsd_tcp_state(99), "UNKNOWN");
    }

    #[test]
    fn net_dev() {
        let l = "  eth0: 1000 10 1 0 0 0 0 0 2000 20 2 0 0 0 0 0\n";
        let d = parse_net_dev_line(l).unwrap();
        assert_eq!(d.name, "eth0");
        assert_eq!((d.rx_bytes, d.rx_packets, d.rx_errors), (1000, 10, 1));
        assert_eq!((d.tx_bytes, d.tx_packets, d.tx_errors), (2000, 20, 2));
        assert_eq!(parse_net_dev_line("lo:5 1 0 0 0 0 0 0 5 1 0 0 0 0 0 0").unwrap().name, "lo");
        assert!(parse_net_dev_line("Inter-|   Receive  |  Transmit").is_none());
        assert!(parse_net_dev_line(" face |bytes packets").is_none());
        assert!(parse_net_dev_line("eth0: 1 2").is_none());
        assert!(parse_net_dev_line(": 1 2 3 4 5 6 7 8 9 10 11").is_none());
    }

    #[test]
    fn link_values() {
        assert_eq!(parse_link_speed("1000\n"), Some(1_000_000_000));
        assert_eq!(parse_link_speed("-1\n"), None);
        assert_eq!(parse_link_speed(""), None);
        assert_eq!(parse_mac("aa:bb:cc:00:00:01\n").as_deref(), Some("aa:bb:cc:00:00:01"));
        assert_eq!(parse_mac("00:00:00:00:00:00\n"), None);
        assert_eq!(parse_mac(""), None);
    }

    #[test]
    #[cfg(target_endian = "little")]
    fn tcp4() {
        let s = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
                 0: 0100007F:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 ffff 100 0 0 10 0\n\
                 1: 0100007F:C350 0101A8C0:01BB 01 00000000:00000000 00:00000000 00000000  1000        0 777 1 ffff\n\
                 junk line\n";
        let v = parse_sockets(s, Proto::Tcp);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].conn.local_addr, "127.0.0.1");
        assert_eq!(v[0].conn.local_port, 80);
        assert_eq!(v[0].conn.state, "LISTEN");
        assert_eq!(v[0].inode, 12345);
        assert_eq!(v[1].conn.remote_addr, "192.168.1.1");
        assert_eq!(v[1].conn.remote_port, 443);
        assert_eq!(v[1].conn.state, "ESTABLISHED");
    }

    #[test]
    #[cfg(target_endian = "little")]
    fn tcp6_and_udp() {
        let s = "header\n\
                 0: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 0:0 0:0 0 0 0 9 1\n\
                 1: 0000000000000000FFFF00000100007F:1F90 00000000000000000000000000000000:0000 07 0:0 0:0 0 0 0 10 1\n";
        let v = parse_sockets(s, Proto::Tcp6);
        assert_eq!(v[0].conn.local_addr, "::1");
        assert_eq!(v[0].conn.local_port, 8080);
        assert_eq!(v[1].conn.local_addr, "::ffff:127.0.0.1");
        assert_eq!(v[1].conn.state, "CLOSE");
        let u = parse_sockets("h\n0: 00000000:0035 00000000:0000 07 0:0 0:0 0 0 0 5 2\n", Proto::Udp);
        assert_eq!(u[0].conn.state, "");
        assert_eq!(u[0].conn.proto, Proto::Udp);
        assert_eq!(u[0].conn.local_port, 53);
    }

    #[test]
    fn sockets_malformed() {
        for bad in [
            "h\n0: nothex:0 0:0 0A 0 0 0 0 0 1\n",
            "h\n0: 0100007F 0:0\n",
            "h\n0: 0100007F:ZZZZ 00000000:0000 0A 0 0 0 0 0 1\n",
            "h\n0: 0100007F:0050 00000000:0000 QQ 0 0 0 0 0 1\n",
            "h\n0: 0100007F:0050 00000000:0000 0A 0 0 0 0 0 x\n",
            // Multi-byte text cut at a group boundary must not panic.
            "h\n0: ééééééééééééééééé:0050 00000000:0000 0A 0 0 0 0 0 1\n",
            "",
        ] {
            assert!(parse_sockets(bad, Proto::Tcp).is_empty(), "{bad:?}");
        }
        assert_eq!(tcp_state(99), "UNKNOWN");
    }
}

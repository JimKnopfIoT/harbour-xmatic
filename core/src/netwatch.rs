//! Connections a network switch left behind: their address is gone, or the route now
//! leaves by another interface. Linux reports nothing; they wait out their timeout.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Once};
use std::time::Duration;

use serde_json::json;

use crate::protocol::event;
use crate::runtime::Sink;

/// Address and route changes come in bursts; one sweep after the last of them.
const SETTLE: Duration = Duration::from_millis(1000);

static STARTED: Once = Once::new();

/// Once per process; the sweep covers every socket of it.
pub fn start(sink: Arc<Sink>) {
    STARTED.call_once(|| {
        let watching = sink.clone();
        let spawned = std::thread::Builder::new()
            .name("xmatic-net".into())
            .spawn(move || watch(watching));
        if spawned.is_err() {
            log_line(&sink, "network watch could not start");
        }
    });
}

fn log_line(sink: &Sink, message: &str) {
    sink.emit(event(
        "core.log",
        json!({ "level": "warn", "target": "xmatic::netwatch", "message": message }),
    ));
}

fn watch(sink: Arc<Sink>) {
    let Some(fd) = subscribe() else {
        log_line(&sink, "network watch unavailable, stale connections wait for their timeout");
        return;
    };
    let mut buffer = vec![0u8; 16384];
    loop {
        // SAFETY: `buffer` outlives the call and its length is passed with it.
        let read = unsafe { libc::recv(fd, buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        if read < 0 {
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                // Overflow means events were lost, which is itself a change.
                Some(libc::ENOBUFS) => {}
                Some(libc::EINTR) => continue,
                _ => {
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
            }
        }
        std::thread::sleep(SETTLE);
        drain(fd, &mut buffer);
        let closed = sweep();
        if closed > 0 {
            log_line(&sink, &format!("network switched, {closed} stale connections closed"));
        }
    }
}

fn subscribe() -> Option<libc::c_int> {
    // SAFETY: plain socket calls; the address is a zeroed `sockaddr_nl` with its fields set.
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, libc::NETLINK_ROUTE);
        if fd < 0 {
            return None;
        }
        let mut address: libc::sockaddr_nl = std::mem::zeroed();
        address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        address.nl_groups = (libc::RTMGRP_IPV4_IFADDR
            | libc::RTMGRP_IPV6_IFADDR
            | libc::RTMGRP_IPV4_ROUTE
            | libc::RTMGRP_IPV6_ROUTE) as u32;
        let bound = libc::bind(
            fd,
            (&address as *const libc::sockaddr_nl).cast(),
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        );
        if bound < 0 {
            libc::close(fd);
            return None;
        }
        Some(fd)
    }
}

fn drain(fd: libc::c_int, buffer: &mut [u8]) {
    loop {
        // SAFETY: as in `watch`.
        let read =
            unsafe { libc::recv(fd, buffer.as_mut_ptr().cast(), buffer.len(), libc::MSG_DONTWAIT) };
        if read <= 0 {
            let error = std::io::Error::last_os_error().raw_os_error();
            if read < 0 && error == Some(libc::ENOBUFS) {
                continue;
            }
            return;
        }
    }
}

/// Shuts every own TCP socket whose address or route a switch has taken away.
fn sweep() -> usize {
    let Some(interfaces) = local_interfaces() else { return 0 };
    let mut routes = HashMap::new();
    let mut route = |remote: IpAddr| *routes.entry(remote).or_insert_with(|| probe(remote));
    let mut stale = std::collections::HashSet::new();
    for (path, v6) in [("/proc/self/net/tcp", false), ("/proc/self/net/tcp6", true)] {
        let Ok(table) = std::fs::read_to_string(path) else { continue };
        stale.extend(stale_inodes(&table, v6, &interfaces, &mut route));
    }
    if stale.is_empty() {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else { return 0 };
    let mut closed = 0;
    for entry in entries.flatten() {
        let Some(fd) = entry.file_name().to_str().and_then(|name| name.parse::<libc::c_int>().ok())
        else {
            continue;
        };
        let Some(inode) = socket_inode(&entry.path()) else { continue };
        if !stale.contains(&inode) {
            continue;
        }
        // Re-checked on the descriptor itself: the number may have been reused since.
        // SAFETY: `stat` is plain data, written by `fstat` before it is read.
        let same = unsafe {
            let mut stat: libc::stat = std::mem::zeroed();
            libc::fstat(fd, &mut stat) == 0 && stat.st_ino as u64 == inode
        };
        // SAFETY: shuts the connection down without closing the descriptor its owner holds.
        if same && unsafe { libc::shutdown(fd, libc::SHUT_RDWR) } == 0 {
            closed += 1;
        }
    }
    closed
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Route {
    /// The source address the kernel would pick now.
    Via(IpAddr),
    Unreachable,
    /// Could not ask; such a connection is left alone.
    Unknown,
}

/// A connected UDP socket sends nothing and tells which source the route picks.
fn probe(remote: IpAddr) -> Route {
    let socket = std::net::UdpSocket::bind(match remote {
        IpAddr::V4(_) => "0.0.0.0:0",
        IpAddr::V6(_) => "[::]:0",
    });
    let Ok(socket) = socket else { return Route::Unknown };
    match socket.connect((remote, 9)) {
        Ok(()) => socket.local_addr().map(|local| Route::Via(local.ip())).unwrap_or(Route::Unknown),
        Err(error) => match error.raw_os_error() {
            Some(libc::ENETUNREACH) | Some(libc::EHOSTUNREACH) => Route::Unreachable,
            _ => Route::Unknown,
        },
    }
}

fn socket_inode(link: &std::path::Path) -> Option<u64> {
    let target = std::fs::read_link(link).ok()?;
    let target = target.to_str()?;
    target.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

/// Every local address with the interface that carries it.
fn local_interfaces() -> Option<HashMap<IpAddr, String>> {
    let mut present = HashMap::new();
    // SAFETY: the list is walked as `getifaddrs` built it and freed once.
    unsafe {
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut list) != 0 {
            return None;
        }
        let mut cursor = list;
        while !cursor.is_null() {
            let address = (*cursor).ifa_addr;
            let name = std::ffi::CStr::from_ptr((*cursor).ifa_name).to_string_lossy().into_owned();
            if !address.is_null() {
                match i32::from((*address).sa_family) {
                    libc::AF_INET => {
                        let v4 = &*(address as *const libc::sockaddr_in);
                        let ip = IpAddr::V4(Ipv4Addr::from(v4.sin_addr.s_addr.to_ne_bytes()));
                        present.insert(ip, name);
                    }
                    libc::AF_INET6 => {
                        let v6 = &*(address as *const libc::sockaddr_in6);
                        present.insert(IpAddr::V6(Ipv6Addr::from(v6.sin6_addr.s6_addr)), name);
                    }
                    _ => {}
                }
            }
            cursor = (*cursor).ifa_next;
        }
        libc::freeifaddrs(list);
    }
    Some(present)
}

/// TCP state `0A` is a listener; it has no peer to lose.
const LISTEN: &str = "0A";

fn stale_inodes(
    table: &str,
    v6: bool,
    interfaces: &HashMap<IpAddr, String>,
    route: &mut impl FnMut(IpAddr) -> Route,
) -> Vec<u64> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if *fields.get(3)? == LISTEN {
                return None;
            }
            let local = parse_address(fields.get(1)?.split(':').next()?, v6)?;
            let remote = parse_address(fields.get(2)?.split(':').next()?, v6)?;
            let inode: u64 = fields.get(9)?.parse().ok()?;
            if inode == 0 || local.is_unspecified() || local.is_loopback() {
                return None;
            }
            let Some(own) = interfaces.get(&local) else { return Some(inode) };
            if remote.is_unspecified() || remote.is_loopback() {
                return None;
            }
            // Same interface, other source (a rotated IPv6 address) still works.
            let gone = match route(remote) {
                Route::Via(source) => interfaces.get(&source).is_some_and(|other| other != own),
                Route::Unreachable => true,
                Route::Unknown => false,
            };
            gone.then_some(inode)
        })
        .collect()
}

/// The kernel prints each 32-bit word of the address in host order.
fn parse_address(hex: &str, v6: bool) -> Option<IpAddr> {
    let word = |index: usize| -> Option<[u8; 4]> {
        let digits = hex.get(index * 8..index * 8 + 8)?;
        Some(u32::from_str_radix(digits, 16).ok()?.to_ne_bytes())
    };
    if !v6 {
        return (hex.len() == 8).then(|| word(0)).flatten().map(|bytes| IpAddr::V4(bytes.into()));
    }
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for index in 0..4 {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&word(index)?);
    }
    let address = Ipv6Addr::from(bytes);
    Some(match address.to_ipv4_mapped() {
        Some(v4) => IpAddr::V4(v4),
        None => IpAddr::V6(address),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(address: [u8; 4]) -> String {
        format!("{:08X}", u32::from_ne_bytes(address))
    }

    #[test]
    fn v4_round_trips_through_the_kernel_format() {
        let hex = le([192, 0, 2, 7]);
        assert_eq!(parse_address(&hex, false), Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))));
    }

    #[test]
    fn mapped_v6_compares_as_v4() {
        let hex = format!("{}{}{}{}", le([0; 4]), le([0; 4]), le([0, 0, 0xff, 0xff]), le([192, 0, 2, 7]));
        assert_eq!(parse_address(&hex, true), Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))));
    }

    #[test]
    fn only_connections_a_switch_cut_off_are_stale() {
        let wifi = le([192, 0, 2, 7]);
        let mobile = le([198, 51, 100, 9]);
        let gone = le([203, 0, 113, 5]);
        let server = le([203, 0, 113, 80]);
        let table = format!(
            "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
             0: {gone}:C350 {server}:01BB 01 00000000:00000000 00:00000000 00000000 100000 0 111 1\n\
             1: {wifi}:C351 {server}:01BB 01 00000000:00000000 00:00000000 00000000 100000 0 222 1\n\
             2: {mobile}:C352 {server}:01BB 01 00000000:00000000 00:00000000 00000000 100000 0 333 1\n\
             3: {gone}:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000 100000 0 444 1\n\
             4: {lo}:1F91 {lo}:01BB 01 00000000:00000000 00:00000000 00000000 100000 0 555 1\n",
            lo = le([127, 0, 0, 1]),
        );
        let interfaces: HashMap<IpAddr, String> = [
            (IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7)), "wlan0".to_owned()),
            (IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9)), "ccmni2".to_owned()),
        ]
        .into();
        let mut via_wifi = |_| Route::Via(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7)));
        assert_eq!(stale_inodes(&table, false, &interfaces, &mut via_wifi), vec![111, 333]);
        let mut unknown = |_| Route::Unknown;
        assert_eq!(stale_inodes(&table, false, &interfaces, &mut unknown), vec![111]);
        let mut nowhere = |_| Route::Unreachable;
        assert_eq!(stale_inodes(&table, false, &interfaces, &mut nowhere), vec![111, 222, 333]);
    }
}

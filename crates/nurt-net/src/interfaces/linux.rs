// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interfaces on Linux: `getifaddrs` for addresses, `/sys/class/net` for the
//! state, hardware address, MTU and kind, `/proc/net/route` and
//! `/proc/net/ipv6_route` for the default gateway.

// The module calls `getifaddrs` and `freeifaddrs` and reads the list they
// return; every unsafe block states its invariant.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{InterfaceKind, NetworkInterface};

use super::{InterfaceAddress, SysRoot, prefix_length};

/// `ARPHRD_LOOPBACK` from `linux/if_arp.h`.
const ARPHRD_LOOPBACK: u32 = 772;
/// `ARPHRD_ETHER` from `linux/if_arp.h`.
const ARPHRD_ETHER: u32 = 1;
/// `IFF_UP` from `linux/if.h`.
const IFF_UP: u32 = 0x1;
/// `RTF_GATEWAY` from `linux/route.h`.
const RTF_GATEWAY: u32 = 0x2;
/// Number of hexadecimal digits of an IPv6 address in `/proc/net/ipv6_route`.
const IPV6_HEX_DIGITS: usize = 32;
/// Hardware address the kernel reports for interfaces without one.
const ZERO_MAC: &str = "00:00:00:00:00:00";

/// Addresses of all interfaces from `getifaddrs`.
pub(super) fn addresses() -> io::Result<Vec<InterfaceAddress>> {
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `head` is a valid location for the list pointer; on success the
    // list is released below with `freeifaddrs`.
    if unsafe { libc::getifaddrs(&raw mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut found = Vec::new();
    let mut cursor = head;
    while !cursor.is_null() {
        // SAFETY: `cursor` is a non-null element of the list returned by
        // `getifaddrs`, which stays valid until `freeifaddrs`.
        let entry = unsafe { &*cursor };
        if let Some(address) = entry_address(entry) {
            found.push(address);
        }
        cursor = entry.ifa_next;
    }
    // SAFETY: `head` was returned by a successful `getifaddrs` and is freed
    // once; no reference into the list outlives this call.
    unsafe { libc::freeifaddrs(head) };
    Ok(found)
}

fn entry_address(entry: &libc::ifaddrs) -> Option<InterfaceAddress> {
    if entry.ifa_addr.is_null() || entry.ifa_name.is_null() {
        return None;
    }
    // SAFETY: `ifa_name` is a non-null NUL-terminated string owned by the
    // list.
    let name = unsafe { CStr::from_ptr(entry.ifa_name) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: `ifa_addr` is non-null and points to a socket address whose
    // family field is valid for every family.
    let family = i32::from(unsafe { (*entry.ifa_addr).sa_family });
    let (address, mask) = match family {
        libc::AF_INET => {
            // SAFETY: the family is `AF_INET`, so the address and the mask
            // (when present) are `sockaddr_in`.
            let address = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in>() };
            let mask = (!entry.ifa_netmask.is_null()).then(|| {
                // SAFETY: as above, a non-null mask of an `AF_INET` entry.
                let mask = unsafe { &*entry.ifa_netmask.cast::<libc::sockaddr_in>() };
                mask.sin_addr.s_addr.to_ne_bytes().to_vec()
            });
            let ip = Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes());
            (IpAddr::V4(ip), mask)
        }
        libc::AF_INET6 => {
            // SAFETY: the family is `AF_INET6`, so the address and the mask
            // (when present) are `sockaddr_in6`.
            let address = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in6>() };
            let mask = (!entry.ifa_netmask.is_null()).then(|| {
                // SAFETY: as above, a non-null mask of an `AF_INET6` entry.
                let mask = unsafe { &*entry.ifa_netmask.cast::<libc::sockaddr_in6>() };
                mask.sin6_addr.s6_addr.to_vec()
            });
            (IpAddr::V6(Ipv6Addr::from(address.sin6_addr.s6_addr)), mask)
        }
        _ => return None,
    };
    let interface = name.split(':').next().unwrap_or(&name).to_owned();
    let full = if address.is_ipv4() { 32 } else { 128 };
    Some(InterfaceAddress {
        interface,
        address,
        prefix: mask.as_deref().map_or(full, prefix_length),
    })
}

/// Builds the interface records from the files of `/sys/class/net` and the
/// routing tables in `/proc/net` below `root`, with `addresses` as reported
/// by `getifaddrs`.
///
/// Interfaces are returned in the byte order of their names. Addresses of an
/// interface keep the order of `addresses`.
///
/// # Errors
///
/// The function fails with `E_IO` when `/sys/class/net` below `root` cannot
/// be listed.
///
/// # Examples
///
/// ```no_run
/// use nurt_net::interfaces::{SysRoot, from_sysfs};
///
/// let interfaces = from_sysfs(&SysRoot::host(), &[]).unwrap_or_default();
/// assert!(interfaces.iter().all(|interface| interface.addresses.is_empty()));
/// ```
pub fn from_sysfs(root: &SysRoot, addresses: &[InterfaceAddress]) -> Result<Vec<NetworkInterface>> {
    let class = root.path("/sys/class/net");
    let entries = fs::read_dir(&class).map_err(|error| read_error(&class, &error))?;
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let gateways = gateways(root);
    Ok(names
        .into_iter()
        .map(|name| {
            let dir = class.join(&name);
            let gateway = gateways.get(&name).map(ToString::to_string);
            interface(&dir, name, addresses, gateway)
        })
        .collect())
}

fn interface(
    dir: &Path,
    name: String,
    addresses: &[InterfaceAddress],
    gateway: Option<String>,
) -> NetworkInterface {
    let flags = read_text(&dir.join("flags"))
        .and_then(|text| u32::from_str_radix(text.trim_start_matches("0x"), 16).ok());
    let up = match read_text(&dir.join("operstate")).as_deref() {
        Some("up") => true,
        Some("unknown") | None => flags.is_some_and(|flags| flags & IFF_UP != 0),
        Some(_) => false,
    };
    let mac = read_text(&dir.join("address")).filter(|mac| !mac.is_empty() && mac != ZERO_MAC);
    let mtu = read_text(&dir.join("mtu")).and_then(|text| text.parse().ok());
    let addresses = addresses
        .iter()
        .filter(|address| address.interface == name)
        .map(InterfaceAddress::to_text)
        .collect();
    NetworkInterface {
        kind: Some(kind(dir)),
        name,
        addresses,
        up,
        mac,
        mtu,
        gateway,
    }
}

fn kind(dir: &Path) -> InterfaceKind {
    let link_type = read_text(&dir.join("type")).and_then(|text| text.parse::<u32>().ok());
    if link_type == Some(ARPHRD_LOOPBACK) {
        InterfaceKind::Loopback
    } else if dir.join("wireless").exists() || dir.join("phy80211").exists() {
        InterfaceKind::Wireless
    } else if !dir.join("device").exists() {
        InterfaceKind::Virtual
    } else if link_type == Some(ARPHRD_ETHER) {
        InterfaceKind::Ethernet
    } else {
        InterfaceKind::Other
    }
}

/// Default gateway per interface: IPv4 from `/proc/net/route`, IPv6 from
/// `/proc/net/ipv6_route` for interfaces without an IPv4 gateway.
fn gateways(root: &SysRoot) -> HashMap<String, IpAddr> {
    let mut found = HashMap::new();
    if let Some(text) = read_text(&root.path("/proc/net/route")) {
        for (name, gateway) in text.lines().skip(1).filter_map(ipv4_default_route) {
            found.entry(name).or_insert(IpAddr::V4(gateway));
        }
    }
    if let Some(text) = read_text(&root.path("/proc/net/ipv6_route")) {
        for (name, gateway) in text.lines().filter_map(ipv6_default_route) {
            found.entry(name).or_insert(IpAddr::V6(gateway));
        }
    }
    found
}

/// Parses a line `Iface Destination Gateway Flags …` of `/proc/net/route`;
/// addresses are hexadecimal in the byte order of the host.
fn ipv4_default_route(line: &str) -> Option<(String, Ipv4Addr)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let [name, destination, gateway, flags, ..] = fields.as_slice() else {
        return None;
    };
    let flags = u32::from_str_radix(flags, 16).ok()?;
    if *destination != "00000000" || flags & RTF_GATEWAY == 0 {
        return None;
    }
    let gateway = u32::from_str_radix(gateway, 16).ok()?;
    Some(((*name).to_owned(), Ipv4Addr::from(gateway.to_le_bytes())))
}

/// Parses a line of `/proc/net/ipv6_route`: destination, prefix, source,
/// source prefix, next hop, metric, references, use, flags, interface.
fn ipv6_default_route(line: &str) -> Option<(String, Ipv6Addr)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let [destination, prefix, _, _, next_hop, _, _, _, flags, name] = fields.as_slice() else {
        return None;
    };
    let flags = u32::from_str_radix(flags, 16).ok()?;
    let is_default = destination.bytes().all(|digit| digit == b'0') && *prefix == "00";
    if !is_default || flags & RTF_GATEWAY == 0 || *name == "lo" {
        return None;
    }
    let gateway = parse_ipv6_hex(next_hop)?;
    (!gateway.is_unspecified()).then(|| ((*name).to_owned(), gateway))
}

fn parse_ipv6_hex(text: &str) -> Option<Ipv6Addr> {
    if text.len() != IPV6_HEX_DIGITS {
        return None;
    }
    u128::from_str_radix(text, 16).ok().map(Ipv6Addr::from)
}

fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

fn read_error(path: &Path, error: &io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("{} cannot be read: {error}", path.display()),
    )
    .with_detail("path", serde_json::Value::from(path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_default_route_reads_gateway_in_host_order() {
        let line = "eth0\t00000000\t0102A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0";
        assert_eq!(
            ipv4_default_route(line),
            Some(("eth0".to_owned(), Ipv4Addr::new(192, 168, 2, 1)))
        );
        let network = "eth0\t0002A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0";
        assert_eq!(ipv4_default_route(network), None);
    }

    #[test]
    fn ipv6_default_route_skips_unspecified_next_hop() {
        let zero = "0".repeat(IPV6_HEX_DIGITS);
        let line = format!(
            "{zero} 00 {zero} 00 fe800000000000000000000000000001 00000400 00000001 00000000 00450003 eth0"
        );
        assert_eq!(
            ipv6_default_route(&line),
            Some(("eth0".to_owned(), "fe80::1".parse().unwrap()))
        );
        let unreachable = format!("{zero} 00 {zero} 00 {zero} ffffffff 00000001 00000000 00200200 lo");
        assert_eq!(ipv6_default_route(&unreachable), None);
    }

    #[test]
    fn addresses_of_host_carry_prefix() {
        let found = addresses().unwrap();
        assert!(found.iter().all(|address| match address.address {
            IpAddr::V4(_) => address.prefix <= 32,
            IpAddr::V6(_) => address.prefix <= 128,
        }));
    }
}

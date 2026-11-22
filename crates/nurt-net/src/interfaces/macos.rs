// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interfaces on macOS: `getifaddrs` for addresses, link-layer addresses,
//! MTU and interface type; the routing table read with `sysctl` for the
//! default gateway.

// The module calls `getifaddrs`, `freeifaddrs` and `sysctl` and reads the
// structures they return; every unsafe block states its invariant.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::CStr;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use nurt_contracts::record::{InterfaceKind, NetworkInterface};

use super::{InterfaceAddress, format_mac, prefix_length};

/// `IFT_ETHER` from `net/if_types.h`.
const IFT_ETHER: u8 = 0x06;
/// `IFT_LOOP` from `net/if_types.h`.
const IFT_LOOP: u8 = 0x18;
/// `IFT_IEEE80211` from `net/if_types.h`.
const IFT_IEEE80211: u8 = 0x47;
/// Name prefixes of software interfaces: tunnels, bridges, peer-to-peer
/// Wi-Fi links and interfaces of virtual machines.
const VIRTUAL_PREFIXES: &[&str] = &[
    "utun", "gif", "stf", "bridge", "awdl", "llw", "ipsec", "vmenet", "anpi", "ap",
];
/// Offset of the size of the message in `rt_msghdr`.
const RTM_MSGLEN_OFFSET: usize = 0;
/// Offset of the interface index in `rt_msghdr`.
const RTM_INDEX_OFFSET: usize = 4;
/// Offset of the bit set of socket addresses in `rt_msghdr`.
const RTM_ADDRS_OFFSET: usize = 12;
/// Number of socket address slots of a routing message (`RTAX_MAX`).
const RTAX_MAX: usize = 8;
/// Socket addresses in routing messages are padded to this alignment.
const SOCKADDR_ALIGN: usize = 4;
/// Offset of the IPv4 address in `sockaddr_in`.
const SIN_ADDR_OFFSET: usize = 4;
/// Offset of the IPv6 address in `sockaddr_in6`.
const SIN6_ADDR_OFFSET: usize = 8;

#[derive(Default)]
struct Link {
    index: u16,
    up: bool,
    mac: Option<String>,
    mtu: Option<u64>,
    link_type: Option<u8>,
    addresses: Vec<InterfaceAddress>,
}

/// Interfaces of the host.
pub(super) fn list() -> io::Result<Vec<NetworkInterface>> {
    let links = links()?;
    let gateways = default_gateways().unwrap_or_default();
    Ok(links
        .into_iter()
        .map(|(name, link)| {
            let gateway = gateways.get(&link.index).map(ToString::to_string);
            NetworkInterface {
                kind: Some(kind(&name, link.link_type)),
                addresses: link.addresses.iter().map(InterfaceAddress::to_text).collect(),
                up: link.up,
                mac: link.mac,
                mtu: link.mtu,
                gateway,
                name,
            }
        })
        .collect())
}

fn kind(name: &str, link_type: Option<u8>) -> InterfaceKind {
    match link_type {
        Some(IFT_LOOP) => InterfaceKind::Loopback,
        Some(IFT_IEEE80211) => InterfaceKind::Wireless,
        _ if VIRTUAL_PREFIXES.iter().any(|prefix| name.starts_with(prefix)) => {
            InterfaceKind::Virtual
        }
        Some(IFT_ETHER) => InterfaceKind::Ethernet,
        _ => InterfaceKind::Other,
    }
}

fn links() -> io::Result<Vec<(String, Link)>> {
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `head` is a valid location for the list pointer; on success the
    // list is released below with `freeifaddrs`.
    if unsafe { libc::getifaddrs(&raw mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut order: Vec<(String, Link)> = Vec::new();
    let mut cursor = head;
    while !cursor.is_null() {
        // SAFETY: `cursor` is a non-null element of the list returned by
        // `getifaddrs`, which stays valid until `freeifaddrs`.
        let entry = unsafe { &*cursor };
        record_entry(entry, &mut order);
        cursor = entry.ifa_next;
    }
    // SAFETY: `head` was returned by a successful `getifaddrs` and is freed
    // once; no reference into the list outlives this call.
    unsafe { libc::freeifaddrs(head) };
    Ok(order)
}

fn record_entry(entry: &libc::ifaddrs, order: &mut Vec<(String, Link)>) {
    if entry.ifa_name.is_null() {
        return;
    }
    // SAFETY: `ifa_name` is a non-null NUL-terminated string owned by the
    // list.
    let name = unsafe { CStr::from_ptr(entry.ifa_name) }
        .to_string_lossy()
        .into_owned();
    let position = match order.iter().position(|(known, _)| *known == name) {
        Some(position) => position,
        None => {
            order.push((name.clone(), Link::default()));
            order.len() - 1
        }
    };
    let Some((_, link)) = order.get_mut(position) else {
        return;
    };
    link.up = entry.ifa_flags & libc::IFF_UP as u32 != 0;
    if entry.ifa_addr.is_null() {
        return;
    }
    // SAFETY: `ifa_addr` is non-null and points to a socket address whose
    // family field is valid for every family.
    let family = i32::from(unsafe { (*entry.ifa_addr).sa_family });
    match family {
        libc::AF_LINK => record_link(entry, link),
        libc::AF_INET | libc::AF_INET6 => {
            if let Some(address) = ip_address(entry, &name, family) {
                link.addresses.push(address);
            }
        }
        _ => {}
    }
}

fn record_link(entry: &libc::ifaddrs, link: &mut Link) {
    // SAFETY: the family is `AF_LINK`, so the address is a `sockaddr_dl`.
    let dl = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_dl>() };
    link.index = dl.sdl_index;
    let name_len = usize::from(dl.sdl_nlen);
    let mac_len = usize::from(dl.sdl_alen);
    let data_offset = std::mem::offset_of!(libc::sockaddr_dl, sdl_data);
    if mac_len > 0 && data_offset + name_len + mac_len <= usize::from(dl.sdl_len) {
        // SAFETY: `sdl_len` covers the whole structure, so the name and the
        // link-layer address that follow `sdl_data` are inside it.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                dl.sdl_data.as_ptr().cast::<u8>().add(name_len),
                mac_len,
            )
        };
        link.mac = format_mac(bytes);
    }
    if !entry.ifa_data.is_null() {
        // SAFETY: for `AF_LINK` entries `ifa_data` points to the `if_data`
        // statistics of the interface.
        let data = unsafe { &*entry.ifa_data.cast::<libc::if_data>() };
        link.mtu = Some(u64::from(data.ifi_mtu));
        link.link_type = Some(data.ifi_type);
    }
}

fn ip_address(entry: &libc::ifaddrs, name: &str, family: i32) -> Option<InterfaceAddress> {
    let (address, mask) = if family == libc::AF_INET {
        // SAFETY: the family is `AF_INET`, so the address and the mask (when
        // present) are `sockaddr_in`.
        let address = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in>() };
        let mask = (!entry.ifa_netmask.is_null()).then(|| {
            // SAFETY: as above, a non-null mask of an `AF_INET` entry.
            let mask = unsafe { &*entry.ifa_netmask.cast::<libc::sockaddr_in>() };
            mask.sin_addr.s_addr.to_ne_bytes().to_vec()
        });
        let ip = Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes());
        (IpAddr::V4(ip), mask)
    } else {
        // SAFETY: the family is `AF_INET6`, so the address and the mask (when
        // present) are `sockaddr_in6`.
        let address = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in6>() };
        let mask = (!entry.ifa_netmask.is_null()).then(|| {
            // SAFETY: as above, a non-null mask of an `AF_INET6` entry.
            let mask = unsafe { &*entry.ifa_netmask.cast::<libc::sockaddr_in6>() };
            mask.sin6_addr.s6_addr.to_vec()
        });
        (IpAddr::V6(Ipv6Addr::from(address.sin6_addr.s6_addr)), mask)
    };
    let full = if address.is_ipv4() { 32 } else { 128 };
    Some(InterfaceAddress {
        interface: name.to_owned(),
        address,
        prefix: mask.as_deref().map_or(full, prefix_length),
    })
}

/// Default gateways by interface index, from the routing table.
fn default_gateways() -> io::Result<HashMap<u16, IpAddr>> {
    let mut mib = [
        libc::CTL_NET,
        libc::PF_ROUTE,
        0,
        libc::AF_UNSPEC,
        libc::NET_RT_FLAGS,
        libc::RTF_GATEWAY,
    ];
    let mib_len = libc::c_uint::try_from(mib.len()).unwrap_or(0);
    let mut size: libc::size_t = 0;
    // SAFETY: the name has `mib_len` elements; a null buffer asks for the
    // size, which is written to `size`.
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib_len,
            std::ptr::null_mut(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0_u8; size];
    // SAFETY: `buffer` holds `size` writable bytes and `size` tells the
    // kernel its length; the kernel writes the used length back.
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib_len,
            buffer.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    buffer.truncate(size);
    Ok(parse_routes(&buffer))
}

fn parse_routes(buffer: &[u8]) -> HashMap<u16, IpAddr> {
    let header = std::mem::size_of::<libc::rt_msghdr>();
    let mut found = HashMap::new();
    let mut offset = 0;
    while let Some(length) = read_u16(buffer, offset + RTM_MSGLEN_OFFSET) {
        let length = usize::from(length);
        let Some(message) = buffer.get(offset..offset + length) else {
            break;
        };
        if length == 0 {
            break;
        }
        if let Some((index, gateway)) = default_route(message, header) {
            found.entry(index).or_insert(gateway);
        }
        offset += length;
    }
    found
}

fn default_route(message: &[u8], header: usize) -> Option<(u16, IpAddr)> {
    let index = read_u16(message, RTM_INDEX_OFFSET)?;
    let addrs = read_u32(message, RTM_ADDRS_OFFSET)?;
    let mut slots: [Option<&[u8]>; RTAX_MAX] = [None; RTAX_MAX];
    let mut position = header;
    for (bit, slot) in slots.iter_mut().enumerate() {
        if addrs & (1 << bit) == 0 {
            continue;
        }
        let length = usize::from(*message.get(position)?);
        *slot = Some(message.get(position..position + length)?);
        position += if length == 0 {
            SOCKADDR_ALIGN
        } else {
            length.div_ceil(SOCKADDR_ALIGN) * SOCKADDR_ALIGN
        };
    }
    let [destination, gateway, netmask, ..] = slots;
    let destination = sockaddr_ip(destination?)?;
    if !destination.is_unspecified() {
        return None;
    }
    let netmask_is_zero = netmask.is_none_or(|mask| {
        mask.get(SIN_ADDR_OFFSET..)
            .is_none_or(|bytes| bytes.iter().all(|byte| *byte == 0))
    });
    if !netmask_is_zero {
        return None;
    }
    Some((index, sockaddr_ip(gateway?)?))
}

fn sockaddr_ip(sockaddr: &[u8]) -> Option<IpAddr> {
    let family = i32::from(*sockaddr.get(1)?);
    if family == libc::AF_INET {
        let bytes: [u8; 4] = sockaddr
            .get(SIN_ADDR_OFFSET..SIN_ADDR_OFFSET + 4)?
            .try_into()
            .ok()?;
        return Some(IpAddr::V4(Ipv4Addr::from(bytes)));
    }
    if family == libc::AF_INET6 {
        let mut bytes: [u8; 16] = sockaddr
            .get(SIN6_ADDR_OFFSET..SIN6_ADDR_OFFSET + 16)?
            .try_into()
            .ok()?;
        let address = Ipv6Addr::from(bytes);
        if address.is_unicast_link_local() {
            // The kernel stores the scope of a link-local address in the
            // second 16-bit word; it is not part of the address.
            bytes[2] = 0;
            bytes[3] = 0;
        }
        return Some(IpAddr::V6(Ipv6Addr::from(bytes)));
    }
    None
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let raw: [u8; 2] = bytes.get(offset..offset + 2)?.try_into().ok()?;
    Some(u16::from_ne_bytes(raw))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_ne_bytes(raw))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-net::interfaces::windows
// Responsibility:   Interfaces on Windows: `GetAdaptersAddresses` with unicast and gateway.
// Dependencies:     nurt_contracts, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Interfaces on Windows: `GetAdaptersAddresses` with unicast and gateway
//! addresses.

// The module calls `GetAdaptersAddresses` and walks the linked lists it
// writes into a buffer owned here; every unsafe block states its invariant.
#![allow(unsafe_code)]

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use nurt_contracts::record::{InterfaceKind, NetworkInterface};
use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
    GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6, SOCKET_ADDRESS,
};

use super::format_mac;

/// `IF_TYPE_ETHERNET_CSMACD` from `ipifcons.h`.
const IF_TYPE_ETHERNET: u32 = 6;
/// `IF_TYPE_SOFTWARE_LOOPBACK` from `ipifcons.h`.
const IF_TYPE_LOOPBACK: u32 = 24;
/// `IF_TYPE_IEEE80211` from `ipifcons.h`.
const IF_TYPE_WIRELESS: u32 = 71;
/// `IF_TYPE_PROP_VIRTUAL` from `ipifcons.h`.
const IF_TYPE_PROP_VIRTUAL: u32 = 53;
/// `IF_TYPE_TUNNEL` from `ipifcons.h`.
const IF_TYPE_TUNNEL: u32 = 131;
/// Buffer size recommended by the documentation of `GetAdaptersAddresses`.
const INITIAL_BUFFER: u32 = 15 * 1024;
/// Calls made before giving up when the list keeps growing.
const ATTEMPTS: usize = 4;

/// Interfaces of the host.
pub(super) fn list() -> io::Result<Vec<NetworkInterface>> {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS
        | GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size = INITIAL_BUFFER;
    for _ in 0..ATTEMPTS {
        let words = usize::try_from(size).unwrap_or(usize::MAX) / 8 + 1;
        let mut buffer = vec![0_u64; words];
        let head = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: `head` points to a buffer of at least `size` bytes aligned
        // to 8 bytes; the function writes no more than `size` bytes and
        // updates `size` when the buffer is too small.
        let status = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC),
                flags,
                std::ptr::null(),
                head,
                &raw mut size,
            )
        };
        match status {
            NO_ERROR => return Ok(walk(head)),
            ERROR_NO_DATA => return Ok(Vec::new()),
            ERROR_BUFFER_OVERFLOW => continue,
            other => {
                return Err(io::Error::from_raw_os_error(
                    i32::try_from(other).unwrap_or(i32::MAX),
                ));
            }
        }
    }
    Err(io::Error::from_raw_os_error(
        i32::try_from(ERROR_BUFFER_OVERFLOW).unwrap_or(i32::MAX),
    ))
}

fn walk(head: *const IP_ADAPTER_ADDRESSES_LH) -> Vec<NetworkInterface> {
    let mut found = Vec::new();
    let mut cursor = head;
    while !cursor.is_null() {
        // SAFETY: `cursor` is the head written by `GetAdaptersAddresses` or a
        // `Next` link of it; all nodes live in the buffer of the caller.
        let adapter = unsafe { &*cursor };
        found.push(adapter_record(adapter));
        cursor = adapter.Next;
    }
    found
}

fn adapter_record(adapter: &IP_ADAPTER_ADDRESSES_LH) -> NetworkInterface {
    let mac_len = usize::try_from(adapter.PhysicalAddressLength)
        .unwrap_or(0)
        .min(adapter.PhysicalAddress.len());
    let mut addresses = Vec::new();
    let mut unicast = adapter.FirstUnicastAddress;
    while !unicast.is_null() {
        // SAFETY: a non-null node of the unicast list in the buffer.
        let node = unsafe { &*unicast };
        if let Some(ip) = socket_address(&node.Address) {
            addresses.push(format!("{ip}/{}", node.OnLinkPrefixLength));
        }
        unicast = node.Next;
    }
    let mut gateways = Vec::new();
    let mut gateway = adapter.FirstGatewayAddress;
    while !gateway.is_null() {
        // SAFETY: a non-null node of the gateway list in the buffer.
        let node = unsafe { &*gateway };
        if let Some(ip) = socket_address(&node.Address) {
            gateways.push(ip);
        }
        gateway = node.Next;
    }
    gateways.sort_by_key(|ip| !ip.is_ipv4());
    NetworkInterface {
        name: wide_text(adapter.FriendlyName),
        addresses,
        up: adapter.OperStatus == IfOperStatusUp,
        mac: adapter.PhysicalAddress.get(..mac_len).and_then(format_mac),
        mtu: (adapter.Mtu != u32::MAX).then_some(u64::from(adapter.Mtu)),
        kind: Some(kind(adapter.IfType)),
        gateway: gateways.first().map(ToString::to_string),
    }
}

fn kind(if_type: u32) -> InterfaceKind {
    match if_type {
        IF_TYPE_ETHERNET => InterfaceKind::Ethernet,
        IF_TYPE_WIRELESS => InterfaceKind::Wireless,
        IF_TYPE_LOOPBACK => InterfaceKind::Loopback,
        IF_TYPE_PROP_VIRTUAL | IF_TYPE_TUNNEL => InterfaceKind::Virtual,
        _ => InterfaceKind::Other,
    }
}

fn socket_address(address: &SOCKET_ADDRESS) -> Option<IpAddr> {
    if address.lpSockaddr.is_null() {
        return None;
    }
    // SAFETY: `lpSockaddr` is non-null and points to a socket address whose
    // family field is valid for every family.
    let family = unsafe { (*address.lpSockaddr).sa_family };
    let length = usize::try_from(address.iSockaddrLength).unwrap_or(0);
    if family == AF_INET && length >= std::mem::size_of::<SOCKADDR_IN>() {
        // SAFETY: the family is `AF_INET` and the length covers `SOCKADDR_IN`.
        let ipv4 = unsafe { &*address.lpSockaddr.cast::<SOCKADDR_IN>() };
        // SAFETY: every variant of the address union is a view of the same
        // four bytes.
        let raw = unsafe { ipv4.sin_addr.S_un.S_addr };
        return Some(IpAddr::V4(Ipv4Addr::from(raw.to_ne_bytes())));
    }
    if family == AF_INET6 && length >= std::mem::size_of::<SOCKADDR_IN6>() {
        // SAFETY: the family is `AF_INET6` and the length covers
        // `SOCKADDR_IN6`.
        let ipv6 = unsafe { &*address.lpSockaddr.cast::<SOCKADDR_IN6>() };
        // SAFETY: every variant of the address union is a view of the same
        // sixteen bytes.
        let bytes = unsafe { ipv6.sin6_addr.u.Byte };
        return Some(IpAddr::V6(Ipv6Addr::from(bytes)));
    }
    None
}

fn wide_text(text: windows_sys::core::PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    let mut length = 0;
    // SAFETY: `text` is a NUL-terminated wide string in the buffer; the loop
    // stops at the terminator.
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: the first `length` characters were read above.
    let units = unsafe { std::slice::from_raw_parts(text, length) };
    String::from_utf16_lossy(units)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

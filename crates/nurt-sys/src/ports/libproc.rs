// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::ports::libproc
// Responsibility:   Decoder of `struct socket_fdinfo` returned by.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Decoder of `struct socket_fdinfo` returned by
//! `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` on macOS.
//!
//! Offsets follow `sys/proc_info.h`: `struct proc_fileinfo` (24 bytes) is
//! followed by `struct socket_info`, whose protocol union starts 240 bytes
//! into it; the whole structure is 792 bytes. Ports are stored in network
//! byte order in the lower 16 bits of an `int`; the other fields are native.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use nurt_contracts::record::{PortState, Protocol};

use super::{SocketEntry, remote_endpoint};

/// Size of `struct socket_fdinfo`.
pub(crate) const SOCKET_FDINFO_SIZE: usize = 792;

/// Offset of `struct socket_info` after `struct proc_fileinfo`.
const SOCKET_INFO: usize = 24;
pub(crate) const SOI_PROTOCOL: usize = SOCKET_INFO + 156;
pub(crate) const SOI_KIND: usize = SOCKET_INFO + 232;
/// Offset of the protocol union `soi_proto`.
const PROTO: usize = SOCKET_INFO + 240;
pub(crate) const INSI_FPORT: usize = PROTO;
pub(crate) const INSI_LPORT: usize = PROTO + 4;
pub(crate) const INSI_VFLAG: usize = PROTO + 24;
pub(crate) const INSI_FADDR: usize = PROTO + 32;
pub(crate) const INSI_LADDR: usize = PROTO + 48;
pub(crate) const TCPSI_STATE: usize = PROTO + 80;

/// Offset of the IPv4 address inside `struct in4in6_addr`.
const IPV4_IN_IN6: usize = 12;

/// `soi_kind` values.
pub(crate) const SOCKINFO_IN: i32 = 1;
pub(crate) const SOCKINFO_TCP: i32 = 2;

/// `insi_vflag` bit of IPv4 sockets.
pub(crate) const INI_IPV4: u8 = 0x1;

const IPPROTO_UDP: i32 = 17;

/// Values of `tcpsi_state` (`TSI_S_*`).
mod state {
    pub(super) const CLOSED: i32 = 0;
    pub(super) const LISTEN: i32 = 1;
    pub(super) const SYN_SENT: i32 = 2;
    pub(super) const SYN_RECEIVED: i32 = 3;
    pub(super) const ESTABLISHED: i32 = 4;
    pub(super) const CLOSE_WAIT: i32 = 5;
    pub(super) const FIN_WAIT_1: i32 = 6;
    pub(super) const CLOSING: i32 = 7;
    pub(super) const LAST_ACK: i32 = 8;
    pub(super) const FIN_WAIT_2: i32 = 9;
    pub(super) const TIME_WAIT: i32 = 10;
}

fn tcp_state(value: i32) -> PortState {
    match value {
        state::LISTEN => PortState::Listen,
        state::SYN_SENT => PortState::SynSent,
        state::SYN_RECEIVED => PortState::SynReceived,
        state::ESTABLISHED => PortState::Established,
        state::CLOSE_WAIT => PortState::CloseWait,
        state::FIN_WAIT_1 | state::FIN_WAIT_2 => PortState::FinWait,
        state::CLOSING => PortState::Closing,
        state::LAST_ACK => PortState::LastAck,
        state::TIME_WAIT => PortState::TimeWait,
        state::CLOSED => PortState::Closed,
        _ => PortState::Closed,
    }
}

fn int(buffer: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_ne_bytes(
        buffer.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn port(buffer: &[u8], offset: usize) -> Option<u16> {
    let raw = int(buffer, offset)?.to_ne_bytes();
    let low = if cfg!(target_endian = "little") {
        [raw[0], raw[1]]
    } else {
        [raw[2], raw[3]]
    };
    Some(u16::from_be(u16::from_ne_bytes(low)))
}

fn address(buffer: &[u8], offset: usize, ipv4: bool) -> Option<IpAddr> {
    let octets: [u8; 16] = buffer.get(offset..offset + 16)?.try_into().ok()?;
    if ipv4 {
        let v4: [u8; 4] = octets.get(IPV4_IN_IN6..)?.try_into().ok()?;
        Some(IpAddr::V4(Ipv4Addr::from(v4)))
    } else {
        Some(IpAddr::V6(Ipv6Addr::from(octets)))
    }
}

/// Decodes a TCP or UDP socket; `None` for other sockets and short buffers.
pub(crate) fn decode(buffer: &[u8]) -> Option<SocketEntry> {
    let kind = int(buffer, SOI_KIND)?;
    let protocol = match kind {
        SOCKINFO_TCP => Protocol::Tcp,
        SOCKINFO_IN if int(buffer, SOI_PROTOCOL)? == IPPROTO_UDP => Protocol::Udp,
        _ => return None,
    };
    let ipv4 = buffer.get(INSI_VFLAG)? & INI_IPV4 != 0;
    let local = SocketAddr::new(
        address(buffer, INSI_LADDR, ipv4)?,
        port(buffer, INSI_LPORT)?,
    );
    let remote = SocketAddr::new(
        address(buffer, INSI_FADDR, ipv4)?,
        port(buffer, INSI_FPORT)?,
    );
    let state = match protocol {
        Protocol::Tcp => tcp_state(int(buffer, TCPSI_STATE)?),
        _ => PortState::None,
    };
    Some(SocketEntry {
        protocol,
        local,
        remote: remote_endpoint(remote),
        state,
        pid: None,
        process: None,
    })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

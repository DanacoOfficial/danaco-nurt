// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Decoders of the socket tables returned by `GetExtendedTcpTable` and
//! `GetExtendedUdpTable`.
//!
//! Every table starts with the number of rows as a `DWORD`, followed by the
//! rows. Addresses are stored in network byte order; ports are stored in
//! network byte order in the lower 16 bits of a `DWORD`. Windows runs only on
//! little-endian processors, so the other fields are read as little-endian.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use nurt_contracts::record::{PortState, Protocol};

use super::{SocketEntry, remote_endpoint};

/// Size of the row count at the start of a table.
const HEADER_SIZE: usize = 4;

/// Row sizes of `MIB_TCPROW_OWNER_PID`, `MIB_TCP6ROW_OWNER_PID`,
/// `MIB_UDPROW_OWNER_PID` and `MIB_UDP6ROW_OWNER_PID`.
pub(crate) const TCP4_ROW_SIZE: usize = 24;
pub(crate) const TCP6_ROW_SIZE: usize = 56;
pub(crate) const UDP4_ROW_SIZE: usize = 12;
pub(crate) const UDP6_ROW_SIZE: usize = 28;

/// Values of `MIB_TCP_STATE`.
mod state {
    pub(super) const CLOSED: u32 = 1;
    pub(super) const LISTEN: u32 = 2;
    pub(super) const SYN_SENT: u32 = 3;
    pub(super) const SYN_RCVD: u32 = 4;
    pub(super) const ESTAB: u32 = 5;
    pub(super) const FIN_WAIT1: u32 = 6;
    pub(super) const FIN_WAIT2: u32 = 7;
    pub(super) const CLOSE_WAIT: u32 = 8;
    pub(super) const CLOSING: u32 = 9;
    pub(super) const LAST_ACK: u32 = 10;
    pub(super) const TIME_WAIT: u32 = 11;
}

/// Kind of a socket table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableKind {
    Tcp4,
    Tcp6,
    Udp4,
    Udp6,
}

impl TableKind {
    pub(crate) fn protocol(self) -> Protocol {
        match self {
            Self::Tcp4 | Self::Tcp6 => Protocol::Tcp,
            Self::Udp4 | Self::Udp6 => Protocol::Udp,
        }
    }

    fn row_size(self) -> usize {
        match self {
            Self::Tcp4 => TCP4_ROW_SIZE,
            Self::Tcp6 => TCP6_ROW_SIZE,
            Self::Udp4 => UDP4_ROW_SIZE,
            Self::Udp6 => UDP6_ROW_SIZE,
        }
    }
}

/// Maps a `MIB_TCP_STATE` value to the record state.
pub(crate) fn tcp_state(value: u32) -> PortState {
    match value {
        state::LISTEN => PortState::Listen,
        state::SYN_SENT => PortState::SynSent,
        state::SYN_RCVD => PortState::SynReceived,
        state::ESTAB => PortState::Established,
        state::FIN_WAIT1 | state::FIN_WAIT2 => PortState::FinWait,
        state::CLOSE_WAIT => PortState::CloseWait,
        state::CLOSING => PortState::Closing,
        state::LAST_ACK => PortState::LastAck,
        state::TIME_WAIT => PortState::TimeWait,
        state::CLOSED => PortState::Closed,
        _ => PortState::Closed,
    }
}

fn dword(row: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        row.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn port(row: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        row.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn ipv4(row: &[u8], offset: usize) -> Option<Ipv4Addr> {
    let octets: [u8; 4] = row.get(offset..offset + 4)?.try_into().ok()?;
    Some(Ipv4Addr::from(octets))
}

fn ipv6(row: &[u8], offset: usize) -> Option<Ipv6Addr> {
    let octets: [u8; 16] = row.get(offset..offset + 16)?.try_into().ok()?;
    Some(Ipv6Addr::from(octets))
}

/// Decodes one row; offsets follow the row structures of `iprtrmib.h`.
fn decode_row(kind: TableKind, row: &[u8]) -> Option<SocketEntry> {
    let (local, remote, state, pid) = match kind {
        TableKind::Tcp4 => (
            SocketAddr::new(ipv4(row, 4)?.into(), port(row, 8)?),
            Some(SocketAddr::new(ipv4(row, 12)?.into(), port(row, 16)?)),
            tcp_state(dword(row, 0)?),
            dword(row, 20)?,
        ),
        TableKind::Tcp6 => (
            SocketAddr::new(ipv6(row, 0)?.into(), port(row, 20)?),
            Some(SocketAddr::new(ipv6(row, 24)?.into(), port(row, 44)?)),
            tcp_state(dword(row, 48)?),
            dword(row, 52)?,
        ),
        TableKind::Udp4 => (
            SocketAddr::new(ipv4(row, 0)?.into(), port(row, 4)?),
            None,
            PortState::None,
            dword(row, 8)?,
        ),
        TableKind::Udp6 => (
            SocketAddr::new(ipv6(row, 0)?.into(), port(row, 20)?),
            None,
            PortState::None,
            dword(row, 24)?,
        ),
    };
    Some(SocketEntry {
        protocol: kind.protocol(),
        local,
        remote: remote.and_then(remote_endpoint),
        state,
        pid: Some(u64::from(pid)),
        process: None,
    })
}

/// Decodes a whole table; `None` when the buffer is shorter than its row
/// count states.
pub(crate) fn decode_table(kind: TableKind, buffer: &[u8]) -> Option<Vec<SocketEntry>> {
    let count = usize::try_from(dword(buffer, 0)?).ok()?;
    let size = kind.row_size();
    (0..count)
        .map(|index| {
            let start = HEADER_SIZE + index * size;
            decode_row(kind, buffer.get(start..start + size)?)
        })
        .collect()
}

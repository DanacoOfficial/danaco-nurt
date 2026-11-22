// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Sockets of Linux, read from `/proc/net`.
//!
//! The tables print addresses as hexadecimal 32-bit words in host byte order
//! (one word for IPv4, four for IPv6) and ports as hexadecimal numbers. The
//! `inode` column identifies the socket; the owning process is the one with
//! a descriptor in `/proc/<pid>/fd` that links to `socket:[<inode>]`.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{PortState, Protocol};

use super::{ProtocolFilter, SocketEntry, SocketSource, SocketTable, remote_endpoint};
use crate::common::SysRoot;

/// Hexadecimal digits of one 32-bit address word.
const WORD_DIGITS: usize = 8;

/// Number of 32-bit words of an IPv6 address.
const IPV6_WORDS: usize = 4;

/// Column of the `inode` field in a table row, counted from 0.
const INODE_COLUMN: usize = 9;

/// TCP states of `include/net/tcp_states.h`.
mod tcp {
    pub(super) const ESTABLISHED: u8 = 0x01;
    pub(super) const SYN_SENT: u8 = 0x02;
    pub(super) const SYN_RECV: u8 = 0x03;
    pub(super) const FIN_WAIT1: u8 = 0x04;
    pub(super) const FIN_WAIT2: u8 = 0x05;
    pub(super) const TIME_WAIT: u8 = 0x06;
    pub(super) const CLOSE: u8 = 0x07;
    pub(super) const CLOSE_WAIT: u8 = 0x08;
    pub(super) const LAST_ACK: u8 = 0x09;
    pub(super) const LISTEN: u8 = 0x0A;
    pub(super) const CLOSING: u8 = 0x0B;
    pub(super) const NEW_SYN_RECV: u8 = 0x0C;
}

/// One row of a `/proc/net` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) local: SocketAddr,
    pub(crate) remote: SocketAddr,
    pub(crate) state: u8,
    pub(crate) inode: u64,
}

/// Maps a TCP state code to the record state.
pub(crate) fn tcp_state(code: u8) -> PortState {
    match code {
        tcp::ESTABLISHED => PortState::Established,
        tcp::SYN_SENT => PortState::SynSent,
        tcp::SYN_RECV | tcp::NEW_SYN_RECV => PortState::SynReceived,
        tcp::FIN_WAIT1 | tcp::FIN_WAIT2 => PortState::FinWait,
        tcp::TIME_WAIT => PortState::TimeWait,
        tcp::CLOSE_WAIT => PortState::CloseWait,
        tcp::LAST_ACK => PortState::LastAck,
        tcp::LISTEN => PortState::Listen,
        tcp::CLOSING => PortState::Closing,
        tcp::CLOSE => PortState::Closed,
        _ => PortState::Closed,
    }
}

fn parse_word(hex: &str) -> Option<[u8; 4]> {
    u32::from_str_radix(hex, 16).ok().map(u32::to_ne_bytes)
}

fn parse_address(hex: &str) -> Option<IpAddr> {
    match hex.len() {
        WORD_DIGITS => parse_word(hex).map(|octets| IpAddr::V4(Ipv4Addr::from(octets))),
        len if len == WORD_DIGITS * IPV6_WORDS => {
            let mut octets = [0_u8; 16];
            let (words, _) = octets.as_chunks_mut::<4>();
            for (index, chunk) in words.iter_mut().enumerate() {
                let word = hex.get(index * WORD_DIGITS..(index + 1) * WORD_DIGITS)?;
                *chunk = parse_word(word)?;
            }
            Some(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        _ => None,
    }
}

/// Parses an endpoint written as `<address>:<port>` in hexadecimal.
pub(crate) fn parse_endpoint(text: &str) -> Option<SocketAddr> {
    let (address, port) = text.split_once(':')?;
    Some(SocketAddr::new(
        parse_address(address)?,
        u16::from_str_radix(port, 16).ok()?,
    ))
}

/// Parses the rows of a table; the header and malformed rows are skipped.
pub(crate) fn parse_table(text: &str) -> Vec<Row> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            Some(Row {
                local: parse_endpoint(columns.get(1)?)?,
                remote: parse_endpoint(columns.get(2)?)?,
                state: u8::from_str_radix(columns.get(3)?, 16).ok()?,
                inode: columns.get(INODE_COLUMN)?.parse().ok()?,
            })
        })
        .collect()
}

/// Processes holding socket inodes.
#[derive(Debug, Default)]
pub(crate) struct Holders {
    pub(crate) pids: HashMap<u64, u64>,
    /// Some descriptor directories were not readable.
    pub(crate) incomplete: bool,
}

/// Reads the socket inodes held by every process under `root`.
pub(crate) fn socket_holders(root: &SysRoot) -> Holders {
    let mut holders = Holders::default();
    let Ok(entries) = fs::read_dir(root.path("/proc")) else {
        holders.incomplete = true;
        return holders;
    };
    for pid in entries.filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u64>().ok()) {
        match fs::read_dir(root.path(format!("/proc/{pid}/fd"))) {
            Ok(descriptors) => {
                for descriptor in descriptors.flatten() {
                    if let Some(inode) = socket_inode(&descriptor.path()) {
                        holders.pids.entry(inode).or_insert(pid);
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::PermissionDenied => holders.incomplete = true,
            Err(_) => {}
        }
    }
    holders
}

fn socket_inode(link: &std::path::Path) -> Option<u64> {
    let target = fs::read_link(link).ok()?;
    target
        .to_str()?
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// Socket source reading `/proc/net` under a [`SysRoot`].
#[derive(Debug, Clone)]
pub(crate) struct ProcNet {
    root: SysRoot,
}

impl ProcNet {
    #[cfg(target_os = "linux")]
    pub(crate) fn live() -> Self {
        Self {
            root: SysRoot::live(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(root: SysRoot) -> Self {
        Self { root }
    }

    fn process_name(&self, pid: u64, names: &mut HashMap<u64, Option<String>>) -> Option<String> {
        names
            .entry(pid)
            .or_insert_with(|| {
                fs::read_to_string(self.root.path(format!("/proc/{pid}/comm")))
                    .ok()
                    .map(|name| name.trim_end().to_owned())
            })
            .clone()
    }
}

const TABLES: [(&str, Protocol); 4] = [
    ("tcp", Protocol::Tcp),
    ("tcp6", Protocol::Tcp),
    ("udp", Protocol::Udp),
    ("udp6", Protocol::Udp),
];

impl SocketSource for ProcNet {
    fn sockets(&self, protocols: ProtocolFilter) -> Result<SocketTable> {
        let mut rows = Vec::new();
        let (mut read, mut denied) = (0, 0);
        for (name, protocol) in TABLES {
            if !protocols.includes(protocol) {
                continue;
            }
            match fs::read_to_string(self.root.path(format!("/proc/net/{name}"))) {
                Ok(text) => {
                    read += 1;
                    rows.extend(parse_table(&text).into_iter().map(|row| (protocol, row)));
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) if error.kind() == ErrorKind::PermissionDenied => denied += 1,
                Err(error) => {
                    return Err(Error::new(
                        ErrorCode::Io,
                        format!("cannot read /proc/net/{name}: {error}"),
                    ));
                }
            }
        }
        if read == 0 && denied > 0 {
            return Err(Error::new(
                ErrorCode::Privilege,
                "the socket tables are not readable by this account",
            )
            .with_detail("required", serde_json::Value::from("root")));
        }
        Ok(self.attribute(rows))
    }
}

impl ProcNet {
    fn attribute(&self, rows: Vec<(Protocol, Row)>) -> SocketTable {
        let holders = if rows.is_empty() {
            Holders::default()
        } else {
            socket_holders(&self.root)
        };
        let mut names = HashMap::new();
        let mut table = SocketTable::default();
        for (protocol, row) in rows {
            let pid = holders.pids.get(&row.inode).copied();
            if pid.is_none() && row.inode != 0 && holders.incomplete {
                table.unattributed += 1;
            }
            table.entries.push(SocketEntry {
                protocol,
                local: row.local,
                remote: remote_endpoint(row.remote),
                state: tcp_state(row.state),
                process: pid.and_then(|pid| self.process_name(pid, &mut names)),
                pid,
            });
        }
        table
    }
}

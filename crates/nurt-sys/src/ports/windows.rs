// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::ports::windows
// Responsibility:   Sockets of Windows, read with `GetExtendedTcpTable` and.
// Dependencies:     nurt_contracts, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Sockets of Windows, read with `GetExtendedTcpTable` and
//! `GetExtendedUdpTable` for IPv4 and IPv6.

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use nurt_contracts::error::{Error, ErrorCode, Result};
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, GetExtendedUdpTable, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

use super::win32::{TableKind, decode_table};
use super::{ProtocolFilter, SocketSource, SocketTable};
use crate::processes::windows::snapshot;

/// Attempts to read a table that grows between the size query and the read.
const ATTEMPTS: usize = 4;

/// Tables read for every call, in the order of the output.
const TABLES: [TableKind; 4] = [
    TableKind::Tcp4,
    TableKind::Tcp6,
    TableKind::Udp4,
    TableKind::Udp6,
];

/// Address family of the rows of a table.
fn family(kind: TableKind) -> u32 {
    let family = match kind {
        TableKind::Tcp4 | TableKind::Udp4 => AF_INET,
        TableKind::Tcp6 | TableKind::Udp6 => AF_INET6,
    };
    u32::from(family)
}

/// Socket source of the running system.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Win32Sockets;

impl SocketSource for Win32Sockets {
    fn sockets(&self, protocols: ProtocolFilter) -> Result<SocketTable> {
        let mut table = SocketTable::default();
        for kind in TABLES {
            if !protocols.includes(kind.protocol()) {
                continue;
            }
            let buffer = read_table(kind)?;
            let entries = decode_table(kind, &buffer).ok_or_else(|| {
                Error::new(ErrorCode::Io, "the socket table of the system is malformed")
            })?;
            table.entries.extend(entries);
        }
        let names: HashMap<u64, String> = snapshot()
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| (u64::from(entry.pid), entry.name))
                    .collect()
            })
            .unwrap_or_default();
        for entry in &mut table.entries {
            entry.process = entry.pid.and_then(|pid| names.get(&pid).cloned());
        }
        Ok(table)
    }
}

fn query(kind: TableKind, buffer: *mut c_void, size: &mut u32) -> u32 {
    let family = family(kind);
    match kind {
        TableKind::Tcp4 | TableKind::Tcp6 => {
            // SAFETY: `buffer` is null or holds `size` writable bytes; the call stores the
            // needed size in `size`.
            unsafe { GetExtendedTcpTable(buffer, size, 0, family, TCP_TABLE_OWNER_PID_ALL, 0) }
        }
        TableKind::Udp4 | TableKind::Udp6 => {
            // SAFETY: as for `GetExtendedTcpTable`.
            unsafe { GetExtendedUdpTable(buffer, size, 0, family, UDP_TABLE_OWNER_PID, 0) }
        }
    }
}

/// Reads one table into a byte buffer.
fn read_table(kind: TableKind) -> Result<Vec<u8>> {
    let mut size = 0_u32;
    for _ in 0..ATTEMPTS {
        let words = usize::try_from(size)
            .unwrap_or(0)
            .div_ceil(size_of::<u32>());
        let mut buffer = vec![0_u32; words.max(1)];
        let pointer = if size == 0 {
            null_mut()
        } else {
            buffer.as_mut_ptr().cast::<c_void>()
        };
        match query(kind, pointer, &mut size) {
            NO_ERROR if size > 0 || !pointer.is_null() => {
                let length = usize::try_from(size).unwrap_or(usize::MAX);
                let bytes: Vec<u8> = buffer.iter().flat_map(|word| word.to_le_bytes()).collect();
                return Ok(bytes.into_iter().take(length).collect());
            }
            NO_ERROR | ERROR_INSUFFICIENT_BUFFER => {}
            ERROR_ACCESS_DENIED => {
                return Err(Error::new(
                    ErrorCode::Privilege,
                    "the socket tables are not readable by this account",
                )
                .with_detail("required", serde_json::Value::from("administrator")));
            }
            code => {
                return Err(Error::new(
                    ErrorCode::Io,
                    format!(
                        "cannot read the socket table: {}",
                        std::io::Error::from_raw_os_error(i32::try_from(code).unwrap_or(i32::MAX))
                    ),
                ));
            }
        }
    }
    Err(Error::new(
        ErrorCode::Io,
        "the socket table kept growing while it was read",
    ))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

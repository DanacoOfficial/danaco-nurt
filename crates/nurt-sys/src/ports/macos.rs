// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::ports::macos
// Responsibility:   Sockets of macOS, read per process with `proc_pidinfo(PROC_PIDLISTFDS)`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Sockets of macOS, read per process with `proc_pidinfo(PROC_PIDLISTFDS)`
//! and `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)`.
//!
//! The system lists the descriptors of processes of other accounts only to a
//! privileged caller; their sockets are not visible and are counted.

use std::ffi::c_int;
use std::mem::size_of;
use std::ptr::null_mut;

use nurt_contracts::error::Result;

use super::libproc::{SOCKET_FDINFO_SIZE, decode};
use super::{ProtocolFilter, SocketSource, SocketTable};
use crate::processes::macos::{list_pids, process_name};

/// `PROC_PIDFDSOCKETINFO` of `sys/proc_info.h`.
const PROC_PIDFDSOCKETINFO: c_int = 3;

/// Socket source of the running system.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LibprocSockets;

/// Descriptors of a process; `None` when the account may not list them.
fn descriptors(pid: c_int) -> Option<Vec<libc::proc_fdinfo>> {
    // SAFETY: a null buffer of length zero asks for the required size only.
    let needed = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, null_mut(), 0) };
    let count = usize::try_from(needed).ok().filter(|&n| n > 0)? / size_of::<libc::proc_fdinfo>();
    let mut buffer = vec![
        libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0
        };
        count
    ];
    let size = c_int::try_from(buffer.len() * size_of::<libc::proc_fdinfo>()).ok()?;
    // SAFETY: `buffer` holds `size` writable bytes of `proc_fdinfo` entries.
    let filled = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            buffer.as_mut_ptr().cast(),
            size,
        )
    };
    let filled = usize::try_from(filled).ok().filter(|&n| n > 0)? / size_of::<libc::proc_fdinfo>();
    buffer.truncate(filled);
    Some(buffer)
}

fn socket_info(pid: c_int, fd: c_int) -> Option<Vec<u8>> {
    let mut buffer = vec![0_u8; SOCKET_FDINFO_SIZE];
    let size = c_int::try_from(buffer.len()).ok()?;
    // SAFETY: `buffer` holds `size` writable bytes for one `socket_fdinfo`.
    let written = unsafe {
        libc::proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDSOCKETINFO,
            buffer.as_mut_ptr().cast(),
            size,
        )
    };
    (written == size).then_some(buffer)
}

impl SocketSource for LibprocSockets {
    fn sockets(&self, protocols: ProtocolFilter) -> Result<SocketTable> {
        let mut table = SocketTable::default();
        for pid in list_pids()? {
            let Ok(id) = c_int::try_from(pid) else {
                continue;
            };
            let Some(list) = descriptors(id) else {
                table.hidden_processes += 1;
                continue;
            };
            let sockets = list
                .iter()
                .filter(|fd| fd.proc_fdtype == u32::try_from(libc::PROX_FDTYPE_SOCKET).unwrap_or(0))
                .filter_map(|fd| decode(&socket_info(id, fd.proc_fd)?))
                .filter(|socket| protocols.includes(socket.protocol));
            let name = process_name(pid);
            for mut socket in sockets {
                socket.pid = Some(pid);
                socket.process.clone_from(&name);
                table.entries.push(socket);
            }
        }
        Ok(table)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

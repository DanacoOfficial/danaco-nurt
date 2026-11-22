// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Echo requests on Windows through the ICMP helper functions
//! `IcmpSendEcho` (IPv4) and `Icmp6SendEcho2` (IPv6), which need no
//! privilege.

// The module calls the ICMP helper functions of `iphlpapi.dll` with buffers
// owned here; every unsafe block states its invariant.
#![allow(unsafe_code)]

use std::io;
use std::net::IpAddr;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ICMP_ECHO_REPLY, ICMPV6_ECHO_REPLY_LH, IP_SUCCESS, Icmp6CreateFile, Icmp6SendEcho2,
    IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET6, SOCKADDR_IN6};

use super::{Echo, PAYLOAD_LEN};

/// Reply buffer in 8-byte words: a reply structure, the echoed payload, an
/// ICMP error message and an `IO_STATUS_BLOCK`, with room to spare.
const REPLY_WORDS: usize = 64;
/// Payload of every request.
const PAYLOAD: [u8; PAYLOAD_LEN] = *b"nurtping";

/// Handle of the ICMP helper for one address family.
pub(super) struct IcmpEcho {
    handle: HANDLE,
    address: IpAddr,
}

impl IcmpEcho {
    /// Opens an ICMP helper handle for the family of `address`.
    pub(super) fn open(address: IpAddr) -> Result<Self> {
        // SAFETY: the functions take no arguments and return a handle or
        // `INVALID_HANDLE_VALUE`.
        let handle = unsafe {
            match address {
                IpAddr::V4(_) => IcmpCreateFile(),
                IpAddr::V6(_) => Icmp6CreateFile(),
            }
        };
        if handle == INVALID_HANDLE_VALUE {
            let error = io::Error::last_os_error();
            return Err(Error::new(
                ErrorCode::Network,
                format!("the ICMP helper cannot be opened: {error}"),
            )
            .with_problem("icmp"));
        }
        Ok(Self { handle, address })
    }
}

impl Drop for IcmpEcho {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by `IcmpCreateFile` or
        // `Icmp6CreateFile` and is closed once.
        unsafe { IcmpCloseHandle(self.handle) };
    }
}

impl Echo for IcmpEcho {
    fn echo(
        &mut self,
        _sequence: u16,
        timeout: Duration,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Option<Duration>> {
        check()?;
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let mut buffer = [0_u64; REPLY_WORDS];
        let size = u32::try_from(std::mem::size_of_val(&buffer)).unwrap_or(u32::MAX);
        let payload_size = u16::try_from(PAYLOAD.len()).unwrap_or(u16::MAX);
        let replies = match self.address {
            IpAddr::V4(ip) => {
                // SAFETY: the handle is open; the payload and the reply buffer
                // are valid for the sizes passed; no options are given.
                unsafe {
                    IcmpSendEcho(
                        self.handle,
                        u32::from_ne_bytes(ip.octets()),
                        PAYLOAD.as_ptr().cast(),
                        payload_size,
                        std::ptr::null(),
                        buffer.as_mut_ptr().cast(),
                        size,
                        timeout_ms,
                    )
                }
            }
            IpAddr::V6(ip) => {
                let source = SOCKADDR_IN6 {
                    sin6_family: AF_INET6,
                    ..SOCKADDR_IN6::default()
                };
                let mut destination = SOCKADDR_IN6 {
                    sin6_family: AF_INET6,
                    ..SOCKADDR_IN6::default()
                };
                destination.sin6_addr.u.Byte = ip.octets();
                // SAFETY: the handle is open; without an event and a callback
                // the call is synchronous; the addresses, the payload and the
                // reply buffer are valid for the sizes passed.
                unsafe {
                    Icmp6SendEcho2(
                        self.handle,
                        std::ptr::null_mut(),
                        None,
                        std::ptr::null(),
                        &raw const source,
                        &raw const destination,
                        PAYLOAD.as_ptr().cast(),
                        payload_size,
                        std::ptr::null(),
                        buffer.as_mut_ptr().cast(),
                        size,
                        timeout_ms,
                    )
                }
            }
        };
        if replies == 0 {
            return Ok(None);
        }
        let (status, rtt) = match self.address {
            IpAddr::V4(_) => {
                // SAFETY: a successful call wrote an `ICMP_ECHO_REPLY` at the
                // start of the 8-byte aligned buffer.
                let reply = unsafe { &*buffer.as_ptr().cast::<ICMP_ECHO_REPLY>() };
                (reply.Status, reply.RoundTripTime)
            }
            IpAddr::V6(_) => {
                // SAFETY: a successful call wrote an `ICMPV6_ECHO_REPLY` at
                // the start of the 8-byte aligned buffer.
                let reply = unsafe { &*buffer.as_ptr().cast::<ICMPV6_ECHO_REPLY_LH>() };
                (reply.Status, reply.RoundTripTime)
            }
        };
        Ok((status == IP_SUCCESS).then(|| Duration::from_millis(u64::from(rtt))))
    }
}

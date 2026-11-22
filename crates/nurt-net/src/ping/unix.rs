// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Echo requests over ICMP datagram sockets on Linux and macOS.
//!
//! An ICMP datagram socket needs no privilege. On Linux the kernel replaces
//! the identifier of a request with the local port of the socket, so replies
//! are matched by sequence number and by a random token in the payload. On
//! macOS a received IPv4 datagram starts with its IP header, which is
//! skipped.

// The module creates the socket with `socket(2)` and takes ownership of the
// descriptor; every unsafe block states its invariant.
#![allow(unsafe_code)]

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::os::fd::{FromRawFd, OwnedFd};
use std::time::{Duration, Instant, SystemTime};

use nurt_contracts::error::{Error, ErrorCode, Result};

use super::{Echo, PAYLOAD_LEN, POLL_SLICE, checksum};
use crate::interfaces::SysRoot;

/// `ICMP_ECHO` (RFC 792).
const ICMP_ECHO_REQUEST: u8 = 8;
/// `ICMP_ECHOREPLY` (RFC 792).
const ICMP_ECHO_REPLY: u8 = 0;
/// `ICMP6_ECHO_REQUEST` (RFC 4443).
const ICMPV6_ECHO_REQUEST: u8 = 128;
/// `ICMP6_ECHO_REPLY` (RFC 4443).
const ICMPV6_ECHO_REPLY: u8 = 129;
/// Length of the echo header: type, code, checksum, identifier, sequence.
const HEADER_LEN: usize = 8;
/// Receive buffer, large enough for an IPv4 header with options and a reply.
const REPLY_BUFFER: usize = 1024;
/// Version field of an IPv4 header in the high nibble of its first byte.
const IPV4_VERSION: u8 = 4;
/// Mask of the header length (in 32-bit words) in the first byte.
const IHL_MASK: u8 = 0x0f;
/// Bytes per 32-bit word of the header length.
const IHL_UNIT: usize = 4;
/// Kernel setting that lists the groups allowed to open ICMP datagram
/// sockets on Linux.
pub(crate) const PING_GROUP_RANGE: &str = "net.ipv4.ping_group_range";
/// File of [`PING_GROUP_RANGE`].
const PING_GROUP_RANGE_FILE: &str = "/proc/sys/net/ipv4/ping_group_range";
/// Range that allows every group.
const ALL_GROUPS: &str = "0 2147483647";

/// ICMP datagram socket connected to nothing; requests are sent to one
/// address.
pub(super) struct DatagramEcho {
    socket: UdpSocket,
    address: IpAddr,
    destination: SocketAddr,
    token: [u8; PAYLOAD_LEN],
}

impl DatagramEcho {
    /// Opens an ICMP datagram socket for the family of `address`.
    pub(super) fn open(address: IpAddr, root: &SysRoot) -> Result<Self> {
        let (domain, protocol) = match address {
            IpAddr::V4(_) => (libc::AF_INET, libc::IPPROTO_ICMP),
            IpAddr::V6(_) => (libc::AF_INET6, libc::IPPROTO_ICMPV6),
        };
        #[cfg(target_os = "linux")]
        let kind = libc::SOCK_DGRAM | libc::SOCK_CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let kind = libc::SOCK_DGRAM;
        // SAFETY: `socket` takes no pointers; a negative result is an error.
        let descriptor = unsafe { libc::socket(domain, kind, protocol) };
        if descriptor < 0 {
            return Err(open_error(&io::Error::last_os_error(), root));
        }
        // SAFETY: `descriptor` is a new, valid descriptor owned by nobody
        // else; `OwnedFd` closes it once.
        let owned = unsafe { OwnedFd::from_raw_fd(descriptor) };
        #[cfg(not(target_os = "linux"))]
        set_close_on_exec(&owned);
        Ok(Self {
            socket: UdpSocket::from(owned),
            address,
            destination: SocketAddr::new(address, 0),
            token: token(),
        })
    }

    fn request(&self, sequence: u16) -> Vec<u8> {
        let request_type = match self.address {
            IpAddr::V4(_) => ICMP_ECHO_REQUEST,
            IpAddr::V6(_) => ICMPV6_ECHO_REQUEST,
        };
        let mut packet = vec![request_type, 0, 0, 0];
        packet.extend_from_slice(&self.token[..2]);
        packet.extend_from_slice(&sequence.to_be_bytes());
        packet.extend_from_slice(&self.token);
        if self.address.is_ipv4() {
            let sum = checksum(&packet);
            packet[2..4].copy_from_slice(&sum.to_be_bytes());
        }
        packet
    }

    fn is_reply(&self, datagram: &[u8], sequence: u16) -> bool {
        let (reply_type, body) = match self.address {
            IpAddr::V4(_) => (ICMP_ECHO_REPLY, strip_ipv4_header(datagram)),
            IpAddr::V6(_) => (ICMPV6_ECHO_REPLY, Some(datagram)),
        };
        let Some(body) = body else {
            return false;
        };
        body.len() >= HEADER_LEN + PAYLOAD_LEN
            && body[0] == reply_type
            && body[6..8] == sequence.to_be_bytes()
            && body[HEADER_LEN..HEADER_LEN + PAYLOAD_LEN] == self.token
    }

    fn network_error(&self, error: &io::Error) -> Error {
        Error::new(
            ErrorCode::Network,
            format!("an echo request to {} failed: {error}", self.address),
        )
        .with_problem("icmp")
        .with_detail("address", serde_json::Value::from(self.address.to_string()))
    }
}

impl Echo for DatagramEcho {
    fn echo(
        &mut self,
        sequence: u16,
        timeout: Duration,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Option<Duration>> {
        let packet = self.request(sequence);
        let started = Instant::now();
        match self.socket.send_to(&packet, self.destination) {
            Ok(_) => {}
            Err(error) if is_unreachable(&error) => return Ok(None),
            Err(error) => return Err(self.network_error(&error)),
        }
        let mut buffer = [0_u8; REPLY_BUFFER];
        loop {
            check()?;
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(None);
            }
            self.socket
                .set_read_timeout(Some(remaining.min(POLL_SLICE)))
                .map_err(|error| self.network_error(&error))?;
            match self.socket.recv_from(&mut buffer) {
                Ok((length, _)) => {
                    let datagram = buffer.get(..length).unwrap_or_default();
                    if self.is_reply(datagram, sequence) {
                        return Ok(Some(started.elapsed()));
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) || is_unreachable(&error) => {}
                Err(error) => return Err(self.network_error(&error)),
            }
        }
    }
}

fn strip_ipv4_header(datagram: &[u8]) -> Option<&[u8]> {
    let first = *datagram.first()?;
    if first >> 4 != IPV4_VERSION {
        return Some(datagram);
    }
    datagram.get(usize::from(first & IHL_MASK) * IHL_UNIT..)
}

fn is_unreachable(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EHOSTUNREACH | libc::ENETUNREACH | libc::ECONNREFUSED)
    )
}

fn token() -> [u8; PAYLOAD_LEN] {
    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    hasher.write_u128(nanos);
    hasher.write_u32(std::process::id());
    hasher.finish().to_ne_bytes()
}

#[cfg(not(target_os = "linux"))]
fn set_close_on_exec(descriptor: &OwnedFd) {
    use std::os::fd::AsRawFd;
    // SAFETY: the descriptor is open for the lifetime of `descriptor`;
    // `F_SETFD` takes an integer argument.
    unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
}

fn open_error(error: &io::Error, root: &SysRoot) -> Error {
    match error.raw_os_error() {
        Some(libc::EACCES | libc::EPERM) => privilege_error(root),
        _ => Error::new(
            ErrorCode::Network,
            format!("an ICMP socket cannot be opened: {error}"),
        )
        .with_problem("icmp")
        .with_detail("os_error", serde_json::Value::from(error.raw_os_error())),
    }
}

/// Error for an ICMP socket refused to the account. On Linux it names the
/// setting `net.ipv4.ping_group_range` with its current value read below
/// `root`, and offers a fix that allows every group.
pub(crate) fn privilege_error(root: &SysRoot) -> Error {
    if cfg!(target_os = "linux") {
        let range = std::fs::read_to_string(root.path(PING_GROUP_RANGE_FILE))
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .ok();
        let current = range
            .as_deref()
            .map_or_else(String::new, |range| format!(" (current range: {range})"));
        let mut error = Error::new(
            ErrorCode::Privilege,
            format!(
                "ICMP echo sockets are not allowed for the groups of this account{current}; add a group of the account to {PING_GROUP_RANGE}"
            ),
        )
        .with_fix(format!("write \"{PING_GROUP_RANGE_FILE}\" \"{ALL_GROUPS}\""))
        .with_detail("required", serde_json::Value::from(PING_GROUP_RANGE))
        .with_detail("setting", serde_json::Value::from(PING_GROUP_RANGE));
        if let Some(range) = range {
            error = error.with_detail("range", serde_json::Value::from(range));
        }
        return error;
    }
    Error::new(
        ErrorCode::Privilege,
        "ICMP echo sockets are not allowed for this account",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo(address: &str) -> DatagramEcho {
        let address: IpAddr = address.parse().unwrap();
        DatagramEcho {
            socket: UdpSocket::bind("127.0.0.1:0").unwrap(),
            address,
            destination: SocketAddr::new(address, 0),
            token: *b"nurttest",
        }
    }

    /// An IPv4 echo socket whose requests go to a UDP socket on the loopback
    /// interface, which plays the remote host.
    fn echo_with_peer() -> (DatagramEcho, UdpSocket) {
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut echo = echo("127.0.0.1");
        echo.destination = peer.local_addr().unwrap();
        (echo, peer)
    }

    fn no_check() -> Result<()> {
        Ok(())
    }

    #[test]
    fn echo_returns_round_trip_of_matching_reply() {
        let (mut echo, peer) = echo_with_peer();
        let answer = std::thread::spawn(move || {
            let mut buffer = [0_u8; REPLY_BUFFER];
            let (length, from) = peer.recv_from(&mut buffer).unwrap();
            let mut stray = buffer[..length].to_vec();
            stray[6] ^= 0xff;
            peer.send_to(&stray, from).unwrap();
            let mut packet = buffer[..length].to_vec();
            packet[0] = ICMP_ECHO_REPLY;
            peer.send_to(&packet, from).unwrap();
        });
        let rtt = echo.echo(9, Duration::from_secs(5), &no_check).unwrap();
        answer.join().unwrap();
        assert!(rtt.is_some_and(|rtt| rtt < Duration::from_secs(5)));
    }

    #[test]
    fn echo_without_reply_ends_after_timeout() {
        let (mut echo, _peer) = echo_with_peer();
        let started = Instant::now();
        let rtt = echo.echo(1, Duration::from_millis(150), &no_check).unwrap();
        assert_eq!(rtt, None);
        assert!(started.elapsed() >= Duration::from_millis(150));
    }

    #[test]
    fn echo_wait_ends_with_error_of_check() {
        let (mut echo, _peer) = echo_with_peer();
        let cancelled = || Err(Error::new(ErrorCode::Cancelled, "cancelled"));
        let error = echo
            .echo(1, Duration::from_secs(5), &cancelled)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
    }

    #[test]
    fn unreachable_errors_are_recognised() {
        assert!(is_unreachable(&io::Error::from_raw_os_error(
            libc::EHOSTUNREACH
        )));
        assert!(!is_unreachable(&io::Error::from_raw_os_error(libc::EINVAL)));
        let echo = echo("192.0.2.1");
        let error = echo.network_error(&io::Error::from_raw_os_error(libc::EINVAL));
        assert_eq!(error.problem.as_deref(), Some("icmp"));
        assert_eq!(error.details["address"], "192.0.2.1");
    }

    #[test]
    fn tokens_differ_between_sockets() {
        assert_ne!(token(), token());
    }

    fn reply(echo: &DatagramEcho, reply_type: u8, sequence: u16) -> Vec<u8> {
        let mut packet = echo.request(sequence);
        packet[0] = reply_type;
        packet
    }

    #[test]
    fn ipv4_request_carries_valid_checksum() {
        let echo = echo("192.0.2.1");
        let packet = echo.request(7);
        assert_eq!(packet[0], ICMP_ECHO_REQUEST);
        assert_eq!(&packet[6..8], &[0, 7]);
        assert_eq!(checksum(&packet), 0);
    }

    #[test]
    fn replies_match_sequence_and_token() {
        let echo = echo("192.0.2.1");
        assert!(echo.is_reply(&reply(&echo, ICMP_ECHO_REPLY, 3), 3));
        assert!(!echo.is_reply(&reply(&echo, ICMP_ECHO_REPLY, 3), 4));
        assert!(!echo.is_reply(&reply(&echo, ICMP_ECHO_REQUEST, 3), 3));
        let mut foreign = reply(&echo, ICMP_ECHO_REPLY, 3);
        foreign[HEADER_LEN] ^= 0xff;
        assert!(!echo.is_reply(&foreign, 3));
    }

    #[test]
    fn ipv4_header_before_reply_is_skipped() {
        let echo = echo("192.0.2.1");
        let mut datagram = vec![0x45];
        datagram.extend_from_slice(&[0; 19]);
        datagram.extend_from_slice(&reply(&echo, ICMP_ECHO_REPLY, 1));
        assert!(echo.is_reply(&datagram, 1));
    }

    #[test]
    fn ipv6_replies_use_icmpv6_type() {
        let echo = echo("::1");
        assert_eq!(echo.request(1)[0], ICMPV6_ECHO_REQUEST);
        assert!(echo.is_reply(&reply(&echo, ICMPV6_ECHO_REPLY, 1), 1));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ping_privilege_error_fix() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("proc/sys/net/ipv4");
        std::fs::create_dir_all(&file).unwrap();
        std::fs::write(file.join("ping_group_range"), "1\t0\n").unwrap();
        let error = open_error(
            &io::Error::from_raw_os_error(libc::EACCES),
            &SysRoot::new(dir.path()),
        );
        assert_eq!(error.code(), ErrorCode::Privilege);
        assert_eq!(
            error.fix,
            vec![format!(
                "write \"{PING_GROUP_RANGE_FILE}\" \"{ALL_GROUPS}\""
            )]
        );
        assert_eq!(error.details["range"], "1 0");
        assert_eq!(error.details["setting"], PING_GROUP_RANGE);
        assert!(error.message.contains("current range: 1 0"));
    }

    #[test]
    fn other_socket_errors_are_network_errors() {
        let error = open_error(
            &io::Error::from_raw_os_error(libc::EAFNOSUPPORT),
            &SysRoot::host(),
        );
        assert_eq!(error.code(), ErrorCode::Network);
        assert_eq!(error.problem.as_deref(), Some("icmp"));
    }
}

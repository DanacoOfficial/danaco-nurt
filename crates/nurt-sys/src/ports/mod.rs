// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::ports
// Responsibility:   Network ports and the processes that hold them.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Network ports and the processes that hold them.
//!
//! The `ports` command lists the TCP and UDP sockets of the host as `port`
//! records. With `state: listen` it returns listening TCP sockets and UDP
//! sockets without a remote address; with `state: all` it returns every
//! socket. UDP sockets have the state `none`.
//!
//! | System | Socket tables | Owning process |
//! |---|---|---|
//! | Linux | `/proc/net/{tcp,tcp6,udp,udp6}` | socket inode in `/proc/<pid>/fd` |
//! | macOS | `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` per process | the process listed |
//! | Windows | `GetExtendedTcpTable`, `GetExtendedUdpTable` | `dwOwningPid` |
//!
//! Sockets whose owning process the account may not read keep an empty
//! `pid`; one `source_unavailable` note of the report gives their number. When the
//! socket tables themselves are not readable, the command fails with
//! `E_PRIVILEGE`.

#[cfg(any(target_os = "macos", test))]
mod libproc;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", all(test, unix)))]
mod procnet;
#[cfg(test)]
mod tests;
#[cfg(any(windows, test))]
mod win32;
#[cfg(windows)]
mod windows;

use std::net::SocketAddr;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{AddressFamily, Port, PortState, Protocol};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, Value};

/// Name of the command in the catalog.
pub const COMMAND: &str = "ports";

const PARAM_STATE: &str = "state";
const PARAM_PROTOCOL: &str = "protocol";

/// Values of the `state` parameter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StateFilter {
    /// Listening TCP sockets and UDP sockets without a remote address.
    #[default]
    Listen,
    /// Every socket.
    All,
}

impl StateFilter {
    const VALUES: [Self; 2] = [Self::Listen, Self::All];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Listen => "listen",
            Self::All => "all",
        }
    }

    fn admits(self, socket: &SocketEntry) -> bool {
        match self {
            Self::All => true,
            Self::Listen => match socket.protocol {
                Protocol::Udp => socket.remote.is_none(),
                _ => socket.state == PortState::Listen,
            },
        }
    }
}

/// Values of the `protocol` parameter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ProtocolFilter {
    /// TCP only.
    Tcp,
    /// UDP only.
    Udp,
    /// TCP and UDP.
    #[default]
    Any,
}

impl ProtocolFilter {
    const VALUES: [Self; 3] = [Self::Tcp, Self::Udp, Self::Any];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Any => "any",
        }
    }

    /// Returns `true` when sockets of `protocol` are listed.
    pub(crate) fn includes(self, protocol: Protocol) -> bool {
        match self {
            Self::Any => true,
            Self::Tcp => protocol == Protocol::Tcp,
            Self::Udp => protocol == Protocol::Udp,
        }
    }
}

/// One socket read from the operating system.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SocketEntry {
    pub(crate) protocol: Protocol,
    pub(crate) local: SocketAddr,
    /// Remote endpoint; `None` for a socket without a peer.
    pub(crate) remote: Option<SocketAddr>,
    pub(crate) state: PortState,
    pub(crate) pid: Option<u64>,
    pub(crate) process: Option<String>,
}

/// Remote endpoint of a socket; the unspecified address with port 0 means
/// that the socket has no peer.
pub(crate) fn remote_endpoint(address: SocketAddr) -> Option<SocketAddr> {
    (!(address.ip().is_unspecified() && address.port() == 0)).then_some(address)
}

/// Sockets of the system.
#[derive(Debug, Default)]
pub(crate) struct SocketTable {
    pub(crate) entries: Vec<SocketEntry>,
    /// Sockets whose owning process the account may not read.
    pub(crate) unattributed: u64,
    /// Processes whose sockets the account may not list.
    pub(crate) hidden_processes: u64,
}

/// Source of socket tables: the operating system or test data.
pub(crate) trait SocketSource: Send + Sync {
    /// Returns the sockets of every protocol that the filter admits.
    ///
    /// # Errors
    ///
    /// Fails with `E_PRIVILEGE` when no socket table is readable by the
    /// account, and with `E_IO` when a table cannot be read.
    fn sockets(&self, protocols: ProtocolFilter) -> Result<SocketTable>;
}

/// Implementation of the `ports` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::ports::Ports;
///
/// assert_eq!(Ports::new().descriptor().command, "ports");
/// ```
#[derive(Clone)]
pub struct Ports {
    source: Arc<dyn SocketSource>,
}

impl Ports {
    /// The command reading the sockets of the running system.
    #[must_use]
    pub fn new() -> Self {
        Self {
            source: system_source(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_source(source: Arc<dyn SocketSource>) -> Self {
        Self { source }
    }
}

impl Default for Ports {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Ports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ports").finish_non_exhaustive()
    }
}

#[cfg(target_os = "linux")]
fn system_source() -> Arc<dyn SocketSource> {
    Arc::new(procnet::ProcNet::live())
}

#[cfg(target_os = "macos")]
fn system_source() -> Arc<dyn SocketSource> {
    Arc::new(macos::LibprocSockets)
}

#[cfg(windows)]
fn system_source() -> Arc<dyn SocketSource> {
    Arc::new(windows::Win32Sockets)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn system_source() -> Arc<dyn SocketSource> {
    Arc::new(Unsupported)
}

/// Source of systems without a socket reader.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
struct Unsupported;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
impl SocketSource for Unsupported {
    fn sockets(&self, _protocols: ProtocolFilter) -> Result<SocketTable> {
        Err(Error::new(
            ErrorCode::OsOnly,
            "`ports` is available on Linux, macOS and Windows",
        ))
    }
}

/// Text of an enumeration argument.
fn enum_arg<'a>(args: &'a BoundArgs, name: &str) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            format!("`{COMMAND}` expects an enumeration value for `{name}`"),
        )),
    }
}

fn invalid(name: &str, value: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{name}` must be one of {expected}, got `{value}`"),
    )
}

fn options(args: &BoundArgs) -> Result<(StateFilter, ProtocolFilter)> {
    let state = match enum_arg(args, PARAM_STATE)? {
        None => StateFilter::default(),
        Some(text) => StateFilter::VALUES
            .into_iter()
            .find(|value| value.as_str() == text)
            .ok_or_else(|| invalid(PARAM_STATE, text, "`listen`, `all`"))?,
    };
    let protocol = match enum_arg(args, PARAM_PROTOCOL)? {
        None => ProtocolFilter::default(),
        Some(text) => ProtocolFilter::VALUES
            .into_iter()
            .find(|value| value.as_str() == text)
            .ok_or_else(|| invalid(PARAM_PROTOCOL, text, "`tcp`, `udp`, `any`"))?,
    };
    Ok((state, protocol))
}

impl StageImpl for Ports {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        options(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Ports {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let (state, protocol) = options(args)?;
        ctx.checkpoint()?;
        let table = self.source.sockets(protocol)?;
        let mut entries: Vec<SocketEntry> = table
            .entries
            .into_iter()
            .filter(|socket| protocol.includes(socket.protocol) && state.admits(socket))
            .collect();
        entries.sort_by_key(|socket| {
            (
                socket.protocol.as_str(),
                socket.local.port(),
                socket.local.ip(),
                socket.pid,
            )
        });
        if table.unattributed > 0 {
            let mut note = NotableEntry::new(Notable::SourceUnavailable);
            note.count = Some(table.unattributed);
            note.detail = Some(format!(
                "the owning process of {} sockets is not readable by this account",
                table.unattributed
            ));
            ctx.note(note);
        }
        if table.hidden_processes > 0 {
            let mut note = NotableEntry::new(Notable::SourceUnavailable);
            note.count = Some(table.hidden_processes);
            note.detail = Some(format!(
                "the sockets of {} processes of other accounts are not listed",
                table.hidden_processes
            ));
            ctx.note(note);
        }
        let records: Vec<Record> = entries
            .into_iter()
            .map(|socket| Record::from(port_record(socket)))
            .collect();
        Ok(StageOutput::records(
            RecordStream::from_records(records).checked(ctx),
        ))
    }
}

/// Converts a socket into a `port` record.
fn port_record(socket: SocketEntry) -> Port {
    let family = if socket.local.is_ipv4() {
        AddressFamily::Ipv4
    } else {
        AddressFamily::Ipv6
    };
    Port {
        protocol: socket.protocol,
        local: socket.local.to_string(),
        address: socket.local.ip().to_string(),
        port: u64::from(socket.local.port()),
        state: if socket.protocol == Protocol::Udp {
            PortState::None
        } else {
            socket.state
        },
        remote: socket.remote.map(|remote| remote.to_string()),
        remote_address: socket.remote.map(|remote| remote.ip().to_string()),
        remote_port: socket.remote.map(|remote| u64::from(remote.port())),
        pid: socket.pid,
        process: socket.process,
        family: Some(family),
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

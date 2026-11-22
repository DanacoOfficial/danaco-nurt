// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Network interfaces.
//!
//! `network.interfaces` lists the interfaces of the host with their
//! addresses, hardware address, state, maximum transmission unit, kind and
//! default gateway. Linux combines `getifaddrs` with the files of
//! `/sys/class/net` and the routing tables in `/proc/net`; macOS combines
//! `getifaddrs` with the routing table read through `sysctl`; Windows uses
//! `GetAdaptersAddresses`.
//!
//! System files are read below a [`SysRoot`], so that tests read synthetic
//! trees instead of the files of the host.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::NetworkInterface;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::from_sysfs;

/// Name of the command in the catalog.
pub const COMMAND: &str = "network.interfaces";

/// Root directory below which system files are read.
///
/// The host root is `/`. Tests point the root at a temporary directory that
/// holds a synthetic tree, for example `sys/class/net/eth0/mtu`.
///
/// ```
/// use std::path::Path;
/// use nurt_net::interfaces::SysRoot;
///
/// let root = SysRoot::new("/tmp/fixture");
/// assert_eq!(root.path("/proc/net/route"), Path::new("/tmp/fixture/proc/net/route"));
/// assert_eq!(SysRoot::host().path("/etc/resolv.conf"), Path::new("/etc/resolv.conf"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysRoot(PathBuf);

impl SysRoot {
    /// The root of the host file system.
    #[must_use]
    pub fn host() -> Self {
        Self(PathBuf::from("/"))
    }

    /// A root at `dir`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self(dir.into())
    }

    /// Location of the absolute system path `system_path` below the root.
    #[must_use]
    pub fn path(&self, system_path: &str) -> PathBuf {
        self.0.join(system_path.trim_start_matches('/'))
    }

    /// Directory of the root.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.0
    }
}

impl Default for SysRoot {
    fn default() -> Self {
        Self::host()
    }
}

/// Address assigned to an interface, as reported by the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceAddress {
    /// Interface name; an alias such as `eth0:1` belongs to `eth0`.
    pub interface: String,
    /// Address.
    pub address: IpAddr,
    /// Length of the network prefix in bits.
    pub prefix: u8,
}

impl InterfaceAddress {
    /// Text form `address/prefix`, for example `192.0.2.10/24`.
    ///
    /// ```
    /// use nurt_net::interfaces::InterfaceAddress;
    ///
    /// let address = InterfaceAddress {
    ///     interface: "eth0".to_owned(),
    ///     address: "192.0.2.10".parse().unwrap(),
    ///     prefix: 24,
    /// };
    /// assert_eq!(address.to_text(), "192.0.2.10/24");
    /// ```
    #[must_use]
    pub fn to_text(&self) -> String {
        format!("{}/{}", self.address, self.prefix)
    }
}

/// Number of leading one bits of a network mask, for example 20 for
/// `255.255.240.0`.
#[cfg(any(unix, test))]
pub(crate) fn prefix_length(mask: &[u8]) -> u8 {
    let mut total: u32 = 0;
    for byte in mask {
        total += byte.leading_ones();
        if *byte != u8::MAX {
            break;
        }
    }
    u8::try_from(total).unwrap_or(u8::MAX)
}

/// Hardware address in the form `aa:bb:cc:dd:ee:ff`; `None` when empty or
/// all zero.
#[cfg(any(target_os = "macos", windows, test))]
pub(crate) fn format_mac(bytes: &[u8]) -> Option<String> {
    if bytes.iter().all(|byte| *byte == 0) {
        return None;
    }
    let parts: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(parts.join(":"))
}

/// Error of a system call that lists interfaces.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn system_error(error: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("network interfaces cannot be listed: {error}"),
    )
    .with_detail("os_error", serde_json::Value::from(error.raw_os_error()))
}

/// Returns the interfaces of the host.
///
/// # Errors
///
/// The function fails with `E_IO` when the system refuses the list of
/// interfaces or a system file below `root` cannot be read, and with
/// `E_OS_ONLY` on systems other than Linux, macOS and Windows.
pub fn list(root: &SysRoot) -> Result<Vec<NetworkInterface>> {
    platform_list(root)
}

#[cfg(target_os = "linux")]
fn platform_list(root: &SysRoot) -> Result<Vec<NetworkInterface>> {
    let addresses = linux::addresses().map_err(|error| system_error(&error))?;
    from_sysfs(root, &addresses)
}

#[cfg(target_os = "macos")]
fn platform_list(_root: &SysRoot) -> Result<Vec<NetworkInterface>> {
    macos::list().map_err(|error| system_error(&error))
}

#[cfg(windows)]
fn platform_list(_root: &SysRoot) -> Result<Vec<NetworkInterface>> {
    windows::list().map_err(|error| system_error(&error))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform_list(_root: &SysRoot) -> Result<Vec<NetworkInterface>> {
    Err(Error::new(
        ErrorCode::OsOnly,
        "network.interfaces is available on Linux, macOS and Windows",
    ))
}

/// Implementation of the `network.interfaces` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_net::interfaces::Interfaces;
///
/// assert_eq!(Interfaces::new().descriptor().command, "network.interfaces");
/// ```
#[derive(Debug, Clone, Default)]
pub struct Interfaces {
    root: SysRoot,
}

impl Interfaces {
    /// The command reading the files of the host.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The command reading system files below `root`.
    #[must_use]
    pub fn with_root(root: SysRoot) -> Self {
        Self { root }
    }
}

impl StageImpl for Interfaces {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Interfaces {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        ctx.checkpoint()?;
        let records: Vec<Record> = list(&self.root)?.into_iter().map(Record::from).collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_length_counts_leading_ones() {
        assert_eq!(prefix_length(&[255, 255, 255, 0]), 24);
        assert_eq!(prefix_length(&[255, 255, 240, 0]), 20);
        assert_eq!(prefix_length(&[0, 0, 0, 0]), 0);
        assert_eq!(prefix_length(&[255; 16]), 128);
    }

    #[test]
    fn format_mac_skips_zero_address() {
        assert_eq!(format_mac(&[0; 6]), None);
        assert_eq!(
            format_mac(&[0x02, 0x42, 0xac, 0x11, 0x00, 0x02]).as_deref(),
            Some("02:42:ac:11:00:02")
        );
    }

    #[test]
    fn sys_root_joins_absolute_paths() {
        let root = SysRoot::new("/srv/root");
        assert_eq!(
            root.path("/sys/class/net"),
            Path::new("/srv/root/sys/class/net")
        );
        assert_eq!(root.dir(), Path::new("/srv/root"));
        assert_eq!(SysRoot::default(), SysRoot::host());
    }
}

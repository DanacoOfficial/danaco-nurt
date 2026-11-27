// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::host
// Responsibility:   Description of the device: the `host` command.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Description of the device: the `host` command.
//!
//! `host` returns one `host` record. Stable facts (operating system version,
//! architecture, host name, account, home and temporary directories,
//! `timezone`, `locale`, processors) come from the [`Host`] of the execution
//! context;
//! values that change while the program runs are read from the system:
//!
//! | Field | Linux | macOS | Windows |
//! |---|---|---|---|
//! | `os_version` | `VERSION_ID` of `/etc/os-release` | product version | version reported by the host |
//! | `kernel` | `uname` release | kernel release | build number |
//! | `memory_total`, `memory_free` | `/proc/meminfo` (`MemTotal`, `MemAvailable`) | host statistics | memory status |
//! | `uptime`, `boot_time` | `/proc/uptime`, `btime` of `/proc/stat` | boot time | tick count |
//! | `virtualization` | container markers, `/sys/class/dmi/id/sys_vendor` | `kern.hv_vmm_present` | not detected |
//! | `uid`, `groups` | user and group identifiers of the process | as Linux | security identifier, local groups |
//! | `service` | heartbeat, `ping`, systemd unit | heartbeat, `ping`, launchd property list | heartbeat, `ping`, service `NurtService` |
//!
//! With `programs`, each name is resolved in `PATH` of the process
//! environment (with `PATHEXT` on Windows). With `versions: true`, the version
//! of each located program is read from the metadata of its file: the version
//! resource of a PE file, the embedded or bundle `Info.plist` of a Mach-O
//! file, or the `.note.package` note of an ELF file. No program is started.
//!
//! [`Host`]: nurt_contracts::env::Host

mod features;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", test))]
pub(crate) mod procfs;
mod service_state;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
mod versions;
#[cfg(windows)]
mod windows;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::env::{HostInfo, Os, PathStyle};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{HostInfo as HostRecord, OsKind, ProgramLocation, ProgramSource};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{self, Instant, Size};

use crate::common::canonical_path;
pub use crate::common::sysroot::SysRoot;

/// Name of the command.
const COMMAND: &str = "host";
/// Parameter with the program names to locate.
const PROGRAMS: &str = "programs";
/// Parameter that requests program versions.
const VERSIONS: &str = "versions";
/// Variables that name the locale, in order of precedence.
const LOCALE_VARIABLES: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

/// Values about the host that change while the program runs, read from the
/// operating system.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct HostFacts {
    /// Version of the operating system when the system reports a more precise
    /// one than the host facts of the context.
    pub(crate) os_version: Option<String>,
    /// Kernel version.
    pub(crate) kernel: Option<String>,
    /// Total memory in bytes.
    pub(crate) memory_total: Option<u64>,
    /// Memory available for new programs in bytes.
    pub(crate) memory_free: Option<u64>,
    /// Time since boot.
    pub(crate) uptime: Option<Duration>,
    /// Boot time in seconds since the Unix epoch.
    pub(crate) boot_time: Option<i64>,
    /// Detected virtualisation or container.
    pub(crate) virtualization: Option<String>,
    /// Identifier of the account of the process.
    pub(crate) uid: Option<String>,
    /// Groups of the account of the process.
    pub(crate) groups: Option<Vec<String>>,
}

/// The `host` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::host::HostStage;
///
/// let stage = HostStage::new();
/// assert_eq!(stage.descriptor().command, "host");
/// ```
#[derive(Debug, Clone, Default)]
pub struct HostStage {
    root: SysRoot,
}

impl HostStage {
    /// The command reading the running system.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The command reading system files under `root`.
    #[must_use]
    pub fn with_root(root: SysRoot) -> Self {
        Self { root }
    }

    fn describe(&self, args: &BoundArgs, ctx: &ExecCtx) -> Result<HostRecord> {
        let names = program_names(args)?;
        let versions = args.bool(VERSIONS)?.unwrap_or(false);
        let info = ctx.host().info();
        let facts = read_facts(&self.root, info);
        let service = service_state::availability(&service_state::Probe::from_ctx(&self.root, ctx));
        let features = features::detect(ctx, &info.home, service);
        let programs = (!names.is_empty()).then(|| locate_programs(&names, versions, ctx));
        Ok(HostRecord {
            os: os_kind(info.os),
            os_version: facts.os_version.unwrap_or_else(|| info.os_version.clone()),
            architecture: normalize_architecture(&info.architecture),
            cpus: u64::from(info.cpus),
            memory_total: Size::from_bytes(facts.memory_total.unwrap_or_default()),
            hostname: info.hostname.clone(),
            user: info.user.clone(),
            home: canonical_path(&info.home),
            temp: canonical_path(&info.temp),
            timezone: info.timezone.clone(),
            path_style: path_style(info.path_style).to_owned(),
            nurt_version: env!("CARGO_PKG_VERSION").to_owned(),
            features,
            kernel: facts.kernel,
            memory_free: facts.memory_free.map(Size::from_bytes),
            uid: facts.uid,
            groups: facts.groups,
            locale: info.locale.clone().or_else(|| locale_from_env(ctx)),
            uptime: facts.uptime.and_then(value::Duration::from_std),
            boot_time: facts.boot_time.and_then(Instant::from_unix_seconds),
            programs,
            virtualization: facts.virtualization,
            service: Some(service),
        })
    }
}

impl StageImpl for HostStage {
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

impl ReadStage for HostStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let record = self.describe(args, ctx)?;
        Ok(StageOutput::records(RecordStream::once(record.into())))
    }
}

#[cfg(target_os = "linux")]
fn read_facts(root: &SysRoot, _info: &HostInfo) -> HostFacts {
    linux::read_facts(root)
}

#[cfg(target_os = "macos")]
fn read_facts(root: &SysRoot, _info: &HostInfo) -> HostFacts {
    macos::read_facts(root)
}

#[cfg(windows)]
fn read_facts(_root: &SysRoot, info: &HostInfo) -> HostFacts {
    windows::read_facts(&info.user)
}

fn program_names(args: &BoundArgs) -> Result<Vec<String>> {
    let Some(value) = args.value(PROGRAMS) else {
        return Ok(Vec::new());
    };
    let items = value.as_list().ok_or_else(list_error)?;
    items
        .iter()
        .map(|item| item.as_str().map(str::to_owned).ok_or_else(list_error))
        .collect()
}

fn list_error() -> Error {
    Error::new(
        ErrorCode::Type,
        "`host` expects a list of program names for `programs`",
    )
}

fn locate_programs(names: &[String], versions: bool, ctx: &ExecCtx) -> Vec<ProgramLocation> {
    let search_path = ctx.host().env_var("PATH").unwrap_or_default();
    let cwd = ctx.host().info().home.clone();
    names
        .iter()
        .map(|name| locate_program(name, &search_path, &cwd, versions))
        .collect()
}

fn locate_program(
    name: &str,
    search_path: &OsString,
    cwd: &Path,
    versions: bool,
) -> ProgramLocation {
    let matches: Vec<PathBuf> = which::which_in_all(name, Some(search_path), cwd)
        .map(Iterator::collect)
        .unwrap_or_default();
    let first = matches.first();
    ProgramLocation {
        name: name.to_owned(),
        found: first.is_some(),
        path: first.map(|path| canonical_path(path)),
        version: first
            .filter(|_| versions)
            .and_then(|path| versions::read_version(path)),
        source: first.map(|_| ProgramSource::Path),
        all_paths: Some(matches.iter().map(|path| canonical_path(path)).collect()),
    }
}

fn locale_from_env(ctx: &ExecCtx) -> Option<String> {
    LOCALE_VARIABLES.iter().find_map(|name| {
        ctx.host()
            .env_var(name)
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.is_empty())
    })
}

fn os_kind(os: Os) -> OsKind {
    match os {
        Os::Macos => OsKind::Macos,
        Os::Windows => OsKind::Windows,
        _ => OsKind::Linux,
    }
}

fn path_style(style: PathStyle) -> &'static str {
    match style {
        PathStyle::Windows => "windows",
        _ => "posix",
    }
}

/// Architecture name in the form used by the records: `x86_64`, `aarch64`,
/// `x86`, `arm`; other names unchanged.
pub(crate) fn normalize_architecture(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "amd64" | "x64" | "x86-64" | "x86_64" => "x86_64".to_owned(),
        "arm64" | "aarch64" => "aarch64".to_owned(),
        "i386" | "i486" | "i586" | "i686" | "x86" => "x86".to_owned(),
        "armv7l" | "armv7" | "arm" => "arm".to_owned(),
        other => other.to_owned(),
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

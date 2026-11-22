// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process table of Linux, read from the `proc` file system.
//!
//! Field positions follow `proc_pid_stat(5)`: the command name is enclosed in
//! parentheses and may itself contain spaces and parentheses, so the fields
//! after it are located from the last closing parenthesis.

use std::collections::HashMap;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::ProcessState;
use nurt_contracts::value::Instant;

use super::model::{ProcessInfo, ProcessReader, ProcessSource, ReadRequest};
use crate::common::SysRoot;

/// Length limit of the kernel command name (`TASK_COMM_LEN` without the
/// terminating zero); longer names are cut to this length.
const COMM_MAX: usize = 15;

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Fields of `/proc/<pid>/stat` used by the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stat {
    pub(crate) comm: String,
    pub(crate) state: char,
    pub(crate) parent_pid: u64,
    pub(crate) user_ticks: u64,
    pub(crate) system_ticks: u64,
    pub(crate) threads: u64,
    pub(crate) start_ticks: u64,
    pub(crate) rss_pages: u64,
}

/// Indexes of the fields after the command name, counted from the state
/// field (field 3 of `proc_pid_stat(5)`) as 0.
mod field {
    pub(super) const PARENT_PID: usize = 1;
    pub(super) const USER_TICKS: usize = 11;
    pub(super) const SYSTEM_TICKS: usize = 12;
    pub(super) const THREADS: usize = 17;
    pub(super) const START_TICKS: usize = 19;
    pub(super) const RSS_PAGES: usize = 21;
}

/// Parses the content of `/proc/<pid>/stat`.
pub(crate) fn parse_stat(text: &str) -> Option<Stat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let comm = text.get(open + 1..close)?.to_owned();
    let rest: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    let number = |index: usize| rest.get(index)?.parse::<u64>().ok();
    let rss = rest.get(field::RSS_PAGES)?.parse::<i64>().ok()?;
    Some(Stat {
        comm,
        state: rest.first()?.chars().next()?,
        parent_pid: number(field::PARENT_PID)?,
        user_ticks: number(field::USER_TICKS)?,
        system_ticks: number(field::SYSTEM_TICKS)?,
        threads: number(field::THREADS)?,
        start_ticks: number(field::START_TICKS)?,
        rss_pages: u64::try_from(rss).unwrap_or(0),
    })
}

/// Maps the state letter of `/proc/<pid>/stat` to the record state; letters
/// without a counterpart map to `other` and are returned as the raw state.
pub(crate) fn map_state(letter: char) -> (ProcessState, Option<String>) {
    match letter {
        'R' => (ProcessState::Running, None),
        'S' | 'D' => (ProcessState::Sleeping, None),
        'T' | 't' => (ProcessState::Stopped, None),
        'Z' => (ProcessState::Zombie, None),
        'I' => (ProcessState::Idle, None),
        other => (ProcessState::Other, Some(other.to_string())),
    }
}

/// Splits `/proc/<pid>/cmdline` into arguments.
pub(crate) fn split_command(bytes: &[u8]) -> Vec<String> {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes
        .split(|byte| *byte == 0)
        .map(|argument| String::from_utf8_lossy(argument).into_owned())
        .collect()
}

/// Display name of a process: the kernel command name, the full program name
/// when the kernel name is a cut prefix of it, and the kernel name in square
/// brackets for kernel threads, which have no arguments.
fn display_name(comm: &str, command: Option<&[String]>) -> String {
    match command {
        Some([]) => format!("[{comm}]"),
        Some([program, ..]) if comm.chars().count() == COMM_MAX => {
            let base = program.rsplit('/').next().unwrap_or(program);
            if base.starts_with(comm) {
                base.to_owned()
            } else {
                comm.to_owned()
            }
        }
        _ => comm.to_owned(),
    }
}

/// Converts clock ticks to a duration.
fn ticks_to_duration(ticks: u64, ticks_per_second: u64) -> Duration {
    let nanos = u128::from(ticks) * u128::from(NANOS_PER_SECOND) / u128::from(ticks_per_second);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// Process source that reads `proc` under a [`SysRoot`].
#[derive(Debug, Clone)]
pub(crate) struct ProcFs {
    root: SysRoot,
    ticks_per_second: u64,
    page_size: u64,
}

impl ProcFs {
    /// The process table of the running system.
    #[cfg(target_os = "linux")]
    pub(crate) fn live() -> Self {
        Self::with_root(SysRoot::live())
    }

    /// The process table under `root`, with the clock rate and the page size
    /// of the running system.
    #[cfg(target_os = "linux")]
    pub(crate) fn with_root(root: SysRoot) -> Self {
        let ticks = rustix::param::clock_ticks_per_second();
        Self {
            root,
            ticks_per_second: ticks.max(1),
            page_size: u64::try_from(rustix::param::page_size()).unwrap_or(u64::MAX),
        }
    }

    /// A process table under `root` with the given clock rate and page size.
    #[cfg(test)]
    pub(crate) fn new(root: SysRoot, ticks_per_second: u64, page_size: u64) -> Self {
        Self {
            root,
            ticks_per_second,
            page_size,
        }
    }
}

impl ProcessSource for ProcFs {
    fn open(&self, request: ReadRequest) -> Result<Box<dyn ProcessReader>> {
        Ok(Box::new(ProcFsReader {
            fs: self.clone(),
            request,
            boot_seconds: None,
            accounts: Accounts::new(self.root.clone()),
        }))
    }
}

struct ProcFsReader {
    fs: ProcFs,
    request: ReadRequest,
    boot_seconds: Option<Option<u64>>,
    accounts: Accounts,
}

/// Value of a symbolic link of `/proc/<pid>`: the target, nothing for a
/// missing link, or a mark that the account may not read it.
enum Link {
    Target(String),
    Missing,
    Denied,
}

fn read_link(path: &Path) -> Link {
    match fs::read_link(path) {
        Ok(target) => Link::Target(target.to_string_lossy().into_owned()),
        Err(error) if error.kind() == ErrorKind::PermissionDenied => Link::Denied,
        Err(_) => Link::Missing,
    }
}

impl ProcFsReader {
    fn process_path(&self, pid: u64, name: &str) -> std::path::PathBuf {
        self.fs.root.path(format!("/proc/{pid}/{name}"))
    }

    fn stat(&self, pid: u64) -> Option<Stat> {
        let text = fs::read_to_string(self.process_path(pid, "stat")).ok()?;
        parse_stat(&text)
    }

    fn boot_seconds(&mut self) -> Option<u64> {
        if let Some(value) = self.boot_seconds {
            return value;
        }
        let value = fs::read_to_string(self.fs.root.path("/proc/stat"))
            .ok()
            .and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("btime "))
                    .and_then(|value| value.trim().parse().ok())
            });
        self.boot_seconds = Some(value);
        value
    }

    fn effective_uid(&self, pid: u64) -> Option<u32> {
        let text = fs::read_to_string(self.process_path(pid, "status")).ok()?;
        let line = text.lines().find_map(|line| line.strip_prefix("Uid:"))?;
        line.split_whitespace().nth(1)?.parse().ok()
    }

    fn started(&mut self, start_ticks: u64) -> Option<Instant> {
        let boot = self.boot_seconds()?;
        let since_boot = ticks_to_duration(start_ticks, self.fs.ticks_per_second);
        let nanos = u128::from(boot) * u128::from(NANOS_PER_SECOND) + since_boot.as_nanos();
        i64::try_from(nanos).ok().map(Instant::from_unix_nanos)
    }
}

impl ProcessReader for ProcFsReader {
    fn pids(&mut self) -> Result<Vec<u64>> {
        let proc_dir = self.fs.root.path("/proc");
        let entries = fs::read_dir(&proc_dir).map_err(proc_error)?;
        let mut pids: Vec<u64> = entries
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect();
        pids.sort_unstable();
        Ok(pids)
    }

    fn read(&mut self, pid: u64) -> Option<ProcessInfo> {
        let stat = self.stat(pid)?;
        let mut restricted = false;
        let command = match fs::read(self.process_path(pid, "cmdline")) {
            Ok(bytes) => Some(split_command(&bytes)),
            Err(error) => {
                restricted |= error.kind() == ErrorKind::PermissionDenied;
                None
            }
        };
        let mut link = |name: &str| match read_link(&self.process_path(pid, name)) {
            Link::Target(target) => Some(target),
            Link::Missing => None,
            Link::Denied => {
                restricted = true;
                None
            }
        };
        let executable = link("exe");
        let cwd = if self.request.cwd { link("cwd") } else { None };
        let uid = self.effective_uid(pid);
        let (state, state_raw) = map_state(stat.state);
        Some(ProcessInfo {
            pid,
            parent_pid: stat.parent_pid,
            name: display_name(&stat.comm, command.as_deref()),
            command: command.unwrap_or_default(),
            user: uid.and_then(|uid| self.accounts.name(uid)),
            uid: uid.map(|uid| uid.to_string()),
            memory: stat.rss_pages.saturating_mul(self.fs.page_size),
            started: self.started(stat.start_ticks),
            state,
            state_raw,
            executable,
            cpu_time: Some(ticks_to_duration(
                stat.user_ticks.saturating_add(stat.system_ticks),
                self.fs.ticks_per_second,
            )),
            threads: Some(stat.threads),
            cwd,
            restricted,
        })
    }

    fn cpu_time(&mut self, pid: u64) -> Option<Duration> {
        let stat = self.stat(pid)?;
        Some(ticks_to_duration(
            stat.user_ticks.saturating_add(stat.system_ticks),
            self.fs.ticks_per_second,
        ))
    }
}

/// Error of listing `/proc`.
fn proc_error(error: io::Error) -> Error {
    match error.kind() {
        ErrorKind::PermissionDenied => Error::new(
            ErrorCode::Privilege,
            "the process table is not readable by this account",
        )
        .with_detail("required", serde_json::Value::from("root")),
        ErrorKind::NotFound => Error::new(
            ErrorCode::Io,
            "the process file system is not mounted; mount `proc` at /proc",
        ),
        _ => Error::new(ErrorCode::Io, format!("cannot list processes: {error}")),
    }
}

/// Account names by user identifier, read from `/etc/passwd` and, on the
/// running system, from the account database of the system library for
/// accounts that the file does not list.
pub(crate) struct Accounts {
    root: SysRoot,
    file: Option<HashMap<u32, String>>,
    looked_up: HashMap<u32, Option<String>>,
}

impl Accounts {
    pub(crate) fn new(root: SysRoot) -> Self {
        Self {
            root,
            file: None,
            looked_up: HashMap::new(),
        }
    }

    /// Name of the account with identifier `uid`, when known.
    pub(crate) fn name(&mut self, uid: u32) -> Option<String> {
        let root = &self.root;
        let file = self.file.get_or_insert_with(|| {
            fs::read_to_string(root.path("/etc/passwd"))
                .map(|text| parse_passwd(&text))
                .unwrap_or_default()
        });
        if let Some(name) = file.get(&uid) {
            return Some(name.clone());
        }
        if !self.root.is_live() {
            return None;
        }
        self.looked_up
            .entry(uid)
            .or_insert_with(|| system_account_name(uid))
            .clone()
    }
}

/// Parses the account names of a file in the format of `passwd(5)`.
pub(crate) fn parse_passwd(text: &str) -> HashMap<u32, String> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let uid = fields.nth(1)?.parse().ok()?;
            Some((uid, name.to_owned()))
        })
        .collect()
}

/// Name of an account from the account database of the system library.
fn system_account_name(uid: u32) -> Option<String> {
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
        .ok()
        .flatten()
        .map(|user| user.name)
}

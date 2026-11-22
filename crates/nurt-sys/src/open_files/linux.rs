// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Holders of files on Linux, from the process directories of `/proc`.
//!
//! For each process the links `fd/*`, `cwd`, `root` and `exe` are read with
//! `readlink`, and the path column of `maps` lists the files mapped into
//! memory. The `process` records of the holders are read from `stat`,
//! `status` and `cmdline` of the same directories.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use nurt_contracts::error::Result;
use nurt_contracts::record::{Process, ProcessState};
use nurt_contracts::value::{self, Instant, Size};

use super::{Access, Holder, Scan, Target};
use crate::accounts::unix;
use crate::host::{SysRoot, canonical_path, procfs, source_error};

/// Links of a process directory that name one path each.
const LINKS: [(Access, &str); 3] = [
    (Access::Cwd, "cwd"),
    (Access::Root, "root"),
    (Access::Exe, "exe"),
];
/// Columns of a `maps` line before the path: address, permissions, offset,
/// device and inode.
const MAPS_FIXED_FIELDS: usize = 5;
/// Fields of `stat` after the command name, counted from zero.
const STAT_PARENT: usize = 1;
/// Number of threads.
const STAT_THREADS: usize = 17;
/// Start time in clock ticks after boot.
const STAT_START: usize = 19;
/// Resident set size in pages.
const STAT_RESIDENT: usize = 21;
/// Nanoseconds in one second.
const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// Searches the process directories under `root` for holders of `target`.
///
/// # Errors
///
/// Fails with `E_PRIVILEGE` or `E_IO` when `/proc` cannot be listed.
pub(crate) fn scan(root: &SysRoot, target: &Target) -> Result<Scan> {
    let entries =
        std::fs::read_dir(root.path("/proc")).map_err(|error| source_error(&error, "/proc"))?;
    let mut scan = Scan::default();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let (holder, denied) = inspect(&entry.path(), pid, target);
        scan.denied += u64::from(denied);
        if holder.is_holding() {
            scan.holders.push(holder);
        }
    }
    scan.holders.sort_by_key(|holder| holder.pid);
    Ok(scan)
}

/// Matching paths of the process directory `dir` and whether any part of it
/// was not readable.
fn inspect(dir: &Path, pid: u32, target: &Target) -> (Holder, bool) {
    let mut holder = Holder::new(pid);
    let mut denied = false;
    for (access, name) in LINKS {
        match std::fs::read_link(dir.join(name)) {
            Ok(link) => holder.check(target, access, &link.to_string_lossy()),
            Err(error) => denied |= is_denied(&error),
        }
    }
    match std::fs::read_dir(dir.join("fd")) {
        Ok(descriptors) => {
            for descriptor in descriptors.flatten() {
                if let Ok(link) = std::fs::read_link(descriptor.path()) {
                    holder.check(target, Access::Fd, &link.to_string_lossy());
                }
            }
        }
        Err(error) => denied |= is_denied(&error),
    }
    match std::fs::read_to_string(dir.join("maps")) {
        Ok(maps) => {
            for path in maps.lines().filter_map(mapped_path) {
                holder.check(target, Access::Mmap, path);
            }
        }
        Err(error) => denied |= is_denied(&error),
    }
    if holder.is_holding() {
        holder.name = std::fs::read_to_string(dir.join("comm"))
            .ok()
            .map(|name| name.trim_end().to_owned());
    }
    (holder, denied)
}

fn is_denied(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
}

/// Path column of one line of `maps`; `None` for anonymous mappings and
/// pseudo paths such as `[heap]`.
pub(crate) fn mapped_path(line: &str) -> Option<&str> {
    let mut rest = line;
    for _ in 0..MAPS_FIXED_FIELDS {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        rest = rest.get(end..)?;
    }
    let path = rest.trim();
    path.starts_with('/').then_some(path)
}

/// `process` records of `holders`; processes that ended meanwhile are left
/// out.
pub(crate) fn processes(root: &SysRoot, holders: &[Holder]) -> Vec<Process> {
    if holders.is_empty() {
        return Vec::new();
    }
    let clock = BootClock::read(root);
    let users: HashMap<u32, String> = unix::users(root)
        .unwrap_or_default()
        .into_iter()
        .map(|user| (user.uid, user.name))
        .collect();
    holders
        .iter()
        .filter_map(|holder| read_process(root, holder.pid, &clock, &users))
        .collect()
}

/// Conversion of process start times and sizes.
#[derive(Debug, Clone, Copy)]
struct BootClock {
    /// Boot time in seconds since the Unix epoch.
    boot_seconds: Option<i64>,
    /// Clock ticks per second of the start times in `stat`.
    ticks_per_second: u64,
    /// Size of a memory page in bytes.
    page_size: u64,
}

impl BootClock {
    fn read(root: &SysRoot) -> Self {
        Self {
            boot_seconds: root
                .read_to_string("/proc/stat")
                .ok()
                .and_then(|text| procfs::parse_btime(&text)),
            ticks_per_second: rustix::param::clock_ticks_per_second().max(1),
            page_size: u64::try_from(rustix::param::page_size()).unwrap_or(u64::MAX),
        }
    }

    /// Instant `ticks` clock ticks after boot.
    fn started(&self, ticks: u64) -> Option<Instant> {
        let boot = i128::from(self.boot_seconds?) * NANOS_PER_SECOND;
        let offset = i128::from(ticks) * NANOS_PER_SECOND / i128::from(self.ticks_per_second);
        i64::try_from(boot + offset)
            .ok()
            .map(Instant::from_unix_nanos)
    }
}

fn read_process(
    root: &SysRoot,
    pid: u32,
    clock: &BootClock,
    users: &HashMap<u32, String>,
) -> Option<Process> {
    let dir = root.path(format!("/proc/{pid}"));
    let stat = std::fs::read_to_string(dir.join("stat")).ok()?;
    let (name, fields) = split_stat(&stat)?;
    let field = |index: usize| fields.get(index).and_then(|text| text.parse::<u64>().ok());
    let uid = std::fs::read_to_string(dir.join("status"))
        .ok()
        .and_then(|text| status_uid(&text));
    let command = std::fs::read(dir.join("cmdline"))
        .map(|bytes| split_command(&bytes))
        .unwrap_or_default();
    Some(Process {
        pid: u64::from(pid),
        parent_pid: field(STAT_PARENT).unwrap_or_default(),
        name: name.to_owned(),
        command,
        user: uid
            .and_then(|uid| users.get(&uid).cloned())
            .or_else(|| uid.map(|uid| uid.to_string()))
            .unwrap_or_default(),
        memory: Size::from_bytes(
            field(STAT_RESIDENT)
                .unwrap_or_default()
                .saturating_mul(clock.page_size),
        ),
        started: field(STAT_START)
            .and_then(|ticks| clock.started(ticks))
            .unwrap_or_default(),
        state: fields
            .first()
            .and_then(|state| state.chars().next())
            .map_or(ProcessState::Sleeping, process_state),
        executable: link(&dir, "exe"),
        uid: uid.map(|uid| uid.to_string()),
        threads: field(STAT_THREADS),
        cwd: link(&dir, "cwd"),
        ..Process::default()
    })
}

/// Command name and the fields that follow it in `stat`; the name is
/// enclosed in the first `(` and the last `)` because it may contain both.
fn split_stat(text: &str) -> Option<(&str, Vec<&str>)> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let name = text.get(open + 1..close)?;
    Some((name, text.get(close + 1..)?.split_whitespace().collect()))
}

/// Real user identifier from the `Uid:` line of `status`.
fn status_uid(text: &str) -> Option<u32> {
    text.lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

/// Arguments of `cmdline`, separated by null bytes.
fn split_command(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|&byte| byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

fn process_state(letter: char) -> ProcessState {
    match letter {
        'R' => ProcessState::Running,
        'T' | 't' => ProcessState::Stopped,
        'Z' | 'X' => ProcessState::Zombie,
        'I' => ProcessState::Idle,
        _ => ProcessState::Sleeping,
    }
}

fn link(dir: &Path, name: &str) -> Option<value::Path> {
    std::fs::read_link(dir.join(name))
        .ok()
        .map(|path| canonical_path(&path))
}

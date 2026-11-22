// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process group measurement through `/proc` and memory limits through a
//! delegated cgroup v2.
//!
//! A cgroup is used only for a memory limit and only when the parent of the
//! cgroup of this process enables the memory controller for its children and
//! accepts processes from this process (`cgroup.subtree_control` lists
//! `memory`, `cgroup.procs` is writable). The child joins the new cgroup
//! before `exec`. Without delegation the memory of the group is measured and
//! the supervisor ends the group at the limit.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use nurt_contracts::error::Result;
use nurt_contracts::record::LimitHit;

use super::io_error;
use crate::iface::{ProcessLimits, ResourceUsage};

/// Mount point of the unified cgroup hierarchy.
const CGROUP_ROOT: &str = "/sys/fs/cgroup";
/// Controller required for the memory limit.
const MEMORY_CONTROLLER: &str = "memory";
/// Time to wait for a cgroup to become empty before it is removed.
const CGROUP_RELEASE_WAIT: Duration = Duration::from_secs(1);
/// Interval of the checks for an empty cgroup.
const CGROUP_RELEASE_POLL: Duration = Duration::from_millis(10);

/// Nanoseconds in one second.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Positions in `/proc/<pid>/stat`, counted from the field after the
/// command name.
const STAT_PGRP: usize = 2;
const STAT_UTIME: usize = 11;
const STAT_STIME: usize = 12;
const STAT_RSS: usize = 21;

/// Suffix of the cgroups created by this process.
static CGROUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Group membership, measurement and kernel limits of one tree.
pub(super) struct Membership {
    cgroup: Option<Cgroup>,
    clock_ticks: u64,
    page_size: u64,
    seen_cpu: Mutex<HashMap<u32, Duration>>,
}

impl Membership {
    /// Prepares measurement and, when delegated, a cgroup with the memory
    /// limit. Falls back to measurement when the cgroup cannot be set up.
    pub(super) fn prepare(limits: &ProcessLimits) -> Self {
        let cgroup = limits.max_memory.and_then(Cgroup::create);
        // SAFETY: `sysconf` has no preconditions.
        let clock_ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        // SAFETY: as above.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Self {
            cgroup,
            clock_ticks: u64::try_from(clock_ticks).unwrap_or(0),
            page_size: u64::try_from(page_size).unwrap_or(0),
            seen_cpu: Mutex::new(HashMap::new()),
        }
    }

    /// The kernel enforces the memory limit.
    pub(super) fn enforces_memory(&self) -> bool {
        self.cgroup.is_some()
    }

    /// Descriptor the child writes `0` to in order to join the cgroup.
    pub(super) fn join_descriptor(&self) -> Option<RawFd> {
        self.cgroup.as_ref().map(|cgroup| cgroup.procs.as_raw_fd())
    }

    /// Ends every process of the cgroup, including processes that left the
    /// process group.
    pub(super) fn kill(&self) {
        if let Some(cgroup) = &self.cgroup {
            cgroup.kill();
        }
    }

    /// Processor time and resident memory of the processes in group `pgid`.
    ///
    /// The processor time of a member that ended is kept at its last value,
    /// so the total does not decrease.
    pub(super) fn sample(&self, pgid: libc::pid_t) -> Result<ResourceUsage> {
        let entries = fs::read_dir("/proc").map_err(|error| io_error("read /proc", &error))?;
        let mut rss_pages: u64 = 0;
        let mut seen = self
            .seen_cpu
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse().ok())
            else {
                continue;
            };
            let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            let Some(member) = parse_stat(&stat) else {
                continue;
            };
            if member.pgrp != pgid {
                continue;
            }
            rss_pages = rss_pages.saturating_add(member.rss_pages);
            let cpu = self.ticks(member.cpu_ticks);
            let entry = seen.entry(pid).or_insert(cpu);
            *entry = (*entry).max(cpu);
        }
        let mut cpu_time = seen
            .values()
            .fold(Duration::ZERO, |sum, cpu| sum.saturating_add(*cpu));
        if let Some(cgroup_cpu) = self.cpu_time() {
            cpu_time = cpu_time.max(cgroup_cpu);
        }
        Ok(ResourceUsage {
            cpu_time,
            max_rss: rss_pages.saturating_mul(self.page_size),
        })
    }

    /// The kernel ended a process of the cgroup at the memory limit.
    pub(super) fn limit_hit(&self) -> Option<LimitHit> {
        self.cgroup
            .as_ref()
            .filter(|cgroup| cgroup.out_of_memory())
            .map(|_| LimitHit::Memory)
    }

    /// Processor time of every process that ran in the cgroup.
    pub(super) fn cpu_time(&self) -> Option<Duration> {
        self.cgroup.as_ref().and_then(Cgroup::cpu_time)
    }

    /// Removes the cgroup once its processes ended.
    pub(super) fn release(&self) {
        if let Some(cgroup) = &self.cgroup {
            cgroup.remove();
        }
    }

    fn ticks(&self, ticks: u64) -> Duration {
        if self.clock_ticks == 0 {
            return Duration::ZERO;
        }
        let seconds = ticks / self.clock_ticks;
        let rest = ticks % self.clock_ticks;
        Duration::from_secs(seconds).saturating_add(Duration::from_nanos(
            rest * NANOS_PER_SECOND / self.clock_ticks,
        ))
    }
}

/// Fields of `/proc/<pid>/stat` used for measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatFields {
    pgrp: libc::pid_t,
    cpu_ticks: u64,
    rss_pages: u64,
}

/// Parses `/proc/<pid>/stat`. The command name is in parentheses and may
/// contain spaces and parentheses, so fields are counted after the last
/// closing parenthesis.
fn parse_stat(stat: &str) -> Option<StatFields> {
    let (_, rest) = stat.rsplit_once(')')?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let number = |index: usize| {
        fields
            .get(index)
            .and_then(|field| field.parse::<u64>().ok())
    };
    Some(StatFields {
        pgrp: fields.get(STAT_PGRP)?.parse().ok()?,
        cpu_ticks: number(STAT_UTIME)?.saturating_add(number(STAT_STIME)?),
        rss_pages: number(STAT_RSS)?,
    })
}

/// Cgroup created for one tree.
struct Cgroup {
    dir: PathBuf,
    procs: File,
}

impl Cgroup {
    fn create(max_memory: u64) -> Option<Self> {
        let parent = delegated_parent()?;
        let name = format!(
            "nurt-run-{}-{}",
            std::process::id(),
            CGROUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let dir = parent.join(name);
        fs::create_dir(&dir).ok()?;
        let cgroup = Self::configure(&dir, max_memory);
        if cgroup.is_none() {
            fs::remove_dir(&dir).ok();
        }
        cgroup
    }

    fn configure(dir: &Path, max_memory: u64) -> Option<Self> {
        fs::write(dir.join("memory.max"), max_memory.to_string()).ok()?;
        // The whole tree ends at the limit, as with the measured limit.
        fs::write(dir.join("memory.oom.group"), "1").ok()?;
        // Without swap accounting the file is absent and the limit still
        // applies to resident memory.
        fs::write(dir.join("memory.swap.max"), "0").ok();
        let procs = OpenOptions::new()
            .write(true)
            .open(dir.join("cgroup.procs"))
            .ok()?;
        Some(Self {
            dir: dir.to_path_buf(),
            procs,
        })
    }

    fn kill(&self) {
        if let Ok(mut file) = OpenOptions::new()
            .write(true)
            .open(self.dir.join("cgroup.kill"))
        {
            file.write_all(b"1").ok();
        }
    }

    fn out_of_memory(&self) -> bool {
        let Ok(events) = fs::read_to_string(self.dir.join("memory.events")) else {
            return false;
        };
        ["oom_kill", "oom_group_kill"]
            .iter()
            .any(|key| keyed_value(&events, key).is_some_and(|count| count > 0))
    }

    fn cpu_time(&self) -> Option<Duration> {
        let stat = fs::read_to_string(self.dir.join("cpu.stat")).ok()?;
        keyed_value(&stat, "usage_usec").map(Duration::from_micros)
    }

    fn remove(&self) {
        let deadline = Instant::now() + CGROUP_RELEASE_WAIT;
        loop {
            let populated = fs::read_to_string(self.dir.join("cgroup.events"))
                .ok()
                .and_then(|events| keyed_value(&events, "populated"))
                .unwrap_or(0);
            if populated == 0 || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(CGROUP_RELEASE_POLL);
        }
        fs::remove_dir(&self.dir).ok();
    }
}

impl Drop for Cgroup {
    /// Removes a cgroup left by a start that failed.
    fn drop(&mut self) {
        fs::remove_dir(&self.dir).ok();
    }
}

/// Parent directory where this process may create cgroups with the memory
/// controller.
fn delegated_parent() -> Option<PathBuf> {
    let membership = fs::read_to_string("/proc/self/cgroup").ok()?;
    let relative = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?;
    let own = Path::new(CGROUP_ROOT).join(relative.trim_start_matches('/'));
    let parent = own.parent()?;
    if !parent.starts_with(CGROUP_ROOT) {
        return None;
    }
    let controllers = fs::read_to_string(parent.join("cgroup.subtree_control")).ok()?;
    if !controllers
        .split_whitespace()
        .any(|controller| controller == MEMORY_CONTROLLER)
    {
        return None;
    }
    rustix::fs::access(parent.join("cgroup.procs"), rustix::fs::Access::WRITE_OK).ok()?;
    rustix::fs::access(parent, rustix::fs::Access::WRITE_OK).ok()?;
    Some(parent.to_path_buf())
}

/// Value of `key` in a cgroup file of `key value` lines.
fn keyed_value(content: &str, key: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let (name, value) = line.split_once(' ')?;
        (name == key).then(|| value.trim().parse().ok()).flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields_are_counted_after_the_command_name() {
        let stat = "4242 (a) b (c)) S 1 4240 4240 0 -1 4194560 100 0 0 0 7 3 0 0 20 0 1 0 \
                    123 4096 25 18446744073709551615";
        assert_eq!(
            parse_stat(stat),
            Some(StatFields {
                pgrp: 4240,
                cpu_ticks: 10,
                rss_pages: 25,
            })
        );
    }

    #[test]
    fn truncated_stat_is_ignored() {
        assert_eq!(parse_stat("4242 (a) S 1 4240"), None);
    }

    #[test]
    fn keyed_values_are_read_by_name() {
        let events = "low 0\nhigh 0\nmax 4\noom 1\noom_kill 1\noom_group_kill 0\n";
        assert_eq!(keyed_value(events, "oom_kill"), Some(1));
        assert_eq!(keyed_value(events, "max"), Some(4));
        assert_eq!(keyed_value(events, "missing"), None);
    }

    #[test]
    fn clock_ticks_convert_to_durations() {
        let membership = Membership {
            cgroup: None,
            clock_ticks: 100,
            page_size: 4096,
            seen_cpu: Mutex::new(HashMap::new()),
        };
        assert_eq!(membership.ticks(250), Duration::from_millis(2500));
    }
}

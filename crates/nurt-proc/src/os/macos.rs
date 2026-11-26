// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process group measurement on macOS.
//!
//! The kernel does not enforce an address space or resident memory limit
//! for a group of processes, so the memory limit is measured: members are
//! listed with `proc_listpids(PROC_PGRP_ONLY)` and measured with
//! `proc_pid_rusage`, and the supervisor ends the group at the limit.
//!
//! A descendant that left the group with `setsid` or `setpgid` is not reached
//! by `killpg`. The tree of the main process is walked with the children of
//! each process (`proc_listpids(PROC_PPID_ONLY)`, the list behind
//! `proc_listchildpids`) and remembered with the start times of the
//! processes at every sample and before the group is ended, so `kill` also
//! signals the descendants that left the group and those whose parent
//! ended, and never a process that received the identifier of an ended one.

use std::collections::HashMap;
use std::os::fd::RawFd;
use std::sync::Mutex;
use std::time::Duration;

use nurt_contracts::error::Result;
use nurt_contracts::record::LimitHit;

use super::descendants::{Remembered, tree_of};
use super::io_error;
use crate::iface::{ProcessLimits, ResourceUsage};

/// `PROC_PGRP_ONLY` of `<libproc.h>`: list the members of a process group.
const PROC_PGRP_ONLY: u32 = 2;
/// `PROC_PPID_ONLY` of `<libproc.h>`: list the children of a process.
const PROC_PPID_ONLY: u32 = 6;
/// Rounds of walking and signalling the tree in `kill`; a round that finds a
/// process new to the tree is followed by another, because that process may
/// have started more.
const KILL_ROUNDS: usize = 8;
/// Initial capacity of the member list; doubled while the list is full.
const MEMBER_CAPACITY: usize = 64;

/// Conversion of Mach absolute time units to nanoseconds.
#[derive(Debug, Clone, Copy)]
struct Timebase {
    numer: u64,
    denom: u64,
}

impl Timebase {
    #[allow(
        deprecated,
        reason = "the libc binding is marked in favour of another crate; the system call itself is current"
    )]
    fn query() -> Self {
        let mut info = libc::mach_timebase_info { numer: 1, denom: 1 };
        // SAFETY: `info` is a valid, writable `mach_timebase_info`.
        let status = unsafe { libc::mach_timebase_info(&raw mut info) };
        if status != 0 || info.denom == 0 {
            return Self { numer: 1, denom: 1 };
        }
        Self {
            numer: u64::from(info.numer),
            denom: u64::from(info.denom),
        }
    }

    fn duration(self, units: u64) -> Duration {
        let nanos = u128::from(units) * u128::from(self.numer) / u128::from(self.denom);
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

/// Group membership and measurement of one tree.
pub(super) struct Membership {
    timebase: Timebase,
    seen_cpu: Mutex<HashMap<libc::pid_t, Duration>>,
    tree: Mutex<Remembered>,
}

impl Membership {
    /// Prepares measurement; no kernel limit is available for a group.
    pub(super) fn prepare(_limits: &ProcessLimits) -> Self {
        Self {
            timebase: Timebase::query(),
            seen_cpu: Mutex::new(HashMap::new()),
            tree: Mutex::new(Remembered::default()),
        }
    }

    /// The kernel does not enforce the memory limit of a group.
    pub(super) fn enforces_memory(&self) -> bool {
        false
    }

    /// No descriptor is passed to the child.
    pub(super) fn join_descriptor(&self) -> Option<RawFd> {
        None
    }

    /// Ends the descendants of `root` that are not in its process group:
    /// the processes found below `root` and the remembered ones that still
    /// run with their start time. The group itself is ended by the caller.
    /// `root` is `None` once the main process was collected; its identifier
    /// may then name another process, so nothing is walked.
    pub(super) fn kill(&self, root: Option<libc::pid_t>) {
        let Some(root) = root else {
            return;
        };
        let mut tree = self
            .tree
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for _ in 0..KILL_ROUNDS {
            let found = tree_of(root, children);
            let added = tree.add(&found, start_time);
            for pid in tree.alive(start_time) {
                if pid != root {
                    // SAFETY: the process was found with the start time it
                    // has now, so the identifier names the remembered process.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
            }
            if added == 0 {
                break;
            }
        }
    }

    /// Processor time and resident memory of the processes in group `pgid`.
    ///
    /// The processor time of a member that ended is kept at its last value,
    /// so the total does not decrease.
    pub(super) fn sample(&self, pgid: libc::pid_t) -> Result<ResourceUsage> {
        let members = list(PROC_PGRP_ONLY, pgid)?;
        self.tree
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .add(&tree_of(pgid, children), start_time);
        let mut resident: u64 = 0;
        let mut seen = self
            .seen_cpu
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for pid in members {
            let Some(info) = rusage(pid) else {
                continue;
            };
            resident = resident.saturating_add(info.ri_resident_size);
            let cpu = self
                .timebase
                .duration(info.ri_user_time.saturating_add(info.ri_system_time));
            let entry = seen.entry(pid).or_insert(cpu);
            *entry = (*entry).max(cpu);
        }
        let cpu_time = seen
            .values()
            .fold(Duration::ZERO, |sum, cpu| sum.saturating_add(*cpu));
        Ok(ResourceUsage {
            cpu_time,
            max_rss: resident,
        })
    }

    /// No kernel limit applies to the group.
    pub(super) fn limit_hit(&self) -> Option<LimitHit> {
        None
    }

    /// The group has no kernel accounting beyond its members.
    pub(super) fn cpu_time(&self) -> Option<Duration> {
        None
    }

    /// Nothing to remove.
    pub(super) fn release(&self) {}
}

/// Children of process `pid`; empty when the list cannot be read.
fn children(pid: libc::pid_t) -> Vec<libc::pid_t> {
    list(PROC_PPID_ONLY, pid).unwrap_or_default()
}

/// Start time of process `pid` in microseconds, or `None` when it ended.
fn start_time(pid: libc::pid_t) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
    // SAFETY: `info` is a writable `proc_bsdinfo` of `size` bytes, the
    // structure of the requested flavor.
    let filled = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if filled != size {
        return None;
    }
    // SAFETY: the call filled the whole structure.
    let info = unsafe { info.assume_init() };
    Some(
        info.pbi_start_tvsec
            .saturating_mul(1_000_000)
            .saturating_add(info.pbi_start_tvusec),
    )
}

/// Processes selected by `kind` (`PROC_PGRP_ONLY` or `PROC_PPID_ONLY`) for
/// the identifier `id`.
fn list(kind: u32, id: libc::pid_t) -> Result<Vec<libc::pid_t>> {
    let entry_size = std::mem::size_of::<libc::pid_t>();
    let mut capacity = MEMBER_CAPACITY;
    loop {
        let mut buffer: Vec<libc::pid_t> = vec![0; capacity];
        let size = libc::c_int::try_from(capacity * entry_size).unwrap_or(libc::c_int::MAX);
        // SAFETY: the buffer holds `size` writable bytes.
        let filled = unsafe {
            libc::proc_listpids(kind, id.cast_unsigned(), buffer.as_mut_ptr().cast(), size)
        };
        let Ok(filled) = usize::try_from(filled) else {
            return Err(io_error(
                "list the processes of the group or the children of the process",
                &std::io::Error::last_os_error(),
            ));
        };
        let count = filled / entry_size;
        if count < capacity {
            buffer.truncate(count);
            buffer.retain(|pid| *pid > 0);
            return Ok(buffer);
        }
        capacity = capacity.saturating_mul(2);
    }
}

fn rusage(pid: libc::pid_t) -> Option<libc::rusage_info_v2> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: `info` is a writable `rusage_info_v2`, the structure of the
    // requested flavor.
    let status = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    // SAFETY: the call succeeded and filled the structure.
    (status == 0).then(|| unsafe { info.assume_init() })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process group measurement on macOS.
//!
//! The kernel does not enforce an address space or resident memory limit
//! for a group of processes, so the memory limit is measured: members are
//! listed with `proc_listpids(PROC_PGRP_ONLY)` and measured with
//! `proc_pid_rusage`, and the supervisor ends the group at the limit.

use std::collections::HashMap;
use std::os::fd::RawFd;
use std::sync::Mutex;
use std::time::Duration;

use nurt_contracts::error::Result;
use nurt_contracts::record::LimitHit;

use super::io_error;
use crate::iface::{ProcessLimits, ResourceUsage};

/// `PROC_PGRP_ONLY` of `<libproc.h>`: list the members of a process group.
const PROC_PGRP_ONLY: u32 = 2;
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
}

impl Membership {
    /// Prepares measurement; no kernel limit is available for a group.
    pub(super) fn prepare(_limits: &ProcessLimits) -> Self {
        Self {
            timebase: Timebase::query(),
            seen_cpu: Mutex::new(HashMap::new()),
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

    /// Every member is in the process group; nothing else to end.
    pub(super) fn kill(&self) {}

    /// Processor time and resident memory of the processes in group `pgid`.
    ///
    /// The processor time of a member that ended is kept at its last value,
    /// so the total does not decrease.
    pub(super) fn sample(&self, pgid: libc::pid_t) -> Result<ResourceUsage> {
        let members = members(pgid)?;
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

fn members(pgid: libc::pid_t) -> Result<Vec<libc::pid_t>> {
    let entry_size = std::mem::size_of::<libc::pid_t>();
    let mut capacity = MEMBER_CAPACITY;
    loop {
        let mut buffer: Vec<libc::pid_t> = vec![0; capacity];
        let size = libc::c_int::try_from(capacity * entry_size).unwrap_or(libc::c_int::MAX);
        // SAFETY: the buffer holds `size` writable bytes.
        let filled = unsafe {
            libc::proc_listpids(
                PROC_PGRP_ONLY,
                pgid.cast_unsigned(),
                buffer.as_mut_ptr().cast(),
                size,
            )
        };
        let Ok(filled) = usize::try_from(filled) else {
            return Err(io_error(
                "list the processes of the group",
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

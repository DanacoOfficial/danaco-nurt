// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::os::target
// Responsibility:   One process of a Unix system, named by identifier and start time.
// Dependencies:     libc, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! One process of a Unix system, named by identifier and start time.
//!
//! The kernel reuses a process identifier after the process ended and its
//! parent collected it, so a signal sent to a remembered identifier can reach
//! an unrelated process. On Linux the process is first pinned with a pidfd:
//! the descriptor keeps naming the process it was opened for, and a signal sent
//! through it fails once that process ended. The start time read after the
//! descriptor was opened tells whether the process is the one of the record.
//! Systems without pidfd send the signal by identifier right after the check.

#[cfg(target_os = "macos")]
use std::mem::MaybeUninit;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use libc::{c_int, pid_t};
use nurt_contracts::env::WallTime;
use nurt_contracts::error::Result;
use nurt_contracts::record::Signal;

use super::unix::signal_number;
use super::{io_error, process_denied, process_gone, process_replaced, same_start};

/// Nanoseconds in one second.
const NANOS_PER_SECOND: i128 = 1_000_000_000;
/// Position of the start time in `/proc/<pid>/stat`, counted from the state
/// field that follows the command name.
#[cfg(target_os = "linux")]
const STAT_START_TIME: usize = 19;
/// Nanoseconds in one microsecond.
#[cfg(target_os = "macos")]
const NANOS_PER_MICRO: i128 = 1_000;
/// Status `SZOMB` of the system: the process ended and waits for its parent.
#[cfg(target_os = "macos")]
const ZOMBIE_STATUS: u32 = 5;

/// What the system reports about a process.
struct Identity {
    started: WallTime,
    /// The process ended and only waits for its parent to collect it.
    ended: bool,
}

/// Process named by identifier and start time.
pub(crate) struct Target {
    pid: pid_t,
    started: WallTime,
    #[cfg(target_os = "linux")]
    pidfd: Option<OwnedFd>,
}

impl Target {
    /// Pins process `pid` after checking that it started at `started`.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when no such process runs and with `E_STALE`
    /// when the identifier names a process that started at another time.
    pub(crate) fn open(pid: u64, started: WallTime) -> Result<Self> {
        let Some(id) = pid_t::try_from(pid).ok().filter(|id| *id > 0) else {
            return Err(process_gone(pid));
        };
        // The descriptor is opened before the start time is read: a process
        // that ends in between leaves a descriptor of a process that no
        // longer runs, and a reused identifier fails the check below.
        #[cfg(target_os = "linux")]
        let pidfd = open_pidfd(id);
        let actual = identity(id)
            .filter(|identity| !identity.ended)
            .ok_or_else(|| process_gone(pid))?;
        if !same_start(started, actual.started) {
            return Err(process_replaced(pid));
        }
        Ok(Self {
            pid: id,
            started: actual.started,
            #[cfg(target_os = "linux")]
            pidfd,
        })
    }

    /// Sends `signal` to the process.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNSUPPORTED` for a signal without a name on this system,
    /// with `E_NOT_FOUND` when the process ended and with `E_PERMISSION` when
    /// the account may not signal it.
    pub(crate) fn signal(&self, signal: Signal) -> Result<()> {
        self.send(signal_number(signal)?)
    }

    /// Stops the process until it is resumed.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when the process ended and with
    /// `E_PERMISSION` when the account may not signal it.
    pub(crate) fn suspend(&self) -> Result<()> {
        self.send(libc::SIGSTOP)
    }

    /// Continues a stopped process.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when the process ended and with
    /// `E_PERMISSION` when the account may not signal it.
    pub(crate) fn resume(&self) -> Result<()> {
        self.send(libc::SIGCONT)
    }

    /// Whether the process ended, or its identifier names another process.
    pub(crate) fn has_ended(&self) -> bool {
        identity(self.pid)
            .is_none_or(|identity| identity.ended || !same_start(self.started, identity.started))
    }

    #[allow(
        clippy::useless_conversion,
        reason = "the result of the system call is 32 bits wide on 32-bit Linux"
    )]
    fn send(&self, number: c_int) -> Result<()> {
        let pid = u64::try_from(self.pid).unwrap_or_default();
        #[cfg(target_os = "linux")]
        {
            if let Some(pidfd) = &self.pidfd {
                // SAFETY: the descriptor is open and owned by `self`; the
                // signal information is null and the flags are zero, as the
                // system call requires for a plain signal.
                let status = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        pidfd.as_raw_fd(),
                        number,
                        std::ptr::null::<libc::siginfo_t>(),
                        0_u32,
                    )
                };
                #[allow(
                    clippy::useless_conversion,
                    reason = "`libc::syscall` returns `c_long`, which is 32 bits wide on 32-bit targets"
                )]
                return check(i64::from(status), pid);
            }
        }
        // SAFETY: `self.pid` is positive, so a single process is addressed.
        let status = unsafe { libc::kill(self.pid, number) };
        check(i64::from(status), pid)
    }
}

/// Result of a signal call: `Ok` for 0, otherwise the error of the call.
fn check(status: i64, pid: u64) -> Result<()> {
    if status == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    Err(match error.raw_os_error() {
        Some(libc::ESRCH) => process_gone(pid),
        Some(libc::EPERM) => process_denied(pid),
        _ => io_error("signal the process", &error),
    })
}

/// Descriptor that names the process `pid` for as long as it is open; `None`
/// when the system does not provide it.
#[cfg(target_os = "linux")]
fn open_pidfd(pid: pid_t) -> Option<OwnedFd> {
    // SAFETY: `pidfd_open` takes an identifier and flags and returns a new
    // descriptor or -1.
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) };
    let descriptor = c_int::try_from(descriptor).ok().filter(|fd| *fd >= 0)?;
    // SAFETY: the call returned a new descriptor that nothing else owns.
    Some(unsafe { OwnedFd::from_raw_fd(descriptor) })
}

/// Start time of the process `pid`; `None` when no such process exists or it
/// ended and only waits for its parent.
#[cfg(test)]
pub(crate) fn start_time(pid: u64) -> Option<WallTime> {
    let id = pid_t::try_from(pid).ok().filter(|id| *id > 0)?;
    identity(id)
        .filter(|identity| !identity.ended)
        .map(|identity| identity.started)
}

#[cfg(target_os = "linux")]
fn identity(pid: pid_t) -> Option<Identity> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks = parse_start_ticks(&stat)?;
    let boot = boot_seconds()?;
    Some(Identity {
        started: since_boot(boot, ticks)?,
        ended: stat_ended(&stat)?,
    })
}

/// Start of the process in clock ticks after the boot, from the content of
/// `/proc/<pid>/stat`. The command name is in parentheses and may contain
/// spaces and parentheses, so fields are counted after the last closing one.
#[cfg(target_os = "linux")]
fn parse_start_ticks(stat: &str) -> Option<u64> {
    let (_, rest) = stat.rsplit_once(')')?;
    rest.split_whitespace().nth(STAT_START_TIME)?.parse().ok()
}

/// Whether the state field of `/proc/<pid>/stat` says that the process ended.
#[cfg(target_os = "linux")]
fn stat_ended(stat: &str) -> Option<bool> {
    let (_, rest) = stat.rsplit_once(')')?;
    let state = rest.split_whitespace().next()?;
    Some(matches!(state, "Z" | "X"))
}

/// Boot time of the system in seconds since the Unix epoch.
#[cfg(target_os = "linux")]
fn boot_seconds() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("btime "))
        .and_then(|value| value.trim().parse().ok())
}

/// Wall time of `ticks` clock ticks after the boot at `boot`.
#[cfg(target_os = "linux")]
fn since_boot(boot: u64, ticks: u64) -> Option<WallTime> {
    // SAFETY: `sysconf` has no preconditions.
    let rate = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let rate = i128::from(u64::try_from(rate).ok().filter(|rate| *rate > 0)?);
    let nanos = i128::from(boot) * NANOS_PER_SECOND + i128::from(ticks) * NANOS_PER_SECOND / rate;
    Some(WallTime::from_unix_nanos(nanos))
}

#[cfg(target_os = "macos")]
fn identity(pid: pid_t) -> Option<Identity> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = c_int::try_from(size_of::<libc::proc_bsdinfo>()).ok()?;
    // SAFETY: `info` provides `size` writable bytes for the structure of the
    // flavor `PROC_PIDTBSDINFO`.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: the call filled the whole structure, which is plain data.
    let info = unsafe { info.assume_init() };
    let nanos = i128::from(info.pbi_start_tvsec) * NANOS_PER_SECOND
        + i128::from(info.pbi_start_tvusec) * NANOS_PER_MICRO;
    Some(Identity {
        started: WallTime::from_unix_nanos(nanos),
        ended: info.pbi_status == ZOMBIE_STATUS,
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// Content of `/proc/<pid>/stat` of a process whose name holds spaces and
    /// parentheses, in state `state`, started 4096 ticks after the boot.
    fn stat(state: &str) -> String {
        format!(
            "4242 (a b) c) {state} 1 4240 4240 0 -1 4194560 100 0 0 0 7 3 0 0 20 0 1 0 4096 \
             8192 25 18446744073709551615"
        )
    }

    #[test]
    fn the_start_time_is_counted_after_the_command_name() {
        assert_eq!(parse_start_ticks(&stat("S")), Some(4096));
        assert_eq!(parse_start_ticks("4242 (a) S 1 4240"), None);
        assert_eq!(parse_start_ticks("no parenthesis"), None);
    }

    #[test]
    fn zombies_and_dead_processes_have_ended() {
        assert_eq!(stat_ended(&stat("S")), Some(false));
        assert_eq!(stat_ended(&stat("T")), Some(false));
        assert_eq!(stat_ended(&stat("Z")), Some(true));
        assert_eq!(stat_ended(&stat("X")), Some(true));
        assert_eq!(stat_ended("no parenthesis"), None);
    }

    #[test]
    fn ticks_are_added_to_the_boot_time() {
        let boot = 1_700_000_000;
        let one_tick = since_boot(boot, 1).unwrap();
        let none = since_boot(boot, 0).unwrap();
        assert_eq!(none.unix_nanos(), i128::from(boot) * NANOS_PER_SECOND);
        assert!(one_tick.unix_nanos() > none.unix_nanos());
        assert!(one_tick.unix_nanos() - none.unix_nanos() <= NANOS_PER_SECOND);
    }

    #[test]
    fn the_own_process_runs_and_has_a_start_time() {
        let started = start_time(u64::from(std::process::id())).unwrap();
        let target = Target::open(u64::from(std::process::id()), started).unwrap();
        assert!(!target.has_ended());
    }

    #[test]
    fn identifiers_that_cannot_name_a_process_are_not_found() {
        assert_eq!(start_time(0), None);
        assert_eq!(start_time(u64::MAX), None);
        let started = WallTime::UNIX_EPOCH;
        for pid in [0, u64::MAX, u64::from(u32::MAX)] {
            let error = Target::open(pid, started).err().unwrap();
            assert_eq!(error.code(), nurt_contracts::error::ErrorCode::NotFound);
        }
    }

    #[test]
    fn a_process_with_another_start_time_is_refused() {
        let own = u64::from(std::process::id());
        let started = start_time(own).unwrap();
        let earlier = WallTime::from_unix_nanos(started.unix_nanos() - 3_600 * NANOS_PER_SECOND);
        let error = Target::open(own, earlier).err().unwrap();
        assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Stale);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

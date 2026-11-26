// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Processes of Unix systems.
//!
//! On Linux the process is pinned by a process descriptor opened before its
//! start time is read, so a signal reaches the process whose start time was
//! checked even when the identifier is given to another process meanwhile.
//! Other systems signal by identifier after the check.

use std::io;

use libc::{c_int, pid_t};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Signal;

use super::{Condition, same_start};

#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// Nanoseconds in one second.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// Process found under an identifier.
struct Look {
    /// Start time in nanoseconds since the Unix epoch, when the system
    /// reports it.
    started: Option<i64>,
    /// State of the process.
    condition: Condition,
}

/// Process that is read and signalled.
pub(crate) struct Handle {
    pid: pid_t,
    started: Option<i64>,
    #[cfg(target_os = "linux")]
    descriptor: Option<OwnedFd>,
}

impl Handle {
    /// Opens the process `pid`; `None` when no such process exists.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` for an identifier the system cannot
    /// have.
    pub(crate) fn open(pid: u32) -> Result<Option<Self>> {
        let pid = pid_t::try_from(pid).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("the process identifier {pid} is out of range for this system"),
            )
        })?;
        #[cfg(target_os = "linux")]
        let descriptor = pidfd(pid);
        let Some(look) = look(pid) else {
            return Ok(None);
        };
        Ok(Some(Self {
            pid,
            started: look.started,
            #[cfg(target_os = "linux")]
            descriptor,
        }))
    }

    /// Start time in nanoseconds since the Unix epoch, when the system
    /// reports it.
    pub(crate) fn started(&self) -> Option<i64> {
        self.started
    }

    /// Current state; a process that ended, or whose identifier belongs to
    /// another process now, is `Ended`.
    pub(crate) fn condition(&self) -> Condition {
        #[cfg(target_os = "linux")]
        {
            if self.descriptor.as_ref().is_some_and(exited) {
                return Condition::Ended;
            }
        }
        match look(self.pid) {
            Some(found) if same_start(self.started, found.started) => found.condition,
            _ => Condition::Ended,
        }
    }

    /// Sends `signal`; only `terminate`, `interrupt`, `hangup` and `kill`
    /// can be sent.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNSUPPORTED` for another signal, `E_PERMISSION` when the
    /// account may not signal the process and `E_IO` for another failure.
    pub(crate) fn signal(&self, signal: Signal) -> Result<()> {
        self.send(number(signal)?)
    }

    /// Suspends the process until it is resumed.
    ///
    /// # Errors
    ///
    /// As [`Handle::signal`].
    pub(crate) fn suspend(&self) -> Result<()> {
        self.send(libc::SIGSTOP)
    }

    /// Continues a suspended process.
    ///
    /// # Errors
    ///
    /// As [`Handle::signal`].
    pub(crate) fn resume(&self) -> Result<()> {
        self.send(libc::SIGCONT)
    }

    fn send(&self, number: c_int) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if let Some(descriptor) = &self.descriptor {
                // SAFETY: `pidfd_send_signal` takes a process descriptor that
                // stays open while `descriptor` lives, a signal number, a
                // null pointer for no signal information and zero flags.
                let status = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        descriptor.as_raw_fd(),
                        number,
                        std::ptr::null::<libc::siginfo_t>(),
                        0_u32,
                    )
                };
                return delivered(status == 0, self.pid);
            }
        }
        if self.condition() == Condition::Ended {
            return Ok(());
        }
        // SAFETY: `kill` takes a positive process identifier and a signal
        // number; the identifier was checked against the start time of the
        // process before the call.
        let status = unsafe { libc::kill(self.pid, number) };
        delivered(status == 0, self.pid)
    }
}

/// Number of a signal that `stop` can send.
fn number(signal: Signal) -> Result<c_int> {
    match signal {
        Signal::Terminate => Ok(libc::SIGTERM),
        Signal::Kill => Ok(libc::SIGKILL),
        Signal::Interrupt => Ok(libc::SIGINT),
        Signal::Hangup => Ok(libc::SIGHUP),
        other => Err(Error::new(
            ErrorCode::Unsupported,
            format!(
                "the signal {other} cannot be sent to a process; use terminate, interrupt, hangup or kill"
            ),
        )),
    }
}

/// Outcome of a signal call that succeeded when `succeeded`; reads the error
/// of the call that failed otherwise. A process that is gone counts as
/// signalled.
fn delivered(succeeded: bool, pid: pid_t) -> Result<()> {
    if succeeded {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(()),
        Some(libc::EPERM) => Err(Error::new(
            ErrorCode::Permission,
            format!(
                "the account may not signal process {pid}; run the command with the privilege of its owner"
            ),
        )
        .with_found(pid.to_string())),
        _ => Err(Error::new(
            ErrorCode::Io,
            format!("cannot signal process {pid}: {error}"),
        )),
    }
}

/// Process descriptor of `pid`; `None` when the system has none.
#[cfg(target_os = "linux")]
fn pidfd(pid: pid_t) -> Option<OwnedFd> {
    // SAFETY: `pidfd_open` takes a process identifier and flags and returns a
    // new descriptor or -1.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) };
    let raw = i32::try_from(raw)
        .ok()
        .filter(|descriptor| *descriptor >= 0)?;
    // SAFETY: the call returned a new descriptor that nothing else owns.
    Some(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// Whether the process of `descriptor` terminated.
#[cfg(target_os = "linux")]
fn exited(descriptor: &OwnedFd) -> bool {
    let mut poll = libc::pollfd {
        fd: descriptor.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `poll` is one writable `pollfd`; the count is one and the
    // timeout zero, so the call returns at once.
    let ready = unsafe { libc::poll(&raw mut poll, 1, 0) };
    ready > 0 && poll.revents & libc::POLLIN != 0
}

/// Position of the start time among the fields of `/proc/<pid>/stat` that
/// follow the command name.
#[cfg(target_os = "linux")]
const START_TICKS_FIELD: usize = 19;

#[cfg(target_os = "linux")]
fn look(pid: pid_t) -> Option<Look> {
    let bytes = std::fs::read(format!("/proc/{pid}/stat")).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let rest = text.get(text.rfind(')')? + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let condition = match fields.first()?.chars().next()? {
        'Z' | 'X' => Condition::Ended,
        'T' | 't' => Condition::Stopped,
        _ => Condition::Running,
    };
    let started = fields
        .get(START_TICKS_FIELD)
        .and_then(|ticks| ticks.parse::<u64>().ok())
        .and_then(started_at);
    Some(Look { started, condition })
}

/// Start time of a process that started `ticks` clock ticks after the boot.
#[cfg(target_os = "linux")]
fn started_at(ticks: u64) -> Option<i64> {
    let bytes = std::fs::read("/proc/stat").ok()?;
    let boot: u64 = String::from_utf8_lossy(&bytes)
        .lines()
        .find_map(|line| line.strip_prefix("btime "))
        .and_then(|value| value.trim().parse().ok())?;
    // SAFETY: `sysconf` has no preconditions.
    let per_second = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) })
        .ok()
        .filter(|per_second| *per_second > 0)?;
    let nanos = u128::from(boot) * NANOS_PER_SECOND
        + u128::from(ticks) * NANOS_PER_SECOND / u128::from(per_second);
    i64::try_from(nanos).ok()
}

/// State of a process in `sys/proc.h`: stopped.
#[cfg(target_os = "macos")]
const STATUS_STOP: u32 = 4;
/// State of a process in `sys/proc.h`: ended but not collected.
#[cfg(target_os = "macos")]
const STATUS_ZOMBIE: u32 = 5;

#[cfg(target_os = "macos")]
fn look(pid: pid_t) -> Option<Look> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
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
    // SAFETY: the call filled the whole structure, which is plain data valid
    // for any bytes.
    let info = unsafe { info.assume_init() };
    let nanos = u128::from(info.pbi_start_tvsec) * NANOS_PER_SECOND
        + u128::from(info.pbi_start_tvusec) * 1_000;
    let condition = match info.pbi_status {
        STATUS_STOP => Condition::Stopped,
        STATUS_ZOMBIE => Condition::Ended,
        _ => Condition::Running,
    };
    Some(Look {
        started: i64::try_from(nanos).ok(),
        condition,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn look(pid: pid_t) -> Option<Look> {
    // SAFETY: signal number 0 only checks that the process exists and may be
    // signalled.
    let status = unsafe { libc::kill(pid, 0) };
    (status == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)).then_some(
        Look {
            started: None,
            condition: Condition::Running,
        },
    )
}

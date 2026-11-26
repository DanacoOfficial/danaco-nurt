// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Processes of Windows.
//!
//! A process is opened once and the handle keeps naming it, so a signal never
//! reaches another process that received the identifier later. Every signal
//! that `stop` accepts ends the process at once, as `Stop-Process` does;
//! Windows has no way to ask a process that is not a console program to end.
//! `suspend` and `resume` stop and continue every thread of the process with
//! `NtSuspendProcess` and `NtResumeProcess`.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Signal;
use windows_sys::Win32::Foundation::{FILETIME, HANDLE, NTSTATUS};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE, TerminateProcess,
};

use super::Condition;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtSuspendProcess(process: HANDLE) -> NTSTATUS;
    fn NtResumeProcess(process: HANDLE) -> NTSTATUS;
}

/// Error code of `OpenProcess` for an identifier that names no process.
const ERROR_INVALID_PARAMETER: i32 = 87;
/// Error code of `OpenProcess` for a process the account may not open.
const ERROR_ACCESS_DENIED: i32 = 5;
/// Exit code that `GetExitCodeProcess` reports for a process that runs.
const STILL_ACTIVE: u32 = 259;
/// Exit code given to a process that is ended.
const TERMINATED_EXIT_CODE: u32 = 1;
/// Difference between the epoch of file times and the Unix epoch, in units of
/// 100 nanoseconds.
const EPOCH_DIFFERENCE: u64 = 116_444_736_000_000_000;
/// Nanoseconds in one unit of a file time.
const FILETIME_UNIT_NANOSECONDS: u64 = 100;

/// Process that is read and signalled.
pub(crate) struct Handle {
    process: OwnedHandle,
    started: Option<i64>,
}

impl Handle {
    /// Opens the process `pid`; `None` when no such process exists.
    ///
    /// # Errors
    ///
    /// Fails with `E_PERMISSION` when the account may not open the process
    /// and with `E_IO` for another failure.
    pub(crate) fn open(pid: u32) -> Result<Option<Self>> {
        // SAFETY: plain call; a null result is checked below.
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SUSPEND_RESUME,
                0,
                pid,
            )
        };
        if raw.is_null() {
            let error = io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(ERROR_INVALID_PARAMETER) => Ok(None),
                Some(ERROR_ACCESS_DENIED) => Err(Error::new(
                    ErrorCode::Permission,
                    format!(
                        "the account may not open process {pid}; run the command with the privilege of its owner"
                    ),
                )
                .with_found(pid.to_string())),
                _ => Err(Error::new(
                    ErrorCode::Io,
                    format!("cannot open process {pid}: {error}"),
                )),
            };
        }
        // SAFETY: `OpenProcess` returned an open handle that nothing else
        // owns.
        let process = unsafe { OwnedHandle::from_raw_handle(raw) };
        let started = started(&process);
        Ok(Some(Self { process, started }))
    }

    /// Start time in nanoseconds since the Unix epoch, when the system
    /// reports it.
    pub(crate) fn started(&self) -> Option<i64> {
        self.started
    }

    /// Current state.
    pub(crate) fn condition(&self) -> Condition {
        let mut code: u32 = 0;
        // SAFETY: `code` is a writable `u32` and the handle is open.
        let read = unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) };
        if read == 0 || code == STILL_ACTIVE {
            Condition::Running
        } else {
            Condition::Ended
        }
    }

    /// Ends the process; `terminate`, `interrupt`, `hangup` and `kill` all end
    /// it at once.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNSUPPORTED` for another signal and with `E_IO` when the
    /// process cannot be ended.
    pub(crate) fn signal(&self, signal: Signal) -> Result<()> {
        if !matches!(
            signal,
            Signal::Terminate | Signal::Kill | Signal::Interrupt | Signal::Hangup
        ) {
            return Err(Error::new(
                ErrorCode::Unsupported,
                format!(
                    "the signal {signal} cannot be sent to a process; use terminate, interrupt, hangup or kill"
                ),
            ));
        }
        // SAFETY: the handle is open with the right to end the process.
        let ended = unsafe { TerminateProcess(self.process.as_raw_handle(), TERMINATED_EXIT_CODE) };
        if ended != 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if self.condition() == Condition::Ended {
            return Ok(());
        }
        Err(Error::new(
            ErrorCode::Io,
            format!("cannot end the process: {error}"),
        ))
    }

    /// Stops every thread of the process.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the system refuses to suspend the process.
    pub(crate) fn suspend(&self) -> Result<()> {
        // SAFETY: the handle is open with `PROCESS_SUSPEND_RESUME` access.
        let status = unsafe { NtSuspendProcess(self.process.as_raw_handle()) };
        checked(status, "suspend")
    }

    /// Continues every thread of the process.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the system refuses to resume the process.
    pub(crate) fn resume(&self) -> Result<()> {
        // SAFETY: the handle is open with `PROCESS_SUSPEND_RESUME` access.
        let status = unsafe { NtResumeProcess(self.process.as_raw_handle()) };
        checked(status, "resume")
    }
}

/// Turns the status of a native call into a result.
fn checked(status: NTSTATUS, action: &str) -> Result<()> {
    if status >= 0 {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::Io,
        format!("cannot {action} the process: status 0x{status:08X}"),
    ))
}

fn filetime(time: &FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// Start time of a process in nanoseconds since the Unix epoch.
fn started(process: &OwnedHandle) -> Option<i64> {
    let zero = || FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero(), zero(), zero(), zero());
    // SAFETY: the four pointers refer to writable `FILETIME` values of this
    // frame and the handle is open.
    let read = unsafe {
        GetProcessTimes(
            process.as_raw_handle(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    if read == 0 {
        return None;
    }
    let since_epoch = filetime(&created).checked_sub(EPOCH_DIFFERENCE)?;
    i64::try_from(since_epoch.checked_mul(FILETIME_UNIT_NANOSECONDS)?).ok()
}

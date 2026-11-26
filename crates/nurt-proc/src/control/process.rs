// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Signals to single processes of the system, as the `stop`, `suspend` and
//! `resume` commands send them to `process` records.
//!
//! A `process` record names a process by its identifier and its start time.
//! The identifier alone is not enough: the system gives it to another process
//! once the first one ended, and a signal sent later would reach that
//! process. The functions of this module check the start time of the process
//! and then signal exactly the process that was checked.

use std::time::{Duration, Instant};

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Signal;
use nurt_contracts::stage::CancelToken;
use nurt_contracts::value;

use crate::os::Target;

/// Time a process may need to end after it was killed.
const KILL_WAIT: Duration = Duration::from_secs(5);
/// Longest interval between two checks whether a process ended.
const END_POLL: Duration = Duration::from_millis(10);

/// A process of the system as a `process` record names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessRef {
    /// Process identifier.
    pub pid: u64,
    /// Start time of the process.
    pub started: value::Instant,
}

/// How a stopped process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopEnd {
    /// The process ended after the requested signal, within the grace time.
    Signalled,
    /// The grace time elapsed and the process was killed.
    Killed,
}

impl ProcessRef {
    fn open(self) -> Result<Target> {
        let started = WallTime::from_unix_nanos(i128::from(self.started.as_unix_nanos()));
        Target::open(self.pid, started)
    }
}

/// Sends `signal` to the process and waits up to `grace` for its end; a
/// process that still runs is killed, and the call returns after its end.
///
/// The signal reaches the process only, not its children. On Windows every
/// signal but `kill` is a console break event, which only a process with its
/// own process group in the console of the caller receives; any other
/// process is ended at once.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` when no such process runs, with `E_STALE` when
/// the identifier names a process that started at another time, with
/// `E_PERMISSION` when the account may not signal the process, with
/// `E_CANCELLED` when `cancel` fires while the call waits, and with
/// `E_TIMEOUT` when the process still runs after it was killed.
pub fn stop_process(
    process: ProcessRef,
    signal: Signal,
    grace: Duration,
    cancel: &CancelToken,
) -> Result<StopEnd> {
    let target = process.open()?;
    cancel.check()?;
    ignore_ended(target.signal(signal))?;
    if signal == Signal::Kill {
        return end_within(&target, KILL_WAIT, cancel, process.pid).map(|()| StopEnd::Signalled);
    }
    if wait_for_end(&target, grace, cancel)? {
        return Ok(StopEnd::Signalled);
    }
    ignore_ended(target.signal(Signal::Kill))?;
    end_within(&target, KILL_WAIT, cancel, process.pid).map(|()| StopEnd::Killed)
}

/// Stops the process until it is resumed.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` when no such process runs, with `E_STALE` when
/// the identifier names a process that started at another time and with
/// `E_PERMISSION` when the account may not signal the process.
pub fn suspend_process(process: ProcessRef) -> Result<()> {
    process.open()?.suspend()
}

/// Continues a process stopped by [`suspend_process`].
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` when no such process runs, with `E_STALE` when
/// the identifier names a process that started at another time and with
/// `E_PERMISSION` when the account may not signal the process.
pub fn resume_process(process: ProcessRef) -> Result<()> {
    process.open()?.resume()
}

/// A process that ended before the signal reached it has reached the goal of
/// the call.
fn ignore_ended(result: Result<()>) -> Result<()> {
    match result {
        Err(error) if error.code() == ErrorCode::NotFound => Ok(()),
        other => other,
    }
}

fn end_within(target: &Target, limit: Duration, cancel: &CancelToken, pid: u64) -> Result<()> {
    if wait_for_end(target, limit, cancel)? {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::Timeout,
        format!("process {pid} still runs after it was killed"),
    )
    .with_fix("read the processes again with `processes`"))
}

/// Waits up to `limit` for the end of the process; `false` when it still
/// runs.
fn wait_for_end(target: &Target, limit: Duration, cancel: &CancelToken) -> Result<bool> {
    let deadline = Instant::now().checked_add(limit);
    loop {
        if target.has_ended() {
            return Ok(true);
        }
        let left = match deadline {
            Some(deadline) => deadline.saturating_duration_since(Instant::now()),
            None => END_POLL,
        };
        if left.is_zero() {
            return Ok(false);
        }
        if cancel.wait_timeout(left.min(END_POLL)).is_some() {
            cancel.check()?;
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::os
// Responsibility:   Process control calls of the operating system.
// Dependencies:     nurt_contracts, target, unix, windows
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Process control calls of the operating system.
//!
//! Each system module provides `Tree`: a started program together with every
//! process it starts, with the same operations on all systems. Unix systems
//! keep the tree in a new session and process group, Windows in a job object.
//! Each also provides `Target`: one process of the system named by its
//! identifier and start time, signalled without the risk of reaching another
//! process that received the identifier after the first one ended.

use std::io::{Read, Write};
use std::time::Duration;

use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{LimitHit, Signal};

use crate::iface::ResourceUsage;

#[cfg(any(target_os = "macos", all(unix, test)))]
mod descendants;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod target;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use target::Target;
#[cfg(all(test, unix))]
pub(crate) use target::start_time;
#[cfg(unix)]
pub(crate) use unix::Tree;
#[cfg(windows)]
pub(crate) use windows::{ServiceJob, Target, Tree};

/// Largest difference between the start time of a `process` record and the
/// start time the system reports now for the same process. The systems derive
/// the start time from the boot time, which they report in whole seconds and
/// which moves when the clock is adjusted.
const START_TOLERANCE: Duration = Duration::from_secs(2);

/// Whether `actual` is the start time of the process that started at
/// `recorded`.
pub(crate) fn same_start(recorded: WallTime, actual: WallTime) -> bool {
    recorded
        .saturating_since(actual)
        .max(actual.saturating_since(recorded))
        <= START_TOLERANCE
}

/// Error of a process that does not run.
pub(crate) fn process_gone(pid: u64) -> Error {
    Error::new(ErrorCode::NotFound, format!("no process {pid} runs"))
        .with_problem("the process ended or never existed")
        .with_fix("read the processes again with `processes`")
}

/// Error of an identifier that now names another process than the record.
pub(crate) fn process_replaced(pid: u64) -> Error {
    Error::new(
        ErrorCode::Stale,
        format!("process {pid} is not the process of the record: its start time differs"),
    )
    .with_problem("the identifier was reused after the process ended")
    .with_fix("read the processes again with `processes`")
}

/// Error of a process the account may not signal.
pub(crate) fn process_denied(pid: u64) -> Error {
    Error::new(
        ErrorCode::Permission,
        format!("the account may not signal process {pid}"),
    )
}

/// How the main process of a tree ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exit {
    /// Ended with an exit code.
    Code(i32),
    /// Ended by a signal or, on Windows, by a fault or forced termination.
    Signal {
        /// Name common to all systems.
        signal: Signal,
        /// System number of the signal; `None` on Windows.
        number: Option<i32>,
    },
}

/// Final state of a tree after its main process was collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Finished {
    /// How the main process ended.
    pub(crate) exit: Exit,
    /// Resource use reported by the system for the tree.
    pub(crate) usage: ResourceUsage,
    /// Limit enforced by the system itself.
    pub(crate) limit_hit: Option<LimitHit>,
}

/// Pipes connected to a started program.
pub(crate) struct Streams {
    /// Writable end of the standard input when the input is given as bytes.
    pub(crate) stdin: Option<Box<dyn Write + Send>>,
    /// Standard output, with standard error when the streams are merged.
    pub(crate) stdout: Box<dyn Read + Send>,
    /// Standard error when the streams are separate.
    pub(crate) stderr: Option<Box<dyn Read + Send>>,
}

/// Error of a start that failed before the program ran.
pub(crate) fn spawn_error(program: &std::path::Path, error: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::Spawn,
        format!("cannot start {}: {error}", program.display()),
    )
}

/// Error of a system call on a running tree.
pub(crate) fn io_error(action: &str, error: &std::io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("cannot {action}: {error}"))
}

/// Whether the exit of the main process is the forced termination ordered by
/// the supervisor: the supervisor recorded that it ended the job while the main
/// process was still running (`ended_while_running`) and the process left with
/// the code given to the termination. The code alone never decides, because a
/// program may exit with that code by itself.
#[cfg(any(windows, test))]
pub(crate) fn ended_by_supervisor(
    code: u32,
    termination_code: u32,
    ended_while_running: bool,
) -> bool {
    ended_while_running && code == termination_code
}

/// Error of a signal that does not exist on this system.
pub(crate) fn unsupported_signal(signal: Signal) -> Error {
    Error::new(
        ErrorCode::Unsupported,
        format!("the signal {signal} cannot be sent on this system"),
    )
    .with_problem("use terminate, interrupt, hangup or kill")
}

/// Converts a duration to whole seconds, rounding up.
#[cfg(unix)]
pub(crate) fn ceil_seconds(duration: Duration) -> u64 {
    let whole = duration.as_secs();
    if duration.subsec_nanos() == 0 {
        whole
    } else {
        whole.saturating_add(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i128, nanos: i128) -> WallTime {
        WallTime::from_unix_nanos(seconds * 1_000_000_000 + nanos)
    }

    #[test]
    fn start_times_within_the_tolerance_name_the_same_process() {
        let recorded = at(1_700_000_000, 0);
        assert!(same_start(recorded, recorded));
        assert!(same_start(recorded, at(1_700_000_001, 999_999_999)));
        assert!(same_start(recorded, at(1_699_999_998, 0)));
        assert!(same_start(recorded, at(1_700_000_002, 0)));
    }

    #[test]
    fn start_times_beyond_the_tolerance_name_another_process() {
        let recorded = at(1_700_000_000, 0);
        assert!(!same_start(recorded, at(1_700_000_002, 1)));
        assert!(!same_start(recorded, at(1_699_999_997, 999_999_999)));
        assert!(!same_start(recorded, at(1_700_003_600, 0)));
    }

    #[test]
    fn the_comparison_does_not_depend_on_the_order_of_the_times() {
        let early = at(10, 0);
        let late = at(100, 0);
        assert_eq!(same_start(early, late), same_start(late, early));
        assert!(!same_start(early, late));
    }

    #[test]
    fn errors_of_a_missing_replaced_or_protected_process_have_their_own_codes() {
        assert_eq!(process_gone(7).code(), ErrorCode::NotFound);
        assert_eq!(process_replaced(7).code(), ErrorCode::Stale);
        assert_eq!(process_denied(7).code(), ErrorCode::Permission);
        assert!(process_gone(4242).data().message.contains("4242"));
    }

    #[test]
    fn a_natural_exit_is_not_a_forced_termination_without_the_record() {
        assert!(!ended_by_supervisor(1, 1, false));
        assert!(!ended_by_supervisor(0, 1, false));
    }

    #[test]
    fn the_record_and_the_termination_code_together_mean_a_forced_termination() {
        assert!(ended_by_supervisor(1, 1, true));
    }

    #[test]
    fn a_recorded_termination_does_not_change_another_exit_code() {
        assert!(!ended_by_supervisor(0, 1, true));
        assert!(!ended_by_supervisor(3, 1, true));
    }

    #[cfg(unix)]
    #[test]
    fn durations_round_up_to_whole_seconds() {
        assert_eq!(ceil_seconds(Duration::ZERO), 0);
        assert_eq!(ceil_seconds(Duration::from_secs(3)), 3);
        assert_eq!(ceil_seconds(Duration::from_millis(1)), 1);
        assert_eq!(ceil_seconds(Duration::new(3, 1)), 4);
        assert_eq!(ceil_seconds(Duration::MAX), u64::MAX);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

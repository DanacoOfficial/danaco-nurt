// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::stage::jobs::process::system
// Responsibility:   Calls of the operating system on a process that another program started.
// Dependencies:     unix, windows
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Calls of the operating system on a process that another program started:
//! reading its start time and state and sending it a signal.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::Handle;
#[cfg(windows)]
pub(crate) use windows::Handle;

/// Largest difference, in nanoseconds, between two readings of the start time
/// of one process. Systems derive the start time from the boot time, which
/// the clock adjustments of the system move by a second, and the text form of
/// a record may round it.
const START_TOLERANCE_NANOS: u64 = 2_000_000_000;

/// Whether two start times, in nanoseconds since the Unix epoch, name the
/// same process. A start time that the system does not report is not
/// compared.
pub(crate) fn same_start(left: Option<i64>, right: Option<i64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.abs_diff(right) <= START_TOLERANCE_NANOS,
        _ => true,
    }
}

/// State of a process as far as the commands on processes need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    windows,
    allow(dead_code, reason = "Windows has no stopped state of a process")
)]
pub(crate) enum Condition {
    /// The process runs or waits.
    Running,
    /// The process is suspended.
    Stopped,
    /// The process ended, even when its parent has not collected it yet.
    Ended,
}

impl Condition {
    /// Name of the state in the effect record.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Ended => "ended",
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

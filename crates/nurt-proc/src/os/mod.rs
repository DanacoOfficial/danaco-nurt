// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process control calls of the operating system.
//!
//! Each system module provides `Tree`: a started program together with every
//! process it starts, with the same operations on all systems. Unix systems
//! keep the tree in a new session and process group, Windows in a job object.

use std::io::{Read, Write};
#[cfg(unix)]
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{LimitHit, Signal};

use crate::iface::ResourceUsage;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::Tree;
#[cfg(windows)]
pub(crate) use windows::{ServiceJob, Tree};

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

/// Error of a signal that does not exist on this system.
pub(crate) fn unsupported_signal(signal: Signal) -> Error {
    Error::new(
        ErrorCode::Unsupported,
        format!("the signal {signal} cannot be sent on this system"),
    )
    .with_problem("use terminate, interrupt or kill")
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

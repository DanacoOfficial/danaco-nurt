// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::processes::model
// Responsibility:   Process data read from the operating system, before it becomes a record.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Process data read from the operating system, before it becomes a record.

use std::time::Duration;

use nurt_contracts::error::Result;
use nurt_contracts::record::{Process, ProcessState};
use nurt_contracts::value::{Instant, Path, Size, Value};

/// Optional parts of a process that cost an extra system call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReadRequest {
    /// Read the working directory.
    pub(crate) cwd: bool,
}

/// One process as read from the operating system.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ProcessInfo {
    pub(crate) pid: u64,
    pub(crate) parent_pid: u64,
    pub(crate) name: String,
    pub(crate) command: Vec<String>,
    pub(crate) user: Option<String>,
    pub(crate) uid: Option<String>,
    pub(crate) memory: u64,
    pub(crate) started: Option<Instant>,
    pub(crate) state: ProcessState,
    /// State code of the system, kept when the state maps to `other`.
    pub(crate) state_raw: Option<String>,
    pub(crate) executable: Option<String>,
    /// User and system processor time.
    pub(crate) cpu_time: Option<Duration>,
    pub(crate) threads: Option<u64>,
    pub(crate) cwd: Option<String>,
    /// A field was left empty because the account may not read it.
    pub(crate) restricted: bool,
}

/// Position of a process in the output of `tree: true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreePosition {
    pub(crate) depth: u64,
    pub(crate) children: Vec<u64>,
}

impl ProcessInfo {
    /// Converts the process into a `process` record.
    ///
    /// The account name falls back to the account identifier when the name
    /// is unknown; an unknown start time is written as the Unix epoch.
    pub(crate) fn into_record(self, cpu: Option<f64>, tree: Option<TreePosition>) -> Process {
        let os_raw = self.state_raw.map(|state| {
            [("state".to_owned(), Value::Text(state))]
                .into_iter()
                .collect()
        });
        let (depth, children) = match tree {
            Some(position) => (Some(position.depth), Some(position.children)),
            None => (None, None),
        };
        Process {
            pid: self.pid,
            parent_pid: self.parent_pid,
            name: self.name,
            command: self.command,
            user: self.user.or_else(|| self.uid.clone()).unwrap_or_default(),
            memory: Size::from_bytes(self.memory),
            started: self.started.unwrap_or(Instant::from_unix_nanos(0)),
            state: self.state,
            executable: self.executable.map(Path::from),
            uid: self.uid,
            cpu,
            threads: self.threads,
            cwd: self.cwd.map(Path::from),
            children,
            depth,
            os_raw,
        }
    }
}

/// Source of process tables: the operating system or a synthetic tree.
pub(crate) trait ProcessSource: Send + Sync {
    /// Opens a reader for one execution.
    ///
    /// # Errors
    ///
    /// Fails when the process table of the system cannot be read at all.
    fn open(&self, request: ReadRequest) -> Result<Box<dyn ProcessReader>>;
}

/// Reader of the process table during one execution.
pub(crate) trait ProcessReader: Send {
    /// Identifiers of the processes that exist now, in ascending order.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` or `E_PRIVILEGE` when the table cannot be listed.
    fn pids(&mut self) -> Result<Vec<u64>>;

    /// Reads one process; `None` when it has ended or cannot be read.
    fn read(&mut self, pid: u64) -> Option<ProcessInfo>;

    /// Processor time of one process; `None` when it has ended or cannot be
    /// read.
    fn cpu_time(&mut self, pid: u64) -> Option<Duration>;
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

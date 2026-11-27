// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::api
// Responsibility:   Public entry points: command stages, the process host service and tool.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Public entry points: command stages, the process host service and tool
//! runs for the test and analysis commands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::error::Result;
use nurt_contracts::record::{Exec, Job};
use nurt_contracts::services::{EnvVar, ProcessHost};
use nurt_contracts::stage::{ExecCtx, StageImpl};
use nurt_contracts::value::Record;

use crate::iface::{Components, RunHost};

/// Stages of the commands `run`, `jobs`, `stop`, `retry`, `suspend`,
/// `resume` and `remove` on job records.
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    let mut stages = crate::stage::run::stages();
    stages.extend(crate::stage::jobs::stages());
    stages
}

/// Process host service used by commands outside the run group.
pub fn process_host() -> Arc<dyn ProcessHost> {
    crate::host::process_host(components())
}

/// Kind of a tool run, recorded in the job of a run that continues in the
/// background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    /// Test framework.
    Test,
    /// Analyzer or formatter.
    Analyze,
}

/// Program run on behalf of a test or analysis command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRun {
    /// Command as written, program first.
    pub command: Vec<String>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Variables changed on top of the inherited environment.
    pub env: Vec<EnvVar>,
    /// Time limit.
    pub timeout: Duration,
    /// Directories watched for changes.
    pub watch: Vec<PathBuf>,
    /// Files the tool changes, kept before the start (formatters).
    pub exact: Option<Vec<PathBuf>>,
    /// Kind of the tool.
    pub kind: ToolKind,
    /// Label of the job.
    pub label: Option<String>,
    /// Command parameters needed to finish the job (for example the test run
    /// identifier), passed unchanged to the [`JobFinisher`].
    pub finish_data: Option<serde_json::Value>,
}

/// Finished tool run.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    /// Execution record.
    pub exec: Exec,
    /// Complete standard output.
    pub stdout_file: Option<PathBuf>,
    /// Complete standard error.
    pub stderr_file: Option<PathBuf>,
    /// Journal entry of the changes.
    pub op: Option<u64>,
}

/// Outcome of a tool run.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutcome {
    /// The tool ended within the waiting time.
    Finished(Box<ToolResult>),
    /// The tool continues in the background.
    Background(Box<Job>),
}

/// Runs a tool under the rules of the run command: without a shell, under
/// process tree limits, with spooled output, change detection and journal
/// entries.
///
/// # Errors
///
/// Fails with `E_PROGRAM_NOT_FOUND` or `E_SPAWN` when the tool cannot be
/// started. A non-zero exit code is an outcome, not an error.
pub fn run_tool(ctx: &ExecCtx, run: ToolRun, wait: Option<Duration>) -> Result<ToolOutcome> {
    crate::stage::run::run_tool(ctx, &run_host(ctx), run, wait)
}

/// Produces the final record of a background tool job when the tool ends.
pub trait JobFinisher: Send + Sync {
    /// Builds the final record of the job from the finished run and the data
    /// given in [`ToolRun::finish_data`]. A test job yields a test run record.
    ///
    /// # Errors
    ///
    /// Fails when the output of the tool cannot be processed.
    fn finish(
        &self,
        ctx: &ExecCtx,
        result: &ToolResult,
        data: Option<&serde_json::Value>,
    ) -> Result<Record>;
}

/// Finishers of background tool jobs, registered in the execution context
/// extensions by the crates that start tool runs.
#[derive(Default, Clone)]
pub struct JobFinishers {
    finishers: HashMap<ToolKind, Arc<dyn JobFinisher>>,
}

impl JobFinishers {
    /// Empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the finisher of one tool kind, replacing an earlier one.
    pub fn insert(&mut self, kind: ToolKind, finisher: Arc<dyn JobFinisher>) {
        self.finishers.insert(kind, finisher);
    }

    /// Finisher of one tool kind.
    pub fn get(&self, kind: ToolKind) -> Option<Arc<dyn JobFinisher>> {
        self.finishers.get(&kind).cloned()
    }
}

pub(crate) fn components() -> Components {
    Components {
        control: crate::control::system(),
        spool: crate::output::spool_factory(),
        tracker: crate::track::tracker(),
    }
}

pub(crate) fn run_host(ctx: &ExecCtx) -> Arc<dyn RunHost> {
    crate::jobs::run_host(ctx, components())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

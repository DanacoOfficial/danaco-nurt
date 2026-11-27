// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Program execution for Danaco Nurt.
//!
//! The crate runs programs without a shell, in their own process group or job
//! object, with time, idle, processor and memory limits, captures and decodes
//! their output, manages background jobs, detects the file changes they make
//! and classifies effects outside the file system. Unsafe code is limited to
//! process control calls of the operating system and every block documents its
//! safety conditions.

#![deny(missing_docs)]
#![allow(unsafe_code, reason = "process control calls of the operating system")]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod api;
pub mod elevation;

mod classify;
mod continuation;
mod control;
mod host;
mod iface;
mod jobs;
mod os;
mod output;
mod spawn;
mod stage;
mod track;

pub use api::{
    JobFinisher, JobFinishers, ToolKind, ToolOutcome, ToolResult, ToolRun, process_host, run_tool,
    stages,
};
pub use classify::{
    ArgvClasses, EffectClassifier, EffectMatch, Interactive, OpaqueScript, classify_argv,
    classify_effects,
};
pub use control::{ProcessRef, StopEnd, resume_process, stop_process, suspend_process};
pub use jobs::{Supervision, SupervisorCommand, run_supervisor};

use nurt_contracts::error::Result;
use nurt_contracts::stage::StageRegistrar;

/// Adds the stages of `run`, `jobs`, `stop`, `retry`, `suspend`, `resume`
/// and `remove` on job records to `registrar`, in the order of [`stages`].
///
/// The stages emit the job events `job.started`, `job.output`,
/// `job.progress` and `job.finished`; the execution events `exec.started`
/// and `exec.finished` that enclose them belong to the engine.
///
/// # Errors
///
/// Returns the first error of the registrar: `E_UNKNOWN_COMMAND` when a
/// command is not in the catalog and `E_TYPE` when an accepted record kind
/// is not declared for the command.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::stage::StageImpl;
///
/// let mut registered: Vec<Arc<dyn StageImpl>> = Vec::new();
/// nurt_proc::register(&mut registered)?;
/// let commands: Vec<String> = registered
///     .iter()
///     .map(|stage| stage.descriptor().command)
///     .collect();
/// assert!(commands.iter().any(|command| command == "run"));
/// assert!(commands.iter().any(|command| command == "retry"));
/// assert_eq!(registered.len(), nurt_proc::stages().len());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    stages()
        .into_iter()
        .try_for_each(|stage| registrar.register(stage))
}

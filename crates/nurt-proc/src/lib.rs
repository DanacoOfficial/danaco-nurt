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

mod classify;
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
pub use jobs::{Supervision, SupervisorCommand, run_supervisor};

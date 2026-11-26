// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Runner, statistical analysis and data set tools of the Danaco Nurt
//! benchmark.
//!
//! The benchmark gives the same agent loop the same tasks under four
//! conditions that differ only in the tool: bash, PowerShell 7, Danaco Nurt
//! with the core description, and Danaco Nurt with groups loaded on demand.
//! The protocol is described in `bench/protocol/PROTOCOL.md`.
//!
//! | Module | Content |
//! |---|---|
//! | [`task`], [`snapshot`] | task files `nurt-bench-task/1` and file trees `nurt-fs-snapshot/1` |
//! | [`model`] | model configuration and provider adapters, including the mock model |
//! | [`tool`], [`rpc`], [`process`], [`probe`] | tool executors: shell processes and `nurt serve`; probes |
//! | [`protocol`], [`agent`], [`runner`], [`runlog`] | protocol files, the agent loop, task execution and the run log `nurt-bench-run/1` |
//! | [`oracle`] | evaluation of a run by the oracle of the task set |
//! | [`metrics`], [`stats`], [`analyze`] | measures of a run, statistical tests and result tables |
//! | [`variants`], [`dataset`] | syntax variants of reference programs and the pair data set |

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod agent;
pub mod analyze;
pub mod condition;
pub mod dataset;
pub mod error;
pub mod metrics;
pub mod model;
pub mod oracle;
pub mod probe;
pub mod process;
pub mod protocol;
pub mod rpc;
pub mod runlog;
pub mod runner;
pub mod snapshot;
pub mod stats;
pub mod task;
pub mod tool;
pub mod variants;

#[cfg(test)]
#[cfg(unix)]
mod end_to_end;

pub use condition::Condition;
pub use error::{BenchError, Result};

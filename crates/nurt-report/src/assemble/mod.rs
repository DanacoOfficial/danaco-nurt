// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Assembly of effect reports.
//!
//! A `nurt-report/1` report is assembled from the effects of a plan or an
//! execution: status, targets, summary, changes, bulk summaries of mass
//! operations, notable effects and irreversibility reasons.
//!
//! [`ReportBuilder`] takes the changes of an operation as [`TargetChange`]
//! values, the entries emitted by commands and the journal, and the
//! configuration held by a [`ReportContext`]. It derives the summary, the
//! bulk summary and the notable effects, and returns a [`Report`] that
//! satisfies the rules of the format.
//!
//! [`Report`]: nurt_contracts::report::Report

mod builder;
mod bulk;
mod context;
mod input;
mod summary;

pub use builder::ReportBuilder;
pub use bulk::{BULK_GROUPS, SAMPLE_HEAD, SAMPLE_TAIL, summarize_bulk};
pub use context::ReportContext;
pub use input::{EntryKind, ScopeDirectory, TargetChange};
pub use summary::{count_targets, summarize_changes};

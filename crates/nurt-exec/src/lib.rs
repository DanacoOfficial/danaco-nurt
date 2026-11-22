// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Pipeline executor of Danaco Nurt.
//!
//! The executor runs execution plans as streams of records with parallelism,
//! cancellation, entity tags, assertions and verification, and implements the
//! record stages `filter`, `select`, `sort`, `first`, `last`, `skip`,
//! `count`, `assert`, `verify`, `group`, `summarize`, `distinct`, `parse`
//! and `split`.
//!
//! [`Executor::run`] executes a [`nurt_plan::Plan`] in the environment of an
//! [`ExecEnv`] and returns an [`Outcome`]. Each statement of the plan runs in
//! program order: a named result (`let`) is computed once and kept as a
//! snapshot; every operation runs its pipeline in the execution steps of the
//! language:
//!
//! | Step | Stages | Failure |
//! |---|---|---|
//! | read (`2`), check (`3`) | sources, transformations, assertions | nothing changed |
//! | plan (`4`) | effect stage computes its targets; a preview ends here | nothing changed |
//! | apply (`5`) | effect stage changes the world through one journal transaction | the transaction is rolled back |
//! | after (`5a`) | stages on the records of the effect | the effect stays |
//! | verify (`6`) | `verify` runs its check pipeline | the operation is rolled back |
//!
//! Records flow lazily between the stages before the effect. Per-record
//! expressions of the arguments are evaluated by [`ExprBinding`], pipeline
//! arguments run through [`SubpipelineRunner`], and stages may spread work
//! over the thread pool of the execution with [`ordered_filter_map`] without
//! changing the order of their output.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod context;
mod effect;
mod env;
mod eval;
mod events;
mod executor;
mod flow;
mod grouped;
mod outcome;
mod parallel;
mod runner;
pub mod stages;

pub use env::ExecEnv;
pub use eval::{ExprBinding, is_per_record};
pub use events::SequencedEvents;
pub use executor::Executor;
pub use outcome::{ChangeFacts, EffectReport, OperationOutcome, Outcome, ResultData, StepTiming};
pub use parallel::{BATCH_PER_THREAD, ordered_filter_map};
pub use runner::SubpipelineRunner;

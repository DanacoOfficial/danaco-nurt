// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-exec::stages
// Responsibility:   Record stages of the core command set.
// Dependencies:     aggregate, check, filter, nurt_contracts, order, text
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Record stages of the core command set.
//!
//! The stages `filter`, `select`, `sort`, `first`, `last`, `skip`, `count`,
//! `assert`, `verify`, `group`, `summarize`, `distinct`, `parse` and `split`
//! select, order, aggregate, split and check streams of records. Each stage
//! implements `StageImpl` from `nurt_contracts::stage` with the read
//! interface; [`register`] adds all of them through the registration trait
//! `StageRegistrar` of the stage contract. The executor applies the rollback
//! semantics of `verify`.
//!
//! Stages that only look at one record at a time (`filter`, `select`,
//! `first`, `skip`, `parse`, `split`) are lazy and pull no more records than
//! their output needs. Stages that need the whole input (`sort`, `last`,
//! `assert`, `group`, `summarize`, `distinct`, `verify`) hold it within
//! `limits.max_records` of the context and fail with `E_LIMIT` beyond it.
//!
//! Field arguments (`sort by:`, `distinct by:`, `summarize fields:`) are
//! read from the record by name; expressions are evaluated by the
//! expression evaluator of the context. Values compare by the total order
//! of values of `nurt-types`, so the output of every stage is deterministic
//! for a given input.
//!
//! # Examples
//!
//! ```
//! use std::sync::Arc;
//!
//! use nurt_contracts::stage::StageImpl;
//!
//! let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
//! nurt_exec::stages::register(&mut stages)?;
//! let commands: Vec<String> = stages.iter().map(|stage| stage.descriptor().command).collect();
//! assert_eq!(commands, nurt_exec::stages::COMMANDS);
//! # Ok::<(), nurt_contracts::error::Error>(())
//! ```

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::{StageImpl, StageRegistrar};

mod aggregate;
mod check;
mod common;
mod filter;
mod order;
mod text;

pub use aggregate::{Distinct, Group, Summarize};
pub use check::{Assert, Verify};
pub use filter::{Filter, Select};
pub use order::{Count, First, LIMIT_DETAIL, Last, Skip, Sort};
pub use text::{Parse, Split};

/// Names of the commands implemented by this module, in catalog order.
pub const COMMANDS: [&str; 14] = [
    "filter",
    "select",
    "sort",
    "first",
    "last",
    "skip",
    "count",
    "assert",
    "verify",
    "group",
    "summarize",
    "distinct",
    "parse",
    "split",
];

/// Implementations of the record stages, in the order of [`COMMANDS`].
#[must_use]
pub fn implementations() -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(Filter),
        Arc::new(Select),
        Arc::new(Sort),
        Arc::new(First),
        Arc::new(Last),
        Arc::new(Skip),
        Arc::new(Count),
        Arc::new(Assert),
        Arc::new(Verify),
        Arc::new(Group),
        Arc::new(Summarize),
        Arc::new(Distinct),
        Arc::new(Parse),
        Arc::new(Split),
    ]
}

/// Registers the implementations of the record stages, in the order of
/// [`COMMANDS`].
///
/// # Errors
///
/// Fails with the first error of the registrar, for example when the
/// registry of the engine already holds an implementation of one of the
/// commands for the same input record kinds.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::stage::StageImpl;
///
/// let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
/// nurt_exec::stages::register(&mut stages)?;
/// assert_eq!(stages.len(), 14);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    for implementation in implementations() {
        registrar.register(implementation)?;
    }
    Ok(())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

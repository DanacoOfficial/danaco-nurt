// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test runs and code analysis for Danaco Nurt.
//!
//! The crate runs test frameworks, analyzers and formatters as user programs
//! through `nurt-proc`, parses their machine-readable output into test
//! results and findings, reads coverage reports, computes cyclomatic
//! complexity and duplicated code, and keeps the history of runs and
//! analyses in the state directory.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
#![allow(dead_code, reason = "development interval: parts are joined during integration")]

use std::sync::Arc;

use nurt_contracts::stage::StageImpl;

mod analyze;
mod complexity;
mod coverage;
mod duplication;
mod fingerprint;
mod iface;
mod stage;
mod store;
mod test;

/// Stages of the commands `test.run`, `test.failed`, `test.show`,
/// `test.list`, `analyze.check`, `analyze.show`, `analyze.format` and
/// `analyze.tools`.
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    let mut stages = stage::test::stages();
    stages.extend(stage::analyze::stages());
    stages
}

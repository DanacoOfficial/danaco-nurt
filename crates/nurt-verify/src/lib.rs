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

use std::sync::Arc;

use nurt_contracts::stage::StageImpl;
use nurt_proc::{JobFinishers, ToolKind};

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

/// Finishers of the test runs and analyses that continue in the background.
///
/// A run that does not end within its waiting time continues as a background
/// job. When the tool ends, the job runner of `nurt-proc` looks up
/// [`JobFinishers`] in the extensions of the execution context and passes the
/// finished run to the finisher of its [`ToolKind`]. The finisher parses the
/// output, stores the test run or the analysis in the state directory and
/// returns its header as the final record of the job. Without a registered
/// set the job ends with its execution record only.
///
/// The program that builds the execution context registers the set once, in
/// the extensions given to `ExecCtxBuilder::extensions`.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::stage::Extensions;
/// use nurt_proc::{JobFinishers, ToolKind};
///
/// let mut extensions = Extensions::new();
/// extensions.insert::<JobFinishers>(Arc::new(nurt_verify::job_finishers()));
///
/// let registered = extensions.get::<JobFinishers>();
/// assert!(registered.is_some_and(|set| set.get(ToolKind::Test).is_some()));
/// ```
pub fn job_finishers() -> JobFinishers {
    let mut finishers = JobFinishers::new();
    finishers.insert(ToolKind::Test, stage::test::job_finisher());
    finishers.insert(ToolKind::Analyze, stage::analyze::job_finisher());
    finishers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_finishers_cover_test_runs_and_analyses() {
        let finishers = job_finishers();
        assert!(finishers.get(ToolKind::Test).is_some());
        assert!(finishers.get(ToolKind::Analyze).is_some());
    }
}

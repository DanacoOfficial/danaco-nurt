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

use nurt_contracts::error::Result;
use nurt_contracts::stage::{StageImpl, StageRegistrar};
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

/// Adds the stages of the test and analysis commands to `registrar`.
///
/// The stages are the ones returned by [`stages`], added in the same order.
///
/// # Errors
///
/// Fails with the first error of the registrar, for example
/// `E_UNKNOWN_COMMAND` when a command is missing from its catalog; the
/// stages after the failing one are not added.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::stage::StageImpl;
///
/// let mut registered: Vec<Arc<dyn StageImpl>> = Vec::new();
/// nurt_verify::register(&mut registered)?;
///
/// assert!(registered.iter().any(|stage| stage.descriptor().command == "test.run"));
/// assert!(registered.iter().any(|stage| stage.descriptor().command == "analyze.check"));
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    stages()
        .into_iter()
        .try_for_each(|stage| registrar.register(stage))
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
    use nurt_contracts::error::{Error, ErrorCode};

    use super::*;

    /// Registrar that accepts a fixed number of stages and rejects the next.
    struct Limited {
        accepted: Vec<String>,
        capacity: usize,
    }

    impl StageRegistrar for Limited {
        fn register(&mut self, stage: Arc<dyn StageImpl>) -> Result<()> {
            if self.accepted.len() == self.capacity {
                return Err(Error::new(ErrorCode::UnknownCommand, "unknown command"));
            }
            self.accepted.push(stage.descriptor().command);
            Ok(())
        }
    }

    #[test]
    fn register_adds_every_stage_of_the_crate() {
        let mut registered: Vec<Arc<dyn StageImpl>> = Vec::new();
        register(&mut registered).unwrap();
        let commands: Vec<String> = registered
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect();
        let expected: Vec<String> = stages()
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect();
        assert_eq!(commands, expected);
        for command in [
            "test.run",
            "test.failed",
            "test.show",
            "test.list",
            "analyze.check",
            "analyze.show",
            "analyze.format",
            "analyze.tools",
        ] {
            assert!(commands.iter().any(|name| name == command), "{command}");
        }
    }

    #[test]
    fn register_stops_at_the_first_rejected_stage() {
        let mut registrar = Limited {
            accepted: Vec::new(),
            capacity: 2,
        };
        let error = register(&mut registrar).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownCommand);
        assert_eq!(registrar.accepted.len(), 2);
    }

    #[test]
    fn job_finishers_cover_test_runs_and_analyses() {
        let finishers = job_finishers();
        assert!(finishers.get(ToolKind::Test).is_some());
        assert!(finishers.get(ToolKind::Analyze).is_some());
    }
}

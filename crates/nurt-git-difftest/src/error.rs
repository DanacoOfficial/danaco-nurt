// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-difftest::error
// Responsibility:   Failures of the test runner itself, as opposed to differences found by a scenario.
// Dependencies:     thiserror
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Failures of the test runner itself, as opposed to differences found by a
//! scenario.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Failure of the test runner: an invalid scenario, a failed preparation step or
/// output of the `git` program that cannot be read.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DifftestError {
    /// A file system operation failed.
    #[error("cannot {action} `{}`: {source}", path.display())]
    Io {
        /// Operation, for example `read` or `create directory`.
        action: &'static str,
        /// Path of the entry.
        path: PathBuf,
        /// Error reported by the operating system.
        #[source]
        source: io::Error,
    },
    /// The `git` program could not be started.
    #[error("cannot start git, install git 2.43 or later on the search path: {0}")]
    GitSpawn(#[source] io::Error),
    /// A `git` command ended with an exit code other than the expected one.
    #[error("`git {args}` in `{}` exited with {code}, expected {expected}: {stderr}", dir.display())]
    GitFailed {
        /// Arguments of the command.
        args: String,
        /// Working directory of the command.
        dir: PathBuf,
        /// Exit code; `-1` when the process ended by a signal.
        code: i32,
        /// Expected exit code.
        expected: i32,
        /// Standard error of the command, trimmed.
        stderr: String,
    },
    /// A `git` command printed output the normalizer cannot read.
    #[error("unexpected output of `git {args}`: {detail}")]
    GitOutput {
        /// Arguments of the command.
        args: String,
        /// What was wrong with the output.
        detail: String,
    },
    /// A scenario file is invalid.
    #[error("{}: {message}", path.display())]
    Scenario {
        /// Scenario file.
        path: PathBuf,
        /// What is invalid and how to correct it.
        message: String,
    },
    /// The command catalog cannot be read or does not describe a command.
    #[error("catalog: {0}")]
    Catalog(String),
    /// A corpus repository is unknown or cannot be built.
    #[error("corpus repository `{name}`: {message}")]
    Corpus {
        /// Name of the repository.
        name: String,
        /// What failed.
        message: String,
    },
}

/// Result of the operations of the test runner.
pub type Result<T, E = DifftestError> = std::result::Result<T, E>;

/// Builds a closure that wraps an I/O error with the operation and the path.
pub(crate) fn io_error(
    action: &'static str,
    path: &Path,
) -> impl FnOnce(io::Error) -> DifftestError {
    let path = path.to_path_buf();
    move |source| DifftestError::Io {
        action,
        path,
        source,
    }
}

/// Builds a scenario error for `path`.
pub(crate) fn scenario_error(path: &Path, message: impl Into<String>) -> DifftestError {
    DifftestError::Scenario {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

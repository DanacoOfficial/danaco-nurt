// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::install::test_host
// Responsibility:   Program execution that records the programs it is asked to run, for the launchd installer tests.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Program execution that records the programs it is asked to run, for the
//! launchd installer tests.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{
    ProcessHost, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec,
};
use nurt_contracts::stage::CancelToken;

/// Records every program run and answers with a fixed exit code.
pub(crate) struct RecordingHost {
    exit_code: i32,
    installed: bool,
    calls: Mutex<Vec<ProgramSpec>>,
}

impl RecordingHost {
    /// Every program exists and exits with `exit_code`.
    pub(crate) fn new(exit_code: i32) -> Self {
        Self {
            exit_code,
            installed: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// No program can be found.
    pub(crate) fn missing() -> Self {
        Self {
            installed: false,
            ..Self::new(0)
        }
    }

    /// Programs run so far.
    pub(crate) fn calls(&self) -> Vec<ProgramSpec> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ProcessHost for RecordingHost {
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        if self.installed {
            Ok(Path::new("/bin").join(program))
        } else {
            Err(Error::new(
                ErrorCode::ExternalMissing,
                format!("`{program}` is not in PATH"),
            ))
        }
    }

    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(spec.clone());
        Ok(ProgramOutcome {
            program_path: Path::new("/bin").join(&spec.program),
            exit_code: Some(self.exit_code),
            stderr: if self.exit_code == 0 {
                Vec::new()
            } else {
                b"Bootstrap failed: 5: Input/output error".to_vec()
            },
            ..ProgramOutcome::default()
        })
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "the recording host starts no services",
        ))
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

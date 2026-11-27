// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-ffi::engine
// Responsibility:   Engine handles (`nurt_engine`).
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Engine handles (`nurt_engine`).

use std::process;
use std::sync::{Mutex, Weak};

use nurt_contracts::config::Config;
use nurt_contracts::envelope::Request;
use serde_json::Value;

use crate::backend::{self, Backend, Function, to_json};
use crate::boundary::lock;
use crate::error::CallError;
use crate::exec::{Callback, Exec, Shared};

/// Engine instance behind a `nurt_engine` handle.
///
/// The handle is shared by every thread of the caller; executions started
/// from it keep the engine data they need until they are released.
pub struct Engine {
    backend: Box<dyn Backend>,
    executions: Mutex<Vec<Weak<Shared>>>,
    process: u32,
}

impl Engine {
    /// Opens an engine; `config` is `null` or an object `nurt-config/1`.
    pub(crate) fn open(config: Value) -> Result<Self, CallError> {
        if !config.is_null() {
            serde_json::from_value::<Config>(config.clone()).map_err(|error| {
                CallError::arg(format!(
                    "`config_json` is not a valid configuration: {error}"
                ))
            })?;
        }
        Ok(Self {
            backend: backend::open(&config)?,
            executions: Mutex::new(Vec::new()),
            process: process::id(),
        })
    }

    /// Fails when the engine was opened in another process, which happens
    /// in the child after `fork()` without `exec()`.
    pub(crate) fn check_process(&self) -> Result<(), CallError> {
        if process::id() == self.process {
            Ok(())
        } else {
            Err(CallError::internal(
                "the engine was opened in the parent process; open a new engine after fork",
            ))
        }
    }

    /// Runs an operation and checks the number of its results.
    pub(crate) fn call(
        &self,
        function: Function,
        input: &Value,
    ) -> Result<Vec<Vec<u8>>, CallError> {
        let outputs = self.backend.call(function, input)?;
        let expected = function.outputs().len();
        if outputs.len() == expected {
            Ok(outputs)
        } else {
            Err(CallError::internal(format!(
                "the engine returned {} results for `{}`; expected {expected}",
                outputs.len(),
                function.name()
            )))
        }
    }

    /// Runs a request and returns its response as JSON.
    pub(crate) fn run(&self, request: &Request) -> Result<Vec<u8>, CallError> {
        to_json(&self.backend.run(request)?)
    }

    /// Starts an execution.
    pub(crate) fn start(
        &self,
        request: &Request,
        callback: Option<Callback>,
    ) -> Result<Exec, CallError> {
        let exec = Exec::start(self.backend.as_ref(), request, callback)?;
        let mut executions = lock(&self.executions);
        executions.retain(|shared| shared.strong_count() > 0);
        executions.push(exec.downgrade());
        Ok(exec)
    }

    /// Cancels the running executions and waits until they end.
    pub(crate) fn close(self) {
        let executions = std::mem::take(&mut *lock(&self.executions));
        for shared in executions.iter().filter_map(Weak::upgrade) {
            shared.cancel();
            shared.wait_complete();
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process groups, job objects, limits, signals and suspension.
//!
//! Limits of a tree and how they are enforced:
//!
//! | Limit | Linux | macOS | Windows |
//! |---|---|---|---|
//! | `timeout`, `idle_timeout` | supervisor | supervisor | supervisor |
//! | `max_cpu` | measured tree, `RLIMIT_CPU` on the main process | measured tree, `RLIMIT_CPU` on the main process | `JOB_OBJECT_LIMIT_JOB_TIME`, measured job |
//! | `max_memory` | cgroup v2 `memory.max` when delegated, otherwise measured resident memory with an `RLIMIT_AS` bound | measured resident memory | `JOB_OBJECT_LIMIT_JOB_MEMORY` |
//!
//! A limit reached ends the whole tree: termination is requested, and after
//! the grace time every process of the tree is killed. The memory limit
//! kills at once.

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::CancelToken;

use crate::iface::{OutputSink, ProcessControl, RunningProcess, SpawnRequest, StdinSource};
use crate::os::Tree;

mod process;
mod supervisor;
mod watchdog;

pub use process::{ProcessRef, StopEnd, resume_process, stop_process, suspend_process};

#[cfg(test)]
mod tests;

/// Process control of the operating system the console runs on.
struct SystemControl;

impl ProcessControl for SystemControl {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Arc<dyn OutputSink>,
        cancel: CancelToken,
    ) -> Result<Box<dyn RunningProcess>> {
        Ok(Box::new(start(request, sink, cancel)?))
    }
}

/// Starts the program of `request` and supervises its tree.
fn start(
    request: SpawnRequest,
    sink: Arc<dyn OutputSink>,
    cancel: CancelToken,
) -> Result<supervisor::Supervised> {
    let (tree, streams) = Tree::spawn(&request)?;
    let stdin = match request.stdin {
        StdinSource::Bytes(bytes) => Some(bytes),
        StdinSource::Null | StdinSource::File(_) => None,
    };
    supervisor::Supervised::start(tree, streams, &request.limits, sink, cancel, stdin)
}

/// Process control of this system.
pub(crate) fn system() -> Arc<dyn ProcessControl> {
    Arc::new(SystemControl)
}

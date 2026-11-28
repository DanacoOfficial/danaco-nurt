// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::jobs
// Responsibility:   Run host, job table, supervisor and job events.
// Dependencies:     nurt_contracts, record, state, store
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Run host, job table, supervisor and job events.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::config::Supervisor;
use nurt_contracts::error::Result;
use nurt_contracts::stage::ExecCtx;

use crate::iface::{Components, RunHost};

mod control;
mod events;
mod host;
mod journal;
mod record;
mod runner;
mod state;
mod store;
mod supervisor;
mod table;
mod wire;

#[cfg(test)]
mod tests;

pub(crate) use record::{end_notes, exec_record, external_effect};
pub(crate) use state::{Action, check_action};
pub(crate) use store::JobStore;

/// How runs that continue in the background are supervised, registered by
/// the embedding application in the extensions of the execution context.
///
/// Without a registered value runs are supervised by threads of the calling
/// process, which suits an embedding application or a server that outlives
/// its commands.
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::stage::Extensions;
/// use nurt_proc::{Supervision, SupervisorCommand};
///
/// let mut extensions = Extensions::new();
/// extensions.insert::<Supervision>(Arc::new(Supervision::Detached(SupervisorCommand {
///     program: "/usr/bin/nurt".into(),
///     args: vec!["job-host".into()],
/// })));
/// assert!(extensions.contains::<Supervision>());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Supervision {
    /// The process outlives its commands; threads of the process supervise
    /// the runs.
    InProcess,
    /// The process ends with its command, as the command line does; a
    /// detached supervisor process started on demand with this command line
    /// supervises the runs that may continue in the background.
    Detached(SupervisorCommand),
}

/// Command line that starts the detached supervisor; the started process
/// calls [`run_supervisor`] with the state directory of the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorCommand {
    /// Program file.
    pub program: PathBuf,
    /// Arguments.
    pub args: Vec<OsString>,
}

/// Serves as the detached supervisor of the state directory of `ctx` until
/// no job has run and no run has been requested for a minute. Returns at
/// once when another supervisor serves the directory.
///
/// The context provides the state directory, the configuration, the journal
/// store and the job finishers of tool runs.
///
/// # Errors
///
/// Fails with `E_CONFIG` when the context has no state directory and with
/// `E_IO` when the job table cannot be read.
pub fn run_supervisor(ctx: &ExecCtx) -> Result<()> {
    supervisor::serve(ctx, &crate::api::components())
}

/// Run host for `ctx`: the detached supervisor when the context registers
/// [`Supervision::Detached`] and `run.supervisor` is `auto`, otherwise
/// threads of this process. With `run.supervisor = none` in a process that
/// ends with its command, runs are waited for until they end.
pub(crate) fn run_host(ctx: &ExecCtx, components: Components) -> Arc<dyn RunHost> {
    let detached = match ctx.extensions().get::<Supervision>().as_deref() {
        Some(Supervision::Detached(command)) => Some(command.clone()),
        Some(Supervision::InProcess) | None => None,
    };
    match (detached, ctx.config().run.supervisor) {
        (Some(command), Supervisor::Auto) => Arc::new(host::DetachedHost::new(components, command)),
        (Some(_), Supervisor::Disabled) => Arc::new(host::LocalHost::new(components, false)),
        (None, _) => Arc::new(host::LocalHost::new(components, true)),
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

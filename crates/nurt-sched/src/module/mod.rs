// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface between the background service and its modules.
//!
//! The service process hosts modules, such as the schedule module of this
//! crate and the free space module. The host starts every module on its own
//! thread with [`ServiceModule::start`], which blocks until the module is
//! stopped, and talks to running modules through [`ServiceModule::request`].
//! Modules publish events for connected clients through [`ModuleEvents`].
//!
//! Processes other than the service reach a module through a [`ServiceLink`]
//! registered in the [`Extensions`] of their execution context. The link
//! forwards a [`ModuleRequest`] over the control channel of the service and
//! returns the reply of the module.
//!
//! Requests and events carry JSON bodies, so the host forwards them without
//! knowing the operations of a module.

mod schedule;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::config::Config;
use nurt_contracts::env::Clock;
use nurt_contracts::error::Result;
use nurt_contracts::stage::{CancelToken, ExecCtx, Extensions};
use serde::{Deserialize, Serialize};

pub use schedule::{SCHEDULE_MODULE, ScheduleModule};

/// A module hosted by the background service.
///
/// The host calls [`start`](ServiceModule::start) on a dedicated thread and
/// may call every other method from other threads while `start` runs. A
/// module that panics or returns an error from `start` is restarted by the
/// host; other modules keep running.
pub trait ServiceModule: Send + Sync {
    /// Name of the module, for example `schedule`; requests whose operation
    /// starts with this name followed by `.` or `/` are routed to it.
    fn name(&self) -> &str;

    /// Runs the module until [`ServiceCtx::stop`] is cancelled or
    /// [`stop`](ServiceModule::stop) is called, then returns.
    ///
    /// # Errors
    ///
    /// Returns the error that ended the module, for example `E_IO` when its
    /// state directory cannot be written.
    fn start(&self, ctx: ServiceCtx) -> Result<()>;

    /// Asks a running module to stop; [`start`](ServiceModule::start)
    /// returns soon after. Has no effect on a module that does not run.
    fn stop(&self);

    /// Current health of the module, written into the heartbeat file of the
    /// service.
    fn health(&self) -> ModuleHealth;

    /// Applies a new configuration.
    ///
    /// # Errors
    ///
    /// Returns `E_CONFIG` when the module cannot use the configuration; the
    /// module keeps the previous one.
    fn reload(&self, config: &Config) -> Result<()>;

    /// Handles a request from a client of the service and returns the body
    /// of the reply.
    ///
    /// # Errors
    ///
    /// Returns the error of the operation, for example `E_NOT_FOUND` for an
    /// unknown schedule or `E_UNSUPPORTED` for an unknown operation.
    fn request(&self, request: ModuleRequest) -> Result<serde_json::Value>;

    /// Tells the module that a client connection closed, so that the module
    /// drops registrations made by that client.
    fn client_closed(&self, client: &str);
}

/// Facilities the host gives a module for one start.
#[derive(Clone)]
pub struct ServiceCtx {
    /// Clock of the service.
    pub clock: Arc<dyn Clock>,
    /// Effective configuration at start; later changes arrive through
    /// [`ServiceModule::reload`].
    pub config: Arc<Config>,
    /// State directory of the account the service runs for.
    pub state_dir: PathBuf,
    /// Receiver of events published by the module.
    pub events: Arc<dyn ModuleEvents>,
    /// Stop signal; cancelled by the host when the service stops.
    pub stop: CancelToken,
    /// Facilities specific to a module, registered by type, for example the
    /// [`RunContextFactory`] and the
    /// [`ActionRegistry`](crate::action::ActionRegistry) of the schedule
    /// module.
    pub extensions: Arc<Extensions>,
}

impl fmt::Debug for ServiceCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceCtx")
            .field("state_dir", &self.state_dir)
            .field("stop", &self.stop)
            .finish_non_exhaustive()
    }
}

/// Health of a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModuleHealth {
    /// The module was created and has not started yet.
    Starting,
    /// The module runs normally.
    Running,
    /// The module runs with a problem it reports.
    Degraded {
        /// What is wrong.
        reason: String,
    },
    /// The module stopped because of an error.
    Failed {
        /// The error.
        reason: String,
    },
    /// The module was stopped.
    Stopped,
}

/// A request of a client to a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleRequest {
    /// Operation, for example `schedule.run`.
    pub op: String,
    /// Connection the request came from, assigned by the host; `None` for
    /// requests of the host itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// Arguments of the operation.
    #[serde(default)]
    pub body: serde_json::Value,
}

impl ModuleRequest {
    /// Creates a request without a client.
    #[must_use]
    pub fn new(op: impl Into<String>, body: serde_json::Value) -> Self {
        Self {
            op: op.into(),
            client: None,
            body,
        }
    }

    /// Sets the client connection of the request.
    #[must_use]
    pub fn from_client(mut self, client: impl Into<String>) -> Self {
        self.client = Some(client.into());
        self
    }
}

/// Clients an event is delivered to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", content = "client", rename_all = "snake_case")]
pub enum EventTarget {
    /// Every connected client that subscribed to events.
    All,
    /// One client connection.
    Client(String),
}

/// An event published by a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleEvent {
    /// Event name, for example `schedule.fired`.
    pub event: String,
    /// Recipients.
    pub target: EventTarget,
    /// Content.
    pub data: serde_json::Value,
}

/// Receiver of module events, implemented by the host.
///
/// Publishing never blocks for long: the host queues events and writes them
/// to client connections from its own threads.
pub trait ModuleEvents: Send + Sync {
    /// Accepts one event.
    fn publish(&self, event: ModuleEvent);
}

/// Event receiver that drops all events.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscardModuleEvents;

impl ModuleEvents for DiscardModuleEvents {
    fn publish(&self, _event: ModuleEvent) {}
}

/// Connection of a process to the background service.
///
/// The embedding application registers it in the [`Extensions`] of the
/// execution context as `Arc<dyn ServiceLink>`; the schedule commands use it
/// to notify the service about changed definitions and to run schedules.
pub trait ServiceLink: Send + Sync {
    /// Returns `true` when the service runs and answers.
    fn running(&self) -> bool;

    /// Sends a request to a module of the service and returns the body of
    /// the reply.
    ///
    /// # Errors
    ///
    /// Returns the error replied by the module, or `E_IO` when the service
    /// cannot be reached.
    fn request(&self, request: ModuleRequest) -> Result<serde_json::Value>;
}

/// What the schedule module needs to run the pipeline of one run.
#[derive(Debug, Clone)]
pub struct RunContextRequest {
    /// Schedule identifier.
    pub schedule: String,
    /// Run number within the schedule.
    pub run: u64,
    /// Session recorded in the journal, `schedule:<id>`.
    pub session: String,
    /// Base directory saved when the schedule was created.
    pub base: Option<String>,
    /// Project directories saved when the schedule was created.
    pub project_roots: Vec<String>,
    /// Time limit of the whole run.
    pub timeout: Duration,
    /// Cancellation of the run; cancelled when the schedule is removed or the
    /// service stops.
    pub cancel: CancelToken,
}

/// Builds execution contexts for runs of scheduled pipelines.
///
/// Implemented by the engine facade of the embedding application; the
/// context carries the pipeline runner
/// ([`ExecCtx::pipelines`]) that executes the action as an operation of the
/// session [`RunContextRequest::session`].
pub trait RunContextFactory: Send + Sync {
    /// Builds the context of one run.
    ///
    /// # Errors
    ///
    /// Returns the error that prevents the run, for example `E_CONFIG`.
    fn context(&self, request: &RunContextRequest) -> Result<ExecCtx>;
}

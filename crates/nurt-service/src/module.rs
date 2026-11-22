// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface between the service host and the modules it runs.
//!
//! The host runs every module on its own thread and supervises it: a module
//! that panics, returns from [`ServiceModule::start`] before it was asked to
//! stop, or reports [`ModuleHealth::Failed`] is restarted with a growing
//! delay, and a failure of one module never stops another.
//!
//! A request whose operation starts with the name of a hosted module
//! followed by `.` or `/` is forwarded to that module with
//! [`ModuleRequest::client`] set to the identifier of the connection.
//! Events published through [`ServiceCtx::events`] reach the subscribed
//! connections ([`EventTarget::All`]) or one connection
//! ([`EventTarget::Client`]).

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use nurt_contracts::config::Config;
use nurt_contracts::env::Clock;
use nurt_contracts::error::Result;
use nurt_contracts::stage::{CancelToken, Extensions};
use serde::{Deserialize, Serialize};

use crate::priority::{self, PriorityMode};

pub use nurt_sched::module::SCHEDULE_MODULE;

/// Name of the module that keeps free space on journal volumes.
pub const SPACE_MODULE: &str = "space";

/// A module hosted by the background service.
///
/// The host calls [`start`](ServiceModule::start) on a dedicated thread and
/// may call every other method from other threads while `start` runs.
pub trait ServiceModule: Send + Sync {
    /// Name of the module, for example `schedule`; requests whose operation
    /// starts with this name followed by `.` or `/` are routed to it.
    fn name(&self) -> &str;

    /// Runs the module until the stop token of the context is cancelled or
    /// the host calls `stop`, then returns.
    ///
    /// # Errors
    ///
    /// Returns the error that ended the module. The host restarts it.
    fn start(&self, ctx: ServiceCtx) -> Result<()>;

    /// Asks a running module to stop; [`start`](ServiceModule::start)
    /// returns soon after. Has no effect on a module that does not run.
    fn stop(&self);

    /// Current health of the module, polled by the host once per second.
    fn health(&self) -> ModuleHealth;

    /// Applies a new configuration.
    ///
    /// # Errors
    ///
    /// Returns `E_CONFIG` when the module cannot use the configuration. The
    /// module then keeps the previous one.
    fn reload(&self, config: &Config) -> Result<()>;

    /// Handles a request of a client and returns the body of the reply.
    ///
    /// # Errors
    ///
    /// Returns the error of the operation. An unknown operation gives
    /// `E_UNSUPPORTED`.
    fn request(&self, request: ModuleRequest) -> Result<serde_json::Value>;

    /// Tells the module that a client connection closed, so that it drops
    /// registrations made by that client.
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
    /// Stop signal; cancelled by the host when the service stops or the
    /// module is restarted.
    pub stop: CancelToken,
    /// Facilities registered by the embedding application, by type.
    pub extensions: Arc<Extensions>,
}

impl ServiceCtx {
    /// Changes the CPU and I/O priority of the calling module thread. The
    /// threads of the control channel keep their priority.
    ///
    /// # Errors
    ///
    /// Returns the operating system error, for example when returning to
    /// normal priority is not permitted.
    pub fn set_priority(&self, mode: PriorityMode) -> io::Result<()> {
        priority::set_current_thread(mode)
    }
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
    /// The module cannot continue; the host restarts it.
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
    ///
    /// ```
    /// use nurt_service::module::ModuleRequest;
    ///
    /// let request = ModuleRequest::new("schedule.status", serde_json::json!({})).from_client("c3");
    /// assert_eq!(request.client.as_deref(), Some("c3"));
    /// ```
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
pub trait ModuleEvents: Send + Sync {
    /// Accepts one event and writes it to the connections of its target.
    fn publish(&self, event: ModuleEvent);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::clock::SystemClock;

    struct Discard;

    impl ModuleEvents for Discard {
        fn publish(&self, _event: ModuleEvent) {}
    }

    #[test]
    fn wire_forms_match_the_schedule_module() {
        let request = ModuleRequest::new("schedule.run", json!({"id": "s1"}));
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({"op": "schedule.run", "body": {"id": "s1"}})
        );
        let request: ModuleRequest =
            serde_json::from_value(json!({"op": "schedule/ack", "client": "c2"})).unwrap();
        assert_eq!(request.client.as_deref(), Some("c2"));
        assert_eq!(request.body, serde_json::Value::Null);
        assert_eq!(
            serde_json::to_value(EventTarget::Client("c7".into())).unwrap(),
            json!({"to": "client", "client": "c7"})
        );
        assert_eq!(
            serde_json::to_value(EventTarget::All).unwrap(),
            json!({"to": "all"})
        );
        assert_eq!(
            serde_json::to_value(ModuleHealth::Degraded {
                reason: "slow disk".into()
            })
            .unwrap(),
            json!({"state": "degraded", "reason": "slow disk"})
        );
    }

    #[test]
    fn context_changes_the_priority_of_the_calling_thread() {
        let ctx = ServiceCtx {
            clock: Arc::new(SystemClock::new()),
            config: Arc::new(Config::default()),
            state_dir: PathBuf::from("/var/lib/nurt-state"),
            events: Arc::new(Discard),
            stop: CancelToken::new(),
            extensions: Arc::new(Extensions::new()),
        };
        ctx.events.publish(ModuleEvent {
            event: "space.state".into(),
            target: EventTarget::All,
            data: json!({}),
        });
        assert!(format!("{ctx:?}").contains("/var/lib/nurt-state"));
        let outcome = std::thread::spawn(move || {
            ctx.set_priority(PriorityMode::Background)?;
            ctx.set_priority(PriorityMode::Normal)
        })
        .join()
        .unwrap();
        if let Err(error) = outcome {
            assert!(
                matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
                ),
                "{error}"
            );
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::schedule
// Responsibility:   The schedule module of `nurt-sched` as a module of the service host.
// Dependencies:     nurt_contracts, nurt_sched, sched, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The schedule module of `nurt-sched` as a module of the service host.
//!
//! The schedule crate defines its own module interface because it cannot
//! depend on the host. [`ScheduleService`] translates the host interface to
//! it: the context, requests, health values and published events have the
//! same fields on both sides.

use std::sync::Arc;

use nurt_contracts::config::Config;
use nurt_contracts::error::Result;
use nurt_sched::module as sched;
use nurt_sched::module::ServiceModule as _;
use serde_json::Value;

use crate::module::{
    EventTarget, ModuleEvent, ModuleEvents, ModuleHealth, ModuleRequest, ServiceCtx, ServiceModule,
};

/// The schedule module hosted by the service.
///
/// ```
/// use nurt_service::ScheduleService;
/// use nurt_service::module::{SCHEDULE_MODULE, ServiceModule};
///
/// let module = ScheduleService::new();
/// assert_eq!(module.name(), SCHEDULE_MODULE);
/// ```
#[derive(Debug, Default)]
pub struct ScheduleService {
    inner: sched::ScheduleModule,
}

impl ScheduleService {
    /// Module with an empty schedule state; definitions are read from the
    /// state directory at start.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl From<sched::ScheduleModule> for ScheduleService {
    fn from(inner: sched::ScheduleModule) -> Self {
        Self { inner }
    }
}

impl ServiceModule for ScheduleService {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn start(&self, ctx: ServiceCtx) -> Result<()> {
        self.inner.start(sched::ServiceCtx {
            clock: ctx.clock,
            config: ctx.config,
            state_dir: ctx.state_dir,
            events: Arc::new(EventBridge(ctx.events)),
            stop: ctx.stop,
            extensions: ctx.extensions,
        })
    }

    fn stop(&self) {
        self.inner.stop();
    }

    fn health(&self) -> ModuleHealth {
        health(self.inner.health())
    }

    fn reload(&self, config: &Config) -> Result<()> {
        self.inner.reload(config)
    }

    fn request(&self, request: ModuleRequest) -> Result<Value> {
        self.inner.request(sched::ModuleRequest {
            op: request.op,
            client: request.client,
            body: request.body,
        })
    }

    fn client_closed(&self, client: &str) {
        self.inner.client_closed(client);
    }
}

fn health(health: sched::ModuleHealth) -> ModuleHealth {
    match health {
        sched::ModuleHealth::Starting => ModuleHealth::Starting,
        sched::ModuleHealth::Running => ModuleHealth::Running,
        sched::ModuleHealth::Degraded { reason } => ModuleHealth::Degraded { reason },
        sched::ModuleHealth::Failed { reason } => ModuleHealth::Failed { reason },
        sched::ModuleHealth::Stopped => ModuleHealth::Stopped,
    }
}

/// Passes events of the schedule module to the host.
struct EventBridge(Arc<dyn ModuleEvents>);

impl sched::ModuleEvents for EventBridge {
    fn publish(&self, event: sched::ModuleEvent) {
        let target = match event.target {
            sched::EventTarget::All => EventTarget::All,
            sched::EventTarget::Client(client) => EventTarget::Client(client),
        };
        self.0.publish(ModuleEvent {
            event: event.event,
            target,
            data: event.data,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use sched::ModuleEvents as _;

    #[derive(Default)]
    struct Collected(Mutex<Vec<ModuleEvent>>);

    impl ModuleEvents for Collected {
        fn publish(&self, event: ModuleEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn health_values_and_event_targets_are_translated() {
        assert_eq!(
            health(sched::ModuleHealth::Starting),
            ModuleHealth::Starting
        );
        assert_eq!(health(sched::ModuleHealth::Running), ModuleHealth::Running);
        assert_eq!(health(sched::ModuleHealth::Stopped), ModuleHealth::Stopped);
        assert_eq!(
            health(sched::ModuleHealth::Degraded {
                reason: "slow disk".into()
            }),
            ModuleHealth::Degraded {
                reason: "slow disk".into()
            }
        );
        assert_eq!(
            health(sched::ModuleHealth::Failed {
                reason: "no state".into()
            }),
            ModuleHealth::Failed {
                reason: "no state".into()
            }
        );
        let collected = Arc::new(Collected::default());
        let bridge = EventBridge(Arc::clone(&collected) as Arc<dyn ModuleEvents>);
        bridge.publish(sched::ModuleEvent {
            event: "schedule.fired".into(),
            target: sched::EventTarget::Client("c2".into()),
            data: serde_json::json!({"id": "s1"}),
        });
        bridge.publish(sched::ModuleEvent {
            event: "schedule.finished".into(),
            target: sched::EventTarget::All,
            data: Value::Null,
        });
        let events = collected.0.lock().unwrap();
        assert_eq!(events[0].target, EventTarget::Client("c2".into()));
        assert_eq!(events[0].data["id"], "s1");
        assert_eq!(events[1].target, EventTarget::All);
    }

    #[test]
    fn unknown_operation_reaches_the_schedule_module() {
        let module = ScheduleService::from(sched::ScheduleModule::new());
        assert_eq!(module.name(), "schedule");
        let error = module
            .request(ModuleRequest::new(
                "schedule.frobnicate",
                serde_json::json!({}),
            ))
            .unwrap_err();
        assert_eq!(error.code, nurt_contracts::error::ErrorCode::Unsupported);
        module.client_closed("c1");
        module.stop();
        assert!(module.reload(&Config::default()).is_ok());
        let _ = module.health();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Mock modules `space` and `schedule`.
//!
//! The operations follow the schedule module of `nurt-sched`: `.status`,
//! `.changed`, `.run` with `E_JOB_STATE` while the same schedule runs,
//! `/register_action` and `/ack`. A gate holds `.status` and `.run` until
//! the test releases it, which simulates a blocked module.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use nurt_contracts::config::Config;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{CancelReason, CancelToken};
use nurt_service::module::{
    EventTarget, ModuleEvent, ModuleEvents, ModuleHealth, ModuleRequest, SCHEDULE_MODULE,
    SPACE_MODULE, ServiceCtx, ServiceModule,
};
use serde_json::{Value, json};

/// Behaviour of [`MockModule::start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Behaviour {
    /// Runs until stopped.
    Normal,
    /// Panics at every start.
    PanicOnStart,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Gate {
    held: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn pass(&self) {
        let mut held = lock(&self.held);
        while *held {
            held = self
                .changed
                .wait(held)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn set(&self, held: bool) {
        *lock(&self.held) = held;
        self.changed.notify_all();
    }
}

/// Module that records the calls of the host.
pub(crate) struct MockModule {
    name: &'static str,
    behaviour: Behaviour,
    gate: Gate,
    starts: AtomicUsize,
    reloads: AtomicUsize,
    changes: AtomicUsize,
    stop: Mutex<Option<CancelToken>>,
    events: Mutex<Option<Arc<dyn ModuleEvents>>>,
    running: Mutex<HashSet<String>>,
    remote: Mutex<HashMap<String, String>>,
    pending: Mutex<HashSet<String>>,
    acks: Mutex<Vec<(String, String)>>,
    tokens: AtomicUsize,
}

impl MockModule {
    fn new(name: &'static str, behaviour: Behaviour) -> Arc<Self> {
        Arc::new(Self {
            name,
            behaviour,
            gate: Gate::default(),
            starts: AtomicUsize::new(0),
            reloads: AtomicUsize::new(0),
            changes: AtomicUsize::new(0),
            stop: Mutex::new(None),
            events: Mutex::new(None),
            running: Mutex::default(),
            remote: Mutex::default(),
            pending: Mutex::default(),
            acks: Mutex::default(),
            tokens: AtomicUsize::new(0),
        })
    }

    /// Mock of the space module.
    pub(crate) fn space(behaviour: Behaviour) -> Arc<Self> {
        Self::new(SPACE_MODULE, behaviour)
    }

    /// Mock of the schedule module.
    pub(crate) fn schedule() -> Arc<Self> {
        Self::new(SCHEDULE_MODULE, Behaviour::Normal)
    }

    /// Holds `.status` and `.run` requests until [`release`](Self::release).
    pub(crate) fn hold(&self) {
        self.gate.set(true);
    }

    /// Lets held requests continue.
    pub(crate) fn release(&self) {
        self.gate.set(false);
    }

    pub(crate) fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    pub(crate) fn reloads(&self) -> usize {
        self.reloads.load(Ordering::SeqCst)
    }

    pub(crate) fn changes(&self) -> usize {
        self.changes.load(Ordering::SeqCst)
    }

    pub(crate) fn is_running(&self, id: &str) -> bool {
        lock(&self.running).contains(id)
    }

    pub(crate) fn acks(&self) -> Vec<(String, String)> {
        lock(&self.acks).clone()
    }

    /// Whether a connection has registered the remote action kind.
    pub(crate) fn registered(&self, kind: &str) -> bool {
        lock(&self.remote).contains_key(kind)
    }

    /// Whether the module runs and can publish events.
    pub(crate) fn started(&self) -> bool {
        lock(&self.events).is_some()
    }

    /// Fires a schedule whose action has the remote kind: the event goes to
    /// the connection that registered the kind and the token for `/ack` is
    /// returned; without such a connection `schedule.skipped` goes to all
    /// subscribers.
    pub(crate) fn fire(&self, kind: &str) -> Option<String> {
        let events = lock(&self.events).clone()?;
        let owner = lock(&self.remote).get(kind).cloned();
        match owner {
            Some(client) => {
                let token = format!("t{}", self.tokens.fetch_add(1, Ordering::SeqCst) + 1);
                lock(&self.pending).insert(token.clone());
                events.publish(ModuleEvent {
                    event: "schedule.fired".to_owned(),
                    target: EventTarget::Client(client),
                    data: json!({ "id": "s1", "run": 1, "action_kind": kind, "ack_token": token }),
                });
                Some(token)
            }
            None => {
                events.publish(ModuleEvent {
                    event: "schedule.skipped".to_owned(),
                    target: EventTarget::All,
                    data: json!({ "id": "s1", "reason": "action_unavailable" }),
                });
                None
            }
        }
    }

    fn run_schedule(&self, body: &Value) -> Result<Value> {
        let id = body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !lock(&self.running).insert(id.clone()) {
            return Err(Error::new(
                ErrorCode::JobState,
                format!("the schedule `{id}` is already running; wait for its end"),
            )
            .with_detail("actual", json!(format!("{id}#1"))));
        }
        self.gate.pass();
        lock(&self.running).remove(&id);
        Ok(json!({ "run": { "schedule": id, "state": "ok" }, "output": [] }))
    }

    fn register(&self, request: &ModuleRequest) -> Result<Value> {
        let Some(client) = &request.client else {
            return Err(Error::new(
                ErrorCode::Arg,
                "a remote action kind needs a client connection",
            ));
        };
        let kind = request
            .body
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        lock(&self.remote).insert(kind.clone(), client.clone());
        Ok(json!({ "kind": kind }))
    }

    fn ack(&self, body: &Value) -> Result<Value> {
        let token = body
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !lock(&self.pending).remove(&token) {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("no fired action has the token `{token}`"),
            ));
        }
        let state = body
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        lock(&self.acks).push((token, state));
        Ok(json!({}))
    }
}

impl ServiceModule for MockModule {
    fn name(&self) -> &str {
        self.name
    }

    fn start(&self, ctx: ServiceCtx) -> Result<()> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.behaviour == Behaviour::PanicOnStart {
            panic!("injected failure of the module `{}`", self.name);
        }
        *lock(&self.stop) = Some(ctx.stop.clone());
        *lock(&self.events) = Some(Arc::clone(&ctx.events));
        let _ = ctx.stop.wait_timeout(Duration::MAX);
        lock(&self.events).take();
        Ok(())
    }

    fn stop(&self) {
        if let Some(stop) = lock(&self.stop).as_ref() {
            stop.cancel(CancelReason::Shutdown);
        }
    }

    fn health(&self) -> ModuleHealth {
        ModuleHealth::Running
    }

    fn reload(&self, _config: &Config) -> Result<()> {
        self.reloads.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn request(&self, request: ModuleRequest) -> Result<Value> {
        let local = request.op.strip_prefix(self.name).unwrap_or_default();
        match local {
            ".status" => {
                self.gate.pass();
                Ok(json!({ "module": self.name }))
            }
            ".changed" => {
                self.changes.fetch_add(1, Ordering::SeqCst);
                Ok(json!({ "schedules": 0 }))
            }
            ".run" => self.run_schedule(&request.body),
            "/register_action" => self.register(&request),
            "/ack" => self.ack(&request.body),
            _ => Err(Error::new(
                ErrorCode::Unsupported,
                format!(
                    "the module `{}` has no operation `{}`",
                    self.name, request.op
                ),
            )),
        }
    }

    fn client_closed(&self, client: &str) {
        lock(&self.remote).retain(|_, owner| owner != client);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The schedule module of the background service.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use nurt_contracts::config::Config;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{CancelReason, CancelToken, StageData, StageInput};
use nurt_contracts::value::Instant;
use serde_json::{Value as Json, json};

use super::{
    EventTarget, ModuleEvent, ModuleHealth, ModuleRequest, RunContextFactory, RunContextRequest,
    ServiceCtx, ServiceModule,
};
use crate::action::{ActionRegistry, ActionRun, Outcome, PIPELINE_KIND};
use crate::engine::{EngineEvent, Launch, Scheduler};
use crate::errors::{bad_request, failed};
use crate::settings::ScheduleSettings;
use crate::store::{Fingerprint, Overlap, RunRecord, RunState, RunTrigger, StateFile, Store};
use crate::trigger::time::{self, instant_from_wall, std_duration};

/// Name of the schedule module.
pub const SCHEDULE_MODULE: &str = "schedule";

const POLL: Duration = Duration::from_secs(1);
const DUE_POLL: Duration = Duration::from_millis(100);
const SESSION_PREFIX: &str = "schedule:";
/// End states a client can acknowledge a run with.
const ACK_STATES: [RunState; 5] = [
    RunState::Ok,
    RunState::Exit,
    RunState::Timeout,
    RunState::Stopped,
    RunState::Failed,
];

/// The schedule module: runs schedules at their run times.
///
/// The module reads the definitions written by the schedule commands, keeps
/// the run state in `state.json` and executes actions on their own threads.
/// It re-reads the definitions after `schedule.changed` and whenever the
/// definition files change, so an undo through the journal takes effect
/// without a notification.
///
/// ```
/// use nurt_sched::module::{ModuleHealth, ScheduleModule, ServiceModule};
///
/// let module = ScheduleModule::new();
/// assert_eq!(module.name(), "schedule");
/// assert_eq!(module.health(), ModuleHealth::Starting);
/// ```
#[derive(Debug, Clone)]
pub struct ScheduleModule {
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    core: Mutex<Option<Core>>,
    health: Mutex<ModuleHealth>,
    stop: Mutex<Option<CancelToken>>,
    wake: Mutex<Option<CancelToken>>,
    remote: Mutex<BTreeMap<String, String>>,
    acks: Mutex<BTreeMap<String, PendingAck>>,
}

#[derive(Debug)]
struct PendingAck {
    client: String,
    sender: Sender<Ack>,
}

#[derive(Debug, Clone)]
struct Ack {
    state: RunState,
    summary: Option<String>,
    exit_code: Option<i64>,
}

type Finished = (RunRecord, Vec<Json>);

struct Core {
    ctx: ServiceCtx,
    store: Store,
    scheduler: Scheduler,
    settings: ScheduleSettings,
    registry: Option<Arc<ActionRegistry>>,
    fingerprint: Fingerprint,
    tokens: BTreeMap<String, CancelToken>,
    waiters: BTreeMap<String, Sender<Finished>>,
    ack_sequence: u64,
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core")
            .field("store", &self.store)
            .field("running", &self.tokens.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

enum Work {
    Pipeline,
    Local(Arc<dyn crate::action::ScheduleAction>),
    Remote {
        token: String,
        client: String,
        receiver: Receiver<Ack>,
    },
    Unavailable,
}

struct Job {
    launch: Launch,
    work: Work,
    cancel: CancelToken,
    ctx: ServiceCtx,
    store: Store,
    ack_timeout: Duration,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Default for ScheduleModule {
    fn default() -> Self {
        Self::new()
    }
}

impl ScheduleModule {
    /// A module that has not started.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                core: Mutex::new(None),
                health: Mutex::new(ModuleHealth::Starting),
                stop: Mutex::new(None),
                wake: Mutex::new(None),
                remote: Mutex::new(BTreeMap::new()),
                acks: Mutex::new(BTreeMap::new()),
            }),
        }
    }
}

impl ServiceModule for ScheduleModule {
    fn name(&self) -> &str {
        SCHEDULE_MODULE
    }

    fn start(&self, ctx: ServiceCtx) -> Result<()> {
        let stop = ctx.stop.child();
        *lock(&self.shared.stop) = Some(stop.clone());
        let clock = Arc::clone(&ctx.clock);
        Shared::open(&self.shared, ctx)?;
        while !stop.is_cancelled() {
            // The wake token exists before the step, so a request handled
            // during the step shortens the following sleep.
            let wake = stop.child();
            *lock(&self.shared.wake) = Some(wake.clone());
            let wait = Shared::step(&self.shared);
            if clock.sleep(wait, &wake).is_err() && stop.is_cancelled() {
                break;
            }
        }
        self.shared.close();
        Ok(())
    }

    fn stop(&self) {
        if let Some(stop) = lock(&self.shared.stop).as_ref() {
            stop.cancel(CancelReason::Shutdown);
        }
    }

    fn health(&self) -> ModuleHealth {
        lock(&self.shared.health).clone()
    }

    fn reload(&self, config: &Config) -> Result<()> {
        let mut guard = lock(&self.shared.core);
        if let Some(core) = guard.as_mut() {
            core.ctx.config = Arc::new(config.clone());
            core.settings = ScheduleSettings::from_extensions(&core.ctx.extensions);
            core.scheduler.set_settings(core.settings.clone());
            self.shared.reload(core)?;
            drop(guard);
            self.shared.wake();
        }
        Ok(())
    }

    fn request(&self, request: ModuleRequest) -> Result<Json> {
        match request.op.as_str() {
            "schedule.changed" => self.changed(),
            "schedule.run" => self.run(&request.body),
            "schedule.stop" => self.stop_runs(&request.body),
            "schedule.status" => self.status(),
            "schedule/register_action" => self.register(&request),
            "schedule/ack" => self.ack(&request),
            other => Err(Error::new(
                ErrorCode::Unsupported,
                format!("the schedule module has no operation `{other}`"),
            )),
        }
    }

    fn client_closed(&self, client: &str) {
        lock(&self.shared.remote).retain(|_, owner| owner != client);
        lock(&self.shared.acks).retain(|_, pending| pending.client != client);
    }
}

impl ScheduleModule {
    fn changed(&self) -> Result<Json> {
        let count = self.shared.with_core(|shared, core| {
            let count = shared.reload(core)?;
            shared.publish(core);
            Ok(count)
        })?;
        self.shared.wake();
        Ok(json!({ "schedules": count }))
    }

    fn run(&self, body: &Json) -> Result<Json> {
        let id = required_text(body, "id")?;
        let trigger = match body.get("trigger").and_then(Json::as_str) {
            Some("run_now") => RunTrigger::RunNow,
            _ => RunTrigger::Manual,
        };
        let (sender, receiver) = mpsc::channel();
        let started = self.shared.with_core(|shared, core| {
            let now = core.now();
            let launch = core.scheduler.run_now(&id, trigger, now)?;
            let run = launch.id();
            // The creation of a schedule does not wait for its first run; the
            // record of that run is in the history.
            if trigger != RunTrigger::RunNow {
                core.waiters.insert(run.clone(), sender);
            }
            Shared::dispatch(shared, core, launch);
            shared.publish(core);
            Ok(run)
        })?;
        if trigger == RunTrigger::RunNow {
            return Ok(json!({ "started": started }));
        }
        let (record, output) = receiver.recv().map_err(|_| {
            Error::new(
                ErrorCode::Cancelled,
                "the service stopped before the run ended",
            )
        })?;
        Ok(json!({ "run": record, "output": output }))
    }

    fn stop_runs(&self, body: &Json) -> Result<Json> {
        let id = required_text(body, "id")?;
        let stopped = self.shared.with_core(|shared, core| {
            let stopped = core.scheduler.stop(&id);
            for run in &stopped {
                if let Some(token) = core.tokens.get(run) {
                    token.cancel(CancelReason::Requested);
                }
            }
            shared.publish(core);
            Ok(stopped)
        })?;
        Ok(json!({ "stopped": stopped }))
    }

    fn status(&self) -> Result<Json> {
        let kinds: Vec<String> = lock(&self.shared.remote).keys().cloned().collect();
        self.shared.with_core(|_, core| {
            Ok(json!({
                "schedules": core.scheduler.len(),
                "running": core.scheduler.running(),
                "action_kinds": kinds,
            }))
        })
    }

    fn register(&self, request: &ModuleRequest) -> Result<Json> {
        let client = request
            .client
            .clone()
            .ok_or_else(|| bad_request("`schedule/register_action` needs a client connection"))?;
        let kind = required_text(&request.body, "kind")?;
        lock(&self.shared.remote).insert(kind.clone(), client);
        Ok(json!({ "kind": kind }))
    }

    fn ack(&self, request: &ModuleRequest) -> Result<Json> {
        let body = &request.body;
        let token = required_text(body, "token")?;
        let state_text = body.get("state").and_then(Json::as_str).unwrap_or("ok");
        let state = RunState::parse(state_text)
            .filter(|state| ACK_STATES.contains(state))
            .ok_or_else(|| {
                bad_request(format!(
                    "unknown run state `{state_text}`; use ok, exit, timeout, stopped or failed"
                ))
            })?;
        let pending = {
            let mut acks = lock(&self.shared.acks);
            let owned = acks.get(&token).is_none_or(|pending| {
                request
                    .client
                    .as_ref()
                    .is_none_or(|client| *client == pending.client)
            });
            if !owned {
                return Err(Error::new(
                    ErrorCode::NotFound,
                    format!("no run of this client waits for the acknowledgement `{token}`"),
                ));
            }
            acks.remove(&token)
        }
        .ok_or_else(|| {
            Error::new(
                ErrorCode::NotFound,
                format!("no run waits for the acknowledgement `{token}`"),
            )
        })?;
        let ack = Ack {
            state,
            summary: body
                .get("summary")
                .and_then(Json::as_str)
                .map(str::to_owned),
            exit_code: body.get("exit_code").and_then(Json::as_i64),
        };
        let _ = pending.sender.send(ack);
        Ok(json!({}))
    }
}

impl Shared {
    fn open(shared: &Arc<Self>, ctx: ServiceCtx) -> Result<()> {
        let store = Store::new(&ctx.state_dir);
        let (state, problem) = match store.read_state() {
            Ok(state) => (state, None),
            Err(error) if error.code() == ErrorCode::Format => {
                (StateFile::default(), Some(error.message.clone()))
            }
            Err(error) => {
                *lock(&shared.health) = ModuleHealth::Failed {
                    reason: error.message.clone(),
                };
                return Err(error);
            }
        };
        let settings = ScheduleSettings::from_extensions(&ctx.extensions);
        let registry = ctx.extensions.get::<ActionRegistry>();
        let mut core = Core {
            scheduler: Scheduler::new(settings.clone(), state),
            ctx,
            store,
            settings,
            registry,
            fingerprint: Vec::new(),
            tokens: BTreeMap::new(),
            waiters: BTreeMap::new(),
            ack_sequence: 0,
        };
        *lock(&shared.health) = ModuleHealth::Running;
        shared.reload(&mut core)?;
        if let Some(reason) = problem {
            *lock(&shared.health) = ModuleHealth::Degraded { reason };
        }
        *lock(&shared.core) = Some(core);
        Ok(())
    }

    fn close(&self) {
        if let Some(mut core) = lock(&self.core).take() {
            for token in core.tokens.values() {
                token.cancel(CancelReason::Shutdown);
            }
            core.waiters.clear();
            self.persist(&mut core);
        }
        *lock(&self.health) = ModuleHealth::Stopped;
    }

    fn with_core<T>(
        self: &Arc<Self>,
        work: impl FnOnce(&Arc<Self>, &mut Core) -> Result<T>,
    ) -> Result<T> {
        let mut guard = lock(&self.core);
        let core = guard
            .as_mut()
            .ok_or_else(|| failed("the schedule module is not running"))?;
        work(self, core)
    }

    fn step(shared: &Arc<Self>) -> Duration {
        let mut guard = lock(&shared.core);
        let Some(core) = guard.as_mut() else {
            return POLL;
        };
        let now = core.now();
        if core.store.fingerprint() != core.fingerprint
            && let Err(error) = shared.reload(core)
        {
            *lock(&shared.health) = ModuleHealth::Degraded {
                reason: error.message.clone(),
            };
        }
        for launch in core.scheduler.tick(now) {
            Self::dispatch(shared, core, launch);
        }
        shared.publish(core);
        wait_until(core.scheduler.next_wake(), now)
    }

    fn reload(&self, core: &mut Core) -> Result<usize> {
        core.fingerprint = core.store.fingerprint();
        let definitions = core.store.load_definitions()?;
        let now = core.now();
        let broken = core.scheduler.load(definitions.items, now);
        let mut problems: Vec<String> = definitions
            .invalid
            .iter()
            .map(|(path, problem)| format!("{}: {problem}", path.display()))
            .collect();
        problems.extend(
            broken
                .iter()
                .map(|id| format!("schedule `{id}` has an invalid trigger")),
        );
        *lock(&self.health) = if problems.is_empty() {
            ModuleHealth::Running
        } else {
            ModuleHealth::Degraded {
                reason: problems.join("; "),
            }
        };
        Ok(core.scheduler.len())
    }

    fn dispatch(shared: &Arc<Self>, core: &mut Core, launch: Launch) {
        let run = launch.id();
        let cancel = core.ctx.stop.child();
        core.tokens.insert(run.clone(), cancel.clone());
        let work = shared.work_for(core, &launch);
        let job = Job {
            launch,
            work,
            cancel,
            ctx: core.ctx.clone(),
            store: core.store.clone(),
            ack_timeout: core.settings.action_ack_timeout,
        };
        let worker = Arc::clone(shared);
        let finished_run = run.clone();
        let spawned = thread::Builder::new()
            .name(format!("schedule {run}"))
            .spawn(move || {
                let (outcome, output) = job.execute();
                worker.complete(&finished_run, &outcome, output);
            });
        if let Err(error) = spawned {
            let outcome = Outcome::new(RunState::Failed, format!("the run cannot start: {error}"));
            let now = core.now();
            core.tokens.remove(&run);
            core.waiters.remove(&run);
            if let Some(queued) = core.scheduler.finish(&run, &outcome, now) {
                Self::dispatch(shared, core, queued);
            }
        }
    }

    fn work_for(&self, core: &mut Core, launch: &Launch) -> Work {
        let kind = launch.definition.action_kind.as_str();
        if kind == PIPELINE_KIND {
            return Work::Pipeline;
        }
        if let Some(action) = core
            .registry
            .as_ref()
            .and_then(|registry| registry.get(kind))
        {
            return Work::Local(action);
        }
        let Some(client) = lock(&self.remote).get(kind).cloned() else {
            return Work::Unavailable;
        };
        core.ack_sequence += 1;
        let token = format!("{}:{}", launch.id(), core.ack_sequence);
        let (sender, receiver) = mpsc::channel();
        lock(&self.acks).insert(
            token.clone(),
            PendingAck {
                client: client.clone(),
                sender,
            },
        );
        Work::Remote {
            token,
            client,
            receiver,
        }
    }

    fn complete(self: &Arc<Self>, run: &str, outcome: &Outcome, output: Vec<Json>) {
        let mut guard = lock(&self.core);
        let Some(core) = guard.as_mut() else {
            return;
        };
        core.tokens.remove(run);
        let now = core.now();
        let schedule = run.rsplit_once('#').map_or(run, |(id, _)| id).to_owned();
        let queued = core.scheduler.finish(run, outcome, now);
        let reply = core.waiters.remove(run).and_then(|waiter| {
            core.scheduler
                .history(&schedule)
                .iter()
                .rev()
                .find(|record| record.id() == run)
                .map(|record| (waiter, record.clone()))
        });
        if let Some(queued) = queued {
            Self::dispatch(self, core, queued);
        }
        self.publish(core);
        drop(guard);
        if let Some((waiter, record)) = reply {
            let _ = waiter.send((record, output));
        }
        self.wake();
    }

    fn publish(&self, core: &mut Core) {
        for event in core.scheduler.take_events() {
            let (name, data) = match event {
                EngineEvent::Fired(launch) => {
                    if core.is_remote(&launch.definition.action_kind) {
                        continue;
                    }
                    ("schedule.fired", fired_data(&launch, None))
                }
                EngineEvent::Finished(record) => {
                    ("schedule.finished", json!({ "schedule_run": record }))
                }
                EngineEvent::Skipped(record) => (
                    "schedule.skipped",
                    json!({
                        "id": record.schedule,
                        "scheduled_for": record.scheduled_for,
                        "reason": record.reason.map(|reason| reason.as_str()),
                    }),
                ),
            };
            core.ctx.events.publish(ModuleEvent {
                event: name.to_owned(),
                target: EventTarget::All,
                data,
            });
        }
        self.persist(core);
    }

    fn persist(&self, core: &mut Core) {
        if core.scheduler.take_dirty()
            && let Err(error) = core.store.write_state(core.scheduler.state())
        {
            *lock(&self.health) = ModuleHealth::Degraded {
                reason: error.message.clone(),
            };
        }
    }

    fn wake(&self) {
        if let Some(wake) = lock(&self.wake).as_ref() {
            wake.cancel(CancelReason::Requested);
        }
    }
}

impl Core {
    fn now(&self) -> Instant {
        instant_from_wall(self.ctx.clock.now())
    }

    fn is_remote(&self, kind: &str) -> bool {
        kind != PIPELINE_KIND
            && self
                .registry
                .as_ref()
                .is_none_or(|registry| registry.get(kind).is_none())
    }
}

impl Job {
    fn execute(self) -> (Outcome, Vec<Json>) {
        match &self.work {
            Work::Pipeline => self.run_pipeline(),
            Work::Local(action) => {
                let run = ActionRun {
                    schedule: self.launch.schedule.clone(),
                    run: self.launch.run,
                    label: self.launch.definition.label.clone(),
                };
                let args = self
                    .launch
                    .definition
                    .action_args
                    .clone()
                    .unwrap_or(Json::Null);
                (action.execute(&run, &args, &self.cancel), Vec::new())
            }
            Work::Remote {
                token,
                client,
                receiver,
            } => (self.run_remote(token, client, receiver), Vec::new()),
            Work::Unavailable => (
                Outcome::new(
                    RunState::Skipped,
                    format!(
                        "no connected client provides the action kind `{}`",
                        self.launch.definition.action_kind
                    ),
                ),
                Vec::new(),
            ),
        }
    }

    fn run_remote(&self, token: &str, client: &str, receiver: &Receiver<Ack>) -> Outcome {
        self.ctx.events.publish(ModuleEvent {
            event: "schedule.fired".to_owned(),
            target: EventTarget::Client(client.to_owned()),
            data: fired_data(&self.launch, Some(token)),
        });
        match receiver.recv_timeout(self.ack_timeout) {
            Ok(ack) => {
                let mut outcome = Outcome::new(
                    ack.state,
                    ack.summary
                        .unwrap_or_else(|| "the client ran the action".to_owned()),
                );
                outcome.exit_code = ack.exit_code;
                outcome
            }
            Err(_) => Outcome::new(
                RunState::Skipped,
                "no client acknowledged the action in time",
            ),
        }
    }

    fn run_pipeline(&self) -> (Outcome, Vec<Json>) {
        let definition = &self.launch.definition;
        let lock = match self.store.try_run_lock(&definition.id) {
            Ok(lock) => lock,
            Err(error) => return (Outcome::from_error(&error), Vec::new()),
        };
        if lock.is_none() && definition.overlap != Overlap::Parallel {
            return (
                Outcome::new(
                    RunState::Failed,
                    format!("another process runs schedule `{}`", definition.id),
                ),
                Vec::new(),
            );
        }
        match self.pipeline_output() {
            Ok(records) => {
                let outcome = Outcome::from_output(&records);
                let output = records
                    .iter()
                    .filter_map(|record| serde_json::to_value(record).ok())
                    .collect();
                (outcome, output)
            }
            Err(error) => (Outcome::from_error(&error), Vec::new()),
        }
    }

    fn pipeline_output(&self) -> Result<Vec<nurt_contracts::value::Record>> {
        let definition = &self.launch.definition;
        let pipeline = definition
            .pipeline
            .as_ref()
            .ok_or_else(|| failed(format!("schedule `{}` has no pipeline", definition.id)))?;
        let factory = self
            .ctx
            .extensions
            .get::<dyn RunContextFactory>()
            .ok_or_else(|| failed("the service has no runner for scheduled pipelines"))?;
        let request = RunContextRequest {
            schedule: definition.id.clone(),
            run: self.launch.run,
            session: format!("{SESSION_PREFIX}{}", definition.id),
            base: definition.base.clone(),
            project_roots: definition.project_roots.clone(),
            timeout: std_duration(definition.timeout),
            cancel: self.cancel.clone(),
        };
        let exec = factory.context(&request)?;
        let output = exec.pipelines()?.run(pipeline, StageInput::none(), &exec)?;
        match output.data {
            StageData::Records(stream) => stream.collect_bounded(exec.limits().max_records),
            _ => Ok(Vec::new()),
        }
    }
}

/// Time to sleep before the next step: until `next`, at most [`POLL`]. A run
/// time that is already due after a step waits for a free place among the
/// parallel runs, so the module sleeps [`DUE_POLL`] instead of spinning.
fn wait_until(next: Option<Instant>, now: Instant) -> Duration {
    match next {
        Some(next) => match time::since(next, now) {
            wait if wait.is_zero() => DUE_POLL,
            wait => wait.min(POLL),
        },
        None => POLL,
    }
}

fn fired_data(launch: &Launch, token: Option<&str>) -> Json {
    let mut data = json!({
        "id": launch.schedule,
        "run": launch.run,
        "trigger": launch.trigger.as_str(),
        "scheduled_for": launch.scheduled_for,
        "action_kind": launch.definition.action_kind,
    });
    if let Some(args) = &launch.definition.action_args {
        data["action_args"] = args.clone();
    }
    if let Some(token) = token {
        data["ack_token"] = Json::String(token.to_owned());
    }
    data
}

fn required_text(body: &Json, name: &str) -> Result<String> {
    body.get(name)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                format!("the request needs a text value for `{name}`"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant(nanos: i64) -> Instant {
        Instant::from_unix_nanos(nanos)
    }

    fn pending(module: &ScheduleModule, token: &str, client: &str) -> Receiver<Ack> {
        let (sender, receiver) = mpsc::channel();
        lock(&module.shared.acks).insert(
            token.to_owned(),
            PendingAck {
                client: client.to_owned(),
                sender,
            },
        );
        receiver
    }

    fn ack(token: &str, client: Option<&str>) -> ModuleRequest {
        let request = ModuleRequest::new("schedule/ack", json!({ "token": token }));
        match client {
            Some(client) => request.from_client(client),
            None => request,
        }
    }

    #[test]
    fn a_due_run_time_does_not_make_the_module_spin() {
        let now = instant(1_000_000_000);
        assert_eq!(wait_until(Some(now), now), DUE_POLL);
        assert_eq!(wait_until(Some(instant(0)), now), DUE_POLL);
    }

    #[test]
    fn the_wait_is_the_time_to_the_next_run_at_most_one_poll() {
        let now = instant(0);
        assert_eq!(
            wait_until(Some(instant(250_000_000)), now),
            Duration::from_millis(250)
        );
        assert_eq!(wait_until(Some(instant(60_000_000_000)), now), POLL);
        assert_eq!(wait_until(None, now), POLL);
    }

    #[test]
    fn only_the_client_of_a_run_acknowledges_it() {
        let module = ScheduleModule::new();
        let receiver = pending(&module, "s1#1:1", "client-a");
        let error = module.request(ack("s1#1:1", Some("client-b"))).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        assert!(receiver.try_recv().is_err());
        module.request(ack("s1#1:1", Some("client-a"))).unwrap();
        assert_eq!(receiver.try_recv().unwrap().state, RunState::Ok);
    }

    #[test]
    fn an_unknown_token_is_not_found() {
        let module = ScheduleModule::new();
        let error = module.request(ack("none", None)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        let error = module
            .request(ModuleRequest::new("schedule/ack", json!({})))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
    }

    #[test]
    fn a_closed_client_loses_its_registrations() {
        let module = ScheduleModule::new();
        let _receiver = pending(&module, "s1#1:1", "client-a");
        lock(&module.shared.remote).insert("kind".to_owned(), "client-a".to_owned());
        module.client_closed("client-a");
        assert!(lock(&module.shared.acks).is_empty());
        assert!(lock(&module.shared.remote).is_empty());
    }

    #[test]
    fn unknown_operations_are_unsupported() {
        let module = ScheduleModule::new();
        let error = module
            .request(ModuleRequest::new("schedule.nothing", json!({})))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
        assert!(
            module
                .request(ModuleRequest::new("schedule.status", json!({})))
                .is_err()
        );
    }
}

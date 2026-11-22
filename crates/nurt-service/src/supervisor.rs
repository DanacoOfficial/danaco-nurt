// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Supervision of one hosted module: start on a dedicated thread, health
//! polling, restart with a growing delay and a restart rate limit.

use std::any::Any;
use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nurt_contracts::config::Config;
use nurt_contracts::env::Clock;
use nurt_contracts::error::{Error, Result as ContractResult};
use nurt_contracts::stage::{CancelReason, CancelToken, Extensions};
use serde_json::{Value, json};

use crate::heartbeat::ModuleState;
use crate::hub::Hub;
use crate::module::{ModuleHealth, ModuleRequest, ServiceCtx, ServiceModule};

const HEALTH_POLL: Duration = Duration::from_secs(1);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const PARKED_POLL: Duration = Duration::from_millis(100);

/// Restart policy of failed modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RestartPolicy {
    /// Delay before the first restart; doubled for every further one.
    pub(crate) initial_delay: Duration,
    /// Longest delay between restarts.
    pub(crate) max_delay: Duration,
    /// Window in which restarts are counted.
    pub(crate) window: Duration,
    /// Most restarts in the window; one more failure leaves the module in
    /// state `failed` until `reload`.
    pub(crate) max_restarts: usize,
    /// A run at least this long resets the delay to `initial_delay`.
    pub(crate) stable_after: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            window: Duration::from_secs(600),
            max_restarts: 5,
            stable_after: Duration::from_secs(60),
        }
    }
}

impl RestartPolicy {
    /// Delay before restart number `attempt`, counted from zero.
    pub(crate) fn delay(&self, attempt: u32) -> Duration {
        let factor = 1_u32.checked_shl(attempt).unwrap_or(u32::MAX);
        self.initial_delay
            .checked_mul(factor)
            .map_or(self.max_delay, |delay| delay.min(self.max_delay))
    }
}

/// Values shared by all supervisors of one service.
#[derive(Clone)]
pub(crate) struct Shared {
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) config: Arc<Mutex<Arc<Config>>>,
    pub(crate) state_dir: std::path::PathBuf,
    pub(crate) hub: Arc<Hub>,
    pub(crate) extensions: Arc<Extensions>,
    pub(crate) policy: RestartPolicy,
}

impl Shared {
    pub(crate) fn config(&self) -> Arc<Config> {
        Arc::clone(&self.config.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[derive(Debug)]
struct Status {
    state: ModuleState,
    restarts: u32,
    reason: Option<String>,
}

/// Supervisor of one module.
pub(crate) struct Supervisor {
    name: String,
    module: Arc<dyn ServiceModule>,
    status: Mutex<Status>,
    reset: Mutex<Option<mpsc::Sender<()>>>,
}

enum Outcome {
    Stopping,
    Exited(Result<ContractResult<()>, String>),
    Unhealthy(String),
}

impl Supervisor {
    pub(crate) fn new(module: Arc<dyn ServiceModule>) -> Self {
        Self {
            name: module.name().to_owned(),
            module,
            status: Mutex::new(Status {
                state: ModuleState::Starting,
                restarts: 0,
                reason: None,
            }),
            reset: Mutex::new(None),
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    fn status(&self) -> MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, state: ModuleState, reason: Option<String>) {
        let mut status = self.status();
        status.state = state;
        status.reason = reason;
    }

    pub(crate) fn state(&self) -> ModuleState {
        self.status().state
    }

    /// State, restart count and the last failure, for `status`.
    pub(crate) fn describe(&self) -> Value {
        let status = self.status();
        json!({
            "state": status.state.as_str(),
            "restarts": status.restarts,
            "reason": status.reason,
        })
    }

    /// Starts the supervising thread.
    pub(crate) fn spawn(
        self: &Arc<Self>,
        shared: Shared,
        stop: CancelToken,
    ) -> std::io::Result<JoinHandle<()>> {
        let (sender, receiver) = mpsc::channel();
        *self.reset.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
        let supervisor = Arc::clone(self);
        thread::Builder::new()
            .name(format!("nurt-service-{}", self.name))
            .spawn(move || supervisor.supervise(&shared, &stop, &receiver))
    }

    fn supervise(&self, shared: &Shared, stop: &CancelToken, reset: &mpsc::Receiver<()>) {
        let mut attempt = 0_u32;
        let mut recent: VecDeque<Duration> = VecDeque::new();
        while !stop.is_cancelled() {
            self.set(ModuleState::Starting, None);
            let started = shared.clock.monotonic();
            let reason = match self.run_once(shared, stop) {
                None => break,
                Some(reason) => reason,
            };
            let now = shared.clock.monotonic();
            if now.saturating_sub(started) >= shared.policy.stable_after {
                attempt = 0;
            }
            while recent
                .front()
                .is_some_and(|at| now.saturating_sub(*at) >= shared.policy.window)
            {
                recent.pop_front();
            }
            if recent.len() >= shared.policy.max_restarts {
                self.set(ModuleState::Failed, Some(reason));
                if !wait_for_reset(reset, stop) {
                    break;
                }
                recent.clear();
                attempt = 0;
                continue;
            }
            recent.push_back(now);
            self.status().restarts += 1;
            self.set(ModuleState::Restarting, Some(reason));
            let delay = shared.policy.delay(attempt);
            attempt = attempt.saturating_add(1);
            if shared.clock.sleep(delay, stop).is_err() {
                break;
            }
        }
        self.set(ModuleState::Stopped, None);
    }

    /// Runs the module once; `None` when the service stops, otherwise the
    /// reason of the failure.
    fn run_once(&self, shared: &Shared, stop: &CancelToken) -> Option<String> {
        let run_stop = stop.child();
        let ctx = ServiceCtx {
            clock: Arc::clone(&shared.clock),
            config: shared.config(),
            state_dir: shared.state_dir.clone(),
            events: Arc::clone(&shared.hub) as _,
            stop: run_stop.clone(),
            extensions: Arc::clone(&shared.extensions),
        };
        let (done, finished) = mpsc::channel();
        let module = Arc::clone(&self.module);
        let worker = thread::Builder::new()
            .name(format!("nurt-module-{}", self.name))
            .spawn(move || {
                let result = panic::catch_unwind(AssertUnwindSafe(|| module.start(ctx)))
                    .map_err(|payload| panic_message(&*payload));
                let _ = done.send(result);
            });
        if let Err(error) = worker {
            return Some(format!("cannot start the module thread: {error}"));
        }
        let outcome = self.monitor(&finished, stop);
        match outcome {
            Outcome::Stopping => {
                self.halt(&run_stop, &finished);
                None
            }
            Outcome::Exited(Ok(Ok(()))) if stop.is_cancelled() => None,
            Outcome::Exited(Ok(Ok(()))) => Some("the module stopped by itself".to_owned()),
            Outcome::Exited(Ok(Err(error))) => Some(error.message.clone()),
            Outcome::Exited(Err(panic)) => Some(format!("the module panicked: {panic}")),
            Outcome::Unhealthy(reason) => {
                if self.halt(&run_stop, &finished) {
                    Some(reason)
                } else {
                    self.set(
                        ModuleState::Failed,
                        Some(format!("{reason}; the module did not stop within 10 s")),
                    );
                    let _ = stop.wait_timeout(Duration::MAX);
                    None
                }
            }
        }
    }

    fn monitor(
        &self,
        finished: &mpsc::Receiver<Result<ContractResult<()>, String>>,
        stop: &CancelToken,
    ) -> Outcome {
        loop {
            match finished.recv_timeout(HEALTH_POLL) {
                Ok(result) => return Outcome::Exited(result),
                Err(RecvTimeoutError::Disconnected) => {
                    return Outcome::Exited(Err("the module thread ended".to_owned()));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if stop.is_cancelled() {
                return Outcome::Stopping;
            }
            match self.module.health() {
                ModuleHealth::Failed { reason } => return Outcome::Unhealthy(reason),
                ModuleHealth::Degraded { reason } => {
                    self.set(ModuleState::Degraded, Some(reason));
                }
                ModuleHealth::Running => self.set(ModuleState::Running, None),
                ModuleHealth::Starting | ModuleHealth::Stopped => {}
            }
        }
    }

    /// Asks the module to stop and waits up to ten seconds; `true` when it
    /// stopped.
    fn halt(
        &self,
        run_stop: &CancelToken,
        finished: &mpsc::Receiver<Result<ContractResult<()>, String>>,
    ) -> bool {
        self.module.stop();
        run_stop.cancel(CancelReason::Shutdown);
        !matches!(
            finished.recv_timeout(STOP_TIMEOUT),
            Err(RecvTimeoutError::Timeout)
        )
    }

    /// Passes a configuration to the module; a module in state `failed` is
    /// started again instead.
    pub(crate) fn reload(&self, config: &Config) -> ContractResult<&'static str> {
        if self.state() == ModuleState::Failed {
            if let Some(sender) = &*self.reset.lock().unwrap_or_else(PoisonError::into_inner) {
                let _ = sender.send(());
            }
            return Ok("restarted");
        }
        self.module.reload(config).map(|()| "reloaded")
    }

    /// Forwards a request to a running module.
    pub(crate) fn request(&self, request: ModuleRequest) -> Result<Value, Option<Error>> {
        match self.state() {
            ModuleState::Starting | ModuleState::Running | ModuleState::Degraded => {
                self.module.request(request).map_err(Some)
            }
            _ => Err(None),
        }
    }

    pub(crate) fn client_closed(&self, client: &str) {
        self.module.client_closed(client);
    }
}

fn wait_for_reset(reset: &mpsc::Receiver<()>, stop: &CancelToken) -> bool {
    loop {
        match reset.recv_timeout(PARKED_POLL) {
            Ok(()) => return true,
            Err(RecvTimeoutError::Disconnected) => return false,
            Err(RecvTimeoutError::Timeout) if stop.is_cancelled() => return false,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic payload".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_delay_doubles_up_to_one_minute() {
        let policy = RestartPolicy::default();
        let delays: Vec<u64> = (0..8).map(|n| policy.delay(n).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(policy.delay(u32::MAX), Duration::from_secs(60));
    }

    #[test]
    fn panic_payloads_become_messages() {
        assert_eq!(panic_message(&"boom"), "boom");
        assert_eq!(panic_message(&String::from("bang")), "bang");
        assert_eq!(panic_message(&7_u8), "unknown panic payload");
    }
}

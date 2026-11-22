// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Asynchronous executions (`nurt_exec`) and delivery of their events.
//!
//! An execution runs on its own worker thread. Events emitted by the engine
//! go through an unbounded channel to a delivery thread, which numbers them
//! with `seq` from 1, wraps them between `exec.started` and `exec.finished`
//! and calls the event callback one event at a time. A slow callback
//! therefore never blocks the execution. The execution is complete when
//! `exec.finished` has been delivered; only then does `nurt_exec_wait`
//! return its result, so no callback runs after a successful wait.

use std::ffi::{CString, c_char, c_void};
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nurt_contracts::stage::{CancelReason, CancelToken, Event, EventSink};
use serde_json::{Map, Value, json};

use crate::backend::Backend;
use crate::boundary::{caught_panic, lock};
use crate::error::{CallError, NurtStatus};

/// Receiver of the events of one execution (`nurt_event_cb`): the user
/// pointer and the event as JSON, valid only during the call.
pub type EventCallback = Option<unsafe extern "C" fn(user: *mut c_void, event_json: *const c_char)>;

/// Event callback together with its user pointer.
#[derive(Clone, Copy)]
pub(crate) struct Callback {
    function: unsafe extern "C" fn(*mut c_void, *const c_char),
    user: *mut c_void,
}

// SAFETY: The function `nurt_exec_start` documents that the callback is called from a
// thread of the library; by passing `user` with it the caller allows its use
// from that thread.
unsafe impl Send for Callback {}

impl Callback {
    pub(crate) fn new(callback: EventCallback, user: *mut c_void) -> Option<Self> {
        callback.map(|function| Self { function, user })
    }
}

/// Work of an execution: a request to run, or a response known before the
/// engine is involved (a request that breaks a rule of the envelope).
pub(crate) enum Job {
    Run(Value),
    Done(Vec<u8>),
}

/// Result of a completed execution.
#[derive(Debug, Clone)]
pub(crate) struct Outcome {
    pub(crate) status: NurtStatus,
    pub(crate) response: Vec<u8>,
    pub(crate) error: Option<CallError>,
}

impl Outcome {
    fn finished(response: Vec<u8>, cancelled: bool) -> Self {
        let status = if cancelled {
            NurtStatus::Cancelled
        } else {
            NurtStatus::Ok
        };
        Self {
            status,
            response,
            error: None,
        }
    }

    fn failed(error: CallError) -> Self {
        Self {
            status: error.status(),
            response: Vec::new(),
            error: Some(error),
        }
    }
}

#[derive(Default)]
struct State {
    outcome: Option<Outcome>,
    complete: bool,
}

/// State shared by the handle, the worker and the delivery thread.
pub(crate) struct Shared {
    state: Mutex<State>,
    completed: Condvar,
    cancel: CancelToken,
}

impl Shared {
    fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
            completed: Condvar::new(),
            cancel: CancelToken::new(),
        }
    }

    /// Requests cancellation; repeated requests have no further effect.
    pub(crate) fn cancel(&self) {
        self.cancel.cancel(CancelReason::Requested);
    }

    /// Waits without limit until the execution is complete.
    pub(crate) fn wait_complete(&self) {
        drop(self.wait(None));
    }

    /// Waits until the execution is complete or `timeout` elapses; `None`
    /// waits without limit. Returns `None` on timeout.
    fn wait(&self, timeout: Option<Duration>) -> Option<Outcome> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut state = lock(&self.state);
        while !state.complete {
            state = match deadline {
                None => self
                    .completed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner),
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return None;
                    }
                    self.completed
                        .wait_timeout(state, remaining)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
        }
        state.outcome.clone()
    }

    fn set_outcome(&self, outcome: Outcome) {
        lock(&self.state).outcome = Some(outcome);
    }

    fn status(&self) -> NurtStatus {
        let mut state = lock(&self.state);
        let outcome = state.outcome.get_or_insert_with(|| {
            Outcome::failed(CallError::internal("the execution ended without a result"))
        });
        outcome.status
    }

    fn complete(&self) {
        lock(&self.state).complete = true;
        self.completed.notify_all();
    }
}

enum Message {
    Event(Event),
    Done,
}

struct EventChannel(Sender<Message>);

impl EventSink for EventChannel {
    fn emit(&self, event: Event) {
        // The delivery thread ends only after `Done`, so sending fails only
        // when it could not be started; the event is then dropped.
        drop(self.0.send(Message::Event(event)));
    }
}

/// Asynchronous execution behind a `nurt_exec` handle.
///
/// Dropping the handle cancels a running execution and waits for its
/// threads.
pub struct Exec {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    delivery: Option<JoinHandle<()>>,
}

impl Exec {
    /// Starts the delivery thread and the worker of an execution.
    pub(crate) fn start(
        backend: Arc<dyn Backend>,
        job: Job,
        callback: Option<Callback>,
    ) -> Result<Self, CallError> {
        let shared = Arc::new(Shared::new());
        let (sender, receiver) = mpsc::channel();
        let delivery = spawn("nurt-events", {
            let shared = Arc::clone(&shared);
            move || deliver(&shared, &receiver, callback)
        })?;
        let worker = spawn("nurt-exec", {
            let shared = Arc::clone(&shared);
            move || {
                let events = EventChannel(sender);
                let outcome = execute(backend.as_ref(), job, &events, &shared.cancel);
                shared.set_outcome(outcome);
                drop(events.0.send(Message::Done));
            }
        })?;
        Ok(Self {
            shared,
            worker: Some(worker),
            delivery: Some(delivery),
        })
    }

    /// Weak reference used by the engine to cancel the execution on close.
    pub(crate) fn downgrade(&self) -> Weak<Shared> {
        Arc::downgrade(&self.shared)
    }

    /// Waits for completion; `u32::MAX` waits without limit. Returns `None`
    /// when the execution is still running after `timeout_ms`.
    pub(crate) fn wait(&self, timeout_ms: u32) -> Option<Outcome> {
        let timeout =
            (timeout_ms != u32::MAX).then(|| Duration::from_millis(u64::from(timeout_ms)));
        self.shared.wait(timeout)
    }

    /// Requests cancellation.
    pub(crate) fn cancel(&self) {
        self.shared.cancel();
    }
}

impl Drop for Exec {
    fn drop(&mut self) {
        self.shared.cancel();
        let current = thread::current().id();
        for handle in [self.worker.take(), self.delivery.take()]
            .into_iter()
            .flatten()
        {
            // Releasing the handle from its own event callback is forbidden
            // by the interface; joining the delivery thread there would never
            // return, so the thread is left to finish on its own.
            if handle.thread().id() != current {
                drop(handle.join());
            }
        }
    }
}

fn spawn<F>(name: &str, body: F) -> Result<JoinHandle<()>, CallError>
where
    F: FnOnce() + Send + 'static,
{
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(body)
        .map_err(|error| {
            CallError::internal(format!("cannot start a thread of the execution: {error}"))
        })
}

fn execute(
    backend: &dyn Backend,
    job: Job,
    events: &EventChannel,
    cancel: &CancelToken,
) -> Outcome {
    let request = match job {
        Job::Done(response) => return Outcome::finished(response, false),
        Job::Run(request) => request,
    };
    match panic::catch_unwind(AssertUnwindSafe(|| backend.run(&request, events, cancel))) {
        Ok(Ok(run)) => Outcome::finished(run.response, run.cancelled),
        Ok(Err(error)) => Outcome::failed(error),
        Err(payload) => Outcome::failed(caught_panic("an execution", payload.as_ref())),
    }
}

/// Numbers events and passes them to the callback.
struct Delivery {
    next: u64,
    callback: Option<Callback>,
}

impl Delivery {
    fn send(&mut self, body: Value) {
        let mut event = Map::new();
        event.insert("seq".to_owned(), Value::from(self.next));
        if let Value::Object(fields) = body {
            event.extend(fields);
        }
        self.next += 1;
        let Some(callback) = self.callback else {
            return;
        };
        let Ok(text) = CString::new(serde_json::to_vec(&Value::Object(event)).unwrap_or_default())
        else {
            return;
        };
        // SAFETY: The callback `function` and its `user` pointer were passed together to
        // `nurt_exec_start`; `text` is NUL-terminated and outlives the call,
        // which is all the interface promises about `event_json`.
        unsafe { (callback.function)(callback.user, text.as_ptr()) };
    }
}

fn deliver(shared: &Shared, receiver: &Receiver<Message>, callback: Option<Callback>) {
    // No message arrives when the worker could not be started; the execution
    // then never existed for the caller and nothing is delivered.
    let Ok(first) = receiver.recv() else {
        return;
    };
    let mut delivery = Delivery { next: 1, callback };
    delivery.send(json!({ "t": "exec.started" }));
    let mut message = Some(first);
    while let Some(Message::Event(event)) = message {
        delivery.send(serde_json::to_value(&event).unwrap_or(Value::Null));
        message = receiver.recv().ok();
    }
    let status = shared.status();
    delivery.send(json!({ "t": "exec.finished", "status": status.code() }));
    shared.complete();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use nurt_contracts::stage::{CancelToken, EventSink};
    use serde_json::Value;

    use super::{Exec, Job};
    use crate::backend::{Backend, Function, RunOutcome};
    use crate::error::{CallError, NurtStatus};

    struct WaitForCancel;

    impl Backend for WaitForCancel {
        fn call(&self, _function: Function, _input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
            Ok(Vec::new())
        }

        fn run(
            &self,
            _request: &Value,
            _events: &dyn EventSink,
            cancel: &CancelToken,
        ) -> Result<RunOutcome, CallError> {
            let cancelled = cancel.wait_timeout(Duration::from_secs(60)).is_some();
            Ok(RunOutcome {
                response: b"{}".to_vec(),
                cancelled,
            })
        }
    }

    #[test]
    fn known_response_completes_without_engine_work() {
        let exec = Exec::start(
            Arc::new(WaitForCancel),
            Job::Done(b"{\"nurt\":\"1.0\"}".to_vec()),
            None,
        )
        .unwrap();
        let outcome = exec.wait(u32::MAX).unwrap();
        assert_eq!(outcome.status, NurtStatus::Ok);
        assert_eq!(outcome.response, b"{\"nurt\":\"1.0\"}");
    }

    #[test]
    fn cancellation_ends_a_waiting_execution() {
        let exec = Exec::start(Arc::new(WaitForCancel), Job::Run(Value::Null), None).unwrap();
        assert!(exec.wait(0).is_none());
        exec.cancel();
        exec.cancel();
        let outcome = exec.wait(u32::MAX).unwrap();
        assert_eq!(outcome.status, NurtStatus::Cancelled);
        assert_eq!(exec.wait(0).unwrap().status, NurtStatus::Cancelled);
    }

    #[test]
    fn dropping_a_running_execution_cancels_it() {
        let exec = Exec::start(Arc::new(WaitForCancel), Job::Run(Value::Null), None).unwrap();
        let shared = exec.downgrade();
        drop(exec);
        assert!(shared.upgrade().is_none());
    }
}

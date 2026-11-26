// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Asynchronous executions (`nurt_exec`) and delivery of their events.
//!
//! The backend runs an execution on a thread of its own and passes its
//! events, numbered with `seq` from 1 and framed by `exec.started` and
//! `exec.finished`, to a listener. The listener of the C interface queues
//! them for a delivery thread, which calls the event callback one event at a
//! time, so a slow callback never blocks the execution. When an execution
//! ends without `exec.finished` (the backend failed), the delivery thread
//! adds the missing frame events itself. The execution is complete when its
//! last event has been delivered; only then does `nurt_exec_wait` return its
//! result, so no callback runs after a successful wait.

use std::ffi::{CString, c_char, c_void};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nurt_contracts::envelope::{Request, Response};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::report::ReportStatus;
use nurt_contracts::value::Instant as WireInstant;
use nurt_core::{DeliveredEvent, EventBody, EventListener, Lifecycle};

use crate::backend::{Backend, Run, to_json};
use crate::boundary::lock;
use crate::error::{CallError, NurtStatus};
use crate::system;

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

/// Result of a completed execution.
#[derive(Debug, Clone)]
pub(crate) struct Outcome {
    pub(crate) status: NurtStatus,
    pub(crate) response: Vec<u8>,
    pub(crate) error: Option<CallError>,
}

impl Outcome {
    /// Outcome of the result of a backend: a response that reports
    /// `E_CANCELLED` is [`NurtStatus::Cancelled`].
    fn of(result: Option<Result<Response, CallError>>) -> Self {
        let response = match result {
            Some(Ok(response)) => response,
            Some(Err(error)) => return Self::failed(error),
            None => {
                return Self::failed(CallError::internal(
                    "the execution ended without a response",
                ));
            }
        };
        let cancelled = response
            .report
            .error
            .as_ref()
            .is_some_and(|error| error.code == ErrorCode::Cancelled);
        match to_json(&response) {
            Ok(bytes) => Self {
                status: if cancelled {
                    NurtStatus::Cancelled
                } else {
                    NurtStatus::Ok
                },
                response: bytes,
                error: None,
            },
            Err(error) => Self::failed(error),
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

/// State shared by the handle and the delivery thread.
pub(crate) struct Shared {
    outcome: Mutex<Option<Outcome>>,
    completed: Condvar,
    run: Arc<dyn Run>,
}

impl Shared {
    fn new(run: Arc<dyn Run>) -> Self {
        Self {
            outcome: Mutex::new(None),
            completed: Condvar::new(),
            run,
        }
    }

    /// Requests cancellation; repeated requests have no further effect.
    pub(crate) fn cancel(&self) {
        self.run.cancel();
    }

    /// Waits without limit until the execution is complete.
    pub(crate) fn wait_complete(&self) {
        drop(self.wait(None));
    }

    /// Waits until the execution is complete or `timeout` elapses; `None`
    /// waits without limit. Returns `None` on timeout.
    fn wait(&self, timeout: Option<Duration>) -> Option<Outcome> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut outcome = lock(&self.outcome);
        while outcome.is_none() {
            outcome = match deadline {
                None => self
                    .completed
                    .wait(outcome)
                    .unwrap_or_else(PoisonError::into_inner),
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return None;
                    }
                    self.completed
                        .wait_timeout(outcome, remaining)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
        }
        outcome.clone()
    }

    fn complete(&self, outcome: Outcome) {
        *lock(&self.outcome) = Some(outcome);
        self.completed.notify_all();
    }
}

/// Listener that queues the events of an execution for the delivery thread.
struct Queue(Sender<DeliveredEvent>);

impl EventListener for Queue {
    fn deliver(&self, event: DeliveredEvent) {
        // The delivery thread receives until the listener is released, so
        // sending fails only when that thread could not be started; the
        // event is then dropped with the execution.
        drop(self.0.send(event));
    }
}

/// Asynchronous execution behind a `nurt_exec` handle.
///
/// Dropping the handle cancels a running execution and waits until it is
/// complete.
pub struct Exec {
    shared: Arc<Shared>,
    delivery: Option<JoinHandle<()>>,
}

impl Exec {
    /// Starts an execution on `backend` and the thread delivering its
    /// events.
    pub(crate) fn start(
        backend: &dyn Backend,
        request: &Request,
        callback: Option<Callback>,
    ) -> Result<Self, CallError> {
        let (sender, receiver) = mpsc::channel();
        let run = backend.start(request, Arc::new(Queue(sender)))?;
        let shared = Arc::new(Shared::new(run));
        let delivery = thread::Builder::new()
            .name("nurt-events".to_owned())
            .spawn({
                let shared = Arc::clone(&shared);
                move || deliver(&shared, &receiver, callback)
            })
            .map_err(|error| {
                shared.cancel();
                CallError::internal(format!("cannot start a thread of the execution: {error}"))
            })?;
        Ok(Self {
            shared,
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
        if let Some(handle) = self.delivery.take() {
            // Releasing the handle from its own event callback is forbidden
            // by the interface; joining the delivery thread there would never
            // return, so the thread is left to finish on its own.
            if handle.thread().id() != thread::current().id() {
                drop(handle.join());
            }
        }
    }
}

/// Encodes an event in the form of `nurt-event/1`: the time as an RFC 3339
/// instant in UTC and the identifier of the request as the text `request`.
///
/// The event of the engine carries the time as a number of nanoseconds and the
/// identifier as `request_id`; the schema knows neither.
pub(crate) fn encode_event(event: &DeliveredEvent) -> Vec<u8> {
    let mut document = serde_json::to_value(event).unwrap_or_default();
    if let Some(members) = document.as_object_mut() {
        let nanos = i64::try_from(event.time.unix_nanos()).unwrap_or(i64::MAX);
        members.insert(
            "time".to_owned(),
            serde_json::Value::String(WireInstant::from_unix_nanos(nanos).to_string()),
        );
        if let Some(request) = members.remove("request_id") {
            members.insert("request".to_owned(), request);
        }
    }
    serde_json::to_vec(&document).unwrap_or_default()
}

/// Passes events to the callback and records what was delivered.
struct Delivery {
    callback: Option<Callback>,
    last_seq: u64,
    finished: bool,
}

impl Delivery {
    fn send(&mut self, event: &DeliveredEvent) {
        self.last_seq = event.seq;
        if matches!(event.body, EventBody::Lifecycle(Lifecycle::Finished { .. })) {
            self.finished = true;
        }
        let Some(callback) = self.callback else {
            return;
        };
        let Ok(text) = CString::new(encode_event(event)) else {
            return;
        };
        // SAFETY: The callback `function` and its `user` pointer were passed together to
        // `nurt_exec_start`; `text` is NUL-terminated and outlives the call,
        // which is all the interface promises about `event_json`.
        unsafe { (callback.function)(callback.user, text.as_ptr()) };
    }

    /// Delivers the frame events that an execution ended without.
    fn close(&mut self, request_id: &str, status: ReportStatus) {
        if self.last_seq == 0 {
            self.send(&self.frame(request_id, Lifecycle::Started));
        }
        self.send(&self.frame(request_id, Lifecycle::Finished { status }));
    }

    fn frame(&self, request_id: &str, lifecycle: Lifecycle) -> DeliveredEvent {
        DeliveredEvent {
            seq: self.last_seq + 1,
            time: system::now(),
            request_id: request_id.to_owned(),
            body: EventBody::Lifecycle(lifecycle),
        }
    }
}

fn deliver(shared: &Shared, receiver: &Receiver<DeliveredEvent>, callback: Option<Callback>) {
    let mut delivery = Delivery {
        callback,
        last_seq: 0,
        finished: false,
    };
    for event in receiver {
        delivery.send(&event);
    }
    // The response follows `exec.finished` at once; an execution that ended
    // without it has a response already or none at all.
    let timeout = if delivery.finished {
        None
    } else {
        Some(Duration::ZERO)
    };
    let result = shared.run.result(timeout);
    if !delivery.finished {
        let status = match &result {
            Some(Ok(response)) => response.report.status,
            _ => ReportStatus::Failed,
        };
        delivery.close(shared.run.request_id(), status);
    }
    shared.complete(Outcome::of(result));
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use nurt_contracts::envelope::{Request, Response};
    use nurt_contracts::error::{Error, ErrorCode};
    use nurt_contracts::report::{Report, ReportStatus};
    use nurt_contracts::stage::{CancelReason, CancelToken};
    use nurt_core::{DeliveredEvent, EventBody, EventListener, Lifecycle};
    use serde_json::Value;

    use super::{encode_event, Exec, Outcome};
    use crate::backend::{Backend, Function, Run};
    use crate::error::{CallError, NurtStatus};

    fn response(status: ReportStatus, error: Option<ErrorCode>) -> Response {
        let mut report = Report::new(status, "find");
        report.error = error.map(|code| Error::new(code, "failure"));
        Response::new(report)
    }

    /// Execution that waits for cancellation and reports `E_CANCELLED`.
    struct Waiting {
        cancel: CancelToken,
    }

    impl Run for Waiting {
        fn request_id(&self) -> &str {
            "r1"
        }

        fn cancel(&self) {
            self.cancel.cancel(CancelReason::Requested);
        }

        fn result(&self, timeout: Option<Duration>) -> Option<Result<Response, CallError>> {
            let cancelled = self
                .cancel
                .wait_timeout(timeout.unwrap_or(Duration::from_secs(60)))
                .is_some();
            cancelled.then(|| Ok(response(ReportStatus::Failed, Some(ErrorCode::Cancelled))))
        }
    }

    /// Backend whose executions end in a way chosen by the test.
    enum Scripted {
        /// Waits for cancellation; the listener is kept until then.
        Waiting,
        /// Ends at once without events and without a response.
        Vanished,
        /// Ends at once without events, with an ok response.
        Silent,
    }

    struct Ended(Option<Response>);

    impl Run for Ended {
        fn request_id(&self) -> &str {
            "r9"
        }

        fn cancel(&self) {}

        fn result(&self, _timeout: Option<Duration>) -> Option<Result<Response, CallError>> {
            self.0.clone().map(Ok)
        }
    }

    impl Backend for Scripted {
        fn call(&self, _function: Function, _input: &Value) -> Result<Vec<Vec<u8>>, CallError> {
            Ok(Vec::new())
        }

        fn run(&self, _request: &Request) -> Result<Response, CallError> {
            Ok(response(ReportStatus::Ok, None))
        }

        fn start(
            &self,
            _request: &Request,
            listener: Arc<dyn EventListener>,
        ) -> Result<Arc<dyn Run>, CallError> {
            match self {
                Self::Waiting => {
                    let run = Arc::new(Waiting {
                        cancel: CancelToken::new(),
                    });
                    let waiting = Arc::clone(&run);
                    std::thread::spawn(move || {
                        let _ = waiting.cancel.wait_timeout(Duration::from_secs(60));
                        drop(listener);
                    });
                    Ok(run)
                }
                Self::Vanished => Ok(Arc::new(Ended(None))),
                Self::Silent => Ok(Arc::new(Ended(Some(response(ReportStatus::Ok, None))))),
            }
        }
    }

    static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    unsafe extern "C" fn record(_user: *mut std::ffi::c_void, event: *const std::ffi::c_char) {
        // SAFETY: The library passes a NUL-terminated event valid during the call.
        let text = unsafe { std::ffi::CStr::from_ptr(event) };
        EVENTS
            .lock()
            .unwrap()
            .push(text.to_string_lossy().into_owned());
    }

    #[test]
    fn cancellation_ends_a_waiting_execution() {
        let exec = Exec::start(&Scripted::Waiting, &Request::program("find"), None).unwrap();
        assert!(exec.wait(0).is_none());
        exec.cancel();
        exec.cancel();
        let outcome = exec.wait(u32::MAX).unwrap();
        assert_eq!(outcome.status, NurtStatus::Cancelled);
        assert_eq!(exec.wait(0).unwrap().status, NurtStatus::Cancelled);
    }

    #[test]
    fn dropping_a_running_execution_cancels_it() {
        let exec = Exec::start(&Scripted::Waiting, &Request::program("find"), None).unwrap();
        let shared = exec.downgrade();
        drop(exec);
        assert!(shared.upgrade().is_none());
    }

    #[test]
    fn missing_frame_events_are_added_by_the_delivery_thread() {
        let callback = super::Callback::new(Some(record), std::ptr::null_mut());
        let exec = Exec::start(&Scripted::Vanished, &Request::program("find"), callback).unwrap();
        let outcome = exec.wait(u32::MAX).unwrap();
        assert_eq!(outcome.status, NurtStatus::Internal);
        assert!(outcome.response.is_empty());
        drop(exec);
        let silent = Exec::start(&Scripted::Silent, &Request::program("find"), callback).unwrap();
        assert_eq!(silent.wait(u32::MAX).unwrap().status, NurtStatus::Ok);
        drop(silent);
        let events: Vec<Value> = EVENTS
            .lock()
            .unwrap()
            .iter()
            .map(|text| serde_json::from_str(text).unwrap())
            .collect();
        let frames: Vec<(&str, u64, &str)> = events
            .iter()
            .map(|event| {
                (
                    event["t"].as_str().unwrap(),
                    event["seq"].as_u64().unwrap(),
                    event["status"].as_str().unwrap_or(""),
                )
            })
            .collect();
        assert_eq!(
            frames,
            [
                ("exec.started", 1, ""),
                ("exec.finished", 2, "failed"),
                ("exec.started", 1, ""),
                ("exec.finished", 2, "ok"),
            ]
        );
        assert!(events.iter().all(|event| event["request"] == "r9"));
        assert!(events.iter().all(|event| event.get("request_id").is_none()));
    }

    #[test]
    fn outcomes_follow_the_result_of_the_backend() {
        let ok = Outcome::of(Some(Ok(response(ReportStatus::Ok, None))));
        assert_eq!(ok.status, NurtStatus::Ok);
        assert!(ok.error.is_none());
        let failed = Outcome::of(Some(Ok(response(
            ReportStatus::Failed,
            Some(ErrorCode::Unsupported),
        ))));
        assert_eq!(failed.status, NurtStatus::Ok);
        let cancelled = Outcome::of(Some(Ok(response(
            ReportStatus::Failed,
            Some(ErrorCode::Cancelled),
        ))));
        assert_eq!(cancelled.status, NurtStatus::Cancelled);
        let error = Outcome::of(Some(Err(CallError::arg("bad"))));
        assert_eq!(error.status, NurtStatus::Arg);
        assert!(error.response.is_empty());
        assert_eq!(Outcome::of(None).status, NurtStatus::Internal);
    }

    #[test]
    fn delivered_events_are_encoded_in_the_form_of_the_event_schema() {
        let schema: serde_json::Value = serde_json::from_str(nurt_contracts::schema::EVENT).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        for body in [
            EventBody::Lifecycle(Lifecycle::Started),
            EventBody::Lifecycle(Lifecycle::Finished {
                status: ReportStatus::Ok,
            }),
        ] {
            let event = DeliveredEvent {
                seq: 3,
                time: crate::system::now(),
                request_id: "r2".to_owned(),
                body,
            };
            let json: serde_json::Value = serde_json::from_slice(&encode_event(&event)).unwrap();
            assert!(validator.is_valid(&json), "{json} does not follow nurt-event/1");
            assert_eq!(json["request"], "r2");
            assert!(json.get("request_id").is_none());
            assert!(json["seq"].is_u64());
            let time = json["time"].as_str().unwrap();
            assert!(time.ends_with('Z') && time.contains('T'), "time {time}");
        }
    }

    #[test]
    fn the_time_of_an_event_is_text_of_the_instant_of_delivery() {
        let event = DeliveredEvent {
            seq: 1,
            time: nurt_contracts::env::WallTime::from_unix_nanos(0),
            request_id: "r1".to_owned(),
            body: EventBody::Lifecycle(Lifecycle::Started),
        };
        let json: serde_json::Value = serde_json::from_slice(&encode_event(&event)).unwrap();
        assert_eq!(json["time"], "1970-01-01T00:00:00Z");
    }
}

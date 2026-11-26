// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Delivery of execution events to the embedding application.
//!
//! Stages emit [`Event`]s through the execution context. The engine numbers
//! every event of an execution (`seq`, from 1), stamps it with the wall time
//! and the request identifier and passes it to the [`EventListener`] given to
//! [`Engine::start`](crate::Engine::start). The first event of an execution
//! is `exec.started` and the last one `exec.finished`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use nurt_contracts::env::{Clock, WallTime};
use nurt_contracts::report::ReportStatus;
use nurt_contracts::stage::{Event, EventSink};
use serde::Serialize;

/// Start and end of an execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "t")]
#[non_exhaustive]
pub enum Lifecycle {
    /// The execution started.
    #[serde(rename = "exec.started")]
    Started,
    /// The execution ended; the response is available.
    #[serde(rename = "exec.finished")]
    Finished {
        /// Status of the report.
        status: ReportStatus,
    },
}

/// Content of a delivered event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum EventBody {
    /// Start or end of the execution.
    Lifecycle(Lifecycle),
    /// Event emitted by a stage.
    Stage(Event),
}

/// Event as delivered to the embedding application.
///
/// The JSON form is the content of the event with the members `seq`, `time`
/// (nanoseconds since the Unix epoch) and `request_id` added.
///
/// # Examples
///
/// ```
/// use nurt_contracts::env::WallTime;
/// use nurt_core::events::{DeliveredEvent, EventBody, Lifecycle};
///
/// let event = DeliveredEvent {
///     seq: 1,
///     time: WallTime::from_unix_nanos(5),
///     request_id: "r1".to_owned(),
///     body: EventBody::Lifecycle(Lifecycle::Started),
/// };
/// let json = serde_json::to_value(&event)?;
/// assert_eq!(json["t"], "exec.started");
/// assert_eq!(json["seq"], 1);
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeliveredEvent {
    /// Number of the event within the execution, from 1.
    pub seq: u64,
    /// Wall time of delivery.
    pub time: WallTime,
    /// Identifier of the request.
    pub request_id: String,
    /// The event.
    #[serde(flatten)]
    pub body: EventBody,
}

/// Receiver of the events of an execution.
///
/// Events of one execution arrive in the order of `seq` from the thread of
/// the execution; the receiver returns quickly and does its own buffering.
pub trait EventListener: Send + Sync {
    /// Accepts one event.
    fn deliver(&self, event: DeliveredEvent);
}

/// Event sink of one execution: numbers and stamps events and passes them to
/// a listener.
pub(crate) struct Sequencer {
    listener: Arc<dyn EventListener>,
    clock: Arc<dyn Clock>,
    request_id: String,
    seq: Mutex<u64>,
    started: AtomicBool,
    finished: AtomicBool,
}

impl Sequencer {
    pub(crate) fn new(
        listener: Arc<dyn EventListener>,
        clock: Arc<dyn Clock>,
        request_id: String,
    ) -> Self {
        Self {
            listener,
            clock,
            request_id,
            seq: Mutex::new(0),
            started: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }

    pub(crate) fn lifecycle(&self, lifecycle: Lifecycle) {
        self.deliver(EventBody::Lifecycle(lifecycle));
    }

    /// Completes the sequence: delivers `exec.started` and `exec.finished`
    /// unless the executor has delivered them.
    pub(crate) fn close(&self, status: ReportStatus) {
        if !self.started.swap(true, Ordering::SeqCst) {
            self.lifecycle(Lifecycle::Started);
        }
        if !self.finished.swap(true, Ordering::SeqCst) {
            self.lifecycle(Lifecycle::Finished { status });
        }
    }

    /// Numbers the event and hands it to the listener under one lock, so that
    /// the listener receives the events in the order of `seq` also when stages
    /// emit them from several threads.
    fn deliver(&self, body: EventBody) {
        let mut seq = self.seq.lock().unwrap_or_else(PoisonError::into_inner);
        *seq = seq.saturating_add(1);
        self.listener.deliver(DeliveredEvent {
            seq: *seq,
            time: self.clock.now(),
            request_id: self.request_id.clone(),
            body,
        });
    }
}

impl EventSink for Sequencer {
    fn emit(&self, event: Event) {
        match &event {
            Event::ExecStarted(_) => self.started.store(true, Ordering::SeqCst),
            Event::ExecFinished(_) => self.finished.store(true, Ordering::SeqCst),
            _ => {}
        }
        self.deliver(EventBody::Stage(event));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nurt_contracts::kinds::EffectKind;
    use nurt_contracts::stage::{ExecFinished, ExecStarted, LogEvent, LogLevel};
    use nurt_testkit::clock::VirtualClock;

    use super::*;

    #[derive(Default)]
    struct Collect(Mutex<Vec<DeliveredEvent>>);

    impl EventListener for Collect {
        fn deliver(&self, event: DeliveredEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn events_are_numbered_and_stamped() {
        let listener = Arc::new(Collect::default());
        let clock = Arc::new(VirtualClock::new());
        let sequencer = Sequencer::new(listener.clone(), clock.clone(), "r7".to_owned());
        sequencer.emit(Event::Log(LogEvent {
            level: LogLevel::Info,
            message: "scanning".to_owned(),
            source: None,
        }));
        sequencer.close(ReportStatus::Ok);
        let events = listener.0.lock().unwrap().clone();
        let seqs: Vec<u64> = events.iter().map(|event| event.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        assert!(events.iter().all(|event| event.request_id == "r7"));
        assert_eq!(events[0].time, clock.now());
        let json: Vec<_> = events
            .iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect();
        assert_eq!(json[0]["t"], "log");
        assert_eq!(json[0]["message"], "scanning");
        assert_eq!(json[1]["t"], "exec.started");
        assert_eq!(json[2]["t"], "exec.finished");
        assert_eq!(json[2]["status"], "ok");
    }

    /// Listener that lets other threads run before it records an event.
    #[derive(Default)]
    struct Yielding(Mutex<Vec<u64>>);

    impl EventListener for Yielding {
        fn deliver(&self, event: DeliveredEvent) {
            std::thread::yield_now();
            self.0.lock().unwrap().push(event.seq);
        }
    }

    #[test]
    fn delivery_order_follows_sequence_numbers_across_threads() {
        const THREADS: u64 = 4;
        const PER_THREAD: u64 = 100;
        let listener = Arc::new(Yielding::default());
        let sequencer = Sequencer::new(
            listener.clone(),
            Arc::new(VirtualClock::new()),
            "r3".to_owned(),
        );
        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    for _ in 0..PER_THREAD {
                        sequencer.emit(Event::Log(LogEvent {
                            level: LogLevel::Debug,
                            message: String::new(),
                            source: None,
                        }));
                    }
                });
            }
        });
        let seqs = listener.0.lock().unwrap().clone();
        assert_eq!(seqs, (1..=THREADS * PER_THREAD).collect::<Vec<_>>());
    }

    #[test]
    fn closing_twice_delivers_the_lifecycle_once() {
        let listener = Arc::new(Collect::default());
        let sequencer = Sequencer::new(
            listener.clone(),
            Arc::new(VirtualClock::new()),
            "r2".to_owned(),
        );
        sequencer.close(ReportStatus::Failed);
        sequencer.close(ReportStatus::Ok);
        let events = listener.0.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        let finished = serde_json::to_value(&events[1]).unwrap();
        assert_eq!(finished["t"], "exec.finished");
        assert_eq!(finished["status"], "failed");
    }

    #[test]
    fn events_of_the_executor_are_not_repeated() {
        let listener = Arc::new(Collect::default());
        let sequencer = Sequencer::new(
            listener.clone(),
            Arc::new(VirtualClock::new()),
            "r1".to_owned(),
        );
        sequencer.emit(Event::ExecStarted(ExecStarted {
            ast_hash: None,
            effect: EffectKind::Read,
        }));
        sequencer.emit(Event::ExecFinished(ExecFinished {
            status: ReportStatus::Ok,
            error: None,
        }));
        sequencer.close(ReportStatus::Ok);
        assert_eq!(listener.0.lock().unwrap().len(), 2);
    }
}

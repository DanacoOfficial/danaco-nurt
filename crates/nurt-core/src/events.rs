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

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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
    seq: AtomicU64,
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
            seq: AtomicU64::new(0),
        }
    }

    pub(crate) fn lifecycle(&self, lifecycle: Lifecycle) {
        self.deliver(EventBody::Lifecycle(lifecycle));
    }

    fn deliver(&self, body: EventBody) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.listener.deliver(DeliveredEvent {
            seq,
            time: self.clock.now(),
            request_id: self.request_id.clone(),
            body,
        });
    }
}

impl EventSink for Sequencer {
    fn emit(&self, event: Event) {
        self.deliver(EventBody::Stage(event));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nurt_contracts::stage::{LogEvent, LogLevel};
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
        sequencer.lifecycle(Lifecycle::Started);
        sequencer.emit(Event::Log(LogEvent {
            level: LogLevel::Info,
            message: "scanning".to_owned(),
            source: None,
        }));
        sequencer.lifecycle(Lifecycle::Finished {
            status: ReportStatus::Ok,
        });
        let events = listener.0.lock().unwrap().clone();
        let seqs: Vec<u64> = events.iter().map(|event| event.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        assert!(events.iter().all(|event| event.request_id == "r7"));
        assert_eq!(events[0].time, clock.now());
        let json: Vec<_> = events
            .iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect();
        assert_eq!(json[1]["t"], "log");
        assert_eq!(json[1]["message"], "scanning");
        assert_eq!(json[2]["t"], "exec.finished");
        assert_eq!(json[2]["status"], "ok");
    }
}

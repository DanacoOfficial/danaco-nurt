// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-exec::events
// Responsibility:   Numbering of events for delivery.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Numbering of events for delivery.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use nurt_contracts::env::Clock;
use nurt_contracts::stage::{DeliveredEvent, Event, EventSink};
use nurt_contracts::value::Instant;

type Deliver = dyn Fn(DeliveredEvent) + Send + Sync;

/// Event sink that numbers the events of one execution and passes them on
/// as [`DeliveredEvent`]s.
///
/// Numbers start at 1 and increase by one per event. The number is assigned
/// and the event is delivered under one lock, so the delivery order is the
/// order of `seq` also when stages emit events from several threads. The
/// executor emits [`Event::ExecStarted`] first and [`Event::ExecFinished`]
/// last, so an execution is delivered from `seq: 1` to its last number.
///
/// # Examples
///
/// ```
/// use std::sync::{Arc, Mutex};
/// use nurt_contracts::stage::{Event, EventSink, LogEvent, LogLevel};
/// use nurt_exec::SequencedEvents;
/// use nurt_testkit::clock::VirtualClock;
///
/// let delivered = Arc::new(Mutex::new(Vec::new()));
/// let target = Arc::clone(&delivered);
/// let events = SequencedEvents::new(Arc::new(VirtualClock::new()), Some("r1".to_owned()), move |event| {
///     target.lock().map(|mut list| list.push(event)).ok();
/// });
/// for message in ["first", "second"] {
///     events.emit(Event::Log(LogEvent {
///         level: LogLevel::Info,
///         message: message.to_owned(),
///         source: None,
///     }));
/// }
/// let seqs: Vec<u64> = delivered.lock().map(|list| list.iter().map(|e| e.seq).collect()).unwrap_or_default();
/// assert_eq!(seqs, [1, 2]);
/// assert_eq!(events.last_seq(), 2);
/// ```
pub struct SequencedEvents {
    seq: Mutex<u64>,
    clock: Arc<dyn Clock>,
    request: Option<String>,
    deliver: Box<Deliver>,
}

impl SequencedEvents {
    /// Sink that stamps events with the time of `clock` and the request
    /// identifier `request` and hands them to `deliver`.
    pub fn new<F>(clock: Arc<dyn Clock>, request: Option<String>, deliver: F) -> Self
    where
        F: Fn(DeliveredEvent) + Send + Sync + 'static,
    {
        Self {
            seq: Mutex::new(0),
            clock,
            request,
            deliver: Box::new(deliver),
        }
    }

    /// Number of the last delivered event; 0 before the first.
    #[must_use]
    pub fn last_seq(&self) -> u64 {
        *self.seq.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl EventSink for SequencedEvents {
    fn emit(&self, event: Event) {
        let mut seq = self.seq.lock().unwrap_or_else(PoisonError::into_inner);
        *seq = seq.saturating_add(1);
        (self.deliver)(DeliveredEvent {
            seq: *seq,
            time: Instant::saturating_from_wall(self.clock.now()),
            request: self.request.clone(),
            event,
        });
    }
}

impl fmt::Debug for SequencedEvents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SequencedEvents")
            .field("last_seq", &self.last_seq())
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use nurt_contracts::stage::{LogEvent, LogLevel};
    use nurt_testkit::clock::VirtualClock;

    use super::*;

    const THREADS: u64 = 4;
    const PER_THREAD: u64 = 50;

    #[test]
    fn delivery_order_follows_sequence_numbers_across_threads() {
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let target = Arc::clone(&delivered);
        let events = Arc::new(SequencedEvents::new(
            Arc::new(VirtualClock::new()),
            None,
            move |event| target.lock().unwrap().push(event.seq),
        ));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let events = Arc::clone(&events);
                thread::spawn(move || {
                    for _ in 0..PER_THREAD {
                        events.emit(Event::Log(LogEvent {
                            level: LogLevel::Debug,
                            message: String::new(),
                            source: None,
                        }));
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let seqs = delivered.lock().unwrap().clone();
        let expected: Vec<u64> = (1..=THREADS * PER_THREAD).collect();
        assert_eq!(seqs, expected);
        assert!(format!("{events:?}").contains("last_seq"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Connections of the control channel and delivery of module events.
//!
//! Every connection has a queue of events and a thread that writes them, so
//! a module that publishes an event never waits for a slow reader. A
//! connection whose queue is full is closed.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use crate::ipc::protocol;
use crate::module::{EventTarget, ModuleEvent, ModuleEvents};

/// Writing half of a connection, shared by the connection thread and the
/// thread that writes its events.
pub(crate) type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Closes a connection so that its blocked read returns.
pub(crate) type Closer = Box<dyn Fn() + Send + Sync>;

/// Events a connection may have waiting; a connection that falls further
/// behind is closed.
pub(crate) const EVENT_QUEUE_MAX: usize = 256;

struct Peer {
    closer: Closer,
    /// Queue of the thread that writes the events; `None` when that thread
    /// could not be started and the connection receives no events.
    events: Option<SyncSender<Vec<u8>>>,
    subscribed: bool,
    listening: bool,
}

/// Connections of the control channel known to the service.
#[derive(Default)]
pub(crate) struct Hub {
    peers: Mutex<HashMap<String, Peer>>,
}

impl fmt::Debug for Hub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hub")
            .field("connections", &self.lock().len())
            .finish()
    }
}

impl Hub {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Peer>> {
        self.peers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds a connection that has not subscribed to anything yet.
    pub(crate) fn attach(&self, client: &str, writer: SharedWriter, closer: Closer) {
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(EVENT_QUEUE_MAX);
        let events_writer = Arc::clone(&writer);
        let spawned = thread::Builder::new()
            .name(format!("nurt-service-events-{client}"))
            .spawn(move || {
                for line in receiver {
                    if write_line(&events_writer, &line).is_err() {
                        break;
                    }
                }
            });
        self.lock().insert(
            client.to_owned(),
            Peer {
                closer,
                events: spawned.ok().map(|_| sender),
                subscribed: false,
                listening: false,
            },
        );
    }

    /// Removes a connection.
    pub(crate) fn detach(&self, client: &str) {
        self.lock().remove(client);
    }

    /// Subscribes a connection to events addressed to all connections.
    pub(crate) fn subscribe(&self, client: &str) {
        if let Some(peer) = self.lock().get_mut(client) {
            peer.subscribed = true;
            peer.listening = true;
        }
    }

    /// Marks a connection that waits for events addressed to it, for
    /// example after it registered a remote action kind.
    pub(crate) fn mark_listening(&self, client: &str) {
        if let Some(peer) = self.lock().get_mut(client) {
            peer.listening = true;
        }
    }

    /// Whether the connection waits for events and is exempt from the idle
    /// limit.
    pub(crate) fn is_listening(&self, client: &str) -> bool {
        self.lock().get(client).is_some_and(|peer| peer.listening)
    }

    /// Number of subscribed connections.
    pub(crate) fn subscribers(&self) -> usize {
        self.lock().values().filter(|peer| peer.subscribed).count()
    }

    /// Closes every connection.
    pub(crate) fn close_all(&self) {
        for peer in self.lock().values() {
            (peer.closer)();
        }
    }

    fn targets(&self, target: &EventTarget) -> Vec<(String, SyncSender<Vec<u8>>)> {
        self.lock()
            .iter()
            .filter(|(client, peer)| match target {
                EventTarget::All => peer.subscribed,
                EventTarget::Client(wanted) => *client == wanted,
            })
            .filter_map(|(client, peer)| Some((client.clone(), peer.events.clone()?)))
            .collect()
    }
}

impl ModuleEvents for Hub {
    fn publish(&self, event: ModuleEvent) {
        let line = protocol::event_line(&event.event, &event.data);
        for (client, queue) in self.targets(&event.target) {
            match queue.try_send(line.clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                    if let Some(peer) = self.lock().remove(&client) {
                        (peer.closer)();
                    }
                }
            }
        }
    }
}

/// Writes one line and flushes it.
pub(crate) fn write_line(writer: &SharedWriter, line: &[u8]) -> io::Result<()> {
    let mut writer = writer.lock().unwrap_or_else(PoisonError::into_inner);
    writer.write_all(line)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    /// Writer that takes a line only when the test sends a token.
    struct Gated(Receiver<()>);

    impl Write for Gated {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let _ = self.0.recv();
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Writer that collects what it is given.
    struct Collect(Arc<Mutex<Vec<u8>>>);

    impl Write for Collect {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn event(name: &str) -> ModuleEvent {
        ModuleEvent {
            event: name.to_owned(),
            target: EventTarget::All,
            data: json!({}),
        }
    }

    #[test]
    fn a_reader_that_does_not_read_is_closed_and_never_blocks_the_publisher() {
        let hub = Hub::default();
        let (release, gate) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&closed);
        hub.attach(
            "c1",
            Arc::new(Mutex::new(Box::new(Gated(gate)))),
            Box::new(move || flag.store(true, Ordering::SeqCst)),
        );
        hub.subscribe("c1");
        for _ in 0..EVENT_QUEUE_MAX + 2 {
            hub.publish(event("schedule.fired"));
        }
        assert!(closed.load(Ordering::SeqCst));
        assert_eq!(hub.subscribers(), 0);
        drop(release);
    }

    #[test]
    fn events_reach_a_reader_in_order() {
        let hub = Hub::default();
        let received = Arc::new(Mutex::new(Vec::new()));
        hub.attach(
            "c1",
            Arc::new(Mutex::new(Box::new(Collect(Arc::clone(&received))))),
            Box::new(|| {}),
        );
        hub.subscribe("c1");
        hub.publish(event("first"));
        hub.publish(event("second"));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let text =
                String::from_utf8_lossy(&received.lock().unwrap_or_else(PoisonError::into_inner))
                    .into_owned();
            if text.lines().count() == 2 {
                let first = text.find("first").unwrap();
                assert!(first < text.find("second").unwrap());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "events did not arrive"
            );
            thread::yield_now();
        }
        assert_eq!(hub.subscribers(), 1);
        hub.detach("c1");
        assert_eq!(hub.subscribers(), 0);
    }
}

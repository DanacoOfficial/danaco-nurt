// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Connections of the control channel and delivery of module events.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::ipc::protocol;
use crate::module::{EventTarget, ModuleEvent, ModuleEvents};

/// Writing half of a connection, shared by the connection thread and the
/// threads that publish events.
pub(crate) type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Closes a connection so that its blocked read returns.
pub(crate) type Closer = Box<dyn Fn() + Send + Sync>;

struct Peer {
    writer: SharedWriter,
    closer: Closer,
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
        self.lock().insert(
            client.to_owned(),
            Peer {
                writer,
                closer,
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

    fn targets(&self, target: &EventTarget) -> Vec<(String, SharedWriter)> {
        self.lock()
            .iter()
            .filter(|(client, peer)| match target {
                EventTarget::All => peer.subscribed,
                EventTarget::Client(wanted) => *client == wanted,
            })
            .map(|(client, peer)| (client.clone(), Arc::clone(&peer.writer)))
            .collect()
    }
}

impl ModuleEvents for Hub {
    fn publish(&self, event: ModuleEvent) {
        let line = protocol::event_line(&event.event, &event.data);
        for (client, writer) in self.targets(&event.target) {
            if write_line(&writer, &line).is_err() {
                if let Some(peer) = self.lock().remove(&client) {
                    (peer.closer)();
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

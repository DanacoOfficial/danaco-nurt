// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-service/tests/support/mod.rs
// Responsibility:   Test doubles for the service host: a virtual clock, mock modules and an independent client that applies the liveness rule of the journal.
// Dependencies:     nurt_contracts, serde_json, tempfile
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test doubles for the service host: a virtual clock, mock modules and an
//! independent client that applies the liveness rule of the journal.

pub(crate) mod clock;
pub(crate) mod d2_client;
pub(crate) mod modules;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nurt_contracts::config::Config;
use nurt_contracts::env::Clock;
use nurt_contracts::error::Error;
use nurt_contracts::stage::{CancelReason, CancelToken};
use nurt_service::ipc::client::ControlClient;
use nurt_service::module::ServiceModule;
use nurt_service::{ServicePaths, ServiceSetup, run};
use serde_json::Value;
use tempfile::TempDir;

/// Longest wait for a condition of a test.
pub(crate) const PATIENCE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(5);

/// Waits until `condition` holds; panics after [`PATIENCE`].
pub(crate) fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}

/// A service running on its own thread in a temporary state directory.
pub(crate) struct Running {
    _dir: TempDir,
    pub(crate) paths: ServicePaths,
    stop: CancelToken,
    thread: Mutex<Option<JoinHandle<Result<(), Error>>>>,
}

/// Preparation of the state directory before the service starts.
pub(crate) type Prepare = Box<dyn FnOnce(&Path)>;

/// Options of [`launch`].
#[derive(Default)]
pub(crate) struct Options {
    pub(crate) clock: Option<Arc<dyn Clock>>,
    pub(crate) heartbeat_ms: Option<u32>,
    pub(crate) prepare: Option<Prepare>,
}

/// Starts the service with the modules and waits until it answers.
pub(crate) fn launch(modules: Vec<Arc<dyn ServiceModule>>, options: Options) -> Running {
    let dir = tempfile::tempdir().unwrap();
    let paths = ServicePaths::new(dir.path().join("state"));
    if let Some(prepare) = options.prepare {
        prepare(paths.state_dir());
    }
    let mut config = Config::default();
    if let Some(heartbeat_ms) = options.heartbeat_ms {
        config.space.heartbeat_ms = heartbeat_ms;
    }
    let mut setup = ServiceSetup::with_paths(config, paths.clone()).without_signals();
    if let Some(clock) = options.clock {
        setup = setup.clock(clock);
    }
    let stop = setup.stop();
    let thread = thread::spawn(move || run(setup, modules));
    let running = Running {
        _dir: dir,
        paths,
        stop,
        thread: Mutex::new(Some(thread)),
    };
    eventually("the service to answer", || {
        running.paths.heartbeat_file().exists() && ControlClient::connect(&running.paths).is_ok()
    });
    running
}

impl Running {
    /// New connection to the control channel.
    pub(crate) fn client(&self) -> ControlClient {
        ControlClient::connect(&self.paths).unwrap()
    }

    /// Result of `status`.
    pub(crate) fn status(&self) -> Value {
        self.client().call("status", Value::Null).unwrap().unwrap()
    }

    /// Stops the service and returns the result of `run`; the state
    /// directory stays until the value is dropped.
    pub(crate) fn shutdown(&self) -> Result<(), Error> {
        self.stop.cancel(CancelReason::Shutdown);
        let thread = self.thread.lock().unwrap().take();
        thread.map_or(Ok(()), |thread| thread.join().unwrap())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Reads the next event of a connection within [`PATIENCE`] and gives the
/// connection back.
pub(crate) fn next_event(client: ControlClient) -> (ControlClient, String, Value) {
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut client = client;
        let event = client.next_event();
        let _ = sender.send((client, event));
    });
    let (client, event) = receiver
        .recv_timeout(PATIENCE)
        .expect("no event arrived in time");
    let (name, data) = event.unwrap();
    (client, name, data)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

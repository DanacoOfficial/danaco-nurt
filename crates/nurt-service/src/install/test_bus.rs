// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::install::test_bus
// Responsibility:   Peer-to-peer D-Bus server with the systemd and login1 objects used by the.
// Dependencies:     zbus
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Peer-to-peer D-Bus server with the systemd and login1 objects used by the
//! installer, for tests without a system bus.
//!
//! Every client connection gets its own server connection over a socket
//! pair. Calls that change state are recorded as `Method arg...`;
//! `GetDefaultTarget`, `GetUser` and property reads are not recorded.

use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use zbus::connection::Builder;
use zbus::{Connection, OwnedObjectPath, block_on};

const SERVER_GUID: &str = "6e7572742d736572766963652d746573";
const USER_PATH: &str = "/org/freedesktop/login1/user/_1000";
const DEFAULT_TARGET: &str = "multi-user.target";

/// Answer of `SetUserLinger`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LingerPolicy {
    /// Lingering is enabled.
    Allow,
    /// The call fails with `org.freedesktop.DBus.Error.AccessDenied`.
    Deny,
}

#[derive(Clone)]
struct State {
    calls: Arc<Mutex<Vec<String>>>,
    linger: Arc<AtomicBool>,
    policy: LingerPolicy,
    unit_path: Vec<String>,
}

impl State {
    fn record(&self, call: String) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

struct Systemd(State);

#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl Systemd {
    fn reload(&self) {
        self.0.record("Reload".to_owned());
    }

    fn enable_unit_files(
        &self,
        files: Vec<String>,
        _runtime: bool,
        _force: bool,
    ) -> (bool, Vec<(String, String, String)>) {
        self.0
            .record(format!("EnableUnitFiles {}", files.join(" ")));
        (false, Vec::new())
    }

    fn disable_unit_files(
        &self,
        files: Vec<String>,
        _runtime: bool,
    ) -> Vec<(String, String, String)> {
        self.0
            .record(format!("DisableUnitFiles {}", files.join(" ")));
        Vec::new()
    }

    fn start_unit(&self, name: String, mode: String) -> OwnedObjectPath {
        self.0.record(format!("StartUnit {name} {mode}"));
        job_path()
    }

    fn restart_unit(&self, name: String, mode: String) -> OwnedObjectPath {
        self.0.record(format!("RestartUnit {name} {mode}"));
        job_path()
    }

    fn stop_unit(&self, name: String, mode: String) -> OwnedObjectPath {
        self.0.record(format!("StopUnit {name} {mode}"));
        job_path()
    }

    fn get_default_target(&self) -> String {
        DEFAULT_TARGET.to_owned()
    }

    #[zbus(property)]
    fn unit_path(&self) -> Vec<String> {
        self.0.unit_path.clone()
    }
}

fn job_path() -> OwnedObjectPath {
    OwnedObjectPath::try_from("/org/freedesktop/systemd1/job/1").unwrap_or_default()
}

struct LoginManager(State);

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl LoginManager {
    fn set_user_linger(&self, uid: u32, enable: bool, interactive: bool) -> zbus::fdo::Result<()> {
        self.0
            .record(format!("SetUserLinger {uid} {enable} {interactive}"));
        match self.0.policy {
            LingerPolicy::Allow => {
                self.0.linger.store(enable, Ordering::SeqCst);
                Ok(())
            }
            LingerPolicy::Deny => Err(zbus::fdo::Error::AccessDenied(
                "lingering is not permitted for this account".to_owned(),
            )),
        }
    }

    fn get_user(&self, _uid: u32) -> OwnedObjectPath {
        OwnedObjectPath::try_from(USER_PATH).unwrap_or_default()
    }
}

struct LoginUser(State);

#[zbus::interface(name = "org.freedesktop.login1.User")]
impl LoginUser {
    #[zbus(property)]
    fn linger(&self) -> bool {
        self.0.linger.load(Ordering::SeqCst)
    }
}

/// The fake systemd and login1 of one test.
pub(crate) struct FakeSystem {
    state: State,
    servers: Mutex<Vec<Connection>>,
}

impl FakeSystem {
    /// Server whose `SetUserLinger` follows the policy and whose manager
    /// reports no `UnitPath`.
    pub(crate) fn start(policy: LingerPolicy) -> Self {
        Self::start_with_unit_path(policy, Vec::new())
    }

    /// Server whose manager reports `unit_path` as its `UnitPath`.
    pub(crate) fn start_with_unit_path(policy: LingerPolicy, unit_path: Vec<String>) -> Self {
        Self {
            state: State {
                calls: Arc::default(),
                linger: Arc::default(),
                policy,
                unit_path,
            },
            servers: Mutex::new(Vec::new()),
        }
    }

    /// New peer-to-peer client connection.
    pub(crate) fn client(&self) -> Connection {
        let (server_end, client_end) = UnixStream::pair().unwrap();
        let state = self.state.clone();
        let server = thread::spawn(move || {
            block_on(
                Builder::unix_stream(server_end)
                    .server(SERVER_GUID)
                    .p2p()
                    .serve_at("/org/freedesktop/systemd1", Systemd(state.clone()))
                    .serve_at("/org/freedesktop/login1", LoginManager(state.clone()))
                    .serve_at(USER_PATH, LoginUser(state))
                    .build(),
            )
            .unwrap()
        });
        let client = block_on(Builder::unix_stream(client_end).p2p().build()).unwrap();
        self.servers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(server.join().unwrap());
        client
    }

    /// Recorded calls in order.
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

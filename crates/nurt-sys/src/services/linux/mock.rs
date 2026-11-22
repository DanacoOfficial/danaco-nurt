// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! In-process systemd manager served over a peer-to-peer D-Bus connection.
//!
//! The manager keeps its units in memory. A start or restart queues a job
//! that holds the unit in `activating/start` for a given number of unit
//! lookups (`GetUnit`, `LoadUnit`) and then moves it to its planned state, so
//! that the waiting of the stages can be tested on a virtual clock.

use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use zbus::connection::Builder;
use zbus::{Connection, Guid, OwnedObjectPath, interface};

use super::{MANAGER_PATH, SERVICE_SUFFIX};

/// Object served for a unit name that has no unit file.
const MISSING_PATH: &str = "/org/freedesktop/systemd1/unit/missing";
const JOB_PATH: &str = "/org/freedesktop/systemd1/job/1";
const NO_JOB_PATH: &str = "/";

/// Where a start or restart leads after `lookups` unit lookups.
#[derive(Debug, Clone)]
pub(super) struct Plan {
    pub(super) lookups: u32,
    pub(super) active: &'static str,
    pub(super) sub: &'static str,
    pub(super) result: &'static str,
}

impl Plan {
    pub(super) fn running_after(lookups: u32) -> Self {
        Self {
            lookups,
            active: "active",
            sub: "running",
            result: "success",
        }
    }

    pub(super) fn failed_after(lookups: u32) -> Self {
        Self {
            lookups,
            active: "failed",
            sub: "failed",
            result: "exit-code",
        }
    }
}

/// One unit of the manager.
#[derive(Debug, Clone)]
pub(super) struct MockUnit {
    pub(super) id: String,
    pub(super) description: String,
    pub(super) active: &'static str,
    pub(super) sub: &'static str,
    pub(super) file_state: Option<&'static str>,
    pub(super) loaded: bool,
    pub(super) main_pid: u32,
    pub(super) user: String,
    pub(super) result: &'static str,
    pub(super) start: Plan,
    pending: Option<Plan>,
}

impl MockUnit {
    /// A loaded unit with a unit file in `/etc/systemd/system`.
    pub(super) fn new(name: &str, active: &'static str, sub: &'static str) -> Self {
        Self {
            id: format!("{name}{SERVICE_SUFFIX}"),
            description: format!("{name} daemon"),
            active,
            sub,
            file_state: Some("enabled"),
            loaded: true,
            main_pid: 0,
            user: String::new(),
            result: "success",
            start: Plan::running_after(1),
            pending: None,
        }
    }

    pub(super) fn with_file_state(mut self, state: Option<&'static str>) -> Self {
        self.file_state = state;
        self
    }

    pub(super) fn not_loaded(mut self) -> Self {
        self.loaded = false;
        self
    }

    pub(super) fn with_start(mut self, plan: Plan) -> Self {
        self.start = plan;
        self
    }

    fn fragment_path(&self) -> String {
        if self.file_state.is_some() {
            format!("/etc/systemd/system/{}", self.id)
        } else {
            String::new()
        }
    }
}

/// State of the manager shared with the test.
#[derive(Debug, Default)]
pub(super) struct MockState {
    pub(super) units: Vec<MockUnit>,
    /// Calls of the manager methods, shared with the transaction of a test.
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    /// Every changing call fails with `AccessDenied`.
    pub(super) deny: bool,
    /// The manager predates `ListUnitsByPatterns` (systemd before 230).
    pub(super) legacy: bool,
}

pub(super) type Shared = Arc<Mutex<MockState>>;

fn lock(state: &Shared) -> MutexGuard<'_, MockState> {
    state.lock().unwrap()
}

/// Object path of a unit, escaped as systemd does.
pub(super) fn object_path(id: &str) -> String {
    let mut path = String::from("/org/freedesktop/systemd1/unit/");
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() {
            path.push(char::from(byte));
        } else {
            path.push_str(&format!("_{byte:02x}"));
        }
    }
    path
}

fn owned_path(path: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(path.to_owned()).unwrap()
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop")]
enum MockError {
    #[zbus(error)]
    ZBus(zbus::Error),
    #[zbus(name = "systemd1.NoSuchUnit")]
    NoSuchUnit(String),
    #[zbus(name = "DBus.Error.AccessDenied")]
    AccessDenied(String),
    #[zbus(name = "DBus.Error.UnknownMethod")]
    UnknownMethod(String),
}

type UnitRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    OwnedObjectPath,
    u32,
    String,
    OwnedObjectPath,
);

struct Manager {
    state: Shared,
}

impl Manager {
    fn record(&self, call: String) {
        lock(&self.state).calls.lock().unwrap().push(call);
    }

    fn rows(&self) -> Vec<UnitRow> {
        lock(&self.state)
            .units
            .iter()
            .filter(|unit| unit.loaded)
            .map(|unit| {
                (
                    unit.id.clone(),
                    unit.description.clone(),
                    "loaded".to_owned(),
                    unit.active.to_owned(),
                    unit.sub.to_owned(),
                    String::new(),
                    owned_path(&object_path(&unit.id)),
                    0,
                    String::new(),
                    owned_path(NO_JOB_PATH),
                )
            })
            .collect()
    }

    fn files(&self) -> Vec<(String, String)> {
        lock(&self.state)
            .units
            .iter()
            .filter_map(|unit| {
                unit.file_state
                    .map(|state| (unit.fragment_path(), state.to_owned()))
            })
            .collect()
    }

    /// Advances the job of a unit by one lookup and returns its object.
    fn lookup(&self, name: &str, load: bool) -> Result<OwnedObjectPath, MockError> {
        let mut state = lock(&self.state);
        let Some(unit) = state.units.iter_mut().find(|unit| unit.id == name) else {
            return if load {
                Ok(owned_path(MISSING_PATH))
            } else {
                Err(MockError::NoSuchUnit(format!("Unit {name} not loaded.")))
            };
        };
        if !unit.loaded && !load {
            return Err(MockError::NoSuchUnit(format!("Unit {name} not loaded.")));
        }
        unit.loaded = true;
        if let Some(plan) = unit.pending.as_mut() {
            plan.lookups = plan.lookups.saturating_sub(1);
            if plan.lookups == 0 {
                unit.active = plan.active;
                unit.sub = plan.sub;
                unit.result = plan.result;
                unit.pending = None;
            }
        }
        Ok(owned_path(&object_path(name)))
    }

    fn change(&self, method: &str, name: &str, apply: impl FnOnce(&mut MockUnit)) -> Result<(), MockError> {
        self.record(format!("{method} {name}"));
        let mut state = lock(&self.state);
        if state.deny {
            return Err(MockError::AccessDenied(
                "Interactive authentication required.".to_owned(),
            ));
        }
        let unit = state
            .units
            .iter_mut()
            .find(|unit| unit.id == name)
            .ok_or_else(|| MockError::NoSuchUnit(format!("Unit {name} not found.")))?;
        apply(unit);
        Ok(())
    }

    fn queue_start(&self, method: &str, name: &str) -> Result<OwnedObjectPath, MockError> {
        self.change(method, name, |unit| {
            unit.active = "activating";
            unit.sub = "start";
            unit.pending = Some(unit.start.clone());
        })?;
        Ok(owned_path(JOB_PATH))
    }
}

#[interface(name = "org.freedesktop.systemd1.Manager")]
impl Manager {
    fn list_units(&self) -> Vec<UnitRow> {
        self.record("ListUnits".to_owned());
        self.rows()
    }

    fn list_units_by_patterns(
        &self,
        _states: Vec<String>,
        patterns: Vec<String>,
    ) -> Result<Vec<UnitRow>, MockError> {
        if lock(&self.state).legacy {
            return Err(MockError::UnknownMethod("ListUnitsByPatterns".to_owned()));
        }
        self.record(format!("ListUnitsByPatterns {}", patterns.join(",")));
        Ok(self.rows())
    }

    fn list_unit_files(&self) -> Vec<(String, String)> {
        self.record("ListUnitFiles".to_owned());
        self.files()
    }

    fn list_unit_files_by_patterns(
        &self,
        _states: Vec<String>,
        patterns: Vec<String>,
    ) -> Result<Vec<(String, String)>, MockError> {
        if lock(&self.state).legacy {
            return Err(MockError::UnknownMethod("ListUnitFilesByPatterns".to_owned()));
        }
        self.record(format!("ListUnitFilesByPatterns {}", patterns.join(",")));
        Ok(self.files())
    }

    fn get_unit(&self, name: String) -> Result<OwnedObjectPath, MockError> {
        self.lookup(&name, false)
    }

    fn load_unit(&self, name: String) -> Result<OwnedObjectPath, MockError> {
        self.lookup(&name, true)
    }

    fn start_unit(&self, name: String, _mode: String) -> Result<OwnedObjectPath, MockError> {
        self.queue_start("StartUnit", &name)
    }

    fn restart_unit(&self, name: String, _mode: String) -> Result<OwnedObjectPath, MockError> {
        self.queue_start("RestartUnit", &name)
    }

    fn stop_unit(&self, name: String, _mode: String) -> Result<OwnedObjectPath, MockError> {
        self.change("StopUnit", &name, |unit| {
            unit.active = "inactive";
            unit.sub = "dead";
        })?;
        Ok(owned_path(JOB_PATH))
    }

    fn enable_unit_files(
        &self,
        files: Vec<String>,
        _runtime: bool,
        _force: bool,
    ) -> Result<(bool, Vec<(String, String, String)>), MockError> {
        let name = files.join(",");
        self.change("EnableUnitFiles", &name, |unit| unit.file_state = Some("enabled"))?;
        Ok((true, Vec::new()))
    }

    fn disable_unit_files(
        &self,
        files: Vec<String>,
        _runtime: bool,
    ) -> Result<Vec<(String, String, String)>, MockError> {
        let name = files.join(",");
        self.change("DisableUnitFiles", &name, |unit| unit.file_state = Some("disabled"))?;
        Ok(Vec::new())
    }

    fn reload(&self) {
        self.record("Reload".to_owned());
    }
}

/// The unit interface of one unit object.
struct UnitObject {
    state: Shared,
    id: Option<String>,
}

impl UnitObject {
    fn read<T>(&self, missing: T, read: impl FnOnce(&MockUnit) -> T) -> T {
        let state = lock(&self.state);
        match self
            .id
            .as_ref()
            .and_then(|id| state.units.iter().find(|unit| &unit.id == id))
        {
            Some(unit) => read(unit),
            None => missing,
        }
    }
}

#[interface(name = "org.freedesktop.systemd1.Unit")]
impl UnitObject {
    #[zbus(property)]
    fn id(&self) -> String {
        self.read(String::new(), |unit| unit.id.clone())
    }

    #[zbus(property)]
    fn description(&self) -> String {
        self.read(String::new(), |unit| unit.description.clone())
    }

    #[zbus(property)]
    fn load_state(&self) -> String {
        self.read("not-found".to_owned(), |_| "loaded".to_owned())
    }

    #[zbus(property)]
    fn active_state(&self) -> String {
        self.read("inactive".to_owned(), |unit| unit.active.to_owned())
    }

    #[zbus(property)]
    fn sub_state(&self) -> String {
        self.read("dead".to_owned(), |unit| unit.sub.to_owned())
    }

    #[zbus(property)]
    fn unit_file_state(&self) -> String {
        self.read(String::new(), |unit| {
            unit.file_state.unwrap_or_default().to_owned()
        })
    }

    #[zbus(property)]
    fn fragment_path(&self) -> String {
        self.read(String::new(), MockUnit::fragment_path)
    }

    #[zbus(property)]
    fn job(&self) -> (u32, OwnedObjectPath) {
        self.read((0, owned_path(NO_JOB_PATH)), |unit| {
            if unit.pending.is_some() {
                (1, owned_path(JOB_PATH))
            } else {
                (0, owned_path(NO_JOB_PATH))
            }
        })
    }
}

/// The service interface of one unit object.
struct ServiceObject {
    state: Shared,
    id: String,
}

impl ServiceObject {
    fn read<T: Default>(&self, read: impl FnOnce(&MockUnit) -> T) -> T {
        lock(&self.state)
            .units
            .iter()
            .find(|unit| unit.id == self.id)
            .map(read)
            .unwrap_or_default()
    }
}

#[interface(name = "org.freedesktop.systemd1.Service")]
impl ServiceObject {
    #[zbus(property, name = "MainPID")]
    fn main_pid(&self) -> u32 {
        self.read(|unit| unit.main_pid)
    }

    #[zbus(property)]
    fn user(&self) -> String {
        self.read(|unit| unit.user.clone())
    }

    #[zbus(property)]
    fn result(&self) -> String {
        self.read(|unit| unit.result.to_owned())
    }

    #[zbus(property)]
    fn exec_main_status(&self) -> i32 {
        self.read(|unit| if unit.result == "success" { 0 } else { 1 })
    }
}

/// A served manager and the client connection to it.
pub(super) struct Served {
    pub(super) client: Connection,
    pub(super) state: Shared,
    _server: Connection,
}

/// Serves a manager with `units` and connects a client to it.
pub(super) fn serve(units: Vec<MockUnit>) -> Served {
    let ids: Vec<String> = units.iter().map(|unit| unit.id.clone()).collect();
    let state: Shared = Arc::new(Mutex::new(MockState {
        units,
        ..MockState::default()
    }));
    let (server_socket, client_socket) = UnixStream::pair().unwrap();
    let mut builder = Builder::unix_stream(server_socket)
        .server(Guid::generate())
        .p2p()
        .serve_at(
            MANAGER_PATH,
            Manager {
                state: state.clone(),
            },
        )
        .serve_at(
            MISSING_PATH,
            UnitObject {
                state: state.clone(),
                id: None,
            },
        );
    for id in ids {
        let path = object_path(&id);
        builder = builder
            .serve_at(
                path.as_str(),
                UnitObject {
                    state: state.clone(),
                    id: Some(id.clone()),
                },
            )
            .serve_at(
                path.as_str(),
                ServiceObject {
                    state: state.clone(),
                    id,
                },
            );
    }
    let server = thread::spawn(move || zbus::block_on(builder.build()));
    let client = zbus::block_on(Builder::unix_stream(client_socket).p2p().build()).unwrap();
    let server = server.join().unwrap().unwrap();
    Served {
        client,
        state,
        _server: server,
    }
}

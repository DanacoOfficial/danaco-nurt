// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! systemd backend over D-Bus.
//!
//! The backend talks to the manager object of systemd
//! (`org.freedesktop.systemd1(5)`) on the system bus for `scope: system` and
//! on the session bus, where the manager of the user runs, for
//! `scope: user`. Calls never allow interactive authorization, so an action
//! that polkit would only grant after asking for a password fails with
//! `E_PRIVILEGE`.

use std::collections::{HashMap, HashSet};
use std::path::Path as StdPath;
use std::sync::{Mutex, PoisonError};

use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Service, ServiceManager, ServiceScope, ServiceState, StartType};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::{Path, Value};
use zbus::wire::DynamicDeserialize;
use zbus::{Connection, DBusError, DynamicType, OwnedObjectPath, OwnedVariant, Structure, Variant};

use super::{
    ServiceBackend, ServiceQuery, ServiceStatus, scope_unsupported, service_error,
    service_not_found, service_privilege,
};

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

const MANAGER: ServiceManager = ServiceManager::Systemd;
const DESTINATION: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_INTERFACE: &str = "org.freedesktop.systemd1.Manager";
const UNIT_INTERFACE: &str = "org.freedesktop.systemd1.Unit";
const SERVICE_INTERFACE: &str = "org.freedesktop.systemd1.Service";
const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";

const SERVICE_SUFFIX: &str = ".service";
const SERVICE_PATTERN: &str = "*.service";
/// Job mode of `StartUnit`, `StopUnit` and `RestartUnit`: a conflicting
/// queued job is replaced.
const JOB_MODE: &str = "replace";
/// Present when systemd is the init system (`sd_booted(3)`).
const SYSTEMD_RUNTIME_DIR: &str = "/run/systemd/system";
const LOAD_STATE_NOT_FOUND: &str = "not-found";

const ERROR_NO_SUCH_UNIT: &str = "org.freedesktop.systemd1.NoSuchUnit";
const ERROR_ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";
const ERROR_AUTHORIZATION_REQUIRED: &str =
    "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired";
const ERROR_UNKNOWN_METHOD: &str = "org.freedesktop.DBus.Error.UnknownMethod";
const OS_CODE_NO_SYSTEMD: &str = "no_systemd";
const OS_CODE_NO_SYSTEM_BUS: &str = "no_system_bus";
const OS_CODE_NO_USER_BUS: &str = "no_user_bus";
const OS_CODE_DBUS: &str = "dbus";
const REQUIRED_PRIVILEGE: &str = "root";

/// One row of `ListUnits` and `ListUnitsByPatterns` (`a(ssssssouso)`): name,
/// description, load state, active state, sub state, followed unit, unit
/// object, job identifier, job type and job object.
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

/// One row of `ListUnitFiles` and `ListUnitFilesByPatterns` (`a(ss)`): path
/// of the unit file and its state.
type UnitFileRow = (String, String);

/// One change reported by `EnableUnitFiles` and `DisableUnitFiles`
/// (`a(sss)`): kind, link and target.
type UnitFileChange = (String, String, String);

/// Properties of one interface, as returned by `GetAll`.
type Properties = HashMap<String, OwnedVariant>;

/// Backend of the running system.
pub(crate) fn backend(_ctx: &ExecCtx) -> Box<dyn ServiceBackend + '_> {
    Box::new(SystemdBackend::new())
}

/// The systemd manager over D-Bus, with one connection per scope opened on
/// first use.
#[derive(Debug, Default)]
pub(crate) struct SystemdBackend {
    system: Mutex<Option<Connection>>,
    user: Mutex<Option<Connection>>,
}

impl SystemdBackend {
    fn new() -> Self {
        Self::default()
    }

    /// Backend over connections that are already open, for example to a
    /// manager served in the same process.
    #[cfg(test)]
    fn with_connections(system: Option<Connection>, user: Option<Connection>) -> Self {
        Self {
            system: Mutex::new(system),
            user: Mutex::new(user),
        }
    }

    fn connection(&self, scope: ServiceScope) -> Result<Connection> {
        let slot = match scope {
            ServiceScope::System => &self.system,
            ServiceScope::User => &self.user,
            _ => return Err(scope_unsupported(MANAGER, scope)),
        };
        let mut guard = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(connection) = guard.as_ref() {
            return Ok(connection.clone());
        }
        let connection = connect(scope)?;
        *guard = Some(connection.clone());
        Ok(connection)
    }

    /// Calls a method of systemd and decodes the reply; errors name `unit`.
    fn call<B, R>(
        &self,
        scope: ServiceScope,
        unit: &str,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Result<R>
    where
        B: serde::Serialize + DynamicType,
        R: for<'d> DynamicDeserialize<'d>,
    {
        let connection = self.connection(scope)?;
        zbus::block_on(async {
            let reply = connection
                .call_method(Some(DESTINATION), path, Some(interface), method, body)
                .await?;
            let body = reply.body();
            body.deserialize::<R>()
        })
        .map_err(|error| map_dbus_error(&error, unit))
    }

    fn manager_call<B, R>(
        &self,
        scope: ServiceScope,
        unit: &str,
        method: &str,
        body: &B,
    ) -> Result<R>
    where
        B: serde::Serialize + DynamicType,
        R: for<'d> DynamicDeserialize<'d>,
    {
        self.call(scope, unit, MANAGER_PATH, MANAGER_INTERFACE, method, body)
    }

    fn list_units(&self, scope: ServiceScope) -> Result<Vec<UnitRow>> {
        let filter = (Vec::<String>::new(), vec![SERVICE_PATTERN.to_owned()]);
        match self.manager_call(scope, "", "ListUnitsByPatterns", &filter) {
            Err(error) if has_os_code(&error, ERROR_UNKNOWN_METHOD) => {
                self.manager_call(scope, "", "ListUnits", &())
            }
            other => other,
        }
    }

    fn list_unit_files(&self, scope: ServiceScope) -> Result<Vec<UnitFileRow>> {
        let filter = (Vec::<String>::new(), vec![SERVICE_PATTERN.to_owned()]);
        match self.manager_call(scope, "", "ListUnitFilesByPatterns", &filter) {
            Err(error) if has_os_code(&error, ERROR_UNKNOWN_METHOD) => {
                self.manager_call(scope, "", "ListUnitFiles", &())
            }
            other => other,
        }
    }

    /// Object of a unit; a unit that is not loaded is loaded first.
    fn unit_path(&self, scope: ServiceScope, unit: &str) -> Result<OwnedObjectPath> {
        match self.manager_call(scope, unit, "GetUnit", &unit) {
            Err(error) if error.code() == ErrorCode::NotFound => {
                self.manager_call(scope, unit, "LoadUnit", &unit)
            }
            other => other,
        }
    }

    fn properties(
        &self,
        scope: ServiceScope,
        unit: &str,
        path: &OwnedObjectPath,
        interface: &str,
    ) -> Result<Properties> {
        self.call(
            scope,
            unit,
            path.as_str(),
            PROPERTIES_INTERFACE,
            "GetAll",
            &interface,
        )
    }

    /// Properties of the unit interface of a loaded service.
    fn unit_properties(
        &self,
        scope: ServiceScope,
        name: &str,
    ) -> Result<(String, OwnedObjectPath, Properties)> {
        let unit = unit_name(name);
        let path = self.unit_path(scope, &unit)?;
        let properties = self.properties(scope, &unit, &path, UNIT_INTERFACE)?;
        if text(&properties, "LoadState") == Some(LOAD_STATE_NOT_FOUND) {
            return Err(service_not_found(MANAGER, name));
        }
        Ok((unit, path, properties))
    }

    fn job(&self, scope: ServiceScope, name: &str, method: &str) -> Result<()> {
        let unit = unit_name(name);
        let _job: OwnedObjectPath = self.manager_call(scope, &unit, method, &(&unit, JOB_MODE))?;
        Ok(())
    }
}

impl ServiceBackend for SystemdBackend {
    fn manager(&self) -> ServiceManager {
        MANAGER
    }

    fn list(&self, query: &ServiceQuery) -> Result<Vec<Service>> {
        let units = self.list_units(query.scope)?;
        let files = self.list_unit_files(query.scope)?;
        Ok(merge_units(units, files, query.scope))
    }

    fn get(&self, name: &str, scope: ServiceScope) -> Result<Service> {
        let (unit, path, unit_properties) = self.unit_properties(scope, name)?;
        let service_properties = self.properties(scope, &unit, &path, SERVICE_INTERFACE)?;
        Ok(service_from_properties(
            &unit,
            scope,
            &unit_properties,
            &service_properties,
        ))
    }

    fn status(&self, name: &str, scope: ServiceScope) -> Result<ServiceStatus> {
        let (unit, path, properties) = self.unit_properties(scope, name)?;
        let active = text(&properties, "ActiveState").unwrap_or_default();
        let sub = text(&properties, "SubState").unwrap_or_default();
        let mut state = map_active_state(active, sub);
        if job_pending(&properties) {
            state = pending_state(state);
        }
        let result = if state == ServiceState::Failed {
            let service = self.properties(scope, &unit, &path, SERVICE_INTERFACE)?;
            text(&service, "Result").map(str::to_owned)
        } else {
            None
        };
        Ok(ServiceStatus {
            state,
            state_raw: Some(format!("{active}/{sub}")),
            result,
        })
    }

    fn start(&self, name: &str, scope: ServiceScope) -> Result<()> {
        self.job(scope, name, "StartUnit")
    }

    fn stop(&self, name: &str, scope: ServiceScope) -> Result<()> {
        self.job(scope, name, "StopUnit")
    }

    fn restart(&self, name: &str, scope: ServiceScope) -> Result<()> {
        self.job(scope, name, "RestartUnit")
    }

    fn enable(&self, name: &str, scope: ServiceScope) -> Result<()> {
        let unit = unit_name(name);
        let runtime = false;
        let force = false;
        let _changes: (bool, Vec<UnitFileChange>) = self.manager_call(
            scope,
            &unit,
            "EnableUnitFiles",
            &(vec![&unit], runtime, force),
        )?;
        self.manager_call::<_, ()>(scope, &unit, "Reload", &())
    }

    fn disable(&self, name: &str, scope: ServiceScope) -> Result<()> {
        let unit = unit_name(name);
        let runtime = false;
        let _changes: Vec<UnitFileChange> =
            self.manager_call(scope, &unit, "DisableUnitFiles", &(vec![&unit], runtime))?;
        self.manager_call::<_, ()>(scope, &unit, "Reload", &())
    }
}

fn connect(scope: ServiceScope) -> Result<Connection> {
    match scope {
        ServiceScope::User => zbus::block_on(Connection::session())
            .map_err(|error| unavailable(OS_CODE_NO_USER_BUS, &error.to_string())),
        _ if !StdPath::new(SYSTEMD_RUNTIME_DIR).is_dir() => Err(unavailable(
            OS_CODE_NO_SYSTEMD,
            "systemd is not the service manager of this system",
        )),
        _ => zbus::block_on(Connection::system())
            .map_err(|error| unavailable(OS_CODE_NO_SYSTEM_BUS, &error.to_string())),
    }
}

/// `E_SERVICE` for a manager that cannot be reached.
fn unavailable(os_code: &str, os_message: &str) -> Error {
    let mut error = Error::new(
        ErrorCode::Service,
        format!("the systemd manager cannot be reached: {os_message}"),
    )
    .with_problem(os_code)
    .with_detail("manager", MANAGER.as_str().into())
    .with_detail("unit", "".into())
    .with_detail("os_code", os_code.into())
    .with_detail("os_message", os_message.into());
    if os_code == OS_CODE_NO_USER_BUS {
        error = error.with_fix(format!("services scope: {}", ServiceScope::System));
    }
    error
}

fn has_os_code(error: &Error, code: &str) -> bool {
    error
        .details
        .get("os_code")
        .and_then(serde_json::Value::as_str)
        == Some(code)
}

/// Name and message of a D-Bus error reply.
fn dbus_error_parts(error: &zbus::Error) -> Option<(String, String)> {
    match error {
        zbus::Error::MethodError(name, detail, _) => {
            Some((name.as_str().to_owned(), detail.clone().unwrap_or_default()))
        }
        zbus::Error::FDO(error) => Some((
            error.name().as_str().to_owned(),
            error.description().unwrap_or_default().to_owned(),
        )),
        _ => None,
    }
}

/// Maps an error of a call about `unit` to the error of the language.
fn map_dbus_error(error: &zbus::Error, unit: &str) -> Error {
    let name = service_name(unit).unwrap_or(unit);
    match dbus_error_parts(error) {
        Some((code, _)) if code == ERROR_NO_SUCH_UNIT => service_not_found(MANAGER, name),
        Some((code, message))
            if code == ERROR_ACCESS_DENIED || code == ERROR_AUTHORIZATION_REQUIRED =>
        {
            service_privilege(MANAGER, name, REQUIRED_PRIVILEGE)
                .with_problem(code)
                .with_detail("os_message", message.into())
        }
        Some((code, message)) => service_error(MANAGER, name, &code, &message),
        None => service_error(MANAGER, name, OS_CODE_DBUS, &error.to_string()),
    }
}

/// Unit name of a service: `nginx` and `nginx.service` name the same unit.
pub(super) fn unit_name(name: &str) -> String {
    if name.ends_with(SERVICE_SUFFIX) {
        name.to_owned()
    } else {
        format!("{name}{SERVICE_SUFFIX}")
    }
}

/// Service name of a unit of type `service`; `None` for other unit types.
fn service_name(unit: &str) -> Option<&str> {
    unit.strip_suffix(SERVICE_SUFFIX)
        .filter(|name| !name.is_empty())
}

/// State of a unit from its `ActiveState` and `SubState`.
///
/// A unit that ran to completion and remains active (`active/exited`, a
/// `oneshot` service with `RemainAfterExit=yes`) runs no process and is
/// reported as stopped.
pub(super) fn map_active_state(active: &str, sub: &str) -> ServiceState {
    match active {
        "active" if sub == "exited" => ServiceState::Stopped,
        "active" | "reloading" => ServiceState::Running,
        "inactive" => ServiceState::Stopped,
        "activating" => ServiceState::Starting,
        "deactivating" => ServiceState::Stopping,
        "failed" => ServiceState::Failed,
        _ => ServiceState::Unknown,
    }
}

/// Start type of a unit from its `UnitFileState`; a unit without a unit
/// file is started on request only.
pub(super) fn map_unit_file_state(state: Option<&str>) -> StartType {
    match state {
        None | Some("") => StartType::Manual,
        Some("enabled" | "enabled-runtime") => StartType::Automatic,
        Some("disabled" | "masked" | "masked-runtime") => StartType::Disabled,
        Some("static") => StartType::Static,
        Some("indirect") => StartType::OnDemand,
        Some(_) => StartType::Unknown,
    }
}

/// State while a job of the unit is queued or running: the job leads away
/// from an active unit and towards a stopped one.
fn pending_state(state: ServiceState) -> ServiceState {
    match state {
        ServiceState::Running | ServiceState::Stopping => ServiceState::Stopping,
        _ => ServiceState::Starting,
    }
}

fn text<'p>(properties: &'p Properties, name: &str) -> Option<&'p str> {
    properties
        .get(name)
        .and_then(|value| <&str>::try_from(value).ok())
        .filter(|value| !value.is_empty())
}

fn job_pending(properties: &Properties) -> bool {
    properties
        .get("Job")
        .and_then(|value| <&Structure<'_>>::try_from(value).ok())
        .and_then(|job| job.fields().first())
        .is_some_and(|id| matches!(id, Variant::U32(id) if *id != 0))
}

/// Facts of one unit common to listings and to unit properties.
struct UnitFacts<'a> {
    unit: &'a str,
    description: Option<&'a str>,
    load_state: Option<&'a str>,
    active: &'a str,
    sub: &'a str,
    file_state: Option<&'a str>,
    path: Option<&'a str>,
}

impl UnitFacts<'_> {
    fn into_service(self, scope: ServiceScope) -> Service {
        let mut os_raw = IndexMap::new();
        if let Some(load_state) = self.load_state {
            os_raw.insert("load_state".to_owned(), Value::Text(load_state.to_owned()));
        }
        os_raw.insert(
            "active_state".to_owned(),
            Value::Text(self.active.to_owned()),
        );
        os_raw.insert("sub_state".to_owned(), Value::Text(self.sub.to_owned()));
        if let Some(file_state) = self.file_state {
            os_raw.insert(
                "unit_file_state".to_owned(),
                Value::Text(file_state.to_owned()),
            );
        }
        Service {
            name: service_name(self.unit).unwrap_or(self.unit).to_owned(),
            state: map_active_state(self.active, self.sub),
            start_type: map_unit_file_state(self.file_state),
            manager: MANAGER,
            description: self.description.map(str::to_owned),
            state_raw: Some(format!("{}/{}", self.active, self.sub)),
            scope: Some(scope),
            path: self.path.map(Path::from),
            os_raw: Some(os_raw),
            ..Service::default()
        }
    }
}

/// Services of the loaded units together with the services that have a
/// unit file but are not loaded, sorted by name. Templates (`getty@`) and
/// units that are referenced but have no unit file are left out.
fn merge_units(units: Vec<UnitRow>, files: Vec<UnitFileRow>, scope: ServiceScope) -> Vec<Service> {
    let file_states: HashMap<String, (String, String)> = files
        .into_iter()
        .filter_map(|(path, state)| {
            let unit = path.rsplit('/').next()?.to_owned();
            Some((unit, (path, state)))
        })
        .collect();
    let mut seen = HashSet::new();
    let mut services = Vec::new();
    for row in &units {
        let (unit, description, load_state, active, sub) = (&row.0, &row.1, &row.2, &row.3, &row.4);
        if service_name(unit).is_none() || load_state == LOAD_STATE_NOT_FOUND {
            continue;
        }
        seen.insert(unit.clone());
        let file = file_states.get(unit);
        let facts = UnitFacts {
            unit,
            description: Some(description.as_str()).filter(|text| !text.is_empty()),
            load_state: Some(load_state),
            active,
            sub,
            file_state: file.map(|(_, state)| state.as_str()),
            path: file.map(|(path, _)| path.as_str()),
        };
        services.push(facts.into_service(scope));
    }
    for (unit, (path, state)) in &file_states {
        let template = unit.ends_with(&format!("@{SERVICE_SUFFIX}"));
        if seen.contains(unit) || service_name(unit).is_none() || template {
            continue;
        }
        let facts = UnitFacts {
            unit,
            description: None,
            load_state: None,
            active: "inactive",
            sub: "dead",
            file_state: Some(state),
            path: Some(path),
        };
        services.push(facts.into_service(scope));
    }
    services.sort_by(|left, right| left.name.cmp(&right.name));
    services
}

/// Record of one service from the properties of its unit and service
/// interfaces.
fn service_from_properties(
    unit: &str,
    scope: ServiceScope,
    unit_properties: &Properties,
    service_properties: &Properties,
) -> Service {
    let id = text(unit_properties, "Id").unwrap_or(unit);
    let facts = UnitFacts {
        unit: id,
        description: text(unit_properties, "Description"),
        load_state: text(unit_properties, "LoadState"),
        active: text(unit_properties, "ActiveState").unwrap_or_default(),
        sub: text(unit_properties, "SubState").unwrap_or_default(),
        file_state: text(unit_properties, "UnitFileState"),
        path: text(unit_properties, "FragmentPath"),
    };
    let mut service = facts.into_service(scope);
    service.pid = service_properties
        .get("MainPID")
        .and_then(|value| u32::try_from(value).ok())
        .filter(|pid| *pid != 0)
        .map(u64::from);
    service.account = text(service_properties, "User").map(str::to_owned);
    if let Some(os_raw) = service.os_raw.as_mut() {
        if let Some(result) = text(service_properties, "Result") {
            os_raw.insert("result".to_owned(), Value::Text(result.to_owned()));
        }
        if let Some(status) = service_properties
            .get("ExecMainStatus")
            .and_then(|value| i32::try_from(value).ok())
        {
            os_raw.insert("exec_main_status".to_owned(), Value::Int(i64::from(status)));
        }
    }
    service
}

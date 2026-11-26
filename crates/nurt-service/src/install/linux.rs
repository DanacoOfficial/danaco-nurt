// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! systemd units and lingering over D-Bus, without `systemctl` or
//! `loginctl`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::config::ServiceMode;
use nurt_contracts::error::{Error, ErrorCode};
use serde_json::json;
use zbus::connection::Builder;
use zbus::{Connection, DBusError, Message, OwnedObjectPath, OwnedVariant, block_on};

use super::detect::ModeChoice;
use super::{
    Backend, InstallReport, InstallRequest, Manifest, UninstallReport, files_named,
    privilege_error, remove_files, write_file,
};

/// Name of the systemd unit.
pub(crate) const UNIT_NAME: &str = "nurt-service.service";

const SYSTEMD_DESTINATION: &str = "org.freedesktop.systemd1";
const SYSTEMD_PATH: &str = "/org/freedesktop/systemd1";
const SYSTEMD_MANAGER: &str = "org.freedesktop.systemd1.Manager";
const LOGIN_DESTINATION: &str = "org.freedesktop.login1";
const LOGIN_PATH: &str = "/org/freedesktop/login1";
const LOGIN_MANAGER: &str = "org.freedesktop.login1.Manager";
const LOGIN_USER: &str = "org.freedesktop.login1.User";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const JOB_MODE: &str = "replace";
const BUS_TIMEOUT: Duration = Duration::from_secs(10);
const SYSTEM_UNIT_DIR: &str = "/etc/systemd/system";
const DENIED_ERRORS: &[&str] = &[
    "org.freedesktop.DBus.Error.AccessDenied",
    "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired",
];
const LINGER_DENIED_NOTE: &str = "lingering could not be enabled: the system refused the request; the service starts when the account logs in, until then the journal keeps no data and schedules report `service_not_running`";
const LINGER_UNKNOWN_NOTE: &str = "lingering could not be checked: login1 is not reachable; the service may start only when the account logs in";

/// Error of a D-Bus call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BusError {
    /// D-Bus error name, for example `org.freedesktop.DBus.Error.AccessDenied`.
    pub(crate) name: Option<String>,
    /// Description.
    pub(crate) message: String,
}

impl From<zbus::Error> for BusError {
    fn from(error: zbus::Error) -> Self {
        let name = match &error {
            zbus::Error::MethodError(name, _, _) => Some(name.to_string()),
            zbus::Error::FDO(fdo) => Some(fdo.name().to_string()),
            _ => None,
        };
        Self {
            name,
            message: error.to_string(),
        }
    }
}

impl BusError {
    fn denied(&self) -> bool {
        self.name
            .as_deref()
            .is_some_and(|name| DENIED_ERRORS.contains(&name))
    }
}

/// `org.freedesktop.systemd1.Manager` of one bus.
pub(crate) trait Manager {
    fn reload(&self) -> Result<(), BusError>;
    fn enable(&self, unit: &str) -> Result<(), BusError>;
    fn disable(&self, unit: &str) -> Result<(), BusError>;
    fn start(&self, unit: &str) -> Result<(), BusError>;
    fn restart(&self, unit: &str) -> Result<(), BusError>;
    fn stop(&self, unit: &str) -> Result<(), BusError>;
    fn default_target(&self) -> Result<String, BusError>;
}

/// `org.freedesktop.login1`.
pub(crate) trait Login {
    fn set_linger(&self, uid: u32) -> Result<(), BusError>;
    fn linger(&self, uid: u32) -> Result<bool, BusError>;
}

/// Access to the buses.
pub(crate) trait Buses {
    fn session(&self) -> Result<Box<dyn Manager>, BusError>;
    fn system(&self) -> Result<Box<dyn Manager>, BusError>;
    fn login(&self) -> Result<Box<dyn Login>, BusError>;
}

/// Connection to a bus or to a peer.
pub(crate) struct DbusPeer {
    connection: Connection,
    peer_to_peer: bool,
}

impl DbusPeer {
    /// Wraps a connection; on a peer-to-peer connection messages carry no
    /// destination.
    pub(crate) fn new(connection: Connection, peer_to_peer: bool) -> Self {
        Self {
            connection,
            peer_to_peer,
        }
    }

    fn call<B>(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Result<Message, BusError>
    where
        B: serde::Serialize + zbus::DynamicType,
    {
        let destination = (!self.peer_to_peer).then_some(destination);
        block_on(
            self.connection
                .call_method(destination, path, Some(interface), method, body),
        )
        .map_err(BusError::from)
    }

    fn manager<B>(&self, method: &str, body: &B) -> Result<Message, BusError>
    where
        B: serde::Serialize + zbus::DynamicType,
    {
        self.call(
            SYSTEMD_DESTINATION,
            SYSTEMD_PATH,
            SYSTEMD_MANAGER,
            method,
            body,
        )
    }
}

impl Manager for DbusPeer {
    fn reload(&self) -> Result<(), BusError> {
        self.manager("Reload", &()).map(drop)
    }

    fn enable(&self, unit: &str) -> Result<(), BusError> {
        self.manager("EnableUnitFiles", &(vec![unit], false, true))
            .map(drop)
    }

    fn disable(&self, unit: &str) -> Result<(), BusError> {
        self.manager("DisableUnitFiles", &(vec![unit], false))
            .map(drop)
    }

    fn start(&self, unit: &str) -> Result<(), BusError> {
        self.manager("StartUnit", &(unit, JOB_MODE)).map(drop)
    }

    fn restart(&self, unit: &str) -> Result<(), BusError> {
        self.manager("RestartUnit", &(unit, JOB_MODE)).map(drop)
    }

    fn stop(&self, unit: &str) -> Result<(), BusError> {
        self.manager("StopUnit", &(unit, JOB_MODE)).map(drop)
    }

    fn default_target(&self) -> Result<String, BusError> {
        let reply = self.manager("GetDefaultTarget", &())?;
        reply.body().deserialize::<String>().map_err(BusError::from)
    }
}

impl Login for DbusPeer {
    fn set_linger(&self, uid: u32) -> Result<(), BusError> {
        self.call(
            LOGIN_DESTINATION,
            LOGIN_PATH,
            LOGIN_MANAGER,
            "SetUserLinger",
            &(uid, true, false),
        )
        .map(drop)
    }

    fn linger(&self, uid: u32) -> Result<bool, BusError> {
        let reply = self.call(
            LOGIN_DESTINATION,
            LOGIN_PATH,
            LOGIN_MANAGER,
            "GetUser",
            &(uid,),
        )?;
        let user: OwnedObjectPath = reply.body().deserialize().map_err(BusError::from)?;
        let reply = self.call(
            LOGIN_DESTINATION,
            user.as_str(),
            PROPERTIES,
            "Get",
            &(LOGIN_USER, "Linger"),
        )?;
        let value: OwnedVariant = reply.body().deserialize().map_err(BusError::from)?;
        bool::try_from(value).map_err(BusError::from)
    }
}

/// The session and system buses of the host.
pub(crate) struct SystemBuses;

fn connect(builder: Builder<'_>) -> Result<DbusPeer, BusError> {
    let connection = block_on(builder.method_timeout(BUS_TIMEOUT).build())?;
    Ok(DbusPeer::new(connection, false))
}

impl Buses for SystemBuses {
    fn session(&self) -> Result<Box<dyn Manager>, BusError> {
        connect(Builder::session()).map(|peer| Box::new(peer) as Box<dyn Manager>)
    }

    fn system(&self) -> Result<Box<dyn Manager>, BusError> {
        connect(Builder::system()).map(|peer| Box::new(peer) as Box<dyn Manager>)
    }

    fn login(&self) -> Result<Box<dyn Login>, BusError> {
        connect(Builder::system()).map(|peer| Box::new(peer) as Box<dyn Login>)
    }
}

/// Default target of the system manager, when the system bus answers.
pub(crate) fn default_target() -> Option<String> {
    SystemBuses.system().ok()?.default_target().ok()
}

/// Locations of unit files and the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) user_unit_dir: PathBuf,
    pub(crate) system_unit_dir: PathBuf,
    pub(crate) uid: u32,
    pub(crate) user: String,
}

impl Layout {
    /// Locations of the running account: `$XDG_CONFIG_HOME/systemd/user`
    /// (by default `~/.config/systemd/user`) and `/etc/systemd/system`.
    pub(crate) fn host() -> Result<Self, Error> {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::Io,
                    "HOME is not set; cannot locate the systemd user unit directory",
                )
            })?;
        let uid = nix::unistd::getuid();
        let user = nix::unistd::User::from_uid(uid)
            .ok()
            .flatten()
            .map(|user| user.name)
            .unwrap_or_else(|| uid.to_string());
        Ok(Self {
            user_unit_dir: config_home.join("systemd").join("user"),
            system_unit_dir: PathBuf::from(SYSTEM_UNIT_DIR),
            uid: uid.as_raw(),
            user,
        })
    }
}

/// `ExecStart=` value: the program quoted when needed, then `service run`.
fn exec_start(executable: &Path) -> String {
    let text = executable.display().to_string().replace('%', "%%");
    let plain = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c));
    let program = if plain {
        text
    } else {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    };
    format!("{program} service run")
}

fn unit(executable: &Path, account: Option<(&str, &str)>, wanted_by: &str) -> String {
    let identity = account
        .map(|(user, group)| format!("User={user}\nGroup={group}\n"))
        .unwrap_or_default();
    format!(
        "[Unit]\nDescription=Danaco Nurt background service\nDocumentation=https://github.com/DanacoOfficial/nurt\n\n\
         [Service]\nType=simple\nExecStart={}\n{identity}Restart=on-failure\nRestartSec=5\n\n\
         [Install]\nWantedBy={wanted_by}\n",
        exec_start(executable)
    )
}

/// User unit for mode `user`.
pub(crate) fn user_unit(executable: &Path) -> String {
    unit(executable, None, "default.target")
}

/// System unit for mode `system`, run as the account and its group.
pub(crate) fn system_unit(executable: &Path, user: &str, group: &str) -> String {
    unit(executable, Some((user, group)), "multi-user.target")
}

fn service_error(action: &str, error: &BusError) -> Error {
    Error::new(
        ErrorCode::Service,
        format!("systemd refused to {action} {UNIT_NAME}: {}", error.message),
    )
}

/// Writes the unit file and tells whether a unit file with other content was
/// there before, so that a running service has to be started again to use
/// the new program or settings.
fn write_unit(unit_file: &Path, content: &str) -> Result<bool, Error> {
    let previous = std::fs::read(unit_file).ok();
    write_file(unit_file, content.as_bytes())?;
    Ok(previous.is_some_and(|old| old != content.as_bytes()))
}

fn register(manager: &dyn Manager, changed: bool) -> Result<(), Error> {
    manager
        .reload()
        .map_err(|error| service_error("reload before enabling", &error))?;
    manager
        .enable(UNIT_NAME)
        .map_err(|error| service_error("enable", &error))?;
    if changed {
        manager
            .restart(UNIT_NAME)
            .map_err(|error| service_error("restart", &error))
    } else {
        manager
            .start(UNIT_NAME)
            .map_err(|error| service_error("start", &error))
    }
}

/// Installs the unit of the chosen mode.
pub(crate) fn install_with(
    request: &InstallRequest,
    executable: &Path,
    choice: ModeChoice,
    buses: &dyn Buses,
    layout: &Layout,
    elevated: bool,
) -> Result<InstallReport, Error> {
    match choice.mode {
        ServiceMode::User => install_user(request, executable, choice, buses, layout),
        ServiceMode::System if !elevated => Err(privilege_error("installing")),
        ServiceMode::System => install_system(request, executable, choice, buses, layout),
    }
}

fn install_user(
    request: &InstallRequest,
    executable: &Path,
    choice: ModeChoice,
    buses: &dyn Buses,
    layout: &Layout,
) -> Result<InstallReport, Error> {
    let unit_file = layout.user_unit_dir.join(UNIT_NAME);
    let changed = write_unit(&unit_file, &user_unit(executable))?;
    let mut manifest = Manifest::new(ServiceMode::User, Backend::SystemdUser, executable);
    manifest.files.push(unit_file.clone());
    let manager = match buses.session() {
        Ok(manager) => manager,
        Err(error) => {
            manifest.write(&request.paths)?;
            return Err(Error::new(
                ErrorCode::Service,
                format!(
                    "the systemd user manager is not reachable ({}); the unit was written to {}; log in with a session that runs the user manager, or install in mode `system`",
                    error.message,
                    unit_file.display()
                ),
            )
            .with_problem("no_user_bus")
            .with_detail("unit", json!(unit_file.display().to_string())));
        }
    };
    register(&*manager, changed)?;
    let mut notes = Vec::new();
    let linger = match buses.login() {
        Ok(login) => match login.set_linger(layout.uid) {
            Ok(()) => login.linger(layout.uid).ok(),
            Err(error) if error.denied() => {
                notes.push(LINGER_DENIED_NOTE.to_owned());
                login.linger(layout.uid).ok().or(Some(false))
            }
            Err(error) => {
                notes.push(format!("lingering could not be enabled: {}", error.message));
                Some(false)
            }
        },
        Err(_) => {
            notes.push(LINGER_UNKNOWN_NOTE.to_owned());
            None
        }
    };
    if linger == Some(false) && notes.is_empty() {
        notes.push(LINGER_DENIED_NOTE.to_owned());
    }
    manifest.linger = linger;
    manifest.write(&request.paths)?;
    Ok(InstallReport {
        mode: ServiceMode::User,
        mode_reason: choice.reason,
        backend: Backend::SystemdUser,
        files: manifest.files,
        started: true,
        linger,
        notes,
    })
}

fn install_system(
    request: &InstallRequest,
    executable: &Path,
    choice: ModeChoice,
    buses: &dyn Buses,
    layout: &Layout,
) -> Result<InstallReport, Error> {
    let user = request
        .account
        .clone()
        .or_else(|| {
            std::env::var("SUDO_USER")
                .ok()
                .filter(|name| !name.is_empty())
        })
        .unwrap_or_else(|| layout.user.clone());
    let group = primary_group(&user)?;
    let unit_file = layout.system_unit_dir.join(UNIT_NAME);
    let manager = buses.system().map_err(|error| {
        Error::new(
            ErrorCode::Service,
            format!(
                "the systemd system manager is not reachable: {}",
                error.message
            ),
        )
    })?;
    let changed = write_unit(&unit_file, &system_unit(executable, &user, &group))?;
    let mut manifest = Manifest::new(ServiceMode::System, Backend::SystemdSystem, executable);
    manifest.files.push(unit_file);
    manifest.account = Some(user);
    manifest.write(&request.paths)?;
    register(&*manager, changed)?;
    Ok(InstallReport {
        mode: ServiceMode::System,
        mode_reason: choice.reason,
        backend: Backend::SystemdSystem,
        files: manifest.files,
        started: true,
        linger: None,
        notes: Vec::new(),
    })
}

fn primary_group(user: &str) -> Result<String, Error> {
    let account = nix::unistd::User::from_name(user)
        .ok()
        .flatten()
        .ok_or_else(|| {
            Error::new(
                ErrorCode::NotFound,
                format!("there is no account `{user}`; name an existing account"),
            )
        })?;
    Ok(nix::unistd::Group::from_gid(account.gid)
        .ok()
        .flatten()
        .map_or_else(|| account.gid.to_string(), |group| group.name))
}

/// Stops and disables the unit and deletes the files of the manifest.
pub(crate) fn uninstall_with(
    manifest: &Manifest,
    buses: &dyn Buses,
) -> Result<UninstallReport, Error> {
    if manifest.mode == ServiceMode::System && !crate::process::is_elevated() {
        return Err(privilege_error("removing"));
    }
    let manager = match manifest.backend {
        Backend::SystemdSystem => buses.system(),
        _ => buses.session(),
    };
    let mut notes = Vec::new();
    match &manager {
        Ok(manager) => {
            let _ = manager.stop(UNIT_NAME);
            if let Err(error) = manager.disable(UNIT_NAME) {
                notes.push(format!("systemd did not disable {UNIT_NAME}: {}", error.message));
            }
        }
        Err(error) => notes.push(format!(
            "the systemd manager is not reachable ({}); a running service stops at the next restart of the manager",
            error.message
        )),
    }
    let files = files_named(&manifest.files, UNIT_NAME, &mut notes);
    let removed = remove_files(&files)?;
    if let Ok(manager) = &manager {
        let _ = manager.reload();
    }
    Ok(UninstallReport {
        mode: manifest.mode,
        backend: manifest.backend,
        removed,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::detect::ModeReason;
    use crate::install::io_error;
    use crate::install::test_bus::{FakeSystem, LingerPolicy};
    use crate::paths::ServicePaths;

    const GOLDEN_USER: &str = include_str!("../../tests/golden/nurt-service.user.service");
    const GOLDEN_SYSTEM: &str = include_str!("../../tests/golden/nurt-service.system.service");

    struct FakeBuses {
        system: FakeSystem,
        session_available: bool,
    }

    impl Buses for FakeBuses {
        fn session(&self) -> Result<Box<dyn Manager>, BusError> {
            if !self.session_available {
                return Err(BusError {
                    name: None,
                    message: "XDG_RUNTIME_DIR is not set".to_owned(),
                });
            }
            Ok(Box::new(DbusPeer::new(self.system.client(), true)))
        }

        fn system(&self) -> Result<Box<dyn Manager>, BusError> {
            Ok(Box::new(DbusPeer::new(self.system.client(), true)))
        }

        fn login(&self) -> Result<Box<dyn Login>, BusError> {
            Ok(Box::new(DbusPeer::new(self.system.client(), true)))
        }
    }

    fn fixture(dir: &Path) -> (InstallRequest, Layout) {
        let request = InstallRequest::new(ServicePaths::new(dir.join("state")));
        let layout = Layout {
            user_unit_dir: dir.join("config/systemd/user"),
            system_unit_dir: dir.join("etc/systemd/system"),
            uid: 1000,
            user: "dev".to_owned(),
        };
        (request, layout)
    }

    fn user_choice() -> ModeChoice {
        ModeChoice {
            mode: ServiceMode::User,
            reason: ModeReason::Requested,
        }
    }

    #[test]
    fn install_linux_user_unit_golden() {
        assert_eq!(user_unit(Path::new("/opt/nurt/bin/nurt")), GOLDEN_USER);
        assert_eq!(
            exec_start(Path::new("/opt/My Tools/nurt")),
            "\"/opt/My Tools/nurt\" service run"
        );
        assert_eq!(
            exec_start(Path::new("/opt/100%/nurt")),
            "\"/opt/100%%/nurt\" service run"
        );
    }

    #[test]
    fn install_linux_system_unit_golden() {
        assert_eq!(
            system_unit(Path::new("/opt/nurt/bin/nurt"), "nurt", "nurt"),
            GOLDEN_SYSTEM
        );
    }

    #[test]
    fn install_linger_via_login1_p2p() {
        let dir = tempfile::tempdir().unwrap();
        let (request, layout) = fixture(dir.path());
        let buses = FakeBuses {
            system: FakeSystem::start(LingerPolicy::Allow),
            session_available: true,
        };
        let report = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            user_choice(),
            &buses,
            &layout,
            false,
        )
        .unwrap();
        assert_eq!(report.linger, Some(true));
        assert!(report.notes.is_empty());
        assert_eq!(
            buses.system.calls(),
            [
                "Reload",
                "EnableUnitFiles nurt-service.service",
                "StartUnit nurt-service.service replace",
                "SetUserLinger 1000 true false"
            ]
        );
        let manifest = Manifest::read(&request.paths.install_manifest())
            .unwrap()
            .unwrap();
        assert_eq!(manifest.linger, Some(true));
        assert_eq!(manifest.files, [layout.user_unit_dir.join(UNIT_NAME)]);
        let removed = uninstall_with(&manifest, &buses).unwrap();
        assert_eq!(removed.removed, manifest.files);
        assert!(buses.system.calls().ends_with(&[
            "StopUnit nurt-service.service replace".to_owned(),
            "DisableUnitFiles nurt-service.service".to_owned(),
            "Reload".to_owned(),
        ]));
    }

    #[test]
    fn uninstall_linux_removes_only_the_unit_file_of_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let unit = dir.path().join(UNIT_NAME);
        let foreign = dir.path().join("precious.conf");
        std::fs::write(&unit, "unit").unwrap();
        std::fs::write(&foreign, "keep").unwrap();
        let mut manifest = Manifest::new(
            ServiceMode::User,
            Backend::SystemdUser,
            Path::new("/opt/nurt/bin/nurt"),
        );
        manifest.files = vec![unit.clone(), foreign.clone()];
        let buses = FakeBuses {
            system: FakeSystem::start(LingerPolicy::Allow),
            session_available: true,
        };
        let report = uninstall_with(&manifest, &buses).unwrap();
        assert_eq!(report.removed, [unit]);
        assert!(foreign.exists());
        assert!(report.notes.iter().any(|note| note.contains("precious.conf")));
    }

    #[test]
    fn install_linger_denied_continues_with_message() {
        let dir = tempfile::tempdir().unwrap();
        let (request, layout) = fixture(dir.path());
        let buses = FakeBuses {
            system: FakeSystem::start(LingerPolicy::Deny),
            session_available: true,
        };
        let report = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            user_choice(),
            &buses,
            &layout,
            false,
        )
        .unwrap();
        assert!(report.started);
        assert_eq!(report.linger, Some(false));
        assert!(report.notes[0].contains("service_not_running"));
    }

    #[test]
    fn install_again_with_another_program_restarts_the_unit() {
        let dir = tempfile::tempdir().unwrap();
        let (request, layout) = fixture(dir.path());
        let buses = FakeBuses {
            system: FakeSystem::start(LingerPolicy::Allow),
            session_available: true,
        };
        for program in ["/opt/nurt/bin/nurt", "/opt/nurt/bin/nurt", "/opt/other/nurt"] {
            install_with(
                &request,
                Path::new(program),
                user_choice(),
                &buses,
                &layout,
                false,
            )
            .unwrap();
        }
        let started: Vec<String> = buses
            .system
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("StartUnit") || call.starts_with("RestartUnit"))
            .collect();
        assert_eq!(
            started,
            [
                "StartUnit nurt-service.service replace",
                "StartUnit nurt-service.service replace",
                "RestartUnit nurt-service.service replace",
            ]
        );
    }

    #[test]
    fn install_no_user_bus_service_error_with_fix() {
        let dir = tempfile::tempdir().unwrap();
        let (request, layout) = fixture(dir.path());
        let buses = FakeBuses {
            system: FakeSystem::start(LingerPolicy::Allow),
            session_available: false,
        };
        let error = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            user_choice(),
            &buses,
            &layout,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Service);
        assert_eq!(error.problem.as_deref(), Some("no_user_bus"));
        assert!(error.message.contains("mode `system`"));
        assert!(layout.user_unit_dir.join(UNIT_NAME).is_file());
        let manifest = Manifest::read(&request.paths.install_manifest())
            .unwrap()
            .unwrap();
        assert_eq!(manifest.files.len(), 1);
    }

    #[test]
    fn install_system_mode_needs_privilege_and_writes_system_unit() {
        let dir = tempfile::tempdir().unwrap();
        let (mut request, layout) = fixture(dir.path());
        let buses = FakeBuses {
            system: FakeSystem::start(LingerPolicy::Allow),
            session_available: true,
        };
        let choice = ModeChoice {
            mode: ServiceMode::System,
            reason: ModeReason::Server,
        };
        let error = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            choice,
            &buses,
            &layout,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Privilege);
        assert_eq!(error.details["required"], "administrator");
        request.account = Some("root".to_owned());
        let report = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            choice,
            &buses,
            &layout,
            true,
        )
        .unwrap();
        assert_eq!(report.backend, Backend::SystemdSystem);
        let text = std::fs::read_to_string(layout.system_unit_dir.join(UNIT_NAME)).unwrap();
        assert!(text.contains("User=root\nGroup=root\n"));
        assert!(text.contains("WantedBy=multi-user.target"));
        assert_eq!(
            primary_group("no-such-account-for-tests").unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            buses.system().unwrap().default_target().unwrap(),
            "multi-user.target"
        );
        let missing = io_error(
            Path::new("/x"),
            &std::io::Error::from(std::io::ErrorKind::NotFound),
        );
        assert_eq!(missing.code, ErrorCode::Io);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::services::windows::scm
// Responsibility:   Rules of the service control manager that do not depend on the Win32 interface.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Rules of the service control manager that do not depend on the Win32
//! interface: the mapping of states, start types and service types to the
//! `service` record, the classification of error codes and the restart
//! sequence.
//!
//! The numeric constants are those of `winsvc.h` and `winerror.h`.

use std::time::Duration;

use nurt_contracts::record::{Service, ServiceManager, ServiceScope, ServiceState, StartType};
use nurt_contracts::value::{Path, Value};

/// `SERVICE_STOPPED`.
pub(crate) const SERVICE_STOPPED: u32 = 1;
/// `SERVICE_START_PENDING`.
pub(crate) const SERVICE_START_PENDING: u32 = 2;
/// `SERVICE_STOP_PENDING`.
pub(crate) const SERVICE_STOP_PENDING: u32 = 3;
/// `SERVICE_RUNNING`.
pub(crate) const SERVICE_RUNNING: u32 = 4;
/// `SERVICE_CONTINUE_PENDING`.
pub(crate) const SERVICE_CONTINUE_PENDING: u32 = 5;
/// `SERVICE_PAUSE_PENDING`.
pub(crate) const SERVICE_PAUSE_PENDING: u32 = 6;
/// `SERVICE_PAUSED`.
pub(crate) const SERVICE_PAUSED: u32 = 7;

/// `SERVICE_BOOT_START`.
pub(crate) const SERVICE_BOOT_START: u32 = 0;
/// `SERVICE_SYSTEM_START`.
pub(crate) const SERVICE_SYSTEM_START: u32 = 1;
/// `SERVICE_AUTO_START`.
pub(crate) const SERVICE_AUTO_START: u32 = 2;
/// `SERVICE_DEMAND_START`.
pub(crate) const SERVICE_DEMAND_START: u32 = 3;
/// `SERVICE_DISABLED`.
pub(crate) const SERVICE_DISABLED: u32 = 4;

/// `SERVICE_USER_SERVICE`: a template or an instance of a per-user service.
pub(crate) const SERVICE_USER_SERVICE: u32 = 0x40;
/// `SERVICE_USERSERVICE_INSTANCE`: an instance of a per-user service.
pub(crate) const SERVICE_USERSERVICE_INSTANCE: u32 = 0x80;

/// `ERROR_ACCESS_DENIED`.
pub(crate) const ERROR_ACCESS_DENIED: u32 = 5;
/// `ERROR_INVALID_NAME`: the manager refuses an empty or too long name.
pub(crate) const ERROR_INVALID_NAME: u32 = 123;
/// `ERROR_SERVICE_SPECIFIC_ERROR`: the real code is `dwServiceSpecificExitCode`.
pub(crate) const ERROR_SERVICE_SPECIFIC_ERROR: u32 = 1066;
/// `ERROR_SERVICE_REQUEST_TIMEOUT`.
pub(crate) const ERROR_SERVICE_REQUEST_TIMEOUT: u32 = 1053;
/// `ERROR_SERVICE_ALREADY_RUNNING`.
pub(crate) const ERROR_SERVICE_ALREADY_RUNNING: u32 = 1056;
/// `ERROR_SERVICE_DOES_NOT_EXIST`.
pub(crate) const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
/// `ERROR_SERVICE_NOT_ACTIVE`.
pub(crate) const ERROR_SERVICE_NOT_ACTIVE: u32 = 1062;

/// Interval between two reads of the state while a service stops.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Longest wait for a service to stop during a restart; the manager itself
/// gives a service 20 seconds by default before it reports a timeout.
pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// Error reported by the manager: the Win32 error code and its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OsError {
    /// Win32 error code.
    pub(crate) code: u32,
    /// System message of the code.
    pub(crate) message: String,
}

/// Class of a Win32 error code for the error table of the service backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The service does not exist.
    NotFound,
    /// The account lacks the administrator privilege.
    Privilege,
    /// Any other error of the manager.
    Manager,
}

/// Class of a Win32 error code.
pub(crate) fn classify(code: u32) -> Failure {
    match code {
        ERROR_SERVICE_DOES_NOT_EXIST | ERROR_INVALID_NAME => Failure::NotFound,
        ERROR_ACCESS_DENIED => Failure::Privilege,
        _ => Failure::Manager,
    }
}

/// Common state of a `dwCurrentState` value.
pub(crate) fn map_state(state: u32) -> ServiceState {
    match state {
        SERVICE_RUNNING => ServiceState::Running,
        SERVICE_STOPPED => ServiceState::Stopped,
        SERVICE_START_PENDING | SERVICE_CONTINUE_PENDING => ServiceState::Starting,
        SERVICE_STOP_PENDING => ServiceState::Stopping,
        SERVICE_PAUSED | SERVICE_PAUSE_PENDING => ServiceState::Paused,
        _ => ServiceState::Unknown,
    }
}

/// Name of a `dwCurrentState` value as defined by the manager, for
/// `state_raw`.
pub(crate) fn state_name(state: u32) -> String {
    match state {
        SERVICE_RUNNING => "running".to_owned(),
        SERVICE_STOPPED => "stopped".to_owned(),
        SERVICE_START_PENDING => "start_pending".to_owned(),
        SERVICE_STOP_PENDING => "stop_pending".to_owned(),
        SERVICE_CONTINUE_PENDING => "continue_pending".to_owned(),
        SERVICE_PAUSE_PENDING => "pause_pending".to_owned(),
        SERVICE_PAUSED => "paused".to_owned(),
        other => other.to_string(),
    }
}

/// Common start type of a `dwStartType` value and the raw name of the
/// variants that the common enumeration does not distinguish.
pub(crate) fn map_start_type(start_type: u32, delayed: bool) -> (StartType, Option<&'static str>) {
    match start_type {
        SERVICE_AUTO_START if delayed => (StartType::Automatic, Some("auto_start_delayed")),
        SERVICE_AUTO_START => (StartType::Automatic, None),
        SERVICE_BOOT_START => (StartType::Automatic, Some("boot_start")),
        SERVICE_SYSTEM_START => (StartType::Automatic, Some("system_start")),
        SERVICE_DEMAND_START => (StartType::Manual, None),
        SERVICE_DISABLED => (StartType::Disabled, None),
        _ => (StartType::Unknown, None),
    }
}

/// Scope of a service by its `dwServiceType`: instances of per-user services
/// belong to `user`, templates of per-user services to no scope, every other
/// service to `system`.
pub(crate) fn scope_of(service_type: u32) -> Option<ServiceScope> {
    if service_type & SERVICE_USERSERVICE_INSTANCE != 0 {
        Some(ServiceScope::User)
    } else if service_type & SERVICE_USER_SERVICE != 0 {
        None
    } else {
        Some(ServiceScope::System)
    }
}

/// Facts read from the manager about one service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ServiceFacts {
    /// Service name.
    pub(crate) name: String,
    /// Display name.
    pub(crate) display_name: Option<String>,
    /// `dwCurrentState`.
    pub(crate) state: u32,
    /// Process id; zero when the service has no process.
    pub(crate) pid: u32,
    /// `dwServiceType`.
    pub(crate) service_type: u32,
    /// `dwStartType`, when the configuration was readable.
    pub(crate) start_type: Option<u32>,
    /// Delayed automatic start.
    pub(crate) delayed: bool,
    /// Command line of the service program.
    pub(crate) path: Option<String>,
    /// Account the service runs as.
    pub(crate) account: Option<String>,
    /// Description.
    pub(crate) description: Option<String>,
    /// `dwWin32ExitCode` of the last stop.
    pub(crate) exit_code: u32,
    /// `dwServiceSpecificExitCode`, meaningful when `exit_code` is
    /// [`ERROR_SERVICE_SPECIFIC_ERROR`].
    pub(crate) specific_exit_code: u32,
}

/// Result of the last stop for `ServiceStatus::result`: the Win32 code, with
/// the service specific code when the Win32 code defers to it; `None` for a
/// clean stop.
pub(crate) fn exit_result(exit_code: u32, specific_exit_code: u32) -> Option<String> {
    match (exit_code, specific_exit_code) {
        (0, _) => None,
        (ERROR_SERVICE_SPECIFIC_ERROR, specific) if specific != 0 => {
            Some(format!("{exit_code} (service exit code {specific})"))
        }
        _ => Some(exit_code.to_string()),
    }
}

/// Command line of a service with the separators of the program path written
/// as `/`; the arguments are kept as they are. The program is the quoted text
/// at the start or the text up to the first white space.
fn slash_program(command_line: &str) -> String {
    let split = match command_line.strip_prefix('"') {
        Some(rest) => rest.find('"').map(|end| end + 2),
        None => command_line.find(char::is_whitespace),
    }
    .unwrap_or(command_line.len());
    let (program, arguments) = command_line.split_at(split);
    format!("{}{arguments}", program.replace('\\', "/"))
}

impl ServiceFacts {
    /// Record of the service.
    pub(crate) fn to_record(&self) -> Service {
        let (start_type, start_raw) = self.start_type.map_or((StartType::Unknown, None), |raw| {
            map_start_type(raw, self.delayed)
        });
        let mut os_raw = vec![(
            "service_type".to_owned(),
            Value::Int(i64::from(self.service_type)),
        )];
        if let Some(raw) = start_raw {
            os_raw.push(("start_type".to_owned(), Value::Text(raw.to_owned())));
        }
        if self.exit_code != 0 {
            os_raw.push((
                "exit_code".to_owned(),
                Value::Int(i64::from(self.exit_code)),
            ));
        }
        if self.exit_code == ERROR_SERVICE_SPECIFIC_ERROR && self.specific_exit_code != 0 {
            os_raw.push((
                "service_exit_code".to_owned(),
                Value::Int(i64::from(self.specific_exit_code)),
            ));
        }
        Service {
            name: self.name.clone(),
            state: map_state(self.state),
            start_type,
            manager: ServiceManager::Windows,
            display_name: self.display_name.clone().filter(|name| !name.is_empty()),
            state_raw: Some(state_name(self.state)),
            pid: (self.pid != 0).then_some(u64::from(self.pid)),
            description: self.description.clone().filter(|text| !text.is_empty()),
            scope: scope_of(self.service_type),
            path: self
                .path
                .as_deref()
                .map(|line| Path::new(slash_program(line))),
            account: self.account.clone().filter(|name| !name.is_empty()),
            os_raw: Some(os_raw.into_iter().collect()),
        }
    }
}

/// Requests of the restart sequence to the manager.
pub(crate) trait ScmOps {
    /// `dwCurrentState` of the service.
    fn state(&self, name: &str) -> Result<u32, OsError>;

    /// Names of the active services that depend on the service.
    fn running_dependents(&self, name: &str) -> Result<Vec<String>, OsError>;

    /// Sends `SERVICE_CONTROL_STOP`.
    fn stop(&self, name: &str) -> Result<(), OsError>;

    /// Calls `StartServiceW`.
    fn start(&self, name: &str) -> Result<(), OsError>;

    /// Waits before the next read of the state.
    fn pause(&self, interval: Duration);
}

/// Failure of the restart sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RestartError {
    /// Active services depend on the service; it was not stopped.
    Dependents(Vec<String>),
    /// The manager reported an error.
    Os(OsError),
    /// The service did not stop within [`STOP_TIMEOUT`].
    StopTimeout,
    /// The service did not leave the start pending state within
    /// [`STOP_TIMEOUT`].
    StartTimeout,
}

impl RestartError {
    /// Stable code for the `os_code` detail of `E_SERVICE`.
    pub(crate) fn os_code(&self) -> String {
        match self {
            Self::Dependents(_) => "dependent_services_running".to_owned(),
            Self::Os(error) => error.code.to_string(),
            Self::StopTimeout | Self::StartTimeout => ERROR_SERVICE_REQUEST_TIMEOUT.to_string(),
        }
    }

    /// Message for the `os_message` detail of `E_SERVICE`.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Dependents(names) => format!(
                "running services depend on it: {}; stop them first",
                names.join(", ")
            ),
            Self::Os(error) => error.message.clone(),
            Self::StopTimeout => format!(
                "the service did not stop within {} seconds",
                STOP_TIMEOUT.as_secs()
            ),
            Self::StartTimeout => format!(
                "the service did not finish starting within {} seconds",
                STOP_TIMEOUT.as_secs()
            ),
        }
    }
}

/// Restarts a service: stops it, waits until it is stopped, and starts it.
///
/// A stopped service is only started. A service with active dependent
/// services is left running, because stopping it would stop them as well.
pub(crate) fn restart(ops: &dyn ScmOps, name: &str) -> Result<(), RestartError> {
    let mut state = ops.state(name).map_err(RestartError::Os)?;
    if matches!(state, SERVICE_START_PENDING | SERVICE_CONTINUE_PENDING) {
        state = wait_settled(ops, name)?;
    }
    if state == SERVICE_STOP_PENDING {
        wait_stopped(ops, name)?;
    } else if state != SERVICE_STOPPED {
        let dependents = ops.running_dependents(name).map_err(RestartError::Os)?;
        if !dependents.is_empty() {
            return Err(RestartError::Dependents(dependents));
        }
        tolerate(ops.stop(name), ERROR_SERVICE_NOT_ACTIVE).map_err(RestartError::Os)?;
        wait_stopped(ops, name)?;
    }
    tolerate(ops.start(name), ERROR_SERVICE_ALREADY_RUNNING).map_err(RestartError::Os)
}

/// Waits until the service leaves the start pending states and returns its
/// state; a stop request in those states is refused by the manager.
fn wait_settled(ops: &dyn ScmOps, name: &str) -> Result<u32, RestartError> {
    let attempts = STOP_TIMEOUT.as_millis() / POLL_INTERVAL.as_millis();
    for _ in 0..attempts {
        let state = ops.state(name).map_err(RestartError::Os)?;
        if !matches!(state, SERVICE_START_PENDING | SERVICE_CONTINUE_PENDING) {
            return Ok(state);
        }
        ops.pause(POLL_INTERVAL);
    }
    Err(RestartError::StartTimeout)
}

fn wait_stopped(ops: &dyn ScmOps, name: &str) -> Result<(), RestartError> {
    let attempts = STOP_TIMEOUT.as_millis() / POLL_INTERVAL.as_millis();
    for _ in 0..attempts {
        if ops.state(name).map_err(RestartError::Os)? == SERVICE_STOPPED {
            return Ok(());
        }
        ops.pause(POLL_INTERVAL);
    }
    Err(RestartError::StopTimeout)
}

/// Treats the error `code` as success: the requested state already holds.
pub(crate) fn tolerate(result: Result<(), OsError>, code: u32) -> Result<(), OsError> {
    match result {
        Err(error) if error.code == code => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    struct MockScm {
        states: RefCell<Vec<u32>>,
        dependents: Vec<String>,
        start_error: Option<u32>,
        stop_error: Option<u32>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl MockScm {
        fn with_states(states: &[u32]) -> Self {
            Self {
                states: RefCell::new(states.iter().rev().copied().collect()),
                ..Self::default()
            }
        }
    }

    impl ScmOps for MockScm {
        fn state(&self, _name: &str) -> Result<u32, OsError> {
            let mut states = self.states.borrow_mut();
            Ok(if states.len() > 1 {
                states.pop().unwrap()
            } else {
                *states.last().unwrap()
            })
        }

        fn running_dependents(&self, _name: &str) -> Result<Vec<String>, OsError> {
            self.calls.borrow_mut().push("dependents");
            Ok(self.dependents.clone())
        }

        fn stop(&self, _name: &str) -> Result<(), OsError> {
            self.calls.borrow_mut().push("stop");
            self.stop_error.map_or(Ok(()), |code| {
                Err(OsError {
                    code,
                    message: String::new(),
                })
            })
        }

        fn start(&self, _name: &str) -> Result<(), OsError> {
            self.calls.borrow_mut().push("start");
            self.start_error.map_or(Ok(()), |code| {
                Err(OsError {
                    code,
                    message: String::new(),
                })
            })
        }

        fn pause(&self, _interval: Duration) {
            self.calls.borrow_mut().push("pause");
        }
    }

    #[test]
    fn scm_state_and_start_type_mapping_tables() {
        let states = [
            (SERVICE_RUNNING, ServiceState::Running, "running"),
            (SERVICE_STOPPED, ServiceState::Stopped, "stopped"),
            (
                SERVICE_START_PENDING,
                ServiceState::Starting,
                "start_pending",
            ),
            (
                SERVICE_CONTINUE_PENDING,
                ServiceState::Starting,
                "continue_pending",
            ),
            (SERVICE_STOP_PENDING, ServiceState::Stopping, "stop_pending"),
            (SERVICE_PAUSED, ServiceState::Paused, "paused"),
            (SERVICE_PAUSE_PENDING, ServiceState::Paused, "pause_pending"),
            (99, ServiceState::Unknown, "99"),
        ];
        for (raw, state, name) in states {
            assert_eq!(map_state(raw), state);
            assert_eq!(state_name(raw), name);
        }
        let start_types = [
            (SERVICE_AUTO_START, false, StartType::Automatic, None),
            (
                SERVICE_AUTO_START,
                true,
                StartType::Automatic,
                Some("auto_start_delayed"),
            ),
            (
                SERVICE_BOOT_START,
                false,
                StartType::Automatic,
                Some("boot_start"),
            ),
            (
                SERVICE_SYSTEM_START,
                false,
                StartType::Automatic,
                Some("system_start"),
            ),
            (SERVICE_DEMAND_START, false, StartType::Manual, None),
            (SERVICE_DISABLED, false, StartType::Disabled, None),
            (9, false, StartType::Unknown, None),
        ];
        for (raw, delayed, start_type, os_raw) in start_types {
            assert_eq!(map_start_type(raw, delayed), (start_type, os_raw));
        }
        assert_eq!(scope_of(0x10), Some(ServiceScope::System));
        assert_eq!(scope_of(0x50), None);
        assert_eq!(scope_of(0xd0), Some(ServiceScope::User));
        assert_eq!(classify(ERROR_SERVICE_DOES_NOT_EXIST), Failure::NotFound);
        assert_eq!(classify(ERROR_ACCESS_DENIED), Failure::Privilege);
        assert_eq!(classify(ERROR_INVALID_NAME), Failure::NotFound);
        assert_eq!(classify(ERROR_SERVICE_REQUEST_TIMEOUT), Failure::Manager);
    }

    #[test]
    fn scm_restart_waits_for_a_service_that_is_starting_before_it_stops_it() {
        let scm = MockScm::with_states(&[SERVICE_START_PENDING, SERVICE_RUNNING, SERVICE_STOPPED]);
        restart(&scm, "Spooler").unwrap();
        assert_eq!(*scm.calls.borrow(), ["dependents", "stop", "start"]);
        let stuck = MockScm::with_states(&[SERVICE_START_PENDING]);
        assert_eq!(
            restart(&stuck, "Spooler").unwrap_err(),
            RestartError::StartTimeout
        );
        assert!(!stuck.calls.borrow().contains(&"stop"));
    }

    #[test]
    fn scm_restart_waits_for_a_service_that_is_stopping_without_a_second_stop() {
        let scm =
            MockScm::with_states(&[SERVICE_STOP_PENDING, SERVICE_STOP_PENDING, SERVICE_STOPPED]);
        restart(&scm, "Spooler").unwrap();
        assert_eq!(*scm.calls.borrow(), ["pause", "start"]);
    }

    #[test]
    fn scm_empty_or_invalid_names_are_not_found_and_specific_exit_codes_are_reported() {
        assert_eq!(classify(ERROR_INVALID_NAME), Failure::NotFound);
        assert_eq!(exit_result(0, 5), None);
        assert_eq!(exit_result(1067, 0).as_deref(), Some("1067"));
        assert_eq!(exit_result(1066, 0).as_deref(), Some("1066"));
        assert_eq!(
            exit_result(1066, 42).as_deref(),
            Some("1066 (service exit code 42)")
        );
        let record = ServiceFacts {
            name: "Custom".to_owned(),
            exit_code: ERROR_SERVICE_SPECIFIC_ERROR,
            specific_exit_code: 42,
            ..ServiceFacts::default()
        }
        .to_record();
        assert_eq!(record.os_raw.unwrap()["service_exit_code"], Value::Int(42));
    }

    #[test]
    fn only_the_program_of_a_command_line_gets_slashes() {
        assert_eq!(
            slash_program("\"C:\\Program Files\\App\\svc.exe\" -config C:\\conf\\a.ini"),
            "\"C:/Program Files/App/svc.exe\" -config C:\\conf\\a.ini"
        );
        assert_eq!(
            slash_program("C:\\Windows\\svc.exe /path D:\\x"),
            "C:/Windows/svc.exe /path D:\\x"
        );
        assert_eq!(slash_program("C:\\Windows\\svc.exe"), "C:/Windows/svc.exe");
        assert_eq!(slash_program("\"C:\\unterminated"), "\"C:/unterminated");
    }

    #[test]
    fn scm_restart_with_running_dependents_service_error() {
        let scm = MockScm {
            dependents: vec!["Fax".to_owned(), "PrintWorkflow".to_owned()],
            ..MockScm::with_states(&[SERVICE_RUNNING])
        };
        let error = restart(&scm, "Spooler").unwrap_err();
        assert_eq!(
            error,
            RestartError::Dependents(vec!["Fax".to_owned(), "PrintWorkflow".to_owned()])
        );
        assert_eq!(error.os_code(), "dependent_services_running");
        assert!(error.message().contains("Fax, PrintWorkflow"));
        assert_eq!(*scm.calls.borrow(), ["dependents"]);
    }

    #[test]
    fn restart_stops_waits_and_starts() {
        let scm = MockScm::with_states(&[SERVICE_RUNNING, SERVICE_STOP_PENDING, SERVICE_STOPPED]);
        restart(&scm, "Spooler").unwrap();
        assert_eq!(
            *scm.calls.borrow(),
            ["dependents", "stop", "pause", "start"]
        );
        let stopped = MockScm::with_states(&[SERVICE_STOPPED]);
        restart(&stopped, "Spooler").unwrap();
        assert_eq!(*stopped.calls.borrow(), ["start"]);
        let racing = MockScm {
            start_error: Some(ERROR_SERVICE_ALREADY_RUNNING),
            ..MockScm::with_states(&[SERVICE_STOPPED])
        };
        restart(&racing, "Spooler").unwrap();
    }

    #[test]
    fn restart_reports_timeout_and_start_errors() {
        let stuck = MockScm::with_states(&[SERVICE_STOP_PENDING]);
        let error = restart(&stuck, "Spooler").unwrap_err();
        assert_eq!(error, RestartError::StopTimeout);
        assert_eq!(error.os_code(), "1053");
        assert!(error.message().contains("30 seconds"));
        let disabled = MockScm {
            start_error: Some(1058),
            ..MockScm::with_states(&[SERVICE_STOPPED])
        };
        let error = restart(&disabled, "Spooler").unwrap_err();
        assert_eq!(error.os_code(), "1058");
        assert_eq!(error.message(), "");
    }

    #[test]
    fn restart_stop_of_a_service_that_is_not_active_continues_and_other_errors_end_it() {
        let finished = MockScm {
            stop_error: Some(ERROR_SERVICE_NOT_ACTIVE),
            ..MockScm::with_states(&[SERVICE_RUNNING, SERVICE_STOPPED])
        };
        restart(&finished, "Spooler").unwrap();
        assert_eq!(*finished.calls.borrow(), ["dependents", "stop", "start"]);
        let refused = MockScm {
            stop_error: Some(ERROR_ACCESS_DENIED),
            ..MockScm::with_states(&[SERVICE_RUNNING])
        };
        let error = restart(&refused, "Spooler").unwrap_err();
        assert_eq!(
            error,
            RestartError::Os(OsError {
                code: ERROR_ACCESS_DENIED,
                message: String::new(),
            })
        );
        assert_eq!(error.os_code(), "5");
        assert_eq!(*refused.calls.borrow(), ["dependents", "stop"]);
    }

    #[test]
    fn restart_gives_up_when_the_service_does_not_stop_within_the_timeout() {
        let stuck = MockScm::with_states(&[SERVICE_RUNNING, SERVICE_STOP_PENDING]);
        assert_eq!(
            restart(&stuck, "Spooler").unwrap_err(),
            RestartError::StopTimeout
        );
        let calls = stuck.calls.borrow();
        let pauses = calls.iter().filter(|call| **call == "pause").count();
        assert_eq!(
            u128::try_from(pauses).unwrap(),
            STOP_TIMEOUT.as_millis() / POLL_INTERVAL.as_millis()
        );
        assert!(!calls.contains(&"start"));
    }

    #[test]
    fn tolerate_accepts_only_the_given_code() {
        let error = |code| OsError {
            code,
            message: "text".to_owned(),
        };
        assert_eq!(tolerate(Ok(()), ERROR_SERVICE_NOT_ACTIVE), Ok(()));
        assert_eq!(
            tolerate(
                Err(error(ERROR_SERVICE_NOT_ACTIVE)),
                ERROR_SERVICE_NOT_ACTIVE
            ),
            Ok(())
        );
        assert_eq!(
            tolerate(
                Err(error(ERROR_SERVICE_ALREADY_RUNNING)),
                ERROR_SERVICE_NOT_ACTIVE
            ),
            Err(error(ERROR_SERVICE_ALREADY_RUNNING))
        );
    }

    #[test]
    fn restart_error_messages_name_the_cause() {
        let os = RestartError::Os(OsError {
            code: 1058,
            message: "disabled".to_owned(),
        });
        assert_eq!(os.os_code(), "1058");
        assert_eq!(os.message(), "disabled");
        let dependents = RestartError::Dependents(vec!["Fax".to_owned()]);
        assert_eq!(
            dependents.message(),
            "running services depend on it: Fax; stop them first"
        );
    }

    #[test]
    fn per_user_service_templates_have_no_scope_and_instances_have_the_user_scope() {
        let facts = |service_type| ServiceFacts {
            name: "CDPUserSvc".to_owned(),
            service_type,
            ..ServiceFacts::default()
        };
        assert_eq!(facts(0x60).to_record().scope, None);
        assert_eq!(facts(0xe0).to_record().scope, Some(ServiceScope::User));
        let record = ServiceFacts {
            path: Some("\"C:\\Program Files\\App\\svc.exe\" -k group".to_owned()),
            ..facts(0x10)
        }
        .to_record();
        assert_eq!(
            record.path.unwrap().as_str(),
            "\"C:/Program Files/App/svc.exe\" -k group"
        );
        assert_eq!(record.os_raw.unwrap()["service_type"], Value::Int(0x10));
    }

    #[test]
    fn facts_map_to_service_record() {
        let facts = ServiceFacts {
            name: "Spooler".to_owned(),
            display_name: Some("Print Spooler".to_owned()),
            state: SERVICE_RUNNING,
            pid: 1200,
            service_type: 0x110,
            start_type: Some(SERVICE_AUTO_START),
            delayed: true,
            path: Some("C:\\Windows\\System32\\spoolsv.exe".to_owned()),
            account: Some("LocalSystem".to_owned()),
            description: Some(String::new()),
            exit_code: 1067,
            specific_exit_code: 0,
        };
        let record = facts.to_record();
        assert_eq!(record.state, ServiceState::Running);
        assert_eq!(record.start_type, StartType::Automatic);
        assert_eq!(record.pid, Some(1200));
        assert_eq!(record.scope, Some(ServiceScope::System));
        assert_eq!(record.description, None);
        assert_eq!(
            record.path.unwrap().as_str(),
            "C:/Windows/System32/spoolsv.exe"
        );
        let os_raw = record.os_raw.unwrap();
        assert_eq!(
            os_raw["start_type"],
            Value::Text("auto_start_delayed".into())
        );
        assert_eq!(os_raw["exit_code"], Value::Int(1067));
        let unread = ServiceFacts {
            name: "Locked".to_owned(),
            ..ServiceFacts::default()
        }
        .to_record();
        assert_eq!(unread.start_type, StartType::Unknown);
        assert_eq!(unread.pid, None);
        assert_eq!(unread.state, ServiceState::Unknown);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

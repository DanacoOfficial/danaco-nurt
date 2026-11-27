// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Privilege elevation: the choice of backends, the errors of an elevation
//! that did not happen and the references to elevated journal entries.
//!
//! The engine routes a request whose call parameter `privilege` is `elevated`
//! or `auto` through a backend of [`nurt_proc::elevation`]. The request
//! travels as a syntax tree in the request format of the protocol, never as
//! shell text; the elevated process runs the same operation code, writes the
//! journal entry to the system journal and returns the response. The engine of
//! the account keeps a reference to every elevated journal entry, so that
//! `undo` of the account reaches it through the same backends.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use nurt_contracts::config::PrivilegeConfig;
use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::Report;
use nurt_proc::elevation::{Backend, Refusal, System, default_backends, parse_backends};
pub use nurt_proc::elevation::{
    ELEVATED_EXEC, EXIT_BAD_REQUEST, EXIT_NOT_ADMINISTRATOR, is_administrator, reset_environment,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// File of the references to elevated journal entries in the state directory
/// of the account.
pub const REFERENCE_FILE: &str = "elevated-operations.jsonl";

/// Number added to the number of an entry of the system journal to give the
/// number under which the account sees it; the numbers of the account stay
/// below it.
pub const ELEVATED_OP_BASE: u64 = 1_000_000_000;

/// Reason recorded when `auto` repeated a stage with elevated rights.
pub const REASON_AUTO: &str = "permission_denied";

/// Reason recorded when `undo` reached an entry of the system journal.
pub const REASON_REFERENCE: &str = "system_journal_entry";

/// Reason of `E_PRIVILEGE` when the configuration forbids elevation.
pub const DISABLED_BY_CONFIGURATION: &str = "disabled_by_configuration";

/// Facts about the process that the engine uses for elevation; tests replace
/// them.
#[derive(Debug, Clone, Default)]
pub(crate) struct Setup {
    /// The executable started by the backends; the running executable when
    /// absent.
    pub(crate) executable: Option<PathBuf>,
    /// Search path of the programs of the backends; the path of the process
    /// when absent.
    pub(crate) search_path: Option<OsString>,
    /// Replaces the check of the identity of the administrator.
    pub(crate) administrator: Option<bool>,
}

impl Setup {
    /// Whether the process has the identity of the administrator.
    pub(crate) fn is_administrator(&self) -> bool {
        self.administrator
            .unwrap_or_else(nurt_proc::elevation::is_administrator)
    }

    /// The executable that the backends start.
    pub(crate) fn executable(&self) -> Result<PathBuf, Error> {
        match &self.executable {
            Some(path) => Ok(path.clone()),
            None => std::env::current_exe().map_err(|error| {
                Error::new(
                    ErrorCode::Privilege,
                    format!("the path of the running executable is unknown: {error}"),
                )
            }),
        }
    }
}

/// How an elevated operation ran; stamped on its report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mark {
    pub(crate) backend: Backend,
    pub(crate) as_user: Option<String>,
    pub(crate) reason: Option<&'static str>,
}

impl Mark {
    /// Writes the elevation fields of `report`.
    pub(crate) fn apply(&self, report: &mut Report) {
        report.elevated = true;
        report.backend = Some(self.backend.as_str().to_owned());
        report.as_user.clone_from(&self.as_user);
        report.elevation_reason = self.reason.map(str::to_owned);
    }
}

/// The system of the host.
pub(crate) fn system_of(os: Os) -> System {
    match os {
        Os::Linux => System::Linux,
        Os::Macos => System::MacOs,
        Os::Windows => System::Windows,
        _ => System::Linux,
    }
}

/// Directory of the system journal: the state directory of the
/// administrator.
///
/// `/var/lib/nurt` on Linux, `/Library/Application Support/Nurt` on macOS and
/// `%ProgramData%\Nurt` on Windows. `program_data` is the value of the
/// variable `ProgramData`.
#[must_use]
pub fn system_state_dir(os: Os, program_data: Option<&str>) -> PathBuf {
    match os {
        Os::Linux => PathBuf::from("/var/lib/nurt"),
        Os::Macos => PathBuf::from("/Library/Application Support/Nurt"),
        Os::Windows => {
            let base = program_data.unwrap_or("C:\\ProgramData");
            Path::new(base).join("Nurt")
        }
        _ => PathBuf::from("/var/lib/nurt"),
    }
}

/// Backends allowed by the configuration, in order of trial.
///
/// # Errors
///
/// Returns `E_ARG` when `privilege.backends` names an unknown backend.
pub(crate) fn backends_of(config: &PrivilegeConfig, system: System) -> Result<Vec<Backend>, Error> {
    match &config.backends {
        None => Ok(default_backends(system)),
        Some(names) => parse_backends(names).map_err(|name| {
            Error::new(
                ErrorCode::Arg,
                format!(
                    "`privilege.backends` names `{name}`; expected self, broker, sudo, doas or run0"
                ),
            )
        }),
    }
}

/// `E_PRIVILEGE` for a configuration that forbids elevation.
pub(crate) fn disabled_error() -> Error {
    Error::new(
        ErrorCode::Privilege,
        "elevation is disabled by the configuration (`privilege.allowed` is false)",
    )
    .with_detail("reason", json!(DISABLED_BY_CONFIGURATION))
    .with_next_step("run the operation without `privilege`, or ask the administrator of the installation to set `privilege.allowed` to true")
}

/// Adds to an `E_PRIVILEGE` that a stage returned the way forward: the call
/// parameter `privilege: elevated` and, when the error has none, the backends
/// that this installation can try with the state of each.
pub(crate) fn guide_privilege_error(
    error: Error,
    backends: &[Backend],
    setup: &Setup,
    allowed: bool,
) -> Error {
    if error.code() != ErrorCode::Privilege
        || error.details.contains_key("backend")
        || error.details.contains_key("reason")
    {
        return error;
    }
    let mut error = error;
    if !error.details.contains_key("backend") {
        let listed: Vec<serde_json::Value> = backends
            .iter()
            .map(|backend| {
                let result = match backend {
                    Backend::SelfProcess if setup.is_administrator() => "available",
                    Backend::SelfProcess => "not_administrator",
                    Backend::Broker => "not_installed",
                    other => {
                        if nurt_proc::elevation::locate(*other, setup.search_path.as_deref())
                            .is_some()
                        {
                            "available"
                        } else {
                            "not_installed"
                        }
                    }
                };
                json!({"backend": backend.as_str(), "result": result})
            })
            .collect();
        error = error.with_detail("backend", json!(listed));
    }
    if !error
        .next_steps
        .iter()
        .any(|step| step.contains("privilege: elevated"))
    {
        error = error.with_next_step(if allowed {
            "repeat the call with `privilege: elevated` (or `privilege: auto`)"
        } else {
            "elevation is disabled by `privilege.allowed`; run nurt as an account with the rights"
        });
    }
    error
}

/// `E_UNSUPPORTED` for `as_user` on Windows.
pub(crate) fn as_user_unsupported() -> Error {
    Error::new(
        ErrorCode::Unsupported,
        "`as_user` is not supported on Windows",
    )
    .with_next_step("use `privilege: elevated` to run the operation as the administrator")
}

/// `E_PRIVILEGE` listing the backends that refused and what to do.
pub(crate) fn refused_error(attempts: &[(Backend, Refusal)], as_user: Option<&str>) -> Error {
    let listed: Vec<serde_json::Value> = attempts
        .iter()
        .map(|(backend, refusal)| {
            let mut entry = json!({"backend": backend.as_str(), "result": refusal.code()});
            if let Refusal::Failed(detail) = refusal {
                entry["detail"] = json!(detail);
            }
            entry
        })
        .collect();
    let message = if attempts.is_empty() {
        "no elevation backend is allowed by `privilege.backends`".to_owned()
    } else {
        let names: Vec<String> = attempts
            .iter()
            .map(|(backend, refusal)| format!("{backend}: {}", refusal.message()))
            .collect();
        format!(
            "no elevation backend ran the operation ({})",
            names.join("; ")
        )
    };
    let mut error = Error::new(ErrorCode::Privilege, message).with_detail("backend", json!(listed));
    let target = as_user.map_or_else(
        || "the administrator".to_owned(),
        |user| format!("the account `{user}`"),
    );
    for (backend, refusal) in attempts {
        let step = match (backend, refusal) {
            (Backend::Broker, _) => continue,
            (Backend::SelfProcess, _) => continue,
            (backend, Refusal::NotInstalled) => {
                format!("install `{backend}` or remove it from `privilege.backends`")
            }
            (Backend::Sudo, Refusal::PasswordRequired) => {
                format!(
                    "allow the account to run nurt as {target} without a password in sudoers (`NOPASSWD`), or refresh the sudo timestamp with `sudo -v` in a terminal"
                )
            }
            (backend, Refusal::PasswordRequired) => {
                format!(
                    "configure `{backend}` so that the account may run nurt as {target} without a password (`nopass`)"
                )
            }
            (backend, Refusal::Denied) => {
                format!(
                    "the account is not allowed by `{backend}` to run nurt as {target}; ask the administrator to grant it"
                )
            }
            (_, Refusal::CannotSwitchUser) => {
                format!(
                    "the account `{}` does not exist",
                    as_user.unwrap_or_default()
                )
            }
            (backend, Refusal::Failed(detail)) => format!("`{backend}` failed: {detail}"),
            (_, Refusal::NotAdministrator) => continue,
        };
        error = error.with_next_step(step);
    }
    error.with_next_step("run nurt as the administrator, or use an account that may elevate")
}

/// Reference to an entry of the system journal, kept by the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElevatedReference {
    /// Number of the entry; the same in the system journal.
    pub op: u64,
    /// Backend that ran the operation.
    pub backend: String,
    /// Account the operation ran as, when it was not the administrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_user: Option<String>,
    /// Kind of the operation.
    pub kind: String,
    /// Time of the operation, in milliseconds since the Unix epoch.
    pub time_ms: i64,
    /// State directory that holds the system journal.
    pub journal: String,
    /// The entry was undone through the backend; the newest line of an entry
    /// decides.
    #[serde(default)]
    pub undone: bool,
}

/// File of the references of one account.
#[derive(Debug, Clone)]
pub(crate) struct ReferenceLog {
    path: PathBuf,
}

impl ReferenceLog {
    /// The log in the state directory `root`.
    pub(crate) fn in_state(root: &Path) -> Self {
        Self {
            path: root.join(REFERENCE_FILE),
        }
    }

    /// Appends one reference.
    pub(crate) fn append(&self, reference: &ElevatedReference) -> std::io::Result<()> {
        self.append_all(std::slice::from_ref(reference))
    }

    /// Appends the references with one write and makes them durable. When
    /// the file does not end with a line break (a torn last line of an
    /// earlier failed write), the new text starts on a new line, so the
    /// damaged line never swallows a good one.
    pub(crate) fn append_all(&self, references: &[ElevatedReference]) -> std::io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)?;
        let mut text = String::new();
        let length = file.metadata()?.len();
        if length > 0 {
            file.seek(SeekFrom::Start(length - 1))?;
            let mut last = [0_u8; 1];
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                text.push('\n');
            }
        }
        for reference in references {
            text.push_str(&serde_json::to_string(reference)?);
            text.push('\n');
        }
        file.write_all(text.as_bytes())?;
        file.sync_data()
    }

    /// Every reference, oldest first; a missing file is an empty log and a
    /// damaged line is skipped.
    pub(crate) fn all(&self) -> Vec<ElevatedReference> {
        let Ok(file) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str(&line).ok())
            .collect()
    }

    /// The newest line of every entry, oldest entry first.
    pub(crate) fn latest(&self) -> Vec<ElevatedReference> {
        let mut order: Vec<u64> = Vec::new();
        let mut newest: std::collections::HashMap<u64, ElevatedReference> =
            std::collections::HashMap::new();
        for reference in self.all() {
            if newest.insert(reference.op, reference.clone()).is_none() {
                order.push(reference.op);
            }
        }
        order
            .into_iter()
            .filter_map(|op| newest.remove(&op))
            .collect()
    }

    /// The newest reference to entry `op`.
    pub(crate) fn find(&self, op: u64) -> Option<ElevatedReference> {
        self.all()
            .into_iter()
            .rev()
            .find(|reference| reference.op == op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_journal_directories() {
        assert_eq!(
            system_state_dir(Os::Linux, None),
            PathBuf::from("/var/lib/nurt")
        );
        assert_eq!(
            system_state_dir(Os::Macos, None),
            PathBuf::from("/Library/Application Support/Nurt")
        );
        assert_eq!(
            system_state_dir(Os::Windows, Some("D:\\Data")),
            Path::new("D:\\Data").join("Nurt")
        );
    }

    #[test]
    fn configured_backends_replace_the_default_order() {
        let mut config = PrivilegeConfig::default();
        assert_eq!(
            backends_of(&config, System::Windows).unwrap(),
            vec![Backend::SelfProcess, Backend::Broker]
        );
        config.backends = Some(vec!["doas".to_owned(), "self".to_owned()]);
        assert_eq!(
            backends_of(&config, System::Linux).unwrap(),
            vec![Backend::Doas, Backend::SelfProcess]
        );
        config.backends = Some(vec!["su".to_owned()]);
        assert_eq!(
            backends_of(&config, System::Linux).unwrap_err().code(),
            ErrorCode::Arg
        );
    }

    #[test]
    fn refusal_error_lists_every_backend_and_the_way_forward() {
        let error = refused_error(
            &[
                (Backend::SelfProcess, Refusal::NotAdministrator),
                (Backend::Broker, Refusal::NotInstalled),
                (Backend::Sudo, Refusal::PasswordRequired),
            ],
            None,
        );
        assert_eq!(error.code(), ErrorCode::Privilege);
        let detail = error.details.get("backend").cloned().unwrap();
        assert_eq!(
            detail,
            json!([
                {"backend": "self", "result": "not_administrator"},
                {"backend": "broker", "result": "not_installed"},
                {"backend": "sudo", "result": "password_required"},
            ])
        );
        assert!(
            error
                .next_steps
                .iter()
                .any(|step| step.contains("NOPASSWD"))
        );
    }

    #[test]
    fn disabled_error_names_the_reason() {
        let error = disabled_error();
        assert_eq!(error.code(), ErrorCode::Privilege);
        assert_eq!(
            error.details.get("reason"),
            Some(&json!(DISABLED_BY_CONFIGURATION))
        );
        assert!(!error.next_steps.is_empty());
    }

    #[test]
    fn references_are_appended_and_found_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let log = ReferenceLog::in_state(dir.path());
        assert!(log.all().is_empty());
        let reference = |op, backend: &str| ElevatedReference {
            op,
            backend: backend.to_owned(),
            as_user: None,
            kind: "write".to_owned(),
            time_ms: 1,
            journal: "/var/lib/nurt".to_owned(),
            undone: false,
        };
        log.append(&reference(3, "sudo")).unwrap();
        log.append(&reference(4, "doas")).unwrap();
        log.append(&reference(3, "run0")).unwrap();
        assert_eq!(log.all().len(), 3);
        assert_eq!(log.find(3).unwrap().backend, "run0");
        assert_eq!(log.find(4).unwrap().backend, "doas");
        assert!(log.find(9).is_none());
    }

    fn sample(op: u64) -> ElevatedReference {
        ElevatedReference {
            op,
            backend: "sudo".to_owned(),
            as_user: None,
            kind: "write".to_owned(),
            time_ms: 1,
            journal: "/var/lib/nurt".to_owned(),
            undone: false,
        }
    }

    #[test]
    fn a_torn_last_line_does_not_swallow_the_next_reference() {
        let dir = tempfile::tempdir().unwrap();
        let log = ReferenceLog::in_state(dir.path());
        log.append(&sample(1)).unwrap();
        let path = dir.path().join(REFERENCE_FILE);
        let mut torn = std::fs::read_to_string(&path).unwrap();
        torn.push_str("{\"op\":2,\"backe");
        std::fs::write(&path, torn).unwrap();
        log.append_all(&[sample(3), sample(4)]).unwrap();
        let ops: Vec<u64> = log.all().iter().map(|reference| reference.op).collect();
        assert_eq!(ops, [1, 3, 4]);
    }

    #[test]
    fn a_failed_write_is_reported_to_the_caller() {
        let dir = tempfile::tempdir().unwrap();
        let log = ReferenceLog::in_state(&dir.path().join("missing"));
        assert!(log.append(&sample(1)).is_err());
    }

    #[test]
    fn mark_stamps_the_report() {
        let mark = Mark {
            backend: Backend::Sudo,
            as_user: Some("deploy".to_owned()),
            reason: Some(REASON_AUTO),
        };
        let mut report = Report::new(nurt_contracts::report::ReportStatus::Ok, "write");
        mark.apply(&mut report);
        assert!(report.elevated);
        assert_eq!(report.backend.as_deref(), Some("sudo"));
        assert_eq!(report.as_user.as_deref(), Some("deploy"));
        assert_eq!(
            report.elevation_reason.as_deref(),
            Some("permission_denied")
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Privilege elevation: the choice of backends and the errors of an elevation
//! that did not happen.
//!
//! The engine routes a request whose call parameter `privilege` is `elevated`
//! or `auto` through a backend of [`nurt_proc::elevation`]. The request
//! travels as a syntax tree in the request format of the protocol, never as
//! shell text; the elevated process runs the same operation code and writes
//! the entry, with the copies of the previous content, into the journal of the
//! account that asked (one journal, one numbering; the files belong to the
//! account). `undo` and `redo` of such an entry restore the targets with the
//! same backends.

use std::ffi::OsString;
use std::path::PathBuf;

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

/// Reason recorded when `auto` repeated a stage with elevated rights.
pub const REASON_AUTO: &str = "permission_denied";

/// Reason recorded when `undo` or `redo` of an entry that an elevated
/// operation wrote was sent to a backend.
pub const REASON_ENTRY: &str = "elevated_entry";

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

/// Journal of an account that a process writes with the rights of the
/// administrator: the user, the group and the state directory of the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountJournal {
    /// Numeric user of the account.
    pub uid: u32,
    /// Numeric group of the account.
    pub gid: u32,
    /// State directory of the account.
    pub state_dir: std::path::PathBuf,
}

/// Account that an elevated process writes the journal for, and the backend
/// that started the process.
///
/// The elevated process writes the entries and the copies of the previous
/// content into the journal of this account, with the user and the group of the
/// account (`nurt_fs::account`); the targets it changes with the rights of the
/// administrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElevatedOwner {
    /// Numeric user of the account.
    pub uid: u32,
    /// Numeric group of the account.
    pub gid: u32,
    /// Absolute state directory of the account.
    pub state_dir: String,
    /// Backend that started the process.
    pub backend: String,
}

/// What the standard input of `elevated-exec` carries: the request and, when
/// the process is to write the journal of the account that sent it, the owner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElevatedEnvelope {
    /// Version of the envelope, `nurt-elevated/1`.
    pub nurt_elevated: String,
    /// Account whose journal is written; absent for a process that keeps the
    /// journal of its own account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<ElevatedOwner>,
    /// The request in the request format of the protocol.
    pub request: nurt_contracts::envelope::Request,
}

/// Checks the owner that a request names before a process that runs as the
/// administrator writes a journal for it.
///
/// The state directory must be an absolute path to a directory (not a link)
/// that the named user owns, and the user must be the one that the backend
/// recorded (`SUDO_UID`) when it recorded one. The account that sends the
/// request can already obtain the rights of the administrator through the
/// backend; the check keeps a request from naming another account, and every
/// write below the directory is made with the rights of that account, never
/// with more.
///
/// # Errors
///
/// Returns the reason in words.
pub fn verify_owner(owner: &ElevatedOwner, backend_uid: Option<u32>) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err(
            "the journal of the account cannot be written by an elevated process on this system"
                .to_owned(),
        );
    }
    if backend_uid.is_some_and(|uid| uid != owner.uid) {
        return Err("the owner is not the account that started the backend".to_owned());
    }
    let path = std::path::Path::new(&owner.state_dir);
    if !path.is_absolute() {
        return Err("the state directory of the owner is not an absolute path".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::symlink_metadata(path)
            .map_err(|error| format!("the state directory of the owner: {error}"))?;
        if !meta.is_dir() {
            return Err("the state directory of the owner is not a directory".to_owned());
        }
        if meta.uid() != owner.uid {
            return Err("the state directory is not owned by the owner".to_owned());
        }
    }
    Ok(())
}

/// Version string of [`ElevatedEnvelope`].
pub const ENVELOPE_VERSION: &str = "nurt-elevated/1";

/// What `elevated-exec` read: an envelope, or a bare request.
///
/// # Errors
///
/// Returns the message of the parser for text that is neither.
pub fn parse_envelope(bytes: &[u8]) -> Result<ElevatedEnvelope, String> {
    if let Ok(envelope) = serde_json::from_slice::<ElevatedEnvelope>(bytes) {
        if envelope.nurt_elevated != ENVELOPE_VERSION {
            return Err(format!(
                "unsupported envelope version `{}`",
                envelope.nurt_elevated
            ));
        }
        return Ok(envelope);
    }
    serde_json::from_slice::<nurt_contracts::envelope::Request>(bytes)
        .map(|request| ElevatedEnvelope {
            nurt_elevated: ENVELOPE_VERSION.to_owned(),
            owner: None,
            request,
        })
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[cfg(target_os = "linux")]
    #[test]
    fn an_owner_must_own_its_state_directory_and_match_the_backend() {
        let dir = tempfile::tempdir().unwrap();
        let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(dir.path()).unwrap());
        let owner = ElevatedOwner {
            uid,
            gid: uid,
            state_dir: dir.path().to_string_lossy().into_owned(),
            backend: "sudo".to_owned(),
        };
        assert!(verify_owner(&owner, None).is_ok());
        assert!(verify_owner(&owner, Some(uid)).is_ok());
        assert!(verify_owner(&owner, Some(uid + 1)).is_err());
        let other = ElevatedOwner {
            uid: uid + 1,
            ..owner.clone()
        };
        assert!(verify_owner(&other, None).is_err());
        let relative = ElevatedOwner {
            state_dir: "state".to_owned(),
            ..owner.clone()
        };
        assert!(verify_owner(&relative, None).is_err());
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path(), &link).unwrap();
        let linked = ElevatedOwner {
            state_dir: link.to_string_lossy().into_owned(),
            ..owner
        };
        assert!(verify_owner(&linked, None).is_err(), "a link is refused");
    }

    #[test]
    fn a_bare_request_and_an_envelope_are_both_read() {
        let request = r#"{"nurt":"1.0","program":"find \"/\""}"#;
        let bare = parse_envelope(request.as_bytes()).unwrap();
        assert!(bare.owner.is_none());
        let wrapped = format!(
            r#"{{"nurt_elevated":"nurt-elevated/1","owner":{{"uid":1000,"gid":1000,"state_dir":"/home/a/.local/state/nurt","backend":"sudo"}},"request":{request}}}"#
        );
        let envelope = parse_envelope(wrapped.as_bytes()).unwrap();
        assert_eq!(envelope.owner.unwrap().uid, 1000);
        assert_eq!(envelope.request, bare.request);
        assert!(parse_envelope(b"nonsense").is_err());
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

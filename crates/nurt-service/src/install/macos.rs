// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! launchd jobs on macOS.
//!
//! The property list is written by the crate; loading and unloading the job
//! uses the system program `launchctl` (`bootstrap` and `bootout`), the only
//! external program the service runs, through the program execution of the
//! console with a time limit of ten seconds.

use std::path::{Path, PathBuf};

use nurt_contracts::config::ServiceMode;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::services::{ProcessHost, ProgramSpec};
use nurt_contracts::stage::CancelToken;
use plist::{Dictionary, Value};
use serde_json::json;

use super::detect::ModeChoice;
use super::{
    Backend, InstallReport, InstallRequest, Manifest, UninstallReport, files_named,
    privilege_error, remove_files, write_file,
};

/// Label of the launchd job.
pub(crate) const LABEL: &str = "nurt.service";

const LAUNCHCTL: &str = "launchctl";
const LAUNCHCTL_TIMEOUT_MS: u64 = 10_000;
const DAEMON_DIR: &str = "/Library/LaunchDaemons";
const NO_GUI_NOTE: &str = "there is no graphical session of the account; the agent was loaded into the background domain and starts again at the next login";

/// Account and session of the installing user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Session {
    pub(crate) home: PathBuf,
    pub(crate) uid: u32,
    pub(crate) user: String,
    pub(crate) graphical: bool,
    pub(crate) elevated: bool,
}

#[cfg(target_os = "macos")]
impl Session {
    /// Session of the running account.
    pub(crate) fn host() -> Result<Self, Error> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            Error::new(
                ErrorCode::Io,
                "HOME is not set; cannot locate ~/Library/LaunchAgents",
            )
        })?;
        let facts = super::detect::host_facts();
        let uid = nix::unistd::getuid();
        let user = nix::unistd::User::from_uid(uid)
            .ok()
            .flatten()
            .map(|user| user.name)
            .unwrap_or_else(|| uid.to_string());
        Ok(Self {
            home,
            uid: uid.as_raw(),
            user,
            graphical: facts.graphical_session,
            elevated: facts.elevated,
        })
    }
}

/// The program execution required for `launchctl`.
#[cfg(target_os = "macos")]
pub(crate) fn required_host(host: Option<&dyn ProcessHost>) -> Result<&dyn ProcessHost, Error> {
    host.ok_or_else(|| {
        Error::new(
            ErrorCode::Internal,
            "installing a launchd job needs the program execution service of the console",
        )
    })
}

/// Property list of the job.
pub(crate) fn property_list(
    executable: &Path,
    mode: ServiceMode,
    account: Option<&str>,
) -> Result<Vec<u8>, Error> {
    let mut keep_alive = Dictionary::new();
    keep_alive.insert("SuccessfulExit".to_owned(), Value::Boolean(false));
    let mut job = Dictionary::new();
    job.insert("Label".to_owned(), Value::String(LABEL.to_owned()));
    job.insert(
        "ProgramArguments".to_owned(),
        Value::Array(vec![
            Value::String(executable.display().to_string()),
            Value::String("service".to_owned()),
            Value::String("run".to_owned()),
        ]),
    );
    job.insert("RunAtLoad".to_owned(), Value::Boolean(true));
    job.insert("KeepAlive".to_owned(), Value::Dictionary(keep_alive));
    job.insert(
        "ProcessType".to_owned(),
        Value::String("Background".to_owned()),
    );
    if let (ServiceMode::System, Some(account)) = (mode, account) {
        job.insert("UserName".to_owned(), Value::String(account.to_owned()));
    }
    let mut bytes = Vec::new();
    Value::Dictionary(job)
        .to_writer_xml(&mut bytes)
        .map_err(|error| {
            Error::new(
                ErrorCode::Internal,
                format!("cannot encode the property list: {error}"),
            )
        })?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Location of the property list.
pub(crate) fn property_list_path(mode: ServiceMode, home: &Path) -> PathBuf {
    match mode {
        ServiceMode::User => home.join("Library").join("LaunchAgents"),
        ServiceMode::System => PathBuf::from(DAEMON_DIR),
    }
    .join(format!("{LABEL}.plist"))
}

/// launchd domain of the job: `gui/<uid>` with a graphical session,
/// `user/<uid>` without one, `system` for a daemon.
pub(crate) fn domain(mode: ServiceMode, uid: u32, graphical: bool) -> String {
    match (mode, graphical) {
        (ServiceMode::System, _) => "system".to_owned(),
        (ServiceMode::User, true) => format!("gui/{uid}"),
        (ServiceMode::User, false) => format!("user/{uid}"),
    }
}

/// Arguments of `launchctl bootstrap`.
pub(crate) fn bootstrap_args(domain: &str, plist: &Path) -> Vec<String> {
    vec![
        "bootstrap".to_owned(),
        domain.to_owned(),
        plist.display().to_string(),
    ]
}

/// Arguments of `launchctl bootout`.
pub(crate) fn bootout_args(domain: &str) -> Vec<String> {
    vec!["bootout".to_owned(), format!("{domain}/{LABEL}")]
}

/// Runs `launchctl` with the arguments.
pub(crate) fn launchctl(host: &dyn ProcessHost, args: Vec<String>) -> Result<(), Error> {
    host.resolve_program(LAUNCHCTL, Path::new("/"))
        .map_err(|error| {
            Error::new(
                ErrorCode::ExternalMissing,
                format!("`launchctl` is not available: {}", error.message),
            )
            .with_detail("program", json!(LAUNCHCTL))
        })?;
    let mut spec = ProgramSpec::new(LAUNCHCTL, args, "/");
    spec.timeout_ms = Some(LAUNCHCTL_TIMEOUT_MS);
    let outcome = host.run_program(&spec, &CancelToken::new())?;
    if outcome.exit_code == Some(0) {
        return Ok(());
    }
    let reason = if outcome.timed_out {
        "did not finish within 10 s".to_owned()
    } else {
        String::from_utf8_lossy(&outcome.stderr).trim().to_owned()
    };
    Err(Error::new(
        ErrorCode::Service,
        format!("`launchctl {}` failed: {reason}", spec.args.join(" ")),
    )
    .with_detail("exit_code", json!(outcome.exit_code)))
}

/// Writes the property list and loads the job.
pub(crate) fn install_with(
    request: &InstallRequest,
    executable: &Path,
    choice: ModeChoice,
    host: &dyn ProcessHost,
    session: &Session,
) -> Result<InstallReport, Error> {
    if choice.mode == ServiceMode::System && !session.elevated {
        return Err(privilege_error("installing"));
    }
    let account = match choice.mode {
        ServiceMode::System => Some(
            request
                .account
                .clone()
                .or_else(|| {
                    std::env::var("SUDO_USER")
                        .ok()
                        .filter(|name| !name.is_empty())
                })
                .unwrap_or_else(|| session.user.clone()),
        ),
        ServiceMode::User => None,
    };
    let path = property_list_path(choice.mode, &session.home);
    write_file(
        &path,
        &property_list(executable, choice.mode, account.as_deref())?,
    )?;
    let domain = domain(choice.mode, session.uid, session.graphical);
    let backend = match choice.mode {
        ServiceMode::User => Backend::LaunchAgent,
        ServiceMode::System => Backend::LaunchDaemon,
    };
    let mut manifest = Manifest::new(choice.mode, backend, executable);
    manifest.files.push(path.clone());
    manifest.domain = Some(domain.clone());
    manifest.account = account;
    manifest.write(&request.paths)?;
    // A job that is loaded already, as after an earlier installation, makes
    // `bootstrap` fail; unloading it first makes the installation repeatable
    // and starts the job from the new property list.
    let _not_loaded_is_fine = launchctl(host, bootout_args(&domain));
    launchctl(host, bootstrap_args(&domain, &path))?;
    let notes = if choice.mode == ServiceMode::User && !session.graphical {
        vec![NO_GUI_NOTE.to_owned()]
    } else {
        Vec::new()
    };
    Ok(InstallReport {
        mode: choice.mode,
        mode_reason: choice.reason,
        backend,
        files: manifest.files,
        started: true,
        linger: None,
        notes,
    })
}

/// Unloads the job and deletes the files of the manifest.
pub(crate) fn uninstall_with(
    manifest: &Manifest,
    host: &dyn ProcessHost,
) -> Result<UninstallReport, Error> {
    let mut notes = Vec::new();
    if let Some(domain) = &manifest.domain
        && let Err(error) = launchctl(host, bootout_args(domain))
    {
        notes.push(format!("the job was not loaded: {}", error.message));
    }
    let files = files_named(&manifest.files, &format!("{LABEL}.plist"), &mut notes);
    let removed = remove_files(&files)?;
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
    use crate::install::test_host::RecordingHost;
    use crate::paths::ServicePaths;

    const GOLDEN_AGENT: &str = include_str!("../../tests/golden/nurt.service.plist");

    fn session(dir: &Path, graphical: bool) -> Session {
        Session {
            home: dir.join("home"),
            uid: 501,
            user: "dev".to_owned(),
            graphical,
            elevated: false,
        }
    }

    #[test]
    fn install_macos_plist_golden_and_launchctl_argv() {
        let agent =
            property_list(Path::new("/opt/nurt/bin/nurt"), ServiceMode::User, None).unwrap();
        assert_eq!(String::from_utf8(agent).unwrap(), GOLDEN_AGENT);
        let daemon = property_list(
            Path::new("/opt/nurt/bin/nurt"),
            ServiceMode::System,
            Some("nurt"),
        )
        .unwrap();
        assert!(
            String::from_utf8(daemon)
                .unwrap()
                .contains("<key>UserName</key>\n\t<string>nurt</string>")
        );

        let dir = tempfile::tempdir().unwrap();
        let request = InstallRequest::new(ServicePaths::new(dir.path().join("state")));
        let host = RecordingHost::new(0);
        let choice = ModeChoice {
            mode: ServiceMode::User,
            reason: ModeReason::Requested,
        };
        let report = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            choice,
            &host,
            &session(dir.path(), true),
        )
        .unwrap();
        let plist = dir
            .path()
            .join("home/Library/LaunchAgents/nurt.service.plist");
        assert_eq!(report.files, std::slice::from_ref(&plist));
        assert!(report.notes.is_empty());
        let calls = host.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].args, ["bootout", "gui/501/nurt.service"]);
        assert_eq!(calls[1].program, "launchctl");
        assert_eq!(
            calls[1].args,
            ["bootstrap", "gui/501", plist.to_str().unwrap()]
        );
        assert_eq!(calls[1].timeout_ms, Some(10_000));

        let manifest = Manifest::read(&request.paths.install_manifest())
            .unwrap()
            .unwrap();
        let removed = uninstall_with(&manifest, &host).unwrap();
        assert_eq!(removed.removed, [plist]);
        assert_eq!(host.calls()[2].args, ["bootout", "gui/501/nurt.service"]);
    }

    #[test]
    fn install_macos_twice_unloads_the_loaded_job_before_bootstrap() {
        let dir = tempfile::tempdir().unwrap();
        let request = InstallRequest::new(ServicePaths::new(dir.path().join("state")));
        let host = RecordingHost::new(0);
        let choice = ModeChoice {
            mode: ServiceMode::User,
            reason: ModeReason::Requested,
        };
        for _ in 0..2 {
            install_with(
                &request,
                Path::new("/opt/nurt/bin/nurt"),
                choice,
                &host,
                &session(dir.path(), true),
            )
            .unwrap();
        }
        let verbs: Vec<String> = host
            .calls()
            .iter()
            .map(|call| call.args[0].clone())
            .collect();
        assert_eq!(verbs, ["bootout", "bootstrap", "bootout", "bootstrap"]);
    }

    #[test]
    fn uninstall_macos_removes_only_the_property_list_of_the_job() {
        let dir = tempfile::tempdir().unwrap();
        let plist = dir.path().join("nurt.service.plist");
        let foreign = dir.path().join("precious.conf");
        std::fs::write(&plist, "plist").unwrap();
        std::fs::write(&foreign, "keep").unwrap();
        let mut manifest = Manifest::new(
            ServiceMode::User,
            Backend::LaunchAgent,
            Path::new("/opt/nurt/bin/nurt"),
        );
        manifest.files = vec![plist.clone(), foreign.clone()];
        let report = uninstall_with(&manifest, &RecordingHost::new(0)).unwrap();
        assert_eq!(report.removed, [plist]);
        assert!(foreign.exists());
        assert!(report.notes.iter().any(|note| note.contains("precious.conf")));
    }

    #[test]
    fn install_macos_without_gui_missing_launchctl_and_privilege() {
        assert_eq!(domain(ServiceMode::User, 501, false), "user/501");
        assert_eq!(domain(ServiceMode::System, 501, true), "system");
        assert_eq!(
            property_list_path(ServiceMode::System, Path::new("/Users/user")),
            Path::new("/Library/LaunchDaemons/nurt.service.plist")
        );
        let dir = tempfile::tempdir().unwrap();
        let request = InstallRequest::new(ServicePaths::new(dir.path().join("state")));
        let user = ModeChoice {
            mode: ServiceMode::User,
            reason: ModeReason::NoPrivilege,
        };
        let report = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            user,
            &RecordingHost::new(0),
            &session(dir.path(), false),
        )
        .unwrap();
        assert_eq!(report.notes.len(), 1);
        let error = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            user,
            &RecordingHost::missing(),
            &session(dir.path(), true),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::ExternalMissing);
        assert_eq!(error.details["program"], "launchctl");
        let error = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            user,
            &RecordingHost::new(5),
            &session(dir.path(), true),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Service);
        let system = ModeChoice {
            mode: ServiceMode::System,
            reason: ModeReason::Requested,
        };
        let error = install_with(
            &request,
            Path::new("/opt/nurt/bin/nurt"),
            system,
            &RecordingHost::new(0),
            &session(dir.path(), true),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Privilege);
    }
}

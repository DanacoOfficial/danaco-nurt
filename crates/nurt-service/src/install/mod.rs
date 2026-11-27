// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::install
// Responsibility:   Installation and removal of the service with the service manager of the operating system.
// Dependencies:     detect, nurt_contracts, serde, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Installation and removal of the service with the service manager of the
//! operating system.
//!
//! | System | Mode `user` | Mode `system` |
//! |---|---|---|
//! | Linux | systemd user unit `nurt-service.service`, lingering through `org.freedesktop.login1` | systemd system unit `nurt-service.service` |
//! | macOS | LaunchAgent `nurt.service` | LaunchDaemon `nurt.service` |
//! | Windows | task `Nurt Service` of the Task Scheduler, started at logon | service `NurtService` of the service control manager |
//!
//! Every installation writes the manifest `service.install.json` to the
//! state directory; removal deletes only the files listed there.

pub mod detect;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(any(windows, test))]
mod task_scheduler;
#[cfg(test)]
#[cfg(target_os = "linux")]
mod test_bus;
#[cfg(test)]
mod test_host;
#[cfg(windows)]
mod windows;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::config::ServiceMode;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::services::ProcessHost;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::lock::create_private_dir;
use crate::paths::ServicePaths;
use detect::{ModeChoice, ModeReason};

const MANIFEST_FORMAT: &str = "nurt-service-install/1";

/// Service manager entry created by an installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Backend {
    /// systemd user unit (Linux, mode `user`).
    SystemdUser,
    /// systemd system unit (Linux, mode `system`).
    SystemdSystem,
    /// launchd agent (macOS, mode `user`).
    LaunchAgent,
    /// launchd daemon (macOS, mode `system`).
    LaunchDaemon,
    /// Service of the service control manager (Windows, mode `system`).
    WindowsService,
    /// Task of the Task Scheduler started at logon (Windows, mode `user`).
    TaskScheduler,
}

/// Installation manifest `service.install.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Format identifier, `nurt-service-install/1`.
    pub format: String,
    /// Installation mode.
    pub mode: ServiceMode,
    /// Service manager entry.
    pub backend: Backend,
    /// Version of Danaco Nurt that installed the service.
    pub version: String,
    /// Program started by the service manager.
    pub executable: PathBuf,
    /// Account the service runs as, when it is not the installing account.
    pub account: Option<String>,
    /// launchd domain of the job (macOS).
    pub domain: Option<String>,
    /// Files created by the installation; removal deletes exactly these.
    pub files: Vec<PathBuf>,
    /// Lingering of the account after installation (Linux, mode `user`).
    pub linger: Option<bool>,
}

impl Manifest {
    pub(crate) fn new(mode: ServiceMode, backend: Backend, executable: &Path) -> Self {
        Self {
            format: MANIFEST_FORMAT.to_owned(),
            mode,
            backend,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            executable: executable.to_owned(),
            account: None,
            domain: None,
            files: Vec::new(),
            linger: None,
        }
    }

    /// Reads a manifest; `Ok(None)` when there is none.
    ///
    /// # Errors
    ///
    /// `E_IO` when the file cannot be read or does not hold a manifest.
    pub fn read(path: &Path) -> Result<Option<Self>, Error> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
                Error::new(
                    ErrorCode::Io,
                    format!(
                        "{} is not an installation manifest: {error}",
                        path.display()
                    ),
                )
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error(path, &error)),
        }
    }

    pub(crate) fn write(&self, paths: &ServicePaths) -> Result<(), Error> {
        create_private_dir(paths.state_dir())
            .map_err(|error| io_error(paths.state_dir(), &error))?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| Error::new(ErrorCode::Internal, error.to_string()))?;
        write_file(&paths.install_manifest(), &bytes)
    }
}

/// Parameters of [`install`].
#[derive(Clone)]
pub struct InstallRequest {
    /// Requested mode; `None` selects the mode from the host
    /// ([`detect::choose_mode`]).
    pub mode: Option<ServiceMode>,
    /// State directory of the account the service runs as.
    pub paths: ServicePaths,
    /// Account of the service in mode `system`; by default the account that
    /// invoked `sudo`, or the current account.
    pub account: Option<String>,
    /// Program started by the service manager; by default the running
    /// program.
    pub executable: Option<PathBuf>,
    /// Program execution used for `launchctl` on macOS.
    pub process_host: Option<Arc<dyn ProcessHost>>,
}

impl InstallRequest {
    /// Request with the mode chosen from the host.
    #[must_use]
    pub fn new(paths: ServicePaths) -> Self {
        Self {
            mode: None,
            paths,
            account: None,
            executable: None,
            process_host: None,
        }
    }
}

impl std::fmt::Debug for InstallRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallRequest")
            .field("mode", &self.mode)
            .field("paths", &self.paths)
            .field("account", &self.account)
            .field("executable", &self.executable)
            .finish_non_exhaustive()
    }
}

/// Result of [`install`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallReport {
    /// Installed mode.
    pub mode: ServiceMode,
    /// Why the mode was chosen.
    pub mode_reason: ModeReason,
    /// Service manager entry.
    pub backend: Backend,
    /// Files created.
    pub files: Vec<PathBuf>,
    /// The service manager started the service.
    pub started: bool,
    /// Lingering of the account (Linux, mode `user`).
    pub linger: Option<bool>,
    /// Facts the user has to know, for example that the service starts only
    /// at login.
    pub notes: Vec<String>,
}

/// Parameters of [`uninstall`].
#[derive(Clone)]
pub struct UninstallRequest {
    /// Mode to remove; `None` removes the installed mode.
    pub mode: Option<ServiceMode>,
    /// State directory with the installation manifest.
    pub paths: ServicePaths,
    /// Program execution used for `launchctl` on macOS.
    pub process_host: Option<Arc<dyn ProcessHost>>,
}

impl std::fmt::Debug for UninstallRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UninstallRequest")
            .field("mode", &self.mode)
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

/// Result of [`uninstall`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UninstallReport {
    /// Removed mode.
    pub mode: ServiceMode,
    /// Removed service manager entry.
    pub backend: Backend,
    /// Files deleted.
    pub removed: Vec<PathBuf>,
    /// Facts the user has to know.
    pub notes: Vec<String>,
}

/// Installs the service, registers it with the service manager and starts
/// it.
///
/// # Errors
///
/// `E_PRIVILEGE` (with `required` in `details`) for mode `system` without
/// administrator rights; `E_SERVICE` when the service manager refuses or is
/// not reachable, with the next step in the message; `E_EXTERNAL_MISSING`
/// when `launchctl` is not available (macOS); `E_IO` when a file cannot be
/// written; `E_UNSUPPORTED` on other systems.
pub fn install(request: &InstallRequest) -> Result<InstallReport, Error> {
    let executable = match &request.executable {
        Some(executable) => executable.clone(),
        None => current_executable()?,
    };
    let choice = match request.mode {
        Some(mode) => ModeChoice {
            mode,
            reason: ModeReason::Requested,
        },
        None => detect::choose_mode(&detect::host_facts()),
    };
    let notes = remove_other_mode(request, choice.mode, &uninstall)?;
    let mut report = install_platform(request, &executable, choice)?;
    report.notes.splice(0..0, notes);
    Ok(report)
}

/// Removes an installation of another mode than the one requested, with its
/// manifest, so that the new manifest does not lose track of its files and
/// the old service does not keep running beside the new one. An unreadable
/// manifest counts as none: the installation overwrites it.
fn remove_other_mode(
    request: &InstallRequest,
    mode: ServiceMode,
    remove: &dyn Fn(&UninstallRequest) -> Result<UninstallReport, Error>,
) -> Result<Vec<String>, Error> {
    let previous = Manifest::read(&request.paths.install_manifest())
        .ok()
        .flatten();
    let Some(previous) = previous.filter(|previous| previous.mode != mode) else {
        return Ok(Vec::new());
    };
    let report = remove(&UninstallRequest {
        mode: Some(previous.mode),
        paths: request.paths.clone(),
        process_host: request.process_host.clone(),
    })?;
    let mut notes = vec![format!(
        "the installation in mode `{}` was removed",
        mode_name(previous.mode)
    )];
    notes.extend(report.notes);
    Ok(notes)
}

/// Stops the service, unregisters it and deletes the files listed in the
/// installation manifest.
///
/// # Errors
///
/// `E_NOT_FOUND` when no installation manifest exists; `E_INVALID_VALUE`
/// when `mode` differs from the installed mode; `E_PRIVILEGE` for mode
/// `system` without administrator rights; `E_SERVICE` when the service
/// manager refuses; `E_IO` when a file cannot be deleted.
pub fn uninstall(request: &UninstallRequest) -> Result<UninstallReport, Error> {
    let manifest_path = request.paths.install_manifest();
    let Some(manifest) = Manifest::read(&manifest_path)? else {
        return Err(Error::new(
            ErrorCode::NotFound,
            "the service is not installed: there is no installation manifest",
        )
        .with_detail("path", json!(manifest_path.display().to_string())));
    };
    if let Some(mode) = request.mode
        && mode != manifest.mode
    {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!(
                "the service is installed in mode `{}`; remove it with that mode",
                mode_name(manifest.mode)
            ),
        ));
    }
    let report = uninstall_platform(request, &manifest)?;
    fs::remove_file(&manifest_path).map_err(|error| io_error(&manifest_path, &error))?;
    Ok(report)
}

#[cfg(target_os = "linux")]
fn install_platform(
    request: &InstallRequest,
    executable: &Path,
    choice: ModeChoice,
) -> Result<InstallReport, Error> {
    linux::install_with(
        request,
        executable,
        choice,
        &linux::SystemBuses,
        &linux::Layout::host()?,
        crate::process::is_elevated(),
    )
}

#[cfg(target_os = "linux")]
fn uninstall_platform(
    request: &UninstallRequest,
    manifest: &Manifest,
) -> Result<UninstallReport, Error> {
    let _ = request;
    linux::uninstall_with(manifest, &linux::SystemBuses)
}

#[cfg(target_os = "macos")]
fn install_platform(
    request: &InstallRequest,
    executable: &Path,
    choice: ModeChoice,
) -> Result<InstallReport, Error> {
    let host = macos::required_host(request.process_host.as_deref())?;
    macos::install_with(request, executable, choice, host, &macos::Session::host()?)
}

#[cfg(target_os = "macos")]
fn uninstall_platform(
    request: &UninstallRequest,
    manifest: &Manifest,
) -> Result<UninstallReport, Error> {
    let host = macos::required_host(request.process_host.as_deref())?;
    macos::uninstall_with(manifest, host)
}

#[cfg(windows)]
fn install_platform(
    request: &InstallRequest,
    executable: &Path,
    choice: ModeChoice,
) -> Result<InstallReport, Error> {
    windows::install(request, executable, choice)
}

#[cfg(windows)]
fn uninstall_platform(
    request: &UninstallRequest,
    manifest: &Manifest,
) -> Result<UninstallReport, Error> {
    let _ = request;
    windows::uninstall(manifest)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn install_platform(
    _request: &InstallRequest,
    _executable: &Path,
    _choice: ModeChoice,
) -> Result<InstallReport, Error> {
    Err(unsupported())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn uninstall_platform(
    _request: &UninstallRequest,
    _manifest: &Manifest,
) -> Result<UninstallReport, Error> {
    Err(unsupported())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn unsupported() -> Error {
    Error::new(
        ErrorCode::Unsupported,
        "the service can be installed on Linux, macOS and Windows only",
    )
}

/// Program file the service manager starts: the path the running process was
/// invoked by, when it names the running program, otherwise the canonical
/// file of the process, without the Windows verbatim prefix.
///
/// The canonical file lies in the directory of one version, so a service
/// started from it keeps running that version after the program is updated
/// through a link such as a `current` directory or a `bin` link.
fn current_executable() -> Result<PathBuf, Error> {
    let executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map(strip_verbatim)
        .map_err(|error| {
            Error::new(
                ErrorCode::Io,
                format!("cannot locate the running program: {error}"),
            )
        })?;
    #[cfg(unix)]
    if let Some(invoked) = invoked_executable(&executable) {
        return Ok(invoked);
    }
    Ok(executable)
}

/// The path of this process's invocation, see [`invocation_path`].
#[cfg(unix)]
fn invoked_executable(running: &Path) -> Option<PathBuf> {
    let name = std::env::args_os().next()?;
    let cwd = std::env::current_dir().ok()?;
    let search = std::env::var_os("PATH");
    invocation_path(Path::new(&name), search.as_deref(), &cwd, running)
}

/// Absolute path, with links kept, by which the program `running` was
/// started as `name`: a name with a directory part is taken relative to
/// `cwd`, a bare name is looked up in `search` like a shell does. `None`
/// when the name is not found, contains `..` (which would depend on links) or
/// names another file than `running`.
#[cfg(unix)]
fn invocation_path(
    name: &Path,
    search: Option<&std::ffi::OsStr>,
    cwd: &Path,
    running: &Path,
) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Component;

    let found = if name.components().count() > 1 {
        cwd.join(name)
    } else {
        std::env::split_paths(search?)
            .map(|dir| cwd.join(dir).join(name))
            .find(|file| {
                fs::metadata(file)
                    .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            })?
    };
    let mut clean = PathBuf::new();
    for component in found.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => return None,
            other => clean.push(other),
        }
    }
    (fs::canonicalize(&clean).ok()? == running).then_some(clean)
}

fn strip_verbatim(path: PathBuf) -> PathBuf {
    const VERBATIM_DISK: &str = r"\\?\";
    match path
        .to_str()
        .and_then(|text| text.strip_prefix(VERBATIM_DISK))
    {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// Name of a mode as written in the configuration.
#[must_use]
pub(crate) fn mode_name(mode: ServiceMode) -> &'static str {
    match mode {
        ServiceMode::User => "user",
        ServiceMode::System => "system",
    }
}

/// `E_PRIVILEGE` for an operation of mode `system`.
pub(crate) fn privilege_error(action: &str) -> Error {
    Error::new(
        ErrorCode::Privilege,
        format!("{action} the service in mode `system` requires administrator rights; run the command as an administrator or use mode `user`"),
    )
    .with_detail("required", json!("administrator"))
}

/// `E_IO` for a file.
pub(crate) fn io_error(path: &Path, error: &io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("{}: {error}", path.display()))
        .with_detail("path", json!(path.display().to_string()))
}

/// Writes a file through a temporary file in the same directory, creating
/// the directory when needed.
pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| io_error(parent, &error))?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    // A leftover or planted file of that name is replaced and never written
    // through, as the installer may run with more rights than the account
    // that controls the directory.
    let _absent_is_fine = fs::remove_file(&temp);
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .and_then(|mut file| file.write_all(bytes));
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(io_error(&temp, &error));
    }
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        io_error(path, &error)
    })
}

/// Files of a manifest with the given name. The manifest lies in the state
/// directory of the account, so it can be changed by that account while the
/// removal may run with administrator rights; a listed file of another name
/// is not one the installation wrote, it stays and is reported in `notes`.
#[cfg(any(target_os = "linux", target_os = "macos", test))]
pub(crate) fn files_named(files: &[PathBuf], name: &str, notes: &mut Vec<String>) -> Vec<PathBuf> {
    let mut kept = Vec::new();
    for file in files {
        if file.file_name().is_some_and(|found| found == name) {
            kept.push(file.clone());
        } else {
            notes.push(format!(
                "{} is not a file of the service and was not removed",
                file.display()
            ));
        }
    }
    kept
}

/// Deletes the files of a manifest; files already gone count as deleted.
#[cfg(any(target_os = "linux", target_os = "macos", test))]
pub(crate) fn remove_files(files: &[PathBuf]) -> Result<Vec<PathBuf>, Error> {
    let mut removed = Vec::new();
    for file in files {
        match fs::remove_file(file) {
            Ok(()) => removed.push(file.clone()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(file, &error)),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    mod invocation {
        use std::os::unix::fs::{PermissionsExt, symlink};

        use super::*;

        /// A program in a version directory, reached through a link in `bin`.
        struct Release {
            _dir: tempfile::TempDir,
            base: PathBuf,
            running: PathBuf,
        }

        fn release() -> Release {
            let dir = tempfile::tempdir().unwrap();
            let base = fs::canonicalize(dir.path()).unwrap();
            let version = base.join("releases/1.0");
            fs::create_dir_all(&version).unwrap();
            fs::create_dir_all(base.join("bin")).unwrap();
            let program = version.join("nurt");
            fs::write(&program, "program").unwrap();
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
            symlink(&program, base.join("bin/nurt")).unwrap();
            Release {
                _dir: dir,
                running: program,
                base,
            }
        }

        #[test]
        fn a_bare_name_is_looked_up_in_the_search_path_and_keeps_the_link() {
            let release = release();
            let search =
                std::env::join_paths([release.base.join("missing"), release.base.join("bin")])
                    .unwrap();
            assert_eq!(
                invocation_path(
                    Path::new("nurt"),
                    Some(&search),
                    Path::new("/"),
                    &release.running
                ),
                Some(release.base.join("bin/nurt"))
            );
        }

        #[test]
        fn a_name_with_a_directory_part_is_taken_from_the_working_directory() {
            let release = release();
            let bin = release.base.join("bin");
            for (name, cwd) in [
                ("bin/nurt", release.base.clone()),
                ("./nurt", bin.clone()),
                (release.base.join("bin/nurt").to_str().unwrap(), bin),
            ] {
                assert_eq!(
                    invocation_path(Path::new(name), None, &cwd, &release.running),
                    Some(release.base.join("bin/nurt")),
                    "{name}"
                );
            }
        }

        #[test]
        fn a_path_that_does_not_name_the_running_program_is_refused() {
            let release = release();
            let other = release.base.join("bin/other");
            fs::write(&other, "another program").unwrap();
            fs::set_permissions(&other, fs::Permissions::from_mode(0o755)).unwrap();
            let search = release.base.join("bin");
            let search = search.as_os_str();
            let cwd = release.base.as_path();
            assert_eq!(
                invocation_path(Path::new("other"), Some(search), cwd, &release.running),
                None
            );
            assert_eq!(
                invocation_path(Path::new("absent"), Some(search), cwd, &release.running),
                None
            );
            assert_eq!(
                invocation_path(Path::new("bin/../bin/nurt"), None, cwd, &release.running),
                None
            );
            assert_eq!(
                invocation_path(Path::new("nurt"), None, cwd, &release.running),
                None
            );
        }

        #[test]
        fn a_file_without_execute_permission_is_skipped_in_the_search_path() {
            let release = release();
            let plain = release.base.join("plain");
            fs::create_dir_all(&plain).unwrap();
            fs::write(plain.join("nurt"), "not executable").unwrap();
            let search = std::env::join_paths([plain, release.base.join("bin")]).unwrap();
            assert_eq!(
                invocation_path(
                    Path::new("nurt"),
                    Some(&search),
                    &release.base,
                    &release.running
                ),
                Some(release.base.join("bin/nurt"))
            );
        }
    }

    #[test]
    fn uninstall_removes_only_manifest_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ServicePaths::new(dir.path().join("state"));
        let units = dir.path().join("units");
        fs::create_dir_all(&units).unwrap();
        let listed = units.join("nurt-service.service");
        let foreign = units.join("other.service");
        fs::write(&listed, "unit").unwrap();
        fs::write(&foreign, "unit of another program").unwrap();
        let mut manifest = Manifest::new(
            ServiceMode::User,
            Backend::SystemdUser,
            Path::new("/opt/nurt/bin/nurt"),
        );
        manifest.files = vec![listed.clone(), units.join("already-gone.service")];
        manifest.write(&paths).unwrap();
        assert_eq!(
            Manifest::read(&paths.install_manifest()).unwrap(),
            Some(manifest.clone())
        );
        let removed = remove_files(&manifest.files).unwrap();
        assert_eq!(removed, vec![listed.clone()]);
        assert!(!listed.exists());
        assert!(foreign.exists());
    }

    #[test]
    fn uninstall_without_manifest_or_with_other_mode_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ServicePaths::new(dir.path());
        let request = UninstallRequest {
            mode: None,
            paths: paths.clone(),
            process_host: None,
        };
        assert_eq!(uninstall(&request).unwrap_err().code, ErrorCode::NotFound);
        Manifest::new(
            ServiceMode::User,
            Backend::SystemdUser,
            Path::new("/usr/bin/nurt"),
        )
        .write(&paths)
        .unwrap();
        let request = UninstallRequest {
            mode: Some(ServiceMode::System),
            ..request
        };
        assert_eq!(
            uninstall(&request).unwrap_err().code,
            ErrorCode::InvalidValue
        );
        fs::write(paths.install_manifest(), b"[]").unwrap();
        assert_eq!(
            Manifest::read(&paths.install_manifest()).unwrap_err().code,
            ErrorCode::Io
        );
    }

    fn removal(mode: ServiceMode) -> UninstallReport {
        UninstallReport {
            mode,
            backend: Backend::SystemdUser,
            removed: Vec::new(),
            notes: vec!["unit stopped".to_owned()],
        }
    }

    #[test]
    fn installing_in_another_mode_removes_the_previous_installation_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ServicePaths::new(dir.path());
        let request = InstallRequest::new(paths.clone());
        Manifest::new(
            ServiceMode::User,
            Backend::SystemdUser,
            Path::new("/opt/nurt/bin/nurt"),
        )
        .write(&paths)
        .unwrap();
        let asked = std::cell::RefCell::new(Vec::new());
        let remove = |removal_request: &UninstallRequest| -> Result<UninstallReport, Error> {
            asked.borrow_mut().push(removal_request.mode);
            Ok(removal(ServiceMode::User))
        };
        let same = remove_other_mode(&request, ServiceMode::User, &remove).unwrap();
        assert!(same.is_empty() && asked.borrow().is_empty());
        let other = remove_other_mode(&request, ServiceMode::System, &remove).unwrap();
        assert_eq!(*asked.borrow(), [Some(ServiceMode::User)]);
        assert_eq!(other.len(), 2);
        assert!(other[0].contains("mode `user`"));
        assert_eq!(other[1], "unit stopped");
    }

    #[test]
    fn a_failed_removal_of_the_previous_installation_stops_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ServicePaths::new(dir.path());
        Manifest::new(
            ServiceMode::System,
            Backend::SystemdSystem,
            Path::new("/opt/nurt/bin/nurt"),
        )
        .write(&paths)
        .unwrap();
        let request = InstallRequest::new(paths);
        let remove = |_: &UninstallRequest| -> Result<UninstallReport, Error> {
            Err(privilege_error("removing"))
        };
        let error = remove_other_mode(&request, ServiceMode::User, &remove).unwrap_err();
        assert_eq!(error.code, ErrorCode::Privilege);
        let nothing = remove_other_mode(
            &InstallRequest::new(ServicePaths::new(dir.path().join("none"))),
            ServiceMode::User,
            &remove,
        );
        assert!(nothing.unwrap().is_empty());
    }

    #[test]
    fn files_named_keeps_only_files_of_that_name_and_reports_the_rest() {
        let files = vec![
            PathBuf::from("/units/nurt-service.service"),
            PathBuf::from("/etc/passwd"),
            PathBuf::from("/units/nurt-service.service/.."),
        ];
        let mut notes = Vec::new();
        assert_eq!(
            files_named(&files, "nurt-service.service", &mut notes),
            vec![PathBuf::from("/units/nurt-service.service")]
        );
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("/etc/passwd"));
        assert!(files_named(&[], "x", &mut notes).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn write_file_replaces_a_planted_link_instead_of_writing_through_it() {
        let dir = tempfile::tempdir().unwrap();
        let precious = dir.path().join("precious");
        fs::write(&precious, "keep").unwrap();
        let path = dir.path().join("service.install.json");
        std::os::unix::fs::symlink(&precious, dir.path().join("service.install.json.tmp")).unwrap();
        write_file(&path, b"manifest").unwrap();
        assert_eq!(fs::read_to_string(&precious).unwrap(), "keep");
        assert_eq!(fs::read_to_string(&path).unwrap(), "manifest");
        assert!(!dir.path().join("service.install.json.tmp").exists());
    }

    #[test]
    fn write_file_creates_directories_and_replaces_the_previous_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("units/nested/nurt-service.service");
        write_file(&path, b"first").unwrap();
        write_file(&path, b"second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        let error = write_file(&dir.path().join("units"), b"x").unwrap_err();
        assert_eq!(error.code, ErrorCode::Io);
    }

    #[test]
    fn verbatim_prefix_is_removed_from_disk_paths_only() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\Nurt\nurt.exe")),
            PathBuf::from(r"C:\Nurt\nurt.exe")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\host\share")),
            PathBuf::from(r"\\?\UNC\host\share")
        );
        assert_eq!(privilege_error("installing").code, ErrorCode::Privilege);
        assert_eq!(mode_name(ServiceMode::System), "system");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

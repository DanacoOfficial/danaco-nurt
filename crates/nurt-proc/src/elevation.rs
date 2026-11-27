// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Elevation backends: the programs that run `nurt elevated-exec` with the
//! rights of the administrator.
//!
//! A backend starts the running executable with the subcommand
//! `elevated-exec`, writes one request in the request format of the protocol to
//! its standard input and reads one response from its standard output. No
//! shell text is built and no backend asks for a password: a backend that
//! would ask is reported as unavailable with the reason `password_required`.
//!
//! The functions of this module that build command lines and classify
//! failures are pure and run on every system; only [`run_backend`] starts a
//! process.

use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Name of the hidden subcommand of `nurt` that executes one request with
/// elevated rights.
pub const ELEVATED_EXEC: &str = "elevated-exec";

/// Exit code of `elevated-exec` that started without the identity of the
/// administrator.
pub const EXIT_NOT_ADMINISTRATOR: u8 = 126;

/// Exit code of `elevated-exec` that could not read a request.
pub const EXIT_BAD_REQUEST: u8 = 2;

/// Largest response read from a backend, in bytes.
const MAX_RESPONSE: u64 = 256 * 1024 * 1024;

/// An elevation backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// The process already has the identity of the administrator.
    SelfProcess,
    /// The privileged service `nurt-elevation` (release 0.1.2).
    Broker,
    /// The program `sudo`.
    Sudo,
    /// The program `doas`.
    Doas,
    /// The program `run0` of systemd.
    Run0,
}

impl Backend {
    /// Every backend, in the default order of trial.
    pub const ALL: [Self; 5] = [
        Self::SelfProcess,
        Self::Broker,
        Self::Sudo,
        Self::Doas,
        Self::Run0,
    ];

    /// Stable name, as written in the configuration and in reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SelfProcess => "self",
            Self::Broker => "broker",
            Self::Sudo => "sudo",
            Self::Doas => "doas",
            Self::Run0 => "run0",
        }
    }

    /// Backend with the name `name`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|backend| backend.as_str() == name)
    }

    /// Name of the program that a backend starts, when it starts one.
    #[must_use]
    pub const fn program(self) -> Option<&'static str> {
        match self {
            Self::Sudo => Some("sudo"),
            Self::Doas => Some("doas"),
            Self::Run0 => Some("run0"),
            Self::SelfProcess | Self::Broker => None,
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Operating system family for the choice of backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    /// Linux.
    Linux,
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
}

impl System {
    /// The system this build runs on.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Backends of `system` in the default order of trial.
#[must_use]
pub fn default_backends(system: System) -> Vec<Backend> {
    match system {
        System::Linux => Backend::ALL.to_vec(),
        System::MacOs => vec![Backend::SelfProcess, Backend::Broker, Backend::Sudo],
        System::Windows => vec![Backend::SelfProcess, Backend::Broker],
    }
}

/// Backends named by the configuration key `privilege.backends`.
///
/// # Errors
///
/// Returns the first name that is not a backend.
pub fn parse_backends(names: &[String]) -> Result<Vec<Backend>, String> {
    names
        .iter()
        .map(|name| Backend::from_name(name).ok_or_else(|| name.clone()))
        .collect()
}

/// Why a backend did not run the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The backend is not installed (the program is missing, or the service
    /// does not exist in this release).
    NotInstalled,
    /// The backend would ask for a password.
    PasswordRequired,
    /// The system denies the account the elevation.
    Denied,
    /// The process is not the administrator, so `self` does not apply.
    NotAdministrator,
    /// The backend cannot switch to the requested account.
    CannotSwitchUser,
    /// The backend ran but produced no response.
    Failed(String),
}

impl Refusal {
    /// Stable code of the reason, as written in the `backend` detail of
    /// `E_PRIVILEGE`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::PasswordRequired => "password_required",
            Self::Denied => "denied",
            Self::NotAdministrator => "not_administrator",
            Self::CannotSwitchUser => "cannot_switch_user",
            Self::Failed(_) => "failed",
        }
    }

    /// Text that explains the reason.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Failed(detail) => detail.clone(),
            other => other.code().to_owned(),
        }
    }
}

/// Command line that starts `executable elevated-exec` through `backend`.
///
/// Returns `None` for the backends that start no program. `as_user` selects
/// another account for the programs that support it.
#[must_use]
pub fn command_line(
    backend: Backend,
    program: &Path,
    executable: &Path,
    as_user: Option<&str>,
) -> Option<Vec<String>> {
    let exe = executable.to_string_lossy().into_owned();
    let program = program.to_string_lossy().into_owned();
    let mut line = vec![program];
    match backend {
        Backend::Sudo => {
            line.push("-n".to_owned());
            if let Some(user) = as_user {
                line.push("-u".to_owned());
                line.push(user.to_owned());
            }
            line.push("--".to_owned());
        }
        Backend::Doas => {
            line.push("-n".to_owned());
            if let Some(user) = as_user {
                line.push("-u".to_owned());
                line.push(user.to_owned());
            }
        }
        Backend::Run0 => {
            line.push("--no-ask-password".to_owned());
            if let Some(user) = as_user {
                line.push(format!("--user={user}"));
            }
        }
        Backend::SelfProcess | Backend::Broker => return None,
    }
    line.push(exe);
    line.push(ELEVATED_EXEC.to_owned());
    Some(line)
}

/// Classifies the standard error of a backend that produced no response.
#[must_use]
pub fn classify_failure(backend: Backend, stderr: &str) -> Refusal {
    let text = stderr.to_ascii_lowercase();
    let asks = [
        "password is required",
        "a terminal is required",
        "no tty present",
        "authentication required",
        "authentication is required",
        "interactive authentication",
        "askpass",
    ];
    if asks.iter().any(|marker| text.contains(marker)) {
        return Refusal::PasswordRequired;
    }
    let denied = [
        "is not in the sudoers",
        "may not run sudo",
        "not allowed to execute",
        "not permitted",
        "permission denied",
        "access denied",
        "authentication failed",
        "operation not permitted",
    ];
    if denied.iter().any(|marker| text.contains(marker)) {
        return Refusal::Denied;
    }
    if backend == Backend::Sudo && text.contains("unknown user") {
        return Refusal::CannotSwitchUser;
    }
    let detail = stderr.trim();
    Refusal::Failed(if detail.is_empty() {
        format!("{backend} ended without a response")
    } else {
        detail.lines().next().unwrap_or(detail).to_owned()
    })
}

/// Whether the process has the identity of the administrator: the effective
/// user `root` on Linux and macOS, an elevated token on Windows.
#[must_use]
pub fn is_administrator() -> bool {
    imp::is_administrator()
}

/// Directories of the system searched first for the program of a backend, so
/// that a program placed in a directory of the account never takes the
/// request.
pub const SYSTEM_PROGRAM_DIRS: [&str; 5] =
    ["/usr/bin", "/bin", "/usr/local/bin", "/usr/sbin", "/sbin"];

/// Value of `PATH` of an elevated process and of a backend.
pub const FIXED_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// Finds the program of `backend`: in the directories of `path` when it is
/// given, else in the system directories and last in the `PATH` of the
/// process.
#[must_use]
pub fn locate(backend: Backend, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let name = backend.program()?;
    if let Some(path) = path {
        return which::which_in(name, Some(path), std::env::current_dir().ok()?).ok();
    }
    SYSTEM_PROGRAM_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| candidate.is_file())
        .or_else(|| which::which(name).ok())
}

/// The environment that a backend receives: a fixed `PATH`, the language
/// settings and the terminal type. Nothing else of the environment of the
/// account reaches the elevated process.
#[must_use]
pub fn backend_environment<I>(vars: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut kept: Vec<(String, String)> = vec![("PATH".to_owned(), FIXED_PATH.to_owned())];
    for (name, value) in vars {
        let allowed = name == "LANG" || name == "TERM" || name.starts_with("LC_");
        if allowed && !value.contains('\0') {
            kept.push((name, value));
        }
    }
    kept
}

/// Replaces the environment of this process by the one of an elevated
/// process: the variables of [`backend_environment`] and, on Unix, `HOME`,
/// `USER` and `LOGNAME` of the account database for the effective user. The
/// variables `NURT_*`, `XDG_*` and every other variable of the caller are
/// removed, so that the configuration and the state of the elevated process
/// never come from the caller.
pub fn reset_environment() {
    let kept = backend_environment(std::env::vars());
    let names: Vec<std::ffi::OsString> = std::env::vars_os().map(|(name, _)| name).collect();
    for name in names {
        // SAFETY: called at the start of `elevated-exec` before any thread
        // reads the environment.
        unsafe { std::env::remove_var(name) };
    }
    for (name, value) in kept {
        // SAFETY: as above.
        unsafe { std::env::set_var(name, value) };
    }
    imp::set_account_variables();
}

/// Runs `executable elevated-exec` through `backend` with `request` on its
/// standard input and returns what it wrote to the standard output.
///
/// `path` is the search path of the programs of the backends; `None` takes
/// the path of the process.
///
/// # Errors
///
/// Returns the reason when the backend is not installed, refuses the
/// elevation or ends without a response; a response of `elevated-exec` (also
/// a response that reports an error) is returned as bytes.
pub fn run_backend(
    backend: Backend,
    executable: &Path,
    request: &[u8],
    as_user: Option<&str>,
    path: Option<&std::ffi::OsStr>,
    timeout: Duration,
) -> Result<Vec<u8>, Refusal> {
    if backend == Backend::Broker {
        return Err(Refusal::NotInstalled);
    }
    let program = locate(backend, path).ok_or(Refusal::NotInstalled)?;
    let line = command_line(backend, &program, executable, as_user)
        .ok_or(Refusal::Failed("no program".to_owned()))?;
    let mut command = Command::new(&line[0]);
    command
        .env_clear()
        .envs(backend_environment(std::env::vars()))
        .args(&line[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| Refusal::Failed(error.to_string()))?;
    let mut stdin = child.stdin.take();
    let payload = request.to_vec();
    let writer = thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(&payload);
        }
        drop(stdin);
    });
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(stdout) = stdout.as_mut() {
            let _ = stdout.take(MAX_RESPONSE).read_to_end(&mut buffer);
        }
        buffer
    });
    let err = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.take(1 << 20).read_to_end(&mut buffer);
        }
        buffer
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(Refusal::Failed(format!(
                    "{backend} did not finish within {} s",
                    timeout.as_secs()
                )));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => break Err(Refusal::Failed(error.to_string())),
        }
    };
    let _ = writer.join();
    let stdout = out.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    status?;
    if stdout.iter().any(|byte| !byte.is_ascii_whitespace()) {
        return Ok(stdout);
    }
    Err(classify_failure(backend, &stderr))
}

#[cfg(unix)]
mod imp {
    pub(super) fn is_administrator() -> bool {
        rustix::process::geteuid().is_root()
    }

    pub(super) fn set_account_variables() {
        let Ok(Some(user)) = nix::unistd::User::from_uid(nix::unistd::geteuid()) else {
            return;
        };
        // SAFETY: called before any thread reads the environment.
        unsafe {
            std::env::set_var("HOME", &user.dir);
            std::env::set_var("USER", &user.name);
            std::env::set_var("LOGNAME", &user.name);
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    pub(super) fn set_account_variables() {}
    use std::mem;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    pub(super) fn is_administrator() -> bool {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: the pseudo handle of the current process needs no closing;
        // `token` receives a new handle that is closed below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0_u32;
        let size = u32::try_from(mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(0);
        // SAFETY: the buffer is a `TOKEN_ELEVATION` of the declared size and
        // outlives the call.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                (&raw mut elevation).cast::<c_void>(),
                size,
                &mut returned,
            )
        };
        // SAFETY: `token` is the valid handle opened above.
        unsafe { CloseHandle(token) };
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: Option<Vec<String>>) -> String {
        line.expect("a command line").join(" ")
    }

    #[test]
    fn names_round_trip() {
        for backend in Backend::ALL {
            assert_eq!(Backend::from_name(backend.as_str()), Some(backend));
        }
        assert_eq!(Backend::from_name("su"), None);
    }

    #[test]
    fn default_order_depends_on_the_system() {
        let names = |system| -> Vec<&'static str> {
            default_backends(system)
                .into_iter()
                .map(Backend::as_str)
                .collect()
        };
        assert_eq!(
            names(System::Linux),
            ["self", "broker", "sudo", "doas", "run0"]
        );
        assert_eq!(names(System::MacOs), ["self", "broker", "sudo"]);
        assert_eq!(names(System::Windows), ["self", "broker"]);
    }

    #[test]
    fn configured_names_are_parsed_in_order() {
        let names = vec!["doas".to_owned(), "self".to_owned()];
        assert_eq!(
            parse_backends(&names),
            Ok(vec![Backend::Doas, Backend::SelfProcess])
        );
        assert_eq!(
            parse_backends(&["sudo".to_owned(), "su".to_owned()]),
            Err("su".to_owned())
        );
    }

    #[test]
    fn sudo_line_has_no_shell_and_never_asks() {
        let line = command_line(
            Backend::Sudo,
            Path::new("/usr/bin/sudo"),
            Path::new("/opt/nurt"),
            None,
        );
        assert_eq!(text(line), "/usr/bin/sudo -n -- /opt/nurt elevated-exec");
        let line = command_line(
            Backend::Sudo,
            Path::new("/usr/bin/sudo"),
            Path::new("/opt/nurt"),
            Some("deploy"),
        );
        assert_eq!(
            text(line),
            "/usr/bin/sudo -n -u deploy -- /opt/nurt elevated-exec"
        );
    }

    #[test]
    fn doas_and_run0_lines() {
        let doas = command_line(
            Backend::Doas,
            Path::new("doas"),
            Path::new("/opt/nurt"),
            Some("deploy"),
        );
        assert_eq!(text(doas), "doas -n -u deploy /opt/nurt elevated-exec");
        let run0 = command_line(
            Backend::Run0,
            Path::new("run0"),
            Path::new("/opt/nurt"),
            Some("deploy"),
        );
        assert_eq!(
            text(run0),
            "run0 --no-ask-password --user=deploy /opt/nurt elevated-exec"
        );
        assert!(command_line(Backend::Broker, Path::new("x"), Path::new("y"), None).is_none());
    }

    #[test]
    fn failures_are_classified() {
        assert_eq!(
            classify_failure(Backend::Sudo, "sudo: a password is required\n"),
            Refusal::PasswordRequired
        );
        assert_eq!(
            classify_failure(Backend::Doas, "doas: Authentication required"),
            Refusal::PasswordRequired
        );
        assert_eq!(
            classify_failure(Backend::Run0, "Interactive authentication required."),
            Refusal::PasswordRequired
        );
        assert_eq!(
            classify_failure(Backend::Sudo, "ann is not in the sudoers file."),
            Refusal::Denied
        );
        assert_eq!(
            classify_failure(Backend::Sudo, "sudo: unknown user zed"),
            Refusal::CannotSwitchUser
        );
        assert!(matches!(
            classify_failure(Backend::Sudo, ""),
            Refusal::Failed(_)
        ));
    }

    #[test]
    fn backend_environment_keeps_only_the_allowed_variables() {
        let vars = [
            ("NURT_CONFIG", "/home/a/nurt.toml"),
            ("XDG_CONFIG_HOME", "/home/a/.config"),
            ("HOME", "/home/a"),
            ("PATH", "/home/a/bin"),
            ("LANG", "pl_PL.UTF-8"),
            ("LC_ALL", "C"),
            ("TERM", "xterm"),
        ]
        .map(|(name, value)| (name.to_owned(), value.to_owned()));
        let kept = backend_environment(vars);
        let names: Vec<&str> = kept.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["PATH", "LANG", "LC_ALL", "TERM"]);
        assert_eq!(kept[0].1, FIXED_PATH);
    }

    #[test]
    fn reasons_have_stable_codes() {
        assert_eq!(Refusal::NotInstalled.code(), "not_installed");
        assert_eq!(Refusal::PasswordRequired.code(), "password_required");
        assert_eq!(Refusal::Denied.message(), "denied");
    }

    #[test]
    fn broker_is_not_installed_in_this_release() {
        let result = run_backend(
            Backend::Broker,
            Path::new("/opt/nurt"),
            b"{}",
            None,
            None,
            Duration::from_secs(1),
        );
        assert_eq!(result, Err(Refusal::NotInstalled));
    }

    #[test]
    fn missing_program_is_not_installed() {
        let empty = std::ffi::OsString::new();
        let result = run_backend(
            Backend::Sudo,
            Path::new("/opt/nurt"),
            b"{}",
            None,
            Some(empty.as_os_str()),
            Duration::from_secs(1),
        );
        assert_eq!(result, Err(Refusal::NotInstalled));
    }

    #[test]
    fn administrator_matches_the_effective_user() {
        #[cfg(unix)]
        assert_eq!(is_administrator(), rustix::process::geteuid().is_root());
        #[cfg(windows)]
        let _ = is_administrator();
    }
}

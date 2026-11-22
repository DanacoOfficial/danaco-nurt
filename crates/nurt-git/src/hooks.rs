// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repository hooks run as user programs.
//!
//! A hook is a program in the hooks directory of a repository:
//! `core.hooksPath` when it is set, otherwise `hooks/` in the common
//! repository directory. A relative `core.hooksPath` is resolved against the
//! directory in which hooks run (the root of the working tree, or the
//! repository directory of a bare repository). A hook that is missing or not
//! executable is skipped, as `git` skips it.
//!
//! Hooks are started through [`ProcessHost`], with the rules of the `run`
//! command for program resolution, environment, limits and cancellation.
//! The console never starts a program in any other way.
//!
//! # Examples
//!
//! ```
//! use std::path::Path;
//! use nurt_git::hooks::{Hook, HookDir};
//!
//! let dir = HookDir::new(
//!     Path::new("/home/user/app/.git"),
//!     Some(Path::new("/home/user/app")),
//!     Some(".githooks"),
//!     None,
//! );
//! assert_eq!(dir.dir(), Path::new("/home/user/app/.githooks"));
//! assert_eq!(Hook::CommitMsg.file_name(), "commit-msg");
//! assert!(dir.find(Hook::PreCommit).is_none());
//! ```

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{EnvVar, ProcessHost, ProgramInput, ProgramOutcome, ProgramSpec};
use nurt_contracts::stage::CancelToken;

/// Lines of standard error kept in the error of a failed hook.
const ERROR_SAMPLE_LINES: usize = 20;

/// A client-side hook run by the commands of the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hook {
    /// `pre-commit`: runs before the commit message is prepared.
    PreCommit,
    /// `prepare-commit-msg`: may change the message file before the
    /// message is checked.
    PrepareCommitMsg,
    /// `commit-msg`: checks or changes the message file.
    CommitMsg,
    /// `post-commit`: runs after the commit was recorded; its result is
    /// informational.
    PostCommit,
    /// `pre-push`: runs before references are sent to a remote.
    PrePush,
}

impl Hook {
    /// Returns the file name of the hook in the hooks directory.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::PreCommit => "pre-commit",
            Self::PrepareCommitMsg => "prepare-commit-msg",
            Self::CommitMsg => "commit-msg",
            Self::PostCommit => "post-commit",
            Self::PrePush => "pre-push",
        }
    }
}

impl fmt::Display for Hook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.file_name())
    }
}

/// Hooks directory of a repository and the directory hooks run in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookDir {
    dir: PathBuf,
    cwd: PathBuf,
}

impl HookDir {
    /// Locates the hooks directory.
    ///
    /// The arguments are the common repository directory, the root of the
    /// working tree (`None` for a bare repository), the value of
    /// `core.hooksPath` and the home directory used to expand a leading `~/`.
    #[must_use]
    pub fn new(
        common_dir: &Path,
        workdir: Option<&Path>,
        hooks_path: Option<&str>,
        home: Option<&Path>,
    ) -> Self {
        let cwd = workdir.unwrap_or(common_dir).to_path_buf();
        let dir = match hooks_path.filter(|value| !value.is_empty()) {
            None => common_dir.join("hooks"),
            Some(value) => match (value.strip_prefix("~/"), home) {
                (Some(rest), Some(home)) => home.join(rest),
                _ => cwd.join(value),
            },
        };
        Self { dir, cwd }
    }

    /// Returns the hooks directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Returns the directory hooks run in.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Returns the program of a hook, when it exists and is executable.
    #[must_use]
    pub fn find(&self, hook: Hook) -> Option<PathBuf> {
        let path = self.dir.join(hook.file_name());
        let metadata = std::fs::metadata(&path).ok()?;
        is_executable(&metadata).then_some(path)
    }

    /// Returns `true` when at least one of the hooks exists.
    #[must_use]
    pub fn has_any(&self, hooks: &[Hook]) -> bool {
        hooks.iter().any(|hook| self.find(*hook).is_some())
    }
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    const ANY_EXECUTE: u32 = 0o111;
    metadata.is_file() && metadata.permissions().mode() & ANY_EXECUTE != 0
}

#[cfg(not(unix))]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    metadata.is_file()
}

/// One call of a hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRequest {
    /// Hook to run.
    pub hook: Hook,
    /// Arguments, without the program.
    pub args: Vec<String>,
    /// Variables set for the hook on top of the environment of the console.
    pub env: Vec<EnvVar>,
    /// Standard input.
    pub stdin: ProgramInput,
    /// Time limit.
    pub timeout: Duration,
    /// Report files the hook changed under the directory it runs in.
    pub detect_changes: bool,
}

impl HookRequest {
    /// Creates a request with no arguments, no variables and empty input.
    #[must_use]
    pub fn new(hook: Hook, timeout: Duration) -> Self {
        Self {
            hook,
            args: Vec::new(),
            env: Vec::new(),
            stdin: ProgramInput::Null,
            timeout,
            detect_changes: false,
        }
    }
}

/// Completed run of a hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRun {
    /// Hook that ran.
    pub hook: Hook,
    /// Program file of the hook.
    pub program: PathBuf,
    /// Outcome reported by the process host.
    pub outcome: ProgramOutcome,
}

impl HookRun {
    /// Returns `true` when the hook exited with code 0.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.outcome.exit_code == Some(0) && !self.outcome.timed_out && !self.outcome.cancelled
    }

    /// Returns the run when the hook succeeded, otherwise the error that
    /// stops the command.
    ///
    /// The fix, when given, is a complete call of the command that skips the
    /// hooks.
    ///
    /// # Errors
    ///
    /// Fails with `E_TIMEOUT` when the time limit was reached, `E_CANCELLED`
    /// when the operation was cancelled and `E_GIT_HOOK` when the hook ended
    /// with another exit code or by a signal.
    pub fn into_result(self, fix: Option<String>) -> Result<Self> {
        if self.succeeded() {
            return Ok(self);
        }
        let hook = self.hook;
        let mut error = if self.outcome.timed_out {
            Error::new(
                ErrorCode::Timeout,
                format!("the {hook} hook exceeded its time limit; raise `git.hooks.timeout` or skip the hooks"),
            )
        } else if self.outcome.cancelled {
            Error::new(ErrorCode::Cancelled, format!("the {hook} hook was stopped by cancellation"))
        } else {
            let ending = match (self.outcome.exit_code, self.outcome.signal) {
                (Some(code), _) => format!("exited with code {code}"),
                (None, Some(signal)) => format!("was ended by signal {signal}"),
                (None, None) => "ended abnormally".to_owned(),
            };
            Error::new(
                ErrorCode::GitHook,
                format!("the {hook} hook {ending}; correct the reported problem or skip the hooks"),
            )
            .with_problem(hook.file_name())
        };
        let stderr = String::from_utf8_lossy(&self.outcome.stderr);
        let lines: Vec<&str> = stderr.lines().collect();
        for line in &lines[lines.len().saturating_sub(ERROR_SAMPLE_LINES)..] {
            error = error.with_sample(*line);
        }
        if let Some(fix) = fix {
            error = error.with_fix(fix);
        }
        Err(error.with_detail(
            "hook",
            serde_json::Value::String(self.program.display().to_string()),
        ))
    }
}

/// Runs a hook when it exists.
///
/// Returns `None` when the hook is missing or not executable. A non-zero exit
/// code is an outcome, not an error; [`HookRun::into_result`] turns it into
/// one.
///
/// # Errors
///
/// Fails only when the program cannot be started (`E_SPAWN`,
/// `E_PROGRAM_NOT_FOUND` or `E_PERMISSION` from the process host).
pub fn run(
    host: &dyn ProcessHost,
    dir: &HookDir,
    request: HookRequest,
    cancel: &CancelToken,
) -> Result<Option<HookRun>> {
    let Some(program) = dir.find(request.hook) else {
        return Ok(None);
    };
    let timeout_ms = u64::try_from(request.timeout.as_millis()).unwrap_or(u64::MAX);
    let spec = ProgramSpec {
        program: program.display().to_string(),
        args: request.args,
        cwd: dir.cwd.clone(),
        env: request.env,
        inherit_env: true,
        stdin: request.stdin,
        timeout_ms: Some(timeout_ms),
        output_limit: None,
        detect_changes: request.detect_changes,
    };
    let outcome = host.run_program(&spec, cancel)?;
    Ok(Some(HookRun {
        hook: request.hook,
        program,
        outcome,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nurt_contracts::services::{ServiceChild, ServiceSpawnSpec};

    use super::*;

    struct ScriptedHost {
        exit_code: Option<i32>,
        timed_out: bool,
        seen: Mutex<Vec<ProgramSpec>>,
    }

    impl ScriptedHost {
        fn exiting(code: i32) -> Self {
            Self {
                exit_code: Some(code),
                timed_out: false,
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl ProcessHost for ScriptedHost {
        fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
            Ok(PathBuf::from(program))
        }

        fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
            self.seen.lock().unwrap().push(spec.clone());
            Ok(ProgramOutcome {
                program_path: PathBuf::from(&spec.program),
                exit_code: self.exit_code,
                timed_out: self.timed_out,
                stderr: b"line one\nline two\n".to_vec(),
                ..ProgramOutcome::default()
            })
        }

        fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
            Err(Error::new(ErrorCode::Unsupported, "services are not used by hooks"))
        }
    }

    fn write_hook(dir: &Path, hook: Hook, executable: bool) {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(hook.file_name());
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        #[cfg(not(unix))]
        let _ = executable;
    }

    #[test]
    fn default_directory_is_hooks_in_the_common_directory() {
        let dir = HookDir::new(Path::new("/r/.git"), Some(Path::new("/r")), None, None);
        assert_eq!(dir.dir(), Path::new("/r/.git/hooks"));
        assert_eq!(dir.cwd(), Path::new("/r"));
        let bare = HookDir::new(Path::new("/srv/r.git"), None, Some(""), None);
        assert_eq!(bare.dir(), Path::new("/srv/r.git/hooks"));
        assert_eq!(bare.cwd(), Path::new("/srv/r.git"));
    }

    #[test]
    fn hooks_path_expands_home_and_keeps_absolute_paths() {
        let home = Path::new("/home/user");
        let dir = HookDir::new(Path::new("/r/.git"), Some(Path::new("/r")), Some("~/hooks"), Some(home));
        assert_eq!(dir.dir(), Path::new("/home/user/hooks"));
        let absolute = HookDir::new(Path::new("/r/.git"), Some(Path::new("/r")), Some("/opt/hooks"), None);
        assert_eq!(absolute.dir(), Path::new("/opt/hooks"));
    }

    #[test]
    fn missing_hook_is_not_run() {
        let temp = tempfile::tempdir().unwrap();
        let dir = HookDir::new(temp.path(), Some(temp.path()), None, None);
        let host = ScriptedHost::exiting(0);
        let request = HookRequest::new(Hook::PreCommit, Duration::from_secs(5));
        let ran = run(&host, &dir, request, &CancelToken::new()).unwrap();
        assert!(ran.is_none());
        assert!(host.seen.lock().unwrap().is_empty());
        assert!(!dir.has_any(&[Hook::PreCommit, Hook::CommitMsg]));
    }

    #[cfg(unix)]
    #[test]
    fn hook_without_execute_permission_is_skipped() {
        let temp = tempfile::tempdir().unwrap();
        write_hook(&temp.path().join("hooks"), Hook::CommitMsg, false);
        let dir = HookDir::new(temp.path(), Some(temp.path()), None, None);
        assert!(dir.find(Hook::CommitMsg).is_none());
    }

    #[test]
    fn executable_hook_runs_with_arguments_environment_and_limit() {
        let temp = tempfile::tempdir().unwrap();
        write_hook(&temp.path().join("hooks"), Hook::CommitMsg, true);
        let dir = HookDir::new(temp.path(), Some(temp.path()), None, None);
        let host = ScriptedHost::exiting(0);
        let mut request = HookRequest::new(Hook::CommitMsg, Duration::from_millis(1500));
        request.args = vec!["/tmp/msg".to_owned()];
        request.env = vec![EnvVar {
            name: "GIT_INDEX_FILE".to_owned(),
            value: Some("/r/.git/index".to_owned()),
        }];
        let ran = run(&host, &dir, request, &CancelToken::new()).unwrap().unwrap();
        assert!(ran.succeeded());
        let seen = host.seen.lock().unwrap();
        assert_eq!(seen[0].args, vec!["/tmp/msg".to_owned()]);
        assert_eq!(seen[0].timeout_ms, Some(1500));
        assert_eq!(seen[0].cwd, temp.path());
        assert_eq!(seen[0].env[0].name, "GIT_INDEX_FILE");
        assert!(ran.into_result(None).is_ok());
    }

    #[test]
    fn failing_hook_becomes_git_hook_error_with_fix_and_output() {
        let run = HookRun {
            hook: Hook::PreCommit,
            program: PathBuf::from("/r/.git/hooks/pre-commit"),
            outcome: ProgramOutcome {
                exit_code: Some(1),
                stderr: b"lint failed\n".to_vec(),
                ..ProgramOutcome::default()
            },
        };
        let error = run
            .into_result(Some("git.commit \"/r\" message: \"x\" hooks: false".to_owned()))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitHook);
        let data = error.data();
        assert_eq!(data.problem.as_deref(), Some("pre-commit"));
        assert_eq!(data.sample, vec!["lint failed".to_owned()]);
        assert!(data.fix[0].ends_with("hooks: false"));
        assert!(data.message.starts_with("the pre-commit hook exited with code 1"));
    }

    #[test]
    fn timed_out_and_cancelled_hooks_map_to_their_codes() {
        let base = HookRun {
            hook: Hook::CommitMsg,
            program: PathBuf::from("h"),
            outcome: ProgramOutcome {
                timed_out: true,
                ..ProgramOutcome::default()
            },
        };
        assert_eq!(base.clone().into_result(None).unwrap_err().code(), ErrorCode::Timeout);
        let mut cancelled = base;
        cancelled.outcome.timed_out = false;
        cancelled.outcome.cancelled = true;
        assert_eq!(cancelled.into_result(None).unwrap_err().code(), ErrorCode::Cancelled);
    }

    #[test]
    fn hook_ended_by_signal_reports_the_signal() {
        let run = HookRun {
            hook: Hook::PostCommit,
            program: PathBuf::from("h"),
            outcome: ProgramOutcome {
                signal: Some(9),
                ..ProgramOutcome::default()
            },
        };
        let error = run.into_result(None).unwrap_err();
        assert!(error.data().message.contains("signal 9"));
        assert_eq!(Hook::PrePush.to_string(), "pre-push");
    }

    #[test]
    fn scripted_host_failure_is_returned_as_outcome() {
        let temp = tempfile::tempdir().unwrap();
        write_hook(&temp.path().join("hooks"), Hook::PreCommit, true);
        let dir = HookDir::new(temp.path(), Some(temp.path()), None, None);
        let host = ScriptedHost {
            exit_code: None,
            timed_out: true,
            seen: Mutex::new(Vec::new()),
        };
        let request = HookRequest::new(Hook::PreCommit, Duration::from_secs(1));
        let ran = run(&host, &dir, request, &CancelToken::new()).unwrap().unwrap();
        assert!(!ran.succeeded());
        assert_eq!(ran.into_result(None).unwrap_err().code(), ErrorCode::Timeout);
        assert!(host.resolve_program("x", temp.path()).is_ok());
        let spawn = ServiceSpawnSpec {
            program: "x".to_owned(),
            args: Vec::new(),
            cwd: temp.path().to_path_buf(),
            env: Vec::new(),
            inherit_env: true,
            temp_dir: None,
        };
        assert!(host.spawn_service(&spawn).is_err());
    }
}

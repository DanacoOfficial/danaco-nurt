// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Program execution for the Nurt side of a scenario.
//!
//! [`ScenarioProcessHost`] implements the process service of the contracts
//! for the repository hooks started by the Nurt commands. A program runs with
//! the environment of the scenario, so that a hook sees the same variables on
//! both sides.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{
    ProcessHost, ProgramInput, ProgramOutcome, ProgramSpec, ServiceChild, ServiceSpawnSpec,
};
use nurt_contracts::stage::CancelToken;

/// Time limit of a program that has none of its own.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Interval at which a running program is checked for exit and cancellation.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Bytes kept from each output stream when the program has no limit.
const DEFAULT_OUTPUT_LIMIT: u64 = 1 << 20;

/// Process service that starts programs with the environment of a scenario.
#[derive(Debug, Clone)]
pub struct ScenarioProcessHost {
    vars: Vec<(OsString, OsString)>,
}

impl ScenarioProcessHost {
    /// Service whose programs start from the variables `vars`.
    #[must_use]
    pub fn new(vars: Vec<(OsString, OsString)>) -> Self {
        Self { vars }
    }

    fn search_path(&self) -> Vec<PathBuf> {
        self.vars
            .iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| std::env::split_paths(value).collect())
            .unwrap_or_default()
    }

    fn command(&self, spec: &ProgramSpec, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.args(&spec.args).current_dir(&spec.cwd).env_clear();
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        if spec.inherit_env {
            command.envs(self.vars.iter().map(|(name, value)| (name, value)));
        }
        for var in &spec.env {
            match &var.value {
                Some(value) => command.env(&var.name, value),
                None => command.env_remove(&var.name),
            };
        }
        command
    }
}

impl ProcessHost for ScenarioProcessHost {
    fn resolve_program(&self, program: &str, cwd: &Path) -> Result<PathBuf> {
        let named = Path::new(program);
        if named.components().count() > 1 || named.is_absolute() {
            let path = cwd.join(named);
            return if path.is_file() {
                Ok(path)
            } else {
                Err(not_found(program))
            };
        }
        self.search_path()
            .into_iter()
            .map(|dir| dir.join(program))
            .find(|path| path.is_file())
            .ok_or_else(|| not_found(program))
    }

    fn run_program(&self, spec: &ProgramSpec, cancel: &CancelToken) -> Result<ProgramOutcome> {
        let program = self.resolve_program(&spec.program, &spec.cwd)?;
        let started = Instant::now();
        let mut child = self
            .command(spec, &program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| spawn_error(&spec.program, &source))?;
        let input = match &spec.stdin {
            ProgramInput::Bytes(bytes) => bytes.clone(),
            _ => Vec::new(),
        };
        let mut stdin = child.stdin.take();
        let writer = std::thread::spawn(move || {
            if let Some(pipe) = stdin.as_mut() {
                let _ = pipe.write_all(&input);
            }
        });
        let limit = spec.output_limit.unwrap_or(DEFAULT_OUTPUT_LIMIT);
        let stdout = collect(child.stdout.take(), limit);
        let stderr = collect(child.stderr.take(), limit);
        let timeout = spec
            .timeout_ms
            .map_or(DEFAULT_TIMEOUT, Duration::from_millis);
        let (status, timed_out, cancelled) = loop {
            match child.try_wait() {
                Ok(Some(status)) => break (Some(status), false, false),
                Ok(None) => {}
                Err(source) => return Err(spawn_error(&spec.program, &source)),
            }
            let expired = started.elapsed() >= timeout;
            if expired || cancel.is_cancelled() {
                kill_tree(&mut child);
                let _ = child.wait();
                break (None, expired, !expired);
            }
            std::thread::sleep(POLL_INTERVAL);
        };
        let _ = writer.join();
        let (stdout, stdout_truncated) = stdout.join().unwrap_or_default();
        let (stderr, stderr_truncated) = stderr.join().unwrap_or_default();
        Ok(ProgramOutcome {
            program_path: program,
            exit_code: status.and_then(|status| status.code()),
            signal: status.and_then(signal_of),
            timed_out,
            cancelled,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            ..ProgramOutcome::default()
        })
    }

    fn spawn_service(&self, _spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "long-running services are not available in the scenario runner",
        ))
    }
}

fn collect<R: Read + Send + 'static>(
    pipe: Option<R>,
    limit: u64,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let Some(mut pipe) = pipe else {
            return (kept, false);
        };
        let _ = pipe.by_ref().take(limit).read_to_end(&mut kept);
        // The rest is drained without being kept, so that a program with a
        // large output neither blocks on a full pipe nor fills the memory.
        let drained = std::io::copy(&mut pipe, &mut std::io::sink()).unwrap_or(0);
        (kept, drained > 0)
    })
}

/// Kills the program and the processes it started. Those inherit the pipes of
/// the program, so that the readers of its output would wait for them if only
/// the program were killed.
#[cfg(unix)]
fn kill_tree(child: &mut std::process::Child) {
    let group = rustix::process::Pid::from_child(child);
    let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

#[cfg(unix)]
fn signal_of(status: std::process::ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(&status)
}

#[cfg(not(unix))]
fn signal_of(_status: std::process::ExitStatus) -> Option<i32> {
    None
}

fn not_found(program: &str) -> Error {
    Error::new(
        ErrorCode::Spawn,
        format!("program `{program}` not found, check the path of the program"),
    )
}

fn spawn_error(program: &str, source: &std::io::Error) -> Error {
    Error::new(
        ErrorCode::Spawn,
        format!("cannot start `{program}`: {source}"),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn host() -> ScenarioProcessHost {
        ScenarioProcessHost::new(vec![
            (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
            (OsString::from("SCENARIO_VAR"), OsString::from("kept")),
        ])
    }

    fn spec(dir: &Path, script: &str) -> ProgramSpec {
        ProgramSpec::new("sh", vec!["-c".to_owned(), script.to_owned()], dir)
    }

    fn token() -> CancelToken {
        CancelToken::default()
    }

    #[test]
    fn runs_a_program_with_the_scenario_environment() {
        let dir = tempfile::tempdir().unwrap();
        let mut program = spec(dir.path(), "printf '%s' \"$SCENARIO_VAR\"; exit 3");
        program.inherit_env = true;
        let outcome = host().run_program(&program, &token()).unwrap();
        assert_eq!(outcome.exit_code, Some(3));
        assert_eq!(outcome.stdout, b"kept");
    }

    #[test]
    fn clean_environment_hides_the_scenario_variables() {
        let dir = tempfile::tempdir().unwrap();
        let mut program = spec(dir.path(), "printf '%s' \"${SCENARIO_VAR-unset}\"");
        program.inherit_env = false;
        program.env = vec![nurt_contracts::services::EnvVar {
            name: "PATH".to_owned(),
            value: Some("/usr/bin:/bin".to_owned()),
        }];
        let outcome = host().run_program(&program, &token()).unwrap();
        assert_eq!(outcome.stdout, b"unset");
    }

    #[test]
    fn feeds_standard_input_and_captures_standard_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut program = spec(dir.path(), "cat; echo oops >&2");
        program.stdin = ProgramInput::Bytes(b"input".to_vec());
        program.inherit_env = true;
        let outcome = host().run_program(&program, &token()).unwrap();
        assert_eq!(outcome.stdout, b"input");
        assert_eq!(outcome.stderr, b"oops\n");
    }

    #[test]
    fn stops_the_processes_a_program_started_when_its_time_limit_passes() {
        let dir = tempfile::tempdir().unwrap();
        let mut program = spec(dir.path(), "sleep 30 & wait");
        program.inherit_env = true;
        program.timeout_ms = Some(100);
        let started = Instant::now();
        let outcome = host().run_program(&program, &token()).unwrap();
        assert!(outcome.timed_out);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn stops_a_program_that_exceeds_its_time_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut program = spec(dir.path(), "exec sleep 30");
        program.inherit_env = true;
        program.timeout_ms = Some(50);
        let outcome = host().run_program(&program, &token()).unwrap();
        assert!(outcome.timed_out);
        assert_eq!(outcome.exit_code, None);
    }

    #[test]
    fn truncates_output_beyond_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut program = spec(dir.path(), "printf 0123456789");
        program.inherit_env = true;
        program.output_limit = Some(4);
        let outcome = host().run_program(&program, &token()).unwrap();
        assert_eq!(outcome.stdout, b"0123");
        assert!(outcome.stdout_truncated);
    }

    #[test]
    fn output_within_or_without_the_limit_is_not_truncated_or_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut program = spec(dir.path(), "printf 0123");
        program.inherit_env = true;
        program.output_limit = Some(4);
        let outcome = host().run_program(&program, &token()).unwrap();
        assert_eq!(outcome.stdout, b"0123");
        assert!(!outcome.stdout_truncated);
        let mut program = spec(dir.path(), "printf x; printf yy >&2");
        program.inherit_env = true;
        program.output_limit = Some(0);
        let outcome = host().run_program(&program, &token()).unwrap();
        assert!(outcome.stdout.is_empty() && outcome.stdout_truncated);
        assert!(outcome.stderr.is_empty() && outcome.stderr_truncated);
    }

    #[test]
    fn large_output_beyond_the_limit_is_drained_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let mut program = spec(dir.path(), "head -c 1048576 /dev/zero | tr '\\0' x");
        program.inherit_env = true;
        program.output_limit = Some(16);
        let outcome = host().run_program(&program, &token()).unwrap();
        assert_eq!(outcome.stdout.len(), 16);
        assert!(outcome.stdout_truncated);
        assert_eq!(outcome.exit_code, Some(0));
    }

    #[test]
    fn resolves_relative_paths_against_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hooks").join("run");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let found = host().resolve_program("hooks/run", dir.path()).unwrap();
        assert_eq!(found, script);
        let missing = host().resolve_program("no-such-program", dir.path());
        assert_eq!(missing.unwrap_err().code, ErrorCode::Spawn);
    }

    #[test]
    fn refuses_long_running_services() {
        let spec = ServiceSpawnSpec {
            program: "sh".to_owned(),
            args: Vec::new(),
            cwd: PathBuf::from("."),
            env: Vec::new(),
            inherit_env: true,
            temp_dir: None,
        };
        let error = host().spawn_service(&spec).err().unwrap();
        assert_eq!(error.code, ErrorCode::Unsupported);
    }
}

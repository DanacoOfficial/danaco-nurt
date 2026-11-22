// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process host service for commands outside the run group.
//!
//! Programs are resolved, quoted and given their environment by the rules of
//! `run`. Programs run to completion go through process control, so they get
//! the same process group or job object, time limit and cancellation as a
//! run. Helper processes with piped standard streams are started directly in
//! their own process group, on Windows in their own job object.
//!
//! The service belongs to the console process: the inherited environment is
//! the environment of that process and the defaults are the defaults of the
//! `[run]` configuration table.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nurt_contracts::config::RunConfig;
use nurt_contracts::env::WallTime;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{KilledBy, Launcher};
use nurt_contracts::services::{
    ChildExit, EnvVar, ProcessHost, ProgramInput, ProgramOutcome, ProgramSpec, ServiceChild,
    ServiceSpawnSpec,
};
use nurt_contracts::stage::{CancelToken, OutputStream};

use crate::iface::{Components, OutputSink, Priority, ProcessLimits, StdinSource};
use crate::spawn::{self, ChildEnv, InheritEnv, Launch, Platform};

/// Bytes kept from each output stream when the specification gives no limit.
const DEFAULT_OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// Time to wait for the output streams to close after the program ended.
const OUTPUT_DRAIN: Duration = Duration::from_secs(2);
/// Interval between checks of a helper process while waiting for its end.
const SERVICE_POLL: Duration = Duration::from_millis(10);
/// Largest number of files compared by change detection.
const MAX_SCANNED_FILES: usize = 100_000;
/// Variables that name the temporary directory of a helper process.
const TEMP_VARIABLES: [&str; 3] = ["TMPDIR", "TEMP", "TMP"];

/// The process host service.
struct SystemProcessHost {
    components: Components,
}

/// Process host built on `components`.
pub(crate) fn process_host(components: Components) -> Arc<dyn ProcessHost> {
    Arc::new(SystemProcessHost { components })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn wall_now() -> WallTime {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    WallTime::from_unix_nanos(i128::try_from(nanos).unwrap_or(i128::MAX))
}

/// Environment of a program started by the service.
fn console_env(changes: &[EnvVar], inherit: bool) -> Result<ChildEnv> {
    let defaults = RunConfig::default().env_defaults;
    ChildEnv::build(
        Platform::current(),
        std::env::vars_os().collect(),
        if inherit {
            InheritEnv::All
        } else {
            InheritEnv::None
        },
        defaults
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        changes,
    )
}

fn command_of(program: &str, args: &[String]) -> Vec<String> {
    std::iter::once(program.to_owned())
        .chain(args.iter().cloned())
        .collect()
}

impl ProcessHost for SystemProcessHost {
    fn resolve_program(&self, program: &str, cwd: &Path) -> Result<PathBuf> {
        let env = console_env(&[], true)?;
        spawn::resolve(&[program.to_owned()], cwd, &env)
            .map(|resolved| resolved.path)
            .map_err(|error| {
                if error.code() == ErrorCode::ProgramNotFound {
                    Error::new(ErrorCode::ExternalMissing, error.data().message.clone())
                        .with_cause(error)
                } else {
                    error
                }
            })
    }

    fn run_program(&self, spec: &ProgramSpec, cancel: &CancelToken) -> Result<ProgramOutcome> {
        spawn::check_cwd(&spec.cwd)?;
        let env = console_env(&spec.env, spec.inherit_env)?;
        let launch =
            spawn::prepare_launch(&command_of(&spec.program, &spec.args), &spec.cwd, &env)?;
        let config = RunConfig::default();
        let limits = ProcessLimits {
            timeout: Some(
                spec.timeout_ms
                    .map_or(config.timeout.as_std(), Duration::from_millis),
            ),
            idle_timeout: None,
            kill_grace: config.kill_grace.as_std(),
            max_cpu: None,
            max_memory: None,
            priority: Priority::Normal,
        };
        let stdin = match &spec.stdin {
            ProgramInput::Bytes(bytes) => StdinSource::Bytes(bytes.clone()),
            _ => StdinSource::Null,
        };
        let before = spec.detect_changes.then(|| scan(&spec.cwd));
        let capture = Arc::new(Capture::new(
            spec.output_limit.unwrap_or(DEFAULT_OUTPUT_LIMIT),
        ));
        let request = launch.request(&spec.cwd, env, stdin, false, limits);
        let mut process = self.components.control.spawn(
            request,
            Arc::clone(&capture) as Arc<dyn OutputSink>,
            cancel.clone(),
        )?;
        let end = process.wait(None)?.ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "process control returned before the program ended",
            )
        })?;
        capture.wait_closed(OUTPUT_DRAIN);
        let changed_paths = before
            .map(|before| changed(&before, &scan(&spec.cwd)))
            .unwrap_or_default();
        let streams = capture.take();
        Ok(ProgramOutcome {
            program_path: launch.program,
            launcher: (launch.launcher == Launcher::Cmd).then(|| "cmd.exe".to_owned()),
            exit_code: end.exit_code,
            signal: end.signal_number,
            timed_out: end.killed_by == Some(KilledBy::Timeout),
            cancelled: end.killed_by == Some(KilledBy::Cancel),
            stdout: streams.stdout,
            stderr: streams.stderr,
            stdout_truncated: streams.stdout_truncated,
            stderr_truncated: streams.stderr_truncated,
            duration_ms: u64::try_from(
                end.finished.saturating_since(process.started()).as_millis(),
            )
            .unwrap_or(u64::MAX),
            changed_paths,
        })
    }

    fn spawn_service(&self, spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        spawn::check_cwd(&spec.cwd)?;
        let mut changes = spec.env.clone();
        if let Some(temp) = &spec.temp_dir {
            for name in TEMP_VARIABLES {
                changes.push(EnvVar {
                    name: name.to_owned(),
                    value: Some(temp.to_string_lossy().into_owned()),
                });
            }
        }
        let env = console_env(&changes, spec.inherit_env)?;
        let launch =
            spawn::prepare_launch(&command_of(&spec.program, &spec.args), &spec.cwd, &env)?;
        let mut command = Command::new(&launch.executable);
        add_arguments(&mut command, &launch);
        command
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(env.into_vars())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        separate_group(&mut command);
        let child = command.spawn().map_err(|source| {
            Error::new(
                ErrorCode::Spawn,
                format!(
                    "{} could not be started; check that it is a program for this system",
                    launch.program.display()
                ),
            )
            .with_cause(Error::new(ErrorCode::Io, source.to_string()))
        })?;
        #[cfg(windows)]
        let job = crate::os::ServiceJob::adopt(&child)?;
        Ok(Box::new(ServiceProcess {
            child,
            #[cfg(windows)]
            job,
            started: wall_now(),
            ended: false,
        }))
    }
}

#[cfg(windows)]
fn add_arguments(command: &mut Command, launch: &Launch) {
    use std::os::windows::process::CommandExt;
    match &launch.command_line {
        Some(line) => {
            let text = line.to_string_lossy();
            command.raw_arg(text.strip_prefix("cmd.exe ").unwrap_or(&text));
        }
        None => {
            command.args(&launch.args);
        }
    }
}

#[cfg(not(windows))]
fn add_arguments(command: &mut Command, launch: &Launch) {
    command.args(&launch.args);
}

#[cfg(unix)]
fn separate_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn separate_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// `CREATE_NEW_PROCESS_GROUP`: console interrupts of the console do not
    /// reach the helper.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    /// `CREATE_NO_WINDOW`: the helper gets no console window.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// `CREATE_SUSPENDED`: the helper runs only after it was placed in its
    /// job object, so every process it starts belongs to the job.
    const CREATE_SUSPENDED: u32 = 0x0000_0004;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_SUSPENDED);
}

#[cfg(not(any(unix, windows)))]
fn separate_group(_command: &mut Command) {}

/// Captured output of a program run to completion.
struct Streams {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

struct CaptureState {
    streams: Streams,
    open: u8,
    last: Option<Duration>,
}

/// Output sink that keeps up to a limit of bytes from each stream.
struct Capture {
    limit: usize,
    origin: Instant,
    state: Mutex<CaptureState>,
    closed: Condvar,
}

/// Number of output streams of a program.
const STREAMS: u8 = 2;

impl Capture {
    fn new(limit: u64) -> Self {
        Self {
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
            origin: Instant::now(),
            state: Mutex::new(CaptureState {
                streams: Streams {
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    stdout_truncated: false,
                    stderr_truncated: false,
                },
                open: STREAMS,
                last: None,
            }),
            closed: Condvar::new(),
        }
    }

    /// Waits up to `timeout` until both streams ended.
    fn wait_closed(&self, timeout: Duration) {
        let state = lock(&self.state);
        let _ = self
            .closed
            .wait_timeout_while(state, timeout, |state| state.open > 0);
    }

    fn take(&self) -> Streams {
        let mut state = lock(&self.state);
        Streams {
            stdout: std::mem::take(&mut state.streams.stdout),
            stderr: std::mem::take(&mut state.streams.stderr),
            stdout_truncated: state.streams.stdout_truncated,
            stderr_truncated: state.streams.stderr_truncated,
        }
    }
}

impl OutputSink for Capture {
    fn write(&self, stream: OutputStream, bytes: &[u8]) -> Result<()> {
        let mut state = lock(&self.state);
        state.last = Some(self.origin.elapsed());
        let streams = &mut state.streams;
        let (buffer, truncated) = match stream {
            OutputStream::Stdout => (&mut streams.stdout, &mut streams.stdout_truncated),
            OutputStream::Stderr => (&mut streams.stderr, &mut streams.stderr_truncated),
        };
        let room = self.limit.saturating_sub(buffer.len());
        buffer.extend_from_slice(&bytes[..bytes.len().min(room)]);
        if bytes.len() > room {
            *truncated = true;
        }
        Ok(())
    }

    fn finish(&self, _stream: OutputStream) -> Result<()> {
        let mut state = lock(&self.state);
        state.open = state.open.saturating_sub(1);
        self.closed.notify_all();
        Ok(())
    }

    fn last_activity(&self) -> Option<Duration> {
        lock(&self.state).last
    }
}

/// Size and modification time of a file.
type FileStamp = (u64, Option<SystemTime>);

/// Files below `root`, without the names of `run.track_ignore` and without
/// following symbolic links.
fn scan(root: &Path) -> BTreeMap<PathBuf, FileStamp> {
    let ignored = RunConfig::default().track_ignore;
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if files.len() >= MAX_SCANNED_FILES {
                return files;
            }
            let name = entry.file_name();
            if ignored
                .iter()
                .any(|ignored| OsString::from(ignored) == name)
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                files.insert(entry.path(), (metadata.len(), metadata.modified().ok()));
            }
        }
    }
    files
}

/// Files created, changed or removed between two scans, in path order.
fn changed(
    before: &BTreeMap<PathBuf, FileStamp>,
    after: &BTreeMap<PathBuf, FileStamp>,
) -> Vec<PathBuf> {
    let mut paths: BTreeSet<&PathBuf> = after
        .iter()
        .filter(|(path, stamp)| before.get(*path) != Some(stamp))
        .map(|(path, _)| path)
        .collect();
    paths.extend(before.keys().filter(|path| !after.contains_key(*path)));
    paths.into_iter().cloned().collect()
}

/// Helper process started by [`ProcessHost::spawn_service`].
struct ServiceProcess {
    child: Child,
    /// Job object holding the helper and every process it starts.
    #[cfg(windows)]
    job: crate::os::ServiceJob,
    started: WallTime,
    ended: bool,
}

fn io_error(source: &std::io::Error) -> Error {
    Error::new(ErrorCode::Io, source.to_string())
}

fn child_exit(status: ExitStatus) -> ChildExit {
    ChildExit {
        exit_code: status.code(),
        signal: exit_signal(status),
    }
}

#[cfg(unix)]
fn exit_signal(status: ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_status: ExitStatus) -> Option<i32> {
    None
}

impl ServiceProcess {
    #[cfg(unix)]
    fn stop_group(&self) {
        let pid = rustix::process::Pid::from_child(&self.child);
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }

    #[cfg(windows)]
    fn stop_group(&self) {
        let _ = self.job.kill();
    }

    #[cfg(not(any(unix, windows)))]
    fn stop_group(&self) {}
}

impl ServiceChild for ServiceProcess {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn started(&self) -> WallTime {
        self.started
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        self.child
            .stdin
            .take()
            .map(|stdin| Box::new(stdin) as Box<dyn Write + Send>)
    }

    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.child
            .stdout
            .take()
            .map(|stdout| Box::new(stdout) as Box<dyn Read + Send>)
    }

    fn take_stderr(&mut self) -> Option<Box<dyn Read + Send>> {
        self.child
            .stderr
            .take()
            .map(|stderr| Box::new(stderr) as Box<dyn Read + Send>)
    }

    fn try_wait(&mut self) -> Result<Option<ChildExit>> {
        let status = self.child.try_wait().map_err(|source| io_error(&source))?;
        if status.is_some() {
            self.ended = true;
        }
        Ok(status.map(child_exit))
    }

    fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<ChildExit>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(exit) = self.try_wait()? {
                return Ok(Some(exit));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            std::thread::sleep(SERVICE_POLL.min(deadline - now));
        }
    }

    fn kill_tree(&mut self) -> Result<()> {
        if !self.ended {
            self.stop_group();
            if let Err(source) = self.child.kill()
                && source.kind() != std::io::ErrorKind::InvalidInput
            {
                return Err(io_error(&source));
            }
        }
        self.child.wait().map_err(|source| io_error(&source))?;
        self.ended = true;
        Ok(())
    }
}

impl Drop for ServiceProcess {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.kill_tree();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::services::{ProcessHost, ProgramInput, ProgramSpec, ServiceSpawnSpec};
    use nurt_contracts::stage::CancelToken;

    use super::{SystemProcessHost, changed, scan};
    use crate::iface::Components;
    use crate::spawn::testing::{StdControl, fixture_path};

    fn host() -> SystemProcessHost {
        SystemProcessHost {
            components: Components {
                control: Arc::new(StdControl),
                spool: crate::output::spool_factory(),
                tracker: crate::track::tracker(),
            },
        }
    }

    fn fixture() -> String {
        fixture_path().to_string_lossy().into_owned()
    }

    fn spec(args: &[&str], cwd: &Path) -> ProgramSpec {
        ProgramSpec::new(
            fixture(),
            args.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd,
        )
    }

    #[test]
    fn runs_a_program_to_completion_and_captures_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = host()
            .run_program(
                &spec(&["print-argv", "a b", "c"], dir.path()),
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(outcome.exit_code, Some(0));
        let argv: Vec<String> = serde_json::from_slice(&outcome.stdout).unwrap();
        assert_eq!(argv, vec!["a b".to_owned(), "c".to_owned()]);
        assert_eq!(outcome.program_path, fixture_path());
        assert!(outcome.launcher.is_none());
    }

    #[test]
    fn keeps_output_up_to_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = spec(&["print-argv", "0123456789"], dir.path());
        spec.output_limit = Some(4);
        let outcome = host().run_program(&spec, &CancelToken::new()).unwrap();
        assert_eq!(outcome.stdout.len(), 4);
        assert!(outcome.stdout_truncated);
        assert!(!outcome.stderr_truncated);
    }

    #[test]
    fn passes_input_bytes_and_reports_the_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = spec(&["count-input"], dir.path());
        spec.stdin = ProgramInput::Bytes(b"abc".to_vec());
        let outcome = host().run_program(&spec, &CancelToken::new()).unwrap();
        assert_eq!(String::from_utf8_lossy(&outcome.stdout).trim(), "3");
        let exit = host()
            .run_program(
                &self::spec(&["exit-with", "7"], dir.path()),
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(exit.exit_code, Some(7));
    }

    #[test]
    fn reports_files_changed_under_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("kept.txt"), "same").unwrap();
        std::fs::write(dir.path().join("gone.txt"), "old").unwrap();
        let mut spec = spec(&["write-file", "new.txt", "content"], dir.path());
        spec.detect_changes = true;
        let before = scan(dir.path());
        std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
        let outcome = host().run_program(&spec, &CancelToken::new()).unwrap();
        assert_eq!(outcome.changed_paths, vec![dir.path().join("new.txt")]);
        assert_eq!(
            changed(&before, &scan(dir.path())),
            vec![dir.path().join("gone.txt"), dir.path().join("new.txt")]
        );
    }

    #[test]
    fn a_missing_program_is_reported_as_an_external_program_to_install() {
        let dir = tempfile::tempdir().unwrap();
        let error = host()
            .resolve_program("no-such-program-for-tests", dir.path())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ExternalMissing);
        let cause = error.data().cause.as_ref().map(|cause| cause.code());
        assert_eq!(cause, Some(ErrorCode::ProgramNotFound));
        assert_eq!(
            host().resolve_program(&fixture(), dir.path()).unwrap(),
            fixture_path()
        );
    }

    #[test]
    fn a_missing_working_directory_is_a_spawn_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let error = host()
            .run_program(&spec(&["print-argv"], &missing), &CancelToken::new())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Spawn);
    }

    #[test]
    fn a_helper_process_talks_through_its_streams() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = host()
            .spawn_service(&ServiceSpawnSpec {
                program: fixture(),
                args: vec!["count-input".to_owned()],
                cwd: dir.path().to_path_buf(),
                env: Vec::new(),
                inherit_env: true,
                temp_dir: Some(dir.path().to_path_buf()),
            })
            .unwrap();
        let mut stdin = child.take_stdin().unwrap();
        stdin.write_all(b"hello").unwrap();
        drop(stdin);
        let mut text = String::new();
        child
            .take_stdout()
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text.trim(), "5");
        let exit = child.wait_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(exit.and_then(|exit| exit.exit_code), Some(0));
        assert!(child.take_stdin().is_none());
    }

    #[test]
    fn kill_tree_stops_a_running_helper_process() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = host()
            .spawn_service(&ServiceSpawnSpec {
                program: fixture(),
                args: vec!["exit-with".to_owned(), "0".to_owned(), "30000".to_owned()],
                cwd: dir.path().to_path_buf(),
                env: Vec::new(),
                inherit_env: true,
                temp_dir: None,
            })
            .unwrap();
        assert!(child.try_wait().unwrap().is_none());
        child.kill_tree().unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }
}

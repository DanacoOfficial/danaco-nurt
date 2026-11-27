// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-crashfs::runner
// Responsibility:   Process-level crash runner.
// Dependencies:     nurt_fs, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Process-level crash runner.
//!
//! [`ProcessRunner`] starts a child process that performs a scenario on a
//! real file system, waits until the child reports that the scenario has
//! started, waits a further delay and kills the child without warning:
//! `SIGKILL` on Linux and macOS, `TerminateProcess` on Windows, both through
//! [`std::process::Child::kill`]. The parent then recovers and checks the
//! state left on disk. Only the child itself is killed; scenarios run by the
//! runner start no processes of their own.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use nurt_fs::failpoint::SplitMix64;
use serde::{Deserialize, Serialize};

/// Byte sequence a child writes to its standard output when the scenario
/// starts. The separators keep it distinct from test harness output.
pub const READY_MARKER: &[u8] = b"\x1enurt-crash-ready\x1e";

/// Writes [`READY_MARKER`] to standard output and flushes it. The child
/// calls it when the scenario starts.
///
/// # Errors
///
/// Fails when standard output cannot be written.
pub fn signal_ready() -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(READY_MARKER)?;
    stdout.flush()
}

/// Longest wait for [`READY_MARKER`] by default.
pub const DEFAULT_MARKER_TIMEOUT: Duration = Duration::from_secs(60);

/// Most output bytes a child may write before [`READY_MARKER`].
const MAX_OUTPUT_BEFORE_MARKER: usize = 1 << 20;

/// Kill delay for a point: uniform in `0..max`, derived from `seed`.
#[must_use]
pub fn kill_delay(seed: u64, max: Duration) -> Duration {
    let micros = u64::try_from(max.as_micros()).unwrap_or(u64::MAX);
    Duration::from_micros(SplitMix64::new(seed).below(micros))
}

/// Result of one killed run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KillOutcome {
    /// Whether the child was still running and was killed.
    pub killed: bool,
    /// Delay between the ready marker and the kill.
    pub delay: Duration,
    /// Exit code when the child ended by itself.
    pub exit_code: Option<i32>,
}

/// Command line of the child process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessRunner {
    program: PathBuf,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    marker_timeout: Duration,
}

impl ProcessRunner {
    /// Runner of `program` with `args`.
    pub fn new<I, S>(program: impl AsRef<Path>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Self {
            program: program.as_ref().to_path_buf(),
            args: args
                .into_iter()
                .map(|arg| arg.as_ref().to_os_string())
                .collect(),
            envs: Vec::new(),
            marker_timeout: DEFAULT_MARKER_TIMEOUT,
        }
    }

    /// Sets the longest wait for the ready marker; a child that stays silent
    /// longer is killed and the run fails with [`io::ErrorKind::TimedOut`].
    #[must_use]
    pub fn marker_timeout(mut self, timeout: Duration) -> Self {
        self.marker_timeout = timeout;
        self
    }

    /// Adds an environment variable of the child.
    #[must_use]
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.envs
            .push((key.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    fn spawn(&self) -> io::Result<Child> {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .envs(self.envs.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command.spawn()
    }

    /// Runs the child, waits for [`READY_MARKER`], waits `delay` and kills
    /// the child if it is still running.
    ///
    /// # Errors
    ///
    /// Fails when the child cannot be started, ends before writing the
    /// marker, stays silent for the marker timeout, writes more than one
    /// mebibyte before the marker, or cannot be killed or waited for.
    pub fn run_and_kill(&self, delay: Duration) -> io::Result<KillOutcome> {
        let mut child = self.spawn()?;
        if let Err(err) = wait_for_marker(&mut child, self.marker_timeout) {
            child.kill().ok();
            child.wait().ok();
            return Err(err);
        }
        thread::sleep(delay);
        if let Some(status) = child.try_wait()? {
            return Ok(KillOutcome {
                killed: false,
                delay,
                exit_code: status.code(),
            });
        }
        child.kill()?;
        child.wait()?;
        Ok(KillOutcome {
            killed: true,
            delay,
            exit_code: None,
        })
    }

    /// Runs the child to completion.
    ///
    /// # Errors
    ///
    /// Fails when the child cannot be started or waited for.
    pub fn run_to_end(&self) -> io::Result<ExitStatus> {
        let mut child = self.spawn()?;
        if let Some(mut stdout) = child.stdout.take() {
            io::copy(&mut stdout, &mut io::sink())?;
        }
        child.wait()
    }
}

/// Waits until the child wrote [`READY_MARKER`]. A thread reads the output, so
/// the wait ends at the timeout also while the child writes nothing; after the
/// marker the thread keeps discarding the output so that the child never
/// blocks on a full pipe.
fn wait_for_marker(child: &mut Child, timeout: Duration) -> io::Result<()> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("child output is not captured"))?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let found = find_marker(&mut stdout);
        let ready = found.is_ok();
        let _ = sender.send(found);
        if ready {
            let _ = io::copy(&mut stdout, &mut io::sink());
        }
    });
    match receiver.recv_timeout(timeout) {
        Ok(found) => found,
        Err(RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "child did not start the scenario in time",
        )),
        Err(RecvTimeoutError::Disconnected) => Err(io::Error::other("output reader ended")),
    }
}

/// Reads `stdout` until [`READY_MARKER`] appeared, keeping only the bytes that
/// can still be the start of the marker.
fn find_marker(stdout: &mut impl Read) -> io::Result<()> {
    let mut window: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut total = 0_usize;
    loop {
        let read = match stdout.read(&mut chunk) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "child ended before the scenario started",
                ));
            }
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        window.extend_from_slice(&chunk[..read]);
        if window
            .windows(READY_MARKER.len())
            .any(|part| part == READY_MARKER)
        {
            return Ok(());
        }
        total += read;
        if total > MAX_OUTPUT_BEFORE_MARKER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "child wrote too much output before the scenario started",
            ));
        }
        let keep = READY_MARKER.len() - 1;
        if window.len() > keep {
            window.drain(..window.len() - keep);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_delays_are_deterministic_and_below_the_maximum() {
        let max = Duration::from_millis(50);
        for seed in 0..100 {
            let delay = kill_delay(seed, max);
            assert!(delay < max);
            assert_eq!(delay, kill_delay(seed, max));
        }
        assert_eq!(kill_delay(1, Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn the_ready_marker_is_distinct_from_ordinary_output() {
        assert!(READY_MARKER.starts_with(b"\x1e"));
        assert!(READY_MARKER.ends_with(b"\x1e"));
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_ends_before_the_marker_is_an_error() {
        let runner = ProcessRunner::new("sh", ["-c", "exit 0"]);
        let error = runner.run_and_kill(Duration::ZERO).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[cfg(unix)]
    #[test]
    fn a_silent_child_ends_the_wait_at_the_timeout() {
        let runner = ProcessRunner::new("sh", ["-c", "exec sleep 30"])
            .marker_timeout(Duration::from_millis(200));
        let error = runner.run_and_kill(Duration::ZERO).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_floods_the_output_without_the_marker_is_refused() {
        let runner = ProcessRunner::new(
            "sh",
            [
                "-c",
                "while :; do echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; done",
            ],
        );
        let error = runner.run_and_kill(Duration::ZERO).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn the_marker_is_found_across_chunk_borders() {
        struct Pieces(Vec<Vec<u8>>);
        impl Read for Pieces {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let piece = self.0.remove(0);
                buffer[..piece.len()].copy_from_slice(&piece);
                Ok(piece.len())
            }
        }
        let (head, tail) = READY_MARKER.split_at(5);
        let mut source = Pieces(vec![b"noise".to_vec(), head.to_vec(), tail.to_vec()]);
        assert!(find_marker(&mut source).is_ok());
        let mut partial = Pieces(vec![head.to_vec()]);
        assert_eq!(
            find_marker(&mut partial).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_running_child_is_killed_after_the_marker() {
        let runner = ProcessRunner::new(
            "sh",
            ["-c", "printf '\\036nurt-crash-ready\\036'; exec sleep 30"],
        );
        let outcome = runner.run_and_kill(Duration::ZERO).unwrap();
        assert!(outcome.killed);
        assert_eq!(outcome.exit_code, None);
    }

    #[cfg(unix)]
    #[test]
    fn a_child_is_run_to_the_end_with_its_environment() {
        let runner = ProcessRunner::new("sh", ["-c", "test \"$NURT_CRASH_TEST\" = 1; exit $?"])
            .env("NURT_CRASH_TEST", "1");
        assert!(runner.run_to_end().unwrap().success());
        let failing = ProcessRunner::new("sh", ["-c", "exit 3"]);
        assert_eq!(failing.run_to_end().unwrap().code(), Some(3));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

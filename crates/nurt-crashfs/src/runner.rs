// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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

/// Kill delay for a point: uniform in `0..max`, derived from `seed`.
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
        }
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
    /// marker, or cannot be killed or waited for.
    pub fn run_and_kill(&self, delay: Duration) -> io::Result<KillOutcome> {
        let mut child = self.spawn()?;
        if let Err(err) = wait_for_marker(&mut child) {
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

fn wait_for_marker(child: &mut Child) -> io::Result<()> {
    let stdout = child
        .stdout
        .as_mut()
        .ok_or_else(|| io::Error::other("child output is not captured"))?;
    let mut seen = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        if stdout.read(&mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "child ended before the scenario started",
            ));
        }
        seen.push(byte[0]);
        if seen.ends_with(READY_MARKER) {
            return Ok(());
        }
    }
}

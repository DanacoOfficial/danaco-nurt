// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::sys
// Responsibility:   The system seen by the administration commands: programs that read and.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The system seen by the administration commands: programs that read and
//! change it, files that describe it, the clock and the operating system.
//!
//! Every read and every change goes through [`Sys`], so that the commands run
//! the same code against the real system, against the process host of the
//! engine and against the in-memory system of the tests.

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{ProgramInput, ProgramOutcome, ProgramSpec};
use nurt_contracts::stage::ExecCtx;

/// Bytes of standard error kept in the message of a failed program.
const STDERR_TAIL: usize = 400;

/// Texts of standard error that mean the account lacks rights.
const DENIED: [&str; 7] = [
    "permission denied",
    "operation not permitted",
    "must be root",
    "are you root",
    "only root",
    "access is denied",
    "requires elevation",
];

/// One program to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    /// Program name or path.
    pub program: String,
    /// Arguments without the program name.
    pub args: Vec<String>,
    /// Variables set for the program.
    pub env: Vec<(String, String)>,
    /// Text written to standard input, for values that must not appear in
    /// the arguments.
    pub stdin: Option<String>,
}

impl Call {
    /// A call without environment and input.
    pub(crate) fn new<I, S>(program: &str, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.to_owned(),
            args: args.into_iter().map(Into::into).collect(),
            env: Vec::new(),
            stdin: None,
        }
    }

    /// The call with the text `stdin` as its input.
    #[must_use]
    pub(crate) fn with_stdin(mut self, stdin: impl Into<String>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    /// The call with one more variable.
    #[must_use]
    pub(crate) fn with_env(mut self, name: &str, value: &str) -> Self {
        self.env.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Program and arguments as one line for the tests; the input and the
    /// variables are not shown.
    #[cfg(test)]
    pub(crate) fn line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Result of a program.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Output {
    /// Exit code; `None` when the program was stopped.
    pub code: Option<i32>,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

impl Output {
    /// The program ended with code 0.
    pub(crate) fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// The system that the commands read and change.
pub(crate) trait Sys {
    /// Operating system family.
    fn os(&self) -> Os;

    /// Runs `call` to completion.
    ///
    /// # Errors
    ///
    /// Fails with `E_EXTERNAL_MISSING` when the program does not exist and
    /// with the error of the process host when it cannot be started.
    fn run(&self, call: &Call) -> Result<Output>;

    /// Content of a text file, `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// Fails with `E_PERMISSION` or `E_IO` when the file exists but cannot be
    /// read.
    fn read_file(&self, path: &str) -> Result<Option<String>>;

    /// Nanoseconds since the Unix epoch.
    fn now_nanos(&self) -> i128;

    /// Stops the operation when it was cancelled.
    ///
    /// # Errors
    ///
    /// Fails with `E_CANCELLED`.
    fn checkpoint(&self) -> Result<()> {
        Ok(())
    }
}

/// Runs `call` and requires exit code 0.
///
/// # Errors
///
/// Fails with `E_PERMISSION` when the program reports missing rights and with
/// `E_STAGE_FAILED` with the end of its standard error for any other failure.
pub(crate) fn run_ok(sys: &dyn Sys, call: &Call) -> Result<Output> {
    sys.checkpoint()?;
    let output = sys.run(call)?;
    if output.ok() {
        return Ok(output);
    }
    Err(failure(call, &output))
}

/// Error of a program that did not end with code 0.
pub(crate) fn failure(call: &Call, output: &Output) -> Error {
    let stderr = output.stderr.trim();
    let mut start = stderr.len().saturating_sub(STDERR_TAIL);
    while !stderr.is_char_boundary(start) {
        start += 1;
    }
    let tail = &stderr[start..];
    let status = output.code.map_or_else(
        || "was stopped".to_owned(),
        |code| format!("exited with code {code}"),
    );
    let lower = stderr.to_lowercase();
    let code = if DENIED.iter().any(|text| lower.contains(text)) {
        ErrorCode::Permission
    } else {
        ErrorCode::StageFailed
    };
    Error::new(code, format!("{} {status}: {tail}", call.program))
}

/// The system of a running engine: programs through the process host of the
/// context, files through the standard library.
pub(crate) struct CtxSys<'a> {
    ctx: &'a ExecCtx,
}

impl<'a> CtxSys<'a> {
    /// The system of `ctx`.
    pub(crate) fn new(ctx: &'a ExecCtx) -> Self {
        Self { ctx }
    }
}

impl Sys for CtxSys<'_> {
    fn os(&self) -> Os {
        self.ctx.host().info().os
    }

    fn run(&self, call: &Call) -> Result<Output> {
        let process = self.ctx.process().ok_or_else(|| {
            Error::new(
                ErrorCode::ExternalMissing,
                format!("`{}` needs the process host of the engine", call.program),
            )
        })?;
        let cwd = if self.os() == Os::Windows {
            "C:\\"
        } else {
            "/"
        };
        let mut spec = ProgramSpec::new(call.program.clone(), call.args.clone(), cwd);
        spec.env = call
            .env
            .iter()
            .map(|(name, value)| nurt_contracts::services::EnvVar {
                name: name.clone(),
                value: Some(value.clone()),
            })
            .collect();
        spec.timeout_ms = self
            .ctx
            .remaining()
            .map(|left| u64::try_from(left.as_millis()).unwrap_or(u64::MAX));
        if let Some(text) = &call.stdin {
            spec.stdin = ProgramInput::Bytes(text.clone().into_bytes());
        }
        let outcome = process.run_program(&spec, self.ctx.cancel())?;
        if outcome.cancelled || outcome.timed_out {
            self.ctx.checkpoint()?;
        }
        Ok(output_of(&outcome))
    }

    fn read_file(&self, path: &str) -> Result<Option<String>> {
        read_text(path)
    }

    fn now_nanos(&self) -> i128 {
        self.ctx.clock().now().unix_nanos()
    }

    fn checkpoint(&self) -> Result<()> {
        self.ctx.checkpoint()
    }
}

fn output_of(outcome: &ProgramOutcome) -> Output {
    Output {
        code: outcome.exit_code,
        stdout: String::from_utf8_lossy(&outcome.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
    }
}

/// Reads a text file; a missing file is `None`.
pub(crate) fn read_text(path: &str) -> Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Err(Error::new(
            ErrorCode::Permission,
            format!("cannot read `{path}`: {error}"),
        )),
        Err(error) => Err(Error::new(
            ErrorCode::Io,
            format!("cannot read `{path}`: {error}"),
        )),
    }
}

/// The system of the current process, used by the tests that run on a real
/// machine: programs are started directly.
#[cfg(test)]
pub(crate) struct ProcessSys;

#[cfg(test)]
impl Sys for ProcessSys {
    fn os(&self) -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }

    fn run(&self, call: &Call) -> Result<Output> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut command = Command::new(&call.program);
        command
            .args(&call.args)
            .envs(call.env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|error| {
            Error::new(
                ErrorCode::ExternalMissing,
                format!("cannot start `{}`: {error}", call.program),
            )
        })?;
        if let (Some(mut stdin), Some(text)) = (child.stdin.take(), &call.stdin) {
            let _ = stdin.write_all(text.as_bytes());
        }
        let output = child
            .wait_with_output()
            .map_err(|error| Error::new(ErrorCode::Io, error.to_string()))?;
        Ok(Output {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn read_file(&self, path: &str) -> Result<Option<String>> {
        read_text(path)
    }

    fn now_nanos(&self) -> i128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| i128::try_from(elapsed.as_nanos()).unwrap_or(0))
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

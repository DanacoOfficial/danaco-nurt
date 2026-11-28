// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::admin::tool
// Responsibility:   Running the programs of the operating system for administration commands.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Running the programs of the operating system for administration commands.

use std::path::Path;

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{ProcessHost, ProgramInput, ProgramOutcome, ProgramSpec};
use nurt_contracts::stage::{CancelToken, ExecCtx};

/// Bytes of standard error kept in the message of a failure.
const STDERR_TAIL: usize = 400;

/// Phrases of the programs that report missing rights, in lower case.
///
/// `permissiondenied` and `windows system error 5,` are the error category and
/// the error identifier that Windows PowerShell prints in every display
/// language; `odmowa dost` is the start of the Polish text of the Windows
/// error 5, cut before the first letter outside ASCII.
const DENIED: [&str; 11] = [
    "permission denied",
    "operation not permitted",
    "must be root",
    "need to be root",
    "you need to be root",
    "access is denied",
    "requires elevation",
    "not authorized",
    "permissiondenied",
    "windows system error 5,",
    "odmowa dost",
];

/// Family of the operating system the programs run on.
///
/// The enumeration of the contracts is open; every system that is neither
/// macOS nor Windows is treated as Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    /// Linux and the other Unix systems with the GNU tools.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
}

impl System {
    /// System of the host.
    #[must_use]
    pub fn of(os: Os) -> Self {
        match os {
            Os::Macos => Self::Macos,
            Os::Windows => Self::Windows,
            _ => Self::Linux,
        }
    }
}

/// One program to run: name, arguments and optional standard input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// Program name.
    pub program: String,
    /// Arguments, without the program name.
    pub args: Vec<String>,
    /// Text written to the standard input of the program.
    pub stdin: Option<String>,
}

impl Call {
    /// Call of `program` with `args`.
    #[must_use]
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            stdin: None,
        }
    }

    /// Call of `program` with owned arguments.
    #[must_use]
    pub fn owned(program: &str, args: Vec<String>) -> Self {
        Self {
            program: program.to_owned(),
            args,
            stdin: None,
        }
    }

    /// Call of the PowerShell command `script`.
    #[must_use]
    pub fn powershell(script: &str) -> Self {
        Self::new(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", script],
        )
    }

    /// The same call with `text` on standard input.
    #[must_use]
    pub fn with_stdin(mut self, text: String) -> Self {
        self.stdin = Some(text);
        self
    }

    /// Command line for messages: program and arguments separated by spaces.
    #[must_use]
    pub fn line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Result of a finished program.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// Exit code; `None` when the process was stopped.
    pub code: Option<i32>,
    /// Standard output as text.
    pub stdout: String,
    /// Standard error as text.
    pub stderr: String,
}

impl Output {
    /// The program exited with code zero.
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Access to the programs of the system for one command.
pub struct Tool<'a> {
    process: &'a dyn ProcessHost,
    os: System,
    cancel: &'a CancelToken,
    timeout_ms: Option<u64>,
}

impl<'a> Tool<'a> {
    /// Tool over the process host and the operating system of `ctx`.
    ///
    /// # Errors
    ///
    /// `E_PROGRAM_NOT_FOUND` when the context has no process host.
    pub fn from_ctx(ctx: &'a ExecCtx) -> Result<Self> {
        let process = ctx.process().ok_or_else(|| {
            Error::new(
                ErrorCode::ProgramNotFound,
                "the context has no host to start system programs",
            )
        })?;
        let timeout_ms = ctx
            .remaining()
            .map(|left| u64::try_from(left.as_millis()).unwrap_or(u64::MAX));
        Ok(Self {
            process,
            os: System::of(ctx.host().info().os),
            cancel: ctx.cancel(),
            timeout_ms,
        })
    }

    /// Tool over an explicit process host.
    #[must_use]
    pub fn new(process: &'a dyn ProcessHost, os: System, cancel: &'a CancelToken) -> Self {
        Self {
            process,
            os,
            cancel,
            timeout_ms: None,
        }
    }

    /// Operating system the programs run on.
    #[must_use]
    pub fn os(&self) -> System {
        self.os
    }

    /// Whether `program` is found on the search path.
    #[must_use]
    pub fn available(&self, program: &str) -> bool {
        let cwd = if self.os == System::Windows {
            "C:\\"
        } else {
            "/"
        };
        self.process
            .resolve_program(program, Path::new(cwd))
            .is_ok()
    }

    /// Runs `call` to completion whatever its exit code.
    ///
    /// # Errors
    ///
    /// `E_PROGRAM_NOT_FOUND` when the program does not exist and
    /// `E_CANCELLED` or `E_TIMEOUT` when the run was stopped.
    pub fn run(&self, call: &Call) -> Result<Output> {
        let cwd = if self.os == System::Windows {
            "C:\\"
        } else {
            "/"
        };
        let mut spec = ProgramSpec::new(call.program.clone(), call.args.clone(), cwd);
        spec.timeout_ms = self.timeout_ms;
        if let Some(text) = &call.stdin {
            spec.stdin = ProgramInput::Bytes(text.clone().into_bytes());
        }
        let outcome: ProgramOutcome = self.process.run_program(&spec, self.cancel)?;
        if outcome.cancelled || outcome.timed_out {
            self.cancel.check()?;
        }
        Ok(Output {
            code: outcome.exit_code,
            stdout: String::from_utf8_lossy(&outcome.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
        })
    }

    /// Runs `call` and requires exit code zero.
    ///
    /// # Errors
    ///
    /// `E_PRIVILEGE` when the program reports missing rights, otherwise
    /// `E_STAGE_FAILED` with the exit code and the end of standard error.
    pub fn ok(&self, call: &Call) -> Result<Output> {
        let output = self.run(call)?;
        if output.success() {
            Ok(output)
        } else {
            Err(failure(call, &output))
        }
    }
}

/// Error of a program that did not succeed.
#[must_use]
pub(crate) fn failure(call: &Call, output: &Output) -> Error {
    let text = format!("{}\n{}", output.stderr, output.stdout).to_lowercase();
    let tail = tail_of(&output.stderr, &output.stdout);
    if DENIED.iter().any(|phrase| text.contains(phrase)) {
        return Error::new(
            ErrorCode::Privilege,
            format!("`{}` needs administrator rights: {tail}", call.program),
        )
        .with_fix("repeat the call with `privilege: elevated`")
        .with_detail("program", call.program.clone().into());
    }
    let status = output.code.map_or_else(
        || "was stopped".to_owned(),
        |code| format!("exited with code {code}"),
    );
    Error::new(
        ErrorCode::StageFailed,
        format!("`{}` {status}: {tail}", call.program),
    )
    .with_detail("program", call.program.clone().into())
}

fn tail_of(stderr: &str, stdout: &str) -> String {
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let text = text.trim();
    let mut start = text.len().saturating_sub(STDERR_TAIL);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard error of Windows PowerShell for a refused change, on a system
    /// with the Polish display language.
    const REFUSAL: &str = "New-NetFirewallRule : Odmowa dost\u{119}pu.\n\
        + CategoryInfo : PermissionDenied: (MSFT_NetFirewallRule) [New-NetFirewallRule]\n\
        + FullyQualifiedErrorId : Windows System Error 5,New-NetFirewallRule";

    fn failed(stderr: &str) -> Error {
        let output = Output {
            code: Some(1),
            stdout: String::new(),
            stderr: stderr.to_owned(),
        };
        failure(&Call::powershell("New-NetFirewallRule"), &output)
    }

    #[test]
    fn a_refusal_is_recognised_in_another_display_language() {
        assert_eq!(failed(REFUSAL).code(), ErrorCode::Privilege);
        let category = "+ CategoryInfo : PermissionDenied: (rule) [New-NetFirewallRule]";
        assert_eq!(failed(category).code(), ErrorCode::Privilege);
        let identifier = "+ FullyQualifiedErrorId : Windows System Error 5,New-NetRoute";
        assert_eq!(failed(identifier).code(), ErrorCode::Privilege);
    }

    #[test]
    fn another_system_error_is_not_a_refusal() {
        let identifier = "+ FullyQualifiedErrorId : Windows System Error 50,New-NetRoute";
        assert_eq!(failed(identifier).code(), ErrorCode::StageFailed);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

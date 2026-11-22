// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Resolution of programs, arguments, environment and input of a run.
//!
//! A command is turned into a [`SpawnRequest`] without a shell. The program is
//! looked up in the search path of the child environment, batch files on
//! Windows are started through the command interpreter with every argument
//! quoted, and the environment is built from the inherited variables, the
//! configured defaults and the variables of the call, in this order.

pub(crate) mod batch;
mod env;
mod resolve;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
mod windows_cmd;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Launcher;
use nurt_contracts::value;

use crate::iface::{ProcessLimits, SpawnRequest, StdinSource};

pub(crate) use env::{ChildEnv, InheritEnv};
pub(crate) use resolve::resolve;
pub(crate) use windows_cmd::metacharacter_arguments;

/// Path notation and program search rules of the host that starts the
/// program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    /// Linux and macOS: `PATH` separated by `:`, execute permission required,
    /// variable names compared case-sensitively.
    Unix,
    /// Windows: `PATH` separated by `;`, extensions from `PATHEXT`, batch files
    /// through the command interpreter, variable names compared without case.
    Windows,
}

impl Platform {
    /// Rules of the operating system family `os`.
    pub(crate) fn of(os: Os) -> Self {
        match os {
            Os::Windows => Self::Windows,
            _ => Self::Unix,
        }
    }

    /// Rules of the system this program was built for.
    pub(crate) fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// Program file and arguments ready to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Launch {
    /// File given to the operating system: the program, or the command
    /// interpreter for batch files.
    pub(crate) executable: PathBuf,
    /// Program found in the search path, as reported in the result.
    pub(crate) program: PathBuf,
    /// How the program is started.
    pub(crate) launcher: Launcher,
    /// Arguments without the program name; empty for batch files.
    pub(crate) args: Vec<OsString>,
    /// Complete command line of a batch file.
    pub(crate) command_line: Option<OsString>,
}

impl Launch {
    /// Start request of this launch.
    pub(crate) fn request(
        &self,
        cwd: &Path,
        env: ChildEnv,
        stdin: StdinSource,
        merge_stderr: bool,
        limits: ProcessLimits,
    ) -> SpawnRequest {
        SpawnRequest {
            program: self.executable.clone(),
            args: self.args.clone(),
            launcher: self.launcher,
            raw_command_line: self.command_line.clone(),
            cwd: cwd.to_path_buf(),
            env: env.into_vars(),
            stdin,
            merge_stderr,
            limits,
        }
    }

    /// Arguments as the program receives them, program first, for previews.
    pub(crate) fn argv(&self) -> Vec<String> {
        match &self.command_line {
            Some(line) => vec![line.to_string_lossy().into_owned()],
            None => std::iter::once(self.executable.as_os_str())
                .chain(self.args.iter().map(OsString::as_os_str))
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
        }
    }
}

/// Resolves the program of `command` and prepares its arguments.
///
/// # Errors
///
/// Fails with `E_ARG` for an empty command, a NUL character in an argument or
/// an argument a batch file cannot receive, `E_PROGRAM_NOT_FOUND` when the
/// program is not found and `E_SPAWN` when the program file cannot be run.
pub(crate) fn prepare_launch(command: &[String], cwd: &Path, env: &ChildEnv) -> Result<Launch> {
    if command.is_empty() {
        return Err(Error::new(
            ErrorCode::Arg,
            "the command is empty; give the program as the first element",
        )
        .with_problem("empty_command"));
    }
    if let Some(index) = command.iter().position(|arg| arg.contains('\0')) {
        return Err(unpassable(
            command,
            index,
            "nul_in_argument",
            "contains a NUL character, which no program can receive",
        ));
    }
    let resolved = resolve(command, cwd, env)?;
    let args = &command[1..];
    if resolved.launcher == Launcher::Cmd {
        let script = resolved.path.to_string_lossy();
        let line = windows_cmd::command_line(&script, args).map_err(|problem| {
            unpassable(command, problem.index, problem.problem, problem.reason)
        })?;
        return Ok(Launch {
            executable: windows_cmd::interpreter(env.get("SystemRoot")),
            program: resolved.path,
            launcher: Launcher::Cmd,
            args: Vec::new(),
            command_line: Some(OsString::from(line)),
        });
    }
    Ok(Launch {
        executable: resolved.path.clone(),
        program: resolved.path,
        launcher: Launcher::Direct,
        args: args.iter().map(OsString::from).collect(),
        command_line: None,
    })
}

/// Error for an argument that cannot be passed without changing its meaning.
/// The fix passes the argument on standard input instead.
fn unpassable(command: &[String], index: usize, problem: &str, reason: &str) -> Error {
    let mut error = Error::new(
        ErrorCode::Arg,
        format!(
            "element {index} of the command {reason}; pass the text on standard input or in a file"
        ),
    )
    .with_problem(problem)
    .with_detail("index", serde_json::Value::from(index));
    if index > 0 {
        let rest: Vec<String> = command
            .iter()
            .enumerate()
            .filter(|(position, _)| *position != index)
            .map(|(_, arg)| arg.clone())
            .collect();
        error = error.with_fix(format!(
            "{} stdin: {}",
            program_text(&rest),
            text_literal(&command[index])
        ));
    }
    error
}

/// Checks that the working directory exists and is a directory.
///
/// # Errors
///
/// Fails with `E_SPAWN` whose cause is `E_NOT_FOUND` or `E_IO`.
pub(crate) fn check_cwd(cwd: &Path) -> Result<()> {
    match std::fs::metadata(cwd) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(Error::new(
            ErrorCode::Spawn,
            format!(
                "the working directory {} is not a directory; give a directory in `cwd`",
                cwd.display()
            ),
        )
        .with_problem("cwd_not_directory")
        .with_cause(Error::new(
            ErrorCode::Io,
            format!("{} is not a directory", cwd.display()),
        ))),
        Err(source) => Err(Error::new(
            ErrorCode::Spawn,
            format!(
                "the working directory {} does not exist; give an existing directory in `cwd`",
                cwd.display()
            ),
        )
        .with_problem("cwd_missing")
        .with_cause(Error::new(
            if source.kind() == std::io::ErrorKind::NotFound {
                ErrorCode::NotFound
            } else {
                ErrorCode::Io
            },
            source.to_string(),
        ))),
    }
}

/// Path in the canonical form of records: `/` as the separator on every
/// system.
pub(crate) fn record_path(path: &Path, platform: Platform) -> value::Path {
    let text = path.to_string_lossy();
    match platform {
        Platform::Unix => value::Path::new(text.into_owned()),
        Platform::Windows => {
            let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
            let mut canonical = plain.replace('\\', "/");
            if let Some(drive) = canonical.get(..1)
                && canonical.get(1..2) == Some(":")
            {
                canonical.replace_range(..1, &drive.to_ascii_uppercase());
            }
            value::Path::new(canonical)
        }
    }
}

/// The `run` stage for `command`, as program text.
pub(crate) fn program_text(command: &[String]) -> String {
    let items: Vec<String> = command.iter().map(|arg| text_literal(arg)).collect();
    format!("run [{}]", items.join(", "))
}

/// Text literal of the language with escapes for `\`, `"` and control
/// characters.
pub(crate) fn text_literal(text: &str) -> String {
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('"');
    for character in text.chars() {
        match character {
            '\\' => literal.push_str(r"\\"),
            '"' => literal.push_str("\\\""),
            '\n' => literal.push_str(r"\n"),
            '\r' => literal.push_str(r"\r"),
            '\t' => literal.push_str(r"\t"),
            '\0' => literal.push_str(r"\0"),
            other if other.is_control() => {
                literal.push_str(&format!("\\u{{{:X}}}", u32::from(other)));
            }
            other => literal.push(other),
        }
    }
    literal.push('"');
    literal
}

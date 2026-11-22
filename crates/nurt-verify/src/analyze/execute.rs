// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Running analyzers and formatters through the process layer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::EnvVar;
use nurt_contracts::stage::ExecCtx;
use nurt_proc::{ToolKind, ToolOutcome, ToolResult, ToolRun, run_tool};

use crate::analyze::output::ToolOutput;

/// Time limit of one analyzer or formatter run.
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(600);

/// Output of a finished run.
#[derive(Debug, Clone, Default)]
pub(crate) struct Captured {
    /// Decoded output and exit code.
    pub(crate) output: ToolOutput,
    /// The run was stopped by the time limit.
    pub(crate) timed_out: bool,
}

/// Environment that keeps analyzers from coloring or prompting.
fn environment() -> Vec<EnvVar> {
    [("NO_COLOR", "1"), ("CI", "1")]
        .into_iter()
        .map(|(name, value)| EnvVar {
            name: name.to_owned(),
            value: Some(value.to_owned()),
        })
        .collect()
}

fn read_text(file: Option<&PathBuf>, fallback: &str) -> String {
    file.and_then(|path| std::fs::read(path).ok()).map_or_else(
        || fallback.to_owned(),
        |bytes| String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// Complete output of a finished run, read from the spooled files when the
/// run has them.
pub(crate) fn captured_of(result: &ToolResult) -> Captured {
    let exec = &result.exec;
    Captured {
        output: ToolOutput {
            stdout: read_text(result.stdout_file.as_ref(), &exec.stdout),
            stderr: read_text(result.stderr_file.as_ref(), &exec.stderr),
            exit_code: i32::try_from(exec.exit_code).ok(),
        },
        timed_out: exec.timed_out,
    }
}

/// Runs a program in `dir` and captures its output.
///
/// `exact` lists the files the program is expected to change; they are kept
/// before the run.
///
/// # Errors
///
/// Fails when the program cannot be started or continues in the background.
pub(crate) fn run_captured(
    ctx: &ExecCtx,
    dir: &Path,
    command: Vec<String>,
    exact: Option<Vec<PathBuf>>,
) -> Result<Captured> {
    let run = ToolRun {
        command,
        cwd: dir.to_path_buf(),
        env: environment(),
        timeout: COMMAND_TIMEOUT,
        watch: vec![dir.to_path_buf()],
        exact,
        kind: ToolKind::Analyze,
        label: None,
        finish_data: None,
    };
    match run_tool(ctx, run, None)? {
        ToolOutcome::Finished(result) => Ok(captured_of(&result)),
        ToolOutcome::Background(_) => Err(Error::new(
            ErrorCode::Unsupported,
            "the tool continued in the background",
        )
        .with_fix("run the tool with run wait: 0s")),
    }
}

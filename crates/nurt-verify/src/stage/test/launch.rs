// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Preparation and start of a test run, shared by `test.run` and
//! `test.failed`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::error::{Candidate, Error, ErrorCode, Result};
use nurt_contracts::record::TestResult;
use nurt_contracts::services::EnvVar;
use nurt_contracts::stage::{
    EffectOutcome, EffectPlan, Event, ExecCtx, LogEvent, LogLevel, RecordStream, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_proc::{ToolOutcome, ToolRun, run_tool};
use serde::{Deserialize, Serialize};

use super::args::TOOL_KIND;
use super::finish;
use crate::fingerprint;
use crate::iface::{EntryKind, verify_store};
use crate::test::command::plan as build_command;
use crate::test::detect::detect;
use crate::test::{CommandPlan, Framework, PlanRequest};

/// Directory of framework reports inside the history directory.
const REPORTS_DIR: &str = "verify/reports";

/// Variables that keep frameworks from using colors and interactive
/// features.
const QUIET_ENV: [(&str, &str); 2] = [("CI", "1"), ("NO_COLOR", "1")];

/// Source of log events of the test commands.
const LOG_SOURCE: &str = "test";

/// What a run needs, gathered from the arguments.
#[derive(Debug, Clone)]
pub(super) struct Request {
    pub(super) dir: PathBuf,
    /// `None` detects the framework from the project.
    pub(super) framework: Option<Framework>,
    pub(super) targets: Vec<String>,
    pub(super) filter: Option<String>,
    pub(super) bail: bool,
    /// Results to run again.
    pub(super) only: Vec<TestResult>,
    pub(super) command: Option<Vec<String>>,
    pub(super) coverage: bool,
    pub(super) coverage_file: Option<PathBuf>,
    pub(super) timeout: Duration,
    pub(super) wait: Option<Duration>,
    pub(super) shape: Shape,
    pub(super) retry_of: Option<String>,
}

/// How much of a run is shown in the output.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(super) struct Shape {
    pub(super) failures: usize,
    pub(super) message_lines: usize,
    pub(super) slowest: usize,
}

/// Everything needed to finish a run once the framework has ended; passed
/// unchanged to the finisher of a background run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RunSpec {
    pub(super) run_id: String,
    pub(super) framework: String,
    pub(super) dir: String,
    pub(super) argv: Vec<String>,
    pub(super) report: Option<String>,
    pub(super) coverage_file: Option<String>,
    pub(super) targets: Vec<String>,
    pub(super) filter: Option<String>,
    pub(super) retry_of: Option<String>,
    pub(super) fingerprint: Option<String>,
    pub(super) only_ids: Vec<String>,
    pub(super) shape: Shape,
}

/// Request after the framework was chosen.
#[derive(Debug, Clone)]
pub(super) struct Prepared {
    pub(super) request: Request,
    pub(super) framework: Framework,
}

/// Chooses the framework and checks that the run can start.
///
/// # Errors
///
/// Fails with `E_TEST_NO_FRAMEWORK` when the project has no recognised
/// framework and no command was given.
pub(super) fn prepare(request: Request) -> Result<Prepared> {
    let framework = match (&request.command, request.framework) {
        (Some(_), Some(framework)) => framework,
        (Some(_), None) => Framework::Custom,
        (None, Some(Framework::Custom)) => {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`framework: custom` needs a `command`",
            )
            .with_fix("test.run command: [\"make\", \"test\"]"));
        }
        (None, Some(framework)) => framework,
        (None, None) => {
            let found = detect(&request.dir);
            match found.first() {
                Some(first) => *first,
                None => return Err(no_framework(&request.dir)),
            }
        }
    };
    Ok(Prepared { request, framework })
}

fn no_framework(dir: &Path) -> Error {
    let mut error = Error::new(
        ErrorCode::TestNoFramework,
        format!("no test framework recognised in {}", dir.display()),
    )
    .with_fix("test.run command: [\"<program>\", \"<argument>\"]");
    for (rank, framework) in (1_u32..).zip(
        Framework::ALL
            .into_iter()
            .filter(|framework| *framework != Framework::Custom),
    ) {
        error = error.with_candidate(Candidate {
            rank,
            reference: None,
            program: format!("test.run framework: {}", framework.name()),
            why: None,
        });
    }
    error
}

/// Directory of the reports of the history, created when missing.
fn reports_dir(ctx: &ExecCtx) -> PathBuf {
    let base = ctx
        .state_dir()
        .map_or_else(|| ctx.host().info().temp.clone(), Path::to_path_buf);
    base.join(REPORTS_DIR)
}

/// Program to start for a tool: the installation in the project when there
/// is one, the name itself when it is found on the search path.
fn lookup(ctx: &ExecCtx, dir: &Path) -> impl Fn(&str) -> Option<String> {
    let search = ctx.host().env_var("PATH");
    let dir = dir.to_path_buf();
    move |name: &str| {
        let local = dir.join("node_modules").join(".bin").join(name);
        if local.is_file() {
            return Some(local.to_string_lossy().into_owned());
        }
        let found = search.as_ref().is_some_and(|paths| {
            std::env::split_paths(paths).any(|entry| entry.join(name).is_file())
        });
        found.then(|| name.to_owned())
    }
}

fn is_script_runner(framework: Framework) -> bool {
    matches!(
        framework,
        Framework::Bun | Framework::Vitest | Framework::Jest
    )
}

/// Command of the run and the files it writes.
fn command_for(ctx: &ExecCtx, prepared: &Prepared, run_id: &str) -> CommandPlan {
    let request = &prepared.request;
    let reports = reports_dir(ctx);
    let framework = prepared.framework;
    let report_name = if framework == Framework::Jest {
        format!("{run_id}.json")
    } else {
        format!("{run_id}.xml")
    };
    let report = reports.join(report_name);
    let coverage_target = if is_script_runner(framework) {
        reports.join(format!("{run_id}-coverage"))
    } else {
        reports.join(format!("{run_id}.coverage"))
    };
    let program = lookup(ctx, &request.dir);
    if let Some(command) = &request.command
        && framework != Framework::Custom
    {
        return CommandPlan {
            framework,
            argv: command.clone(),
            report: None,
            coverage_file: None,
            env: Vec::new(),
            notes: Vec::new(),
        };
    }
    let plan_request = PlanRequest {
        framework,
        targets: &request.targets,
        filter: request.filter.as_deref(),
        bail: request.bail,
        only: &request.only,
        report: &report,
        coverage: request.coverage.then_some(coverage_target.as_path()),
        command: request.command.as_deref(),
        program: &program,
    };
    build_command(&plan_request)
}

/// Plan of the run for a preview: the command and the directory.
pub(super) fn preview(ctx: &ExecCtx, prepared: &Prepared) -> EffectPlan {
    let command = command_for(ctx, prepared, "run");
    let record = Record::new(RecordKind::Map)
        .with(
            "command",
            Value::List(command.argv.iter().cloned().map(Value::Text).collect()),
        )
        .with(
            "cwd",
            Value::Path(prepared.request.dir.to_string_lossy().into_owned()),
        )
        .with(
            "framework",
            Value::Text(prepared.framework.name().to_owned()),
        );
    let mut plan = EffectPlan::new();
    plan.preview.push(record);
    plan.with_state(prepared.clone())
}

/// Starts the run and, when it ends within the waiting time, finishes it.
pub(super) fn execute<'a>(ctx: &ExecCtx, prepared: Prepared) -> Result<EffectOutcome<'a>> {
    let store = verify_store();
    let run_id = store.next_id(ctx, EntryKind::TestRun)?;
    let request = &prepared.request;
    let plan = command_for(ctx, &prepared, &run_id);
    fs::create_dir_all(reports_dir(ctx)).map_err(|failure| {
        Error::new(
            ErrorCode::Io,
            format!("cannot create the report directory: {failure}"),
        )
    })?;
    for note in &plan.notes {
        ctx.emit(Event::Log(LogEvent {
            level: LogLevel::Info,
            message: note.clone(),
            source: Some(LOG_SOURCE.to_owned()),
        }));
    }
    let coverage_file = request
        .coverage_file
        .as_ref()
        .map(|file| file.to_string_lossy().into_owned())
        .or_else(|| plan.coverage_file.clone());
    let fingerprint = fingerprint::compute(ctx, &request.dir, &ctx.config().run.preimage_exclude)
        .ok()
        .map(|found| found.id);
    let spec = RunSpec {
        run_id,
        framework: prepared.framework.name().to_owned(),
        dir: request.dir.to_string_lossy().into_owned(),
        argv: plan.argv.clone(),
        report: plan.report.clone(),
        coverage_file,
        targets: request.targets.clone(),
        filter: request.filter.clone(),
        retry_of: request.retry_of.clone(),
        fingerprint,
        only_ids: request.only.iter().map(|case| case.id.clone()).collect(),
        shape: request.shape,
    };
    let mut env: Vec<EnvVar> = QUIET_ENV
        .iter()
        .map(|(name, value)| EnvVar {
            name: (*name).to_owned(),
            value: Some((*value).to_owned()),
        })
        .collect();
    env.extend(plan.env.iter().map(|(name, value)| EnvVar {
        name: name.clone(),
        value: Some(value.clone()),
    }));
    let finish_data = serde_json::to_value(&spec).map_err(|failure| {
        Error::new(
            ErrorCode::Internal,
            format!("cannot encode the run: {failure}"),
        )
    })?;
    let tool = ToolRun {
        command: plan.argv,
        cwd: request.dir.clone(),
        env,
        timeout: request.timeout,
        watch: vec![request.dir.clone()],
        exact: None,
        kind: TOOL_KIND,
        label: Some(format!("test {}", prepared.framework.name())),
        finish_data: Some(finish_data),
    };
    let outcome = run_tool(ctx, tool, request.wait).map_err(missing_program)?;
    match outcome {
        ToolOutcome::Finished(result) => {
            let finished = finish::finish(ctx, &spec, &result)?;
            let output = StageOutput::records(RecordStream::from_records(finished.items))
                .with_header(finished.header);
            Ok(EffectOutcome::new(output))
        }
        ToolOutcome::Background(job) => {
            let output = StageOutput::records(RecordStream::once(Record::from(*job)));
            let mut outcome = EffectOutcome::new(output);
            outcome.detached = true;
            Ok(outcome)
        }
    }
}

fn missing_program(error: Error) -> Error {
    if error.code() != ErrorCode::ProgramNotFound {
        return error;
    }
    Error::new(
        ErrorCode::ExternalMissing,
        "the test program is not installed".to_owned(),
    )
    .with_fix("install the framework or pass command:")
    .with_cause(error)
}

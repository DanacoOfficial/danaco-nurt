// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `run` command and tool runs.
//!
//! The plan step resolves the program, builds the environment and the start
//! request of every process without starting anything; a preview ends there
//! and shows the program, the arguments as the program receives them, the
//! working directory and the names of the variables set. The apply step hands
//! each request to the run host, which detects file changes, starts the
//! program, keeps its output and records the changes in the journal. A
//! single run that outlives its `wait` continues in the background and
//! yields a `job` record; a run over input records waits for every batch and
//! yields one `exec` record with the batches.

mod params;
mod record;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, RecordKind, Value};

use crate::api::{ToolKind, ToolOutcome, ToolResult, ToolRun};
use crate::classify::EffectClassifier;
use crate::iface::{
    OutputView, Priority, ProcessLimits, RunHost, RunKind, RunRequest, RunResult, RunWait,
    ShowMode, StdinSource, TrackMode, TrackSpec,
};
use crate::jobs::{exec_record, external_effect};
use crate::spawn::{self, ChildEnv, InheritEnv, Platform, batch};

use self::params::Settings;
use self::record::{BatchRun, LaunchInfo};

/// Stages of the run group implemented here.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(RunStage::default())]
}

/// The `run` command.
#[derive(Default)]
pub(crate) struct RunStage {
    host: Option<Arc<dyn RunHost>>,
}

impl RunStage {
    /// Stage that hands its runs to `host` instead of the run host of the
    /// context.
    #[cfg(test)]
    pub(crate) fn with_host(host: Arc<dyn RunHost>) -> Self {
        Self { host: Some(host) }
    }

    fn host(&self, ctx: &ExecCtx) -> Arc<dyn RunHost> {
        self.host
            .clone()
            .unwrap_or_else(|| crate::api::run_host(ctx))
    }
}

/// One process of a planned run.
struct PlannedLaunch {
    /// Batch number, from 1.
    no: u64,
    /// Arguments taken from the input records.
    args: Vec<String>,
    /// Arguments as the program receives them.
    argv: Vec<String>,
    info: LaunchInfo,
    request: RunRequest,
}

/// Private state passed from the plan step to the apply step.
struct RunPlan {
    /// Command of the `exec` record of a run over input records.
    command: Vec<String>,
    launches: Vec<PlannedLaunch>,
    /// The stage received input records.
    batched: bool,
    wait: Option<Duration>,
    parallel: usize,
    bail: bool,
}

impl StageImpl for RunStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "run".to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry, RecordKind::Match]),
        }
    }

    fn check(&self, args: &BoundArgs, ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        params::static_info(args, ctx)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for RunStage {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let settings = params::read(args, ctx)?;
        spawn::check_cwd(&settings.cwd)?;
        let batched = !input.is_none();
        let (command, processes) = processes(args, input, ctx, &settings)?;
        let env = ChildEnv::build(
            settings.platform,
            ctx.host().env_vars(),
            settings.inherit,
            env_defaults(ctx),
            &settings.env,
        )?;
        let classifier = effect_classifier(ctx)?;
        let mut launches = Vec::with_capacity(processes.len());
        for (index, (command, batch_args)) in processes.into_iter().enumerate() {
            ctx.checkpoint()?;
            launches.push(plan_launch(
                ctx,
                &settings,
                &env,
                &classifier,
                index,
                command,
                batch_args,
            )?);
        }
        let mut plan = EffectPlan::new();
        plan.preview = launches
            .iter()
            .map(|launch| preview(launch, &settings, batched))
            .collect();
        plan.bulk = launches.len() > 1;
        Ok(plan.with_state(RunPlan {
            command,
            launches,
            batched,
            wait: settings.wait,
            parallel: settings.parallel,
            bail: settings.bail,
        }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let state = plan.take_state::<RunPlan>().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "the plan of `run` holds no processes; plan the stage again",
            )
        })?;
        let host = self.host(ctx);
        if state.batched {
            run_batches(host.as_ref(), ctx, &state)
        } else {
            run_single(host.as_ref(), ctx, state)
        }
    }
}

/// Classifier of external effects with the patterns of
/// `run.irreversible_patterns`.
fn effect_classifier(ctx: &ExecCtx) -> Result<EffectClassifier> {
    EffectClassifier::with_patterns(&ctx.config().run.irreversible_patterns)
}

/// Rules of the external effect table matched by `command`.
fn effects_of(classifier: &EffectClassifier, command: &[String]) -> Vec<String> {
    classifier
        .classify(command)
        .into_iter()
        .map(|found| found.rule)
        .collect()
}

/// Variables of `run.env_defaults`, in order.
fn env_defaults(ctx: &ExecCtx) -> impl Iterator<Item = (&str, &str)> {
    ctx.config()
        .run
        .env_defaults
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
}

/// Command of the result and the processes of a stage: one without input,
/// otherwise one per batch of input records, or one per record when the
/// command depends on the record.
#[allow(
    clippy::type_complexity,
    reason = "pairs of a command and its batch arguments are used only here"
)]
fn processes(
    args: &BoundArgs,
    input: StageInput<'_>,
    ctx: &ExecCtx,
    settings: &Settings,
) -> Result<(Vec<String>, Vec<(Vec<String>, Vec<String>)>)> {
    if input.is_none() {
        let command = params::command(args, None, ctx)?;
        return Ok((command.clone(), vec![(command, Vec::new())]));
    }
    let records = input
        .into_records()?
        .checked(ctx)
        .collect_bounded(ctx.limits().max_records)?;
    if args.expr("command").is_some() {
        let processes = records
            .iter()
            .map(|record| {
                let command = params::command(args, Some(record), ctx)?;
                let rest = command.get(1..).unwrap_or_default().to_vec();
                Ok((command, rest))
            })
            .collect::<Result<Vec<_>>>()?;
        let command = processes
            .first()
            .map(|(command, _)| command.clone())
            .unwrap_or_default();
        return Ok((command, processes));
    }
    let base = params::command(args, None, ctx)?;
    let arguments = records
        .iter()
        .map(params::record_argument)
        .collect::<Result<Vec<_>>>()?;
    let processes = batch::split(&arguments, settings.batch.unwrap_or(1))
        .into_iter()
        .map(|chunk| {
            let mut command = base.clone();
            command.extend(chunk.iter().cloned());
            (command, chunk)
        })
        .collect();
    Ok((base, processes))
}

fn plan_launch(
    ctx: &ExecCtx,
    settings: &Settings,
    env: &ChildEnv,
    classifier: &EffectClassifier,
    index: usize,
    command: Vec<String>,
    batch_args: Vec<String>,
) -> Result<PlannedLaunch> {
    let launch = spawn::prepare_launch(&command, &settings.cwd, env)?;
    let info = LaunchInfo {
        command: command.clone(),
        program: launch.program.clone(),
        cwd: settings.cwd.clone(),
        launcher: launch.launcher,
        env_keys: settings.env_keys(),
        platform: settings.platform,
        timeout: settings.limits.timeout,
        idle_timeout: settings.limits.idle_timeout,
    };
    let external_effect = effects_of(classifier, &command);
    let request = RunRequest {
        command,
        program: launch.program.clone(),
        env_keys: settings.env_keys(),
        spawn: launch.request(
            &settings.cwd,
            env.clone(),
            settings.stdin.clone(),
            settings.merge_stderr,
            settings.limits.clone(),
        ),
        track: settings.track.clone(),
        view: settings.view.clone(),
        kind: RunKind::Run,
        label: settings.label.clone(),
        description: settings.description.clone(),
        session: ctx.options().session.clone(),
        retry_of: None,
        external_effect,
        finish_data: None,
    };
    Ok(PlannedLaunch {
        no: u64::try_from(index).map_or(u64::MAX, |index| index + 1),
        args: batch_args,
        argv: launch.argv(),
        info,
        request,
    })
}

fn texts(items: &[String]) -> Value {
    Value::List(items.iter().map(|item| Value::Text(item.clone())).collect())
}

fn track_mode_name(mode: TrackMode) -> &'static str {
    match mode {
        TrackMode::Auto => "auto",
        TrackMode::Scan => "scan",
        TrackMode::Events => "events",
        TrackMode::None => "none",
    }
}

/// Record shown by a preview for one process.
fn preview(launch: &PlannedLaunch, settings: &Settings, batched: bool) -> Record {
    let platform = settings.platform;
    let info = &launch.info;
    let stdin = match &settings.stdin {
        StdinSource::Null => "none",
        StdinSource::Bytes(_) => "text",
        StdinSource::File(_) => "file",
    };
    let wait = match settings.wait {
        Some(wait) => Value::Duration(record::duration(wait)),
        None => Value::Enum("forever".to_owned()),
    };
    let timeout = settings.limits.timeout.map_or(Value::Null, |timeout| {
        Value::Duration(record::duration(timeout))
    });
    let inherit = match settings.inherit {
        InheritEnv::All => "all",
        InheritEnv::None => "none",
    };
    let mut preview = Record::new(RecordKind::Map)
        .with("command", texts(&info.command))
        .with("program", record::preview_path(&info.program, platform))
        .with("argv", texts(&launch.argv))
        .with("cwd", record::preview_path(&info.cwd, platform))
        .with("launcher", Value::Enum(info.launcher.as_str().to_owned()))
        .with("inherit_env", Value::Enum(inherit.to_owned()))
        .with("env_keys", texts(&info.env_keys))
        .with("stdin", Value::Enum(stdin.to_owned()))
        .with("timeout", timeout)
        .with("wait", wait)
        .with(
            "watch",
            Value::List(
                settings
                    .track
                    .watch
                    .iter()
                    .map(|path| record::preview_path(path, platform))
                    .collect(),
            ),
        )
        .with(
            "track_changes",
            Value::Enum(track_mode_name(settings.track.mode).to_owned()),
        );
    if batched {
        preview = preview
            .with("batch", Value::from(launch.no))
            .with("batch_args", texts(&launch.args));
    }
    preview
}

fn one_record<'a>(record: Record) -> EffectOutcome<'a> {
    EffectOutcome::new(StageOutput::records(RecordStream::once(record)))
}

fn run_single<'a>(host: &dyn RunHost, ctx: &ExecCtx, state: RunPlan) -> Result<EffectOutcome<'a>> {
    let Some(launch) = state.launches.into_iter().next() else {
        return Ok(EffectOutcome::new(StageOutput::records(
            RecordStream::empty(),
        )));
    };
    let irreversible: Vec<_> = external_effect(&launch.request).into_iter().collect();
    match host.run(ctx, launch.request.clone(), state.wait)? {
        RunWait::Finished(result) => {
            let exec = exec_record(&launch.request, &result, launch.info.platform);
            if let Some(error) = record::termination_error(&launch.info, &result.end, &exec) {
                return Err(error);
            }
            let changes = result.track.changes.clone();
            let mut outcome = one_record(Record::from(exec));
            outcome.changes = changes;
            outcome.notable = record::notes(&result);
            outcome.irreversible = irreversible;
            Ok(outcome)
        }
        RunWait::Background(job) => {
            let mut outcome = one_record(Record::from(*job));
            outcome.detached = true;
            outcome
                .notable
                .push(NotableEntry::new(Notable::PostStagesDeferred));
            outcome.irreversible = irreversible;
            Ok(outcome)
        }
    }
}

/// The result of a run that has to end before the call returns.
fn finished(wait: RunWait) -> Result<Box<RunResult>> {
    match wait {
        RunWait::Finished(result) => Ok(result),
        RunWait::Background(job) => Err(Error::new(
            ErrorCode::StageFailed,
            format!(
                "batch job {} moved to the background although the run waits for every batch",
                job.id
            ),
        )),
    }
}

fn run_batches<'a>(
    host: &dyn RunHost,
    ctx: &ExecCtx,
    state: &RunPlan,
) -> Result<EffectOutcome<'a>> {
    let results = batch::run_ordered(&state.launches, state.parallel, state.bail, |launch| {
        let result = host
            .run(ctx, launch.request.clone(), None)
            .and_then(finished);
        let failed = result
            .as_ref()
            .map_or(true, |result| record::exit_code(&result.end) != 0);
        (result, failed)
    });
    let mut runs = Vec::new();
    let mut first_error = None;
    for (launch, result) in state.launches.iter().zip(&results) {
        match result {
            Some(Ok(result)) => runs.push(BatchRun {
                no: launch.no,
                args: launch.args.clone(),
                info: &launch.info,
                request: &launch.request,
                result,
            }),
            Some(Err(error)) => {
                first_error.get_or_insert_with(|| error.clone());
            }
            None => {}
        }
    }
    if let Some(error) = first_error {
        return Err(error.with_effect_applied(!runs.is_empty()));
    }
    let Some(exec) = record::combine(&state.command, &runs) else {
        return Ok(EffectOutcome::new(StageOutput::records(
            RecordStream::empty(),
        )));
    };
    if let Some(error) = runs
        .iter()
        .find_map(|run| record::termination_error(run.info, &run.result.end, &exec))
    {
        return Err(error);
    }
    let changes = exec.changes.clone();
    let mut notable: Vec<NotableEntry> = record::exit_note(exec.exit_code).into_iter().collect();
    let mut irreversible = Vec::new();
    for run in &runs {
        notable.extend(crate::jobs::end_notes(run.result));
        notable.extend(run.result.output.notes.iter().cloned());
        notable.extend(run.result.track.notes.iter().cloned());
        irreversible.extend(external_effect(run.request));
    }
    let mut outcome = one_record(Record::from(exec));
    outcome.changes = changes;
    outcome.notable = notable;
    outcome.irreversible = irreversible;
    Ok(outcome)
}

/// Runs a tool for the test and analysis commands under the rules of `run`.
///
/// # Errors
///
/// Fails with `E_SPAWN`, `E_PROGRAM_NOT_FOUND` or `E_ARG` when the tool cannot
/// be started, and with the error of the run host.
pub(crate) fn run_tool(
    ctx: &ExecCtx,
    host: &Arc<dyn RunHost>,
    run: ToolRun,
    wait: Option<Duration>,
) -> Result<ToolOutcome> {
    let config = ctx.config();
    let platform = Platform::of(ctx.host().info().os);
    spawn::check_cwd(&run.cwd)?;
    let env = ChildEnv::build(
        platform,
        ctx.host().env_vars(),
        InheritEnv::All,
        env_defaults(ctx),
        &run.env,
    )?;
    let launch = spawn::prepare_launch(&run.command, &run.cwd, &env)?;
    let limits = ProcessLimits {
        timeout: Some(run.timeout),
        idle_timeout: params::non_zero(config.run.idle_timeout.as_std()),
        kill_grace: config.run.kill_grace.as_std(),
        max_cpu: None,
        max_memory: None,
        priority: Priority::Normal,
    };
    let external_effect = effects_of(&effect_classifier(ctx)?, &run.command);
    let request = RunRequest {
        program: launch.program.clone(),
        env_keys: run.env.iter().map(|var| var.name.clone()).collect(),
        command: run.command,
        spawn: launch.request(&run.cwd, env, StdinSource::Null, false, limits),
        track: TrackSpec {
            mode: params::track_mode(config.run.track_changes),
            watch: run.watch,
            ignore: config.run.track_ignore.clone(),
            preserve: Vec::new(),
            exact: run.exact,
            preimage: config.run.preimage == nurt_contracts::config::PreimageMode::Auto,
            scan_hash: false,
            timeout: Some(run.timeout),
        },
        view: OutputView {
            show: ShowMode::Both,
            head: Some(config.output.head),
            tail: Some(config.output.tail),
            ..OutputView::default()
        },
        kind: match run.kind {
            ToolKind::Test => RunKind::Test,
            ToolKind::Analyze => RunKind::Analyze,
        },
        label: run.label,
        description: None,
        session: ctx.options().session.clone(),
        retry_of: None,
        external_effect,
        finish_data: run.finish_data,
    };
    Ok(match host.run(ctx, request.clone(), wait)? {
        RunWait::Finished(result) => ToolOutcome::Finished(Box::new(ToolResult {
            exec: exec_record(&request, &result, platform),
            stdout_file: result
                .output
                .stdout_file
                .clone()
                .or_else(|| result.output.output_file.clone()),
            stderr_file: result.output.stderr_file.clone(),
            op: result.op,
        })),
        RunWait::Background(job) => ToolOutcome::Background(job),
    })
}

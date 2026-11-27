// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sched::stages
// Responsibility:   The commands `schedule.create`, `schedule.list`, `schedule.remove`, `schedule.pause`, `schedule.resume`, `schedule.run` and `schedule.history`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The commands `schedule.create`, `schedule.list`, `schedule.remove`,
//! `schedule.pause`, `schedule.resume`, `schedule.run` and
//! `schedule.history`.
//!
//! The commands read the store under the state directory of the execution
//! context. They change definitions only through the transaction of the
//! operation journal and notify the service through the [`ServiceLink`] of
//! the context, when one is registered. Without a running service a schedule
//! is reported with `active: false, reason: service_not_running`.

mod change;
mod create;
mod read;
mod run;

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{Expect, TargetAction, TargetPath, TargetSpec};
use nurt_contracts::record::{Schedule, ScheduleRun, ScheduleRunState, ScheduleState, Trigger};
use nurt_contracts::stage::{BoundArgs, ExecCtx, StageImpl};
use nurt_contracts::value::{Instant, Record, Value};

pub use self::change::ChangeStage;
pub use self::create::Create;
pub use self::read::{History, List};
pub use self::run::Run;
use crate::errors::{conflict, failed, invalid};
use crate::module::{ModuleRequest, ServiceLink};
use crate::store::{Definition, Entry, RunRecord, RunState, SkipReason, Status, Store};
use crate::trigger::Trigger as CompiledTrigger;
use crate::trigger::time::instant_from_wall;

/// Reason given for a schedule that is not active because the service does
/// not run.
pub const SERVICE_NOT_RUNNING: &str = "service_not_running";

/// All schedule commands.
#[must_use]
pub fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(Create),
        Arc::new(List),
        Arc::new(ChangeStage::remove()),
        Arc::new(ChangeStage::pause()),
        Arc::new(ChangeStage::resume()),
        Arc::new(Run),
        Arc::new(History),
    ]
}

fn store(ctx: &ExecCtx) -> Result<Store, Error> {
    ctx.state_dir()
        .map(Store::new)
        .ok_or_else(|| failed("the execution context has no state directory for schedules"))
}

fn link(ctx: &ExecCtx) -> Option<Arc<dyn ServiceLink>> {
    ctx.extensions().get::<dyn ServiceLink>()
}

fn running_link(ctx: &ExecCtx) -> Option<Arc<dyn ServiceLink>> {
    link(ctx).filter(|link| link.running())
}

fn now(ctx: &ExecCtx) -> Instant {
    instant_from_wall(ctx.clock().now())
}

/// Tells a running service that definitions changed. The module also
/// detects changed files by itself, so a failed notification only delays the
/// change.
fn notify_changed(ctx: &ExecCtx) {
    if let Some(link) = running_link(ctx) {
        let _ = link.request(ModuleRequest::new(
            "schedule.changed",
            serde_json::json!({}),
        ));
    }
}

fn text_arg(args: &BoundArgs, name: &str) -> Result<Option<String>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(text) | Value::Enum(text)) => Ok(Some(text.clone())),
        Some(other) => Err(type_error(args, name, "text", other)),
    }
}

fn duration_arg(args: &BoundArgs, name: &str) -> Result<Option<std::time::Duration>, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Duration(duration)) if !duration.is_negative() => Ok(duration.to_std()),
        Some(Value::Duration(_)) => Err(invalid(format!("`{name}` must not be negative"))),
        Some(other) => Err(type_error(args, name, "duration", other)),
    }
}

fn bool_arg(args: &BoundArgs, name: &str, default: bool) -> Result<bool, Error> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(other) => Err(type_error(args, name, "bool", other)),
    }
}

fn ids_arg(args: &BoundArgs) -> Result<Vec<String>, Error> {
    match args.value("id") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Text(id)) => Ok(vec![id.clone()]),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Text(id) => Ok(id.clone()),
                other => Err(type_error(args, "id", "text", other)),
            })
            .collect(),
        Some(other) => Err(type_error(args, "id", "list<text>", other)),
    }
}

fn type_error(args: &BoundArgs, name: &str, expected: &str, found: &Value) -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "`{}` expects {expected} for `{name}`, not {}",
            args.command(),
            found.type_name()
        ),
    )
}

fn exactly_one(args: &BoundArgs, names: &[&str]) -> Result<String, Error> {
    let given: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| {
            args.value(name).is_some_and(|value| !value.is_null()) || args.pipeline(name).is_some()
        })
        .collect();
    match given.as_slice() {
        [one] => Ok((*one).to_owned()),
        [] => Err(Error::new(
            ErrorCode::MissingParam,
            format!(
                "`{}` needs exactly one of `{}`",
                args.command(),
                names.join("`, `")
            ),
        )),
        _ => Err(conflict(format!(
            "`{}` takes only one of `{}`; remove all but one",
            args.command(),
            given.join("`, `")
        ))
        .with_expected(serde_json::json!(names))),
    }
}

/// Targets that create the missing directories above `path`, outermost
/// first.
fn missing_dirs(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut missing: Vec<std::path::PathBuf> = path
        .ancestors()
        .skip(1)
        .take_while(|dir| !dir.exists())
        .map(std::path::Path::to_path_buf)
        .collect();
    missing.reverse();
    missing
}

fn target(
    path: &std::path::Path,
    action: TargetAction,
    expect: Expect,
) -> Result<TargetSpec, Error> {
    let path = AbsPath::new(path).map_err(|_| {
        failed(format!(
            "the schedule store `{}` is not an absolute path",
            path.display()
        ))
    })?;
    Ok(TargetSpec {
        path: TargetPath::Fs(path),
        action,
        expect,
        size_hint: None,
    })
}

/// State shown for a schedule: paused, or the state kept by the service.
fn shown_state(definition: &Definition, entry: Option<&Entry>) -> ScheduleState {
    if definition.paused {
        return ScheduleState::Paused;
    }
    match entry.map(|entry| entry.status) {
        Some(Status::Completed) => ScheduleState::Completed,
        Some(Status::Expired) => ScheduleState::Expired,
        _ => ScheduleState::Active,
    }
}

fn schedule_record(
    definition: &Definition,
    entry: Option<&Entry>,
    service_running: bool,
    now: Instant,
) -> Record {
    let state = shown_state(definition, entry);
    let reason = if !service_running {
        Some(SERVICE_NOT_RUNNING.to_owned())
    } else {
        match state {
            ScheduleState::Active => None,
            other => Some(other.as_str().to_owned()),
        }
    };
    let next_run = entry
        .and_then(|entry| entry.next_run)
        .or_else(|| {
            entry.is_none().then(|| {
                CompiledTrigger::compile(&definition.trigger)
                    .ok()
                    .and_then(|trigger| trigger.first_run(definition.created, now))
            })?
        })
        .or_else(|| entry.and_then(|entry| entry.last_run))
        .unwrap_or(definition.created);
    let record = Schedule {
        id: definition.id.clone(),
        trigger: Trigger {
            kind: definition.trigger.kind,
            expression: definition.trigger.expression.clone(),
            timezone: Some(definition.trigger.timezone.clone()),
        },
        next_run,
        active: service_running && state == ScheduleState::Active,
        action: definition.action.clone(),
        label: Some(definition.label.clone()),
        last_run: entry.and_then(|entry| entry.last_run),
        last_result: entry
            .and_then(|entry| entry.last_state)
            .map(|state| run_state(state, None)),
        state: Some(state),
        runs: entry.map(|entry| entry.run_count),
        max_runs: definition.max_runs,
        expires: definition.expires,
        on_missed: Some(definition.on_missed.as_str().to_owned()),
        misfire: Some(definition.misfire),
        overlap: Some(definition.overlap.as_str().to_owned()),
        timeout: Some(definition.timeout),
        created: Some(definition.created),
        session: definition.session.clone(),
        action_kind: Some(definition.action_kind.clone()),
        reason,
    };
    Record::from(record)
}

fn run_state(state: RunState, reason: Option<SkipReason>) -> ScheduleRunState {
    match state {
        RunState::Ok => ScheduleRunState::Ok,
        RunState::Timeout => ScheduleRunState::TimedOut,
        RunState::Stopped => ScheduleRunState::Cancelled,
        RunState::Skipped if reason == Some(SkipReason::Overlap) => {
            ScheduleRunState::SkippedOverlap
        }
        RunState::Skipped => ScheduleRunState::Missed,
        RunState::Exit | RunState::Rejected | RunState::Failed => ScheduleRunState::Failed,
    }
}

fn run_record(run: &RunRecord) -> Record {
    let mut message = format!(
        "#{} {} {}",
        run.run,
        run.trigger.as_str(),
        run.state.as_str()
    );
    if let Some(reason) = run.reason {
        message.push_str(&format!(" ({})", reason.as_str()));
    }
    if let Some(code) = run.exit_code {
        message.push_str(&format!(", exit code {code}"));
    }
    if let Some(text) = &run.message {
        message.push_str(&format!(": {text}"));
    }
    Record::from(ScheduleRun {
        schedule_id: run.schedule.clone(),
        started: run.started,
        exit_state: run_state(run.state, run.reason),
        trigger_time: run.scheduled_for.unwrap_or(run.started),
        elapsed: Some(run.elapsed),
        operation: run.operation,
        message: Some(message),
    })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

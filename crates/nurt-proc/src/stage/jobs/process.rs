// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::stage::jobs::process
// Responsibility:   The `stop`, `suspend` and `resume` commands on process records.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `stop`, `suspend` and `resume` commands on process records.
//!
//! A process record names a process of the operating system by its
//! identifier and its start time. The identifier alone can name another
//! process when the listed one ended and the system gave the identifier away,
//! so the start time of the process found under the identifier is compared
//! with the start time of the record before every signal. A process with
//! another start time is not signalled and the command fails with
//! `E_NOT_FOUND`. Job records are handled by [`super::control`] and service
//! records by the commands of the services.

mod system;
#[cfg(test)]
mod tests;

use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{Effect, EffectStatus, Process, Signal};
use nurt_contracts::report::IrreversibleEntry;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, RecordKind, Value};

use self::system::{Condition, Handle, same_start};
use super::args::duration;
use super::control::{Control, DEFAULT_GRACE, Kind};

/// Interval between two readings of the state of a process that is ending.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Time allowed for a process to end after it was killed.
const KILL_WAIT: Duration = Duration::from_secs(5);

/// Prefix of the target of an effect on a process, for example `process:4132`.
const TARGET_PREFIX: &str = "process:";

/// `stop`, `suspend` or `resume` on process records.
pub(crate) struct ProcessControl {
    kind: Kind,
}

impl ProcessControl {
    pub(crate) fn new(kind: Kind) -> Self {
        Self { kind }
    }

    /// Acts on the process of `target` and returns its state before and after.
    fn act(
        &self,
        ctx: &ExecCtx,
        target: &Target,
        signal: Signal,
        grace: Duration,
    ) -> Result<(Condition, Condition)> {
        let handle = target.open()?;
        let before = handle.condition();
        match self.kind {
            Kind::Stop => {
                if before != Condition::Ended {
                    stop(ctx, &handle, before, signal, grace)?;
                }
                Ok((before, Condition::Ended))
            }
            Kind::Suspend | Kind::Resume if before == Condition::Ended => Err(target.gone()),
            Kind::Suspend => {
                handle.suspend()?;
                Ok((before, Condition::Stopped))
            }
            Kind::Resume => {
                handle.resume()?;
                Ok((before, Condition::Running))
            }
        }
    }
}

impl StageImpl for ProcessControl {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.kind.name().to_owned(),
            source: false,
            input: InputKinds::Only(vec![RecordKind::Process]),
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        if self.kind == Kind::Stop {
            Control::signal(args)?;
            duration(args, "grace")?;
        }
        Ok(StaticInfo::default())
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for ProcessControl {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let mut plan = EffectPlan::new();
        let mut targets = Vec::new();
        for process in input_processes(input)? {
            let target = Target::of(&process)?;
            target.open()?;
            plan.preview.push(Record::from(process));
            targets.push(target);
        }
        plan.irreversible = targets
            .iter()
            .map(|target| marker(self.kind, target))
            .collect();
        plan.bulk = targets.len() > 1;
        Ok(plan.with_state(targets))
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let targets: Vec<Target> = plan.take_state().unwrap_or_default();
        let signal = Control::signal(args)?;
        let grace = duration(args, "grace")?.unwrap_or(DEFAULT_GRACE);
        let mut records = Vec::with_capacity(targets.len());
        let mut irreversible = Vec::with_capacity(targets.len());
        for target in &targets {
            let acted = self.act(ctx, target, signal, grace).map_err(|error| {
                let sent = error.data().effect_applied || !records.is_empty();
                error.with_effect_applied(sent)
            })?;
            let entry = marker(self.kind, target);
            tx.mark_irreversible(entry.clone());
            irreversible.push(entry.clone());
            records.push(Record::from(effect(self.kind, target, acted, entry)));
        }
        let mut outcome =
            EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
        outcome.irreversible = irreversible;
        Ok(outcome)
    }
}

/// Process named by an input record.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    pid: u32,
    /// Start time of the record in nanoseconds since the Unix epoch.
    started: i64,
    name: String,
}

impl Target {
    /// Target of `record`.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` for the identifier 0, which addresses a
    /// group of processes rather than one process, and for an identifier no
    /// system has.
    fn of(record: &Process) -> Result<Self> {
        let pid = u32::try_from(record.pid)
            .ok()
            .filter(|pid| *pid > 0 && i32::try_from(*pid).is_ok())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "the identifier {} does not name one process; give a record of `processes`",
                        record.pid
                    ),
                )
                .with_found(record.pid.to_string())
            })?;
        Ok(Self {
            pid,
            started: record.started.as_unix_nanos(),
            name: record.name.clone(),
        })
    }

    /// Opens the process and checks that it is the one of the record.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when the process ended or the identifier
    /// belongs to another process now.
    fn open(&self) -> Result<Handle> {
        let Some(handle) = Handle::open(self.pid)? else {
            return Err(self.gone());
        };
        if !same_start(Some(self.started), handle.started()) {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!(
                    "process {} ({}) ended and its identifier belongs to another process; list the processes again",
                    self.pid, self.name
                ),
            )
            .with_problem("identifier_reused")
            .with_found(self.pid.to_string()));
        }
        Ok(handle)
    }

    fn gone(&self) -> Error {
        Error::new(
            ErrorCode::NotFound,
            format!(
                "process {} ({}) has ended; list the processes again",
                self.pid, self.name
            ),
        )
        .with_problem("process_ended")
        .with_found(self.pid.to_string())
    }
}

/// Process records of the input.
fn input_processes(input: StageInput<'_>) -> Result<Vec<Process>> {
    input
        .into_records()?
        .map(|record| Process::try_from(record?).map_err(Error::from))
        .collect()
}

/// Sends `signal`, waits `grace` for the process to end and kills it when it
/// does not.
fn stop(
    ctx: &ExecCtx,
    handle: &Handle,
    before: Condition,
    signal: Signal,
    grace: Duration,
) -> Result<()> {
    handle.signal(signal)?;
    if before == Condition::Stopped && signal != Signal::Kill {
        // A suspended process receives the signal only after it continues.
        handle.resume()?;
    }
    if ended_within(ctx, handle, grace)? {
        return Ok(());
    }
    if signal != Signal::Kill {
        handle.signal(Signal::Kill)?;
    }
    if ended_within(ctx, handle, KILL_WAIT)? {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::Timeout,
        "the process did not end after it was killed; read its state with `processes`",
    )
    .with_effect_applied(true))
}

/// Waits up to `limit` for the process to end.
fn ended_within(ctx: &ExecCtx, handle: &Handle, limit: Duration) -> Result<bool> {
    let deadline = ctx.clock().monotonic().saturating_add(limit);
    loop {
        if handle.condition() == Condition::Ended {
            return Ok(true);
        }
        let now = ctx.clock().monotonic();
        if now >= deadline {
            return Ok(false);
        }
        ctx.sleep(POLL_INTERVAL.min(deadline.saturating_sub(now)))?;
    }
}

fn marker(kind: Kind, target: &Target) -> IrreversibleEntry {
    IrreversibleEntry {
        detail: Some(format!("process {}", target.pid)),
        ..IrreversibleEntry::new(kind.irreversible())
    }
}

/// Effect record of an action on one process.
fn effect(
    kind: Kind,
    target: &Target,
    states: (Condition, Condition),
    entry: IrreversibleEntry,
) -> Effect {
    let (before, after) = states;
    Effect {
        status: EffectStatus::Ok,
        kind: kind.name().to_owned(),
        targets: Some(1),
        irreversible: Some(vec![entry]),
        target: Some(format!("{TARGET_PREFIX}{}", target.pid)),
        action: Some(kind.name().to_owned()),
        before: Some(Value::Enum(before.as_str().to_owned())),
        after: Some(Value::Enum(after.as_str().to_owned())),
        ..Effect::default()
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `stop`, `suspend` and `resume` commands on job records.
//!
//! Process records are handled by [`super::process`] and service records by
//! the commands of the services in other crates; the registry selects this
//! implementation for job records.

use std::str::FromStr;
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{Effect, EffectStatus, Job, Signal};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, RecordKind};

use super::args::{duration, input_jobs, word};
use crate::iface::Components;
use crate::jobs::{Action, JobStore, check_action};

/// Time between the termination request and killing, by default.
pub(super) const DEFAULT_GRACE: Duration = Duration::from_secs(5);

/// Command implemented by [`Control`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Stop,
    Suspend,
    Resume,
}

impl Kind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Suspend => "suspend",
            Self::Resume => "resume",
        }
    }

    fn action(self) -> Action {
        match self {
            Self::Stop => Action::Stop,
            Self::Suspend => Action::Suspend,
            Self::Resume => Action::Resume,
        }
    }

    pub(super) fn irreversible(self) -> Irreversible {
        match self {
            Self::Stop => Irreversible::ProcessStopped,
            Self::Suspend | Self::Resume => Irreversible::ProcessStateChanged,
        }
    }
}

/// `stop`, `suspend` or `resume` on job records.
pub(crate) struct Control {
    kind: Kind,
    components: Components,
}

impl Control {
    pub(crate) fn new(kind: Kind, components: Components) -> Self {
        Self { kind, components }
    }

    pub(super) fn signal(args: &BoundArgs) -> Result<Signal> {
        let name = word(args, "signal")?.unwrap_or("terminate");
        Signal::from_str(name).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                "`stop` accepts `terminate`, `kill`, `interrupt` or `hangup` for `signal`",
            )
            .with_found(name)
        })
    }
}

impl StageImpl for Control {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.kind.name().to_owned(),
            source: false,
            input: InputKinds::Only(vec![RecordKind::Job]),
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        if self.kind == Kind::Stop {
            Self::signal(args)?;
            duration(args, "grace")?;
        }
        Ok(StaticInfo::default())
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Control {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let store = JobStore::open(ctx, &self.components)?;
        let mut plan = EffectPlan::new();
        let mut ids = Vec::new();
        for job in input_jobs(input)? {
            let file = store.get(&job.id)?;
            check_action(file.id(), file.record.state, self.kind.action())?;
            plan.preview.push(Record::from(store.record(&file)));
            ids.push(job.id);
        }
        plan.irreversible = ids.iter().map(|id| marker(self.kind, id)).collect();
        plan.bulk = ids.len() > 1;
        Ok(plan.with_state(ids))
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let ids: Vec<String> = plan.take_state().unwrap_or_default();
        let store = JobStore::open(ctx, &self.components)?;
        let signal = Self::signal(args)?;
        let grace = duration(args, "grace")?.unwrap_or(DEFAULT_GRACE);
        let mut records = Vec::with_capacity(ids.len());
        let mut irreversible = Vec::with_capacity(ids.len());
        for id in &ids {
            let job = match self.kind {
                Kind::Stop => store.stop(id, signal, grace)?,
                Kind::Suspend => store.suspend(id)?,
                Kind::Resume => store.resume(id)?,
            };
            let entry = marker(self.kind, id);
            tx.mark_irreversible(entry.clone());
            irreversible.push(entry.clone());
            records.push(Record::from(effect(self.kind.name(), job, entry)));
        }
        let mut outcome =
            EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
        outcome.irreversible = irreversible;
        Ok(outcome)
    }
}

fn marker(kind: Kind, id: &str) -> IrreversibleEntry {
    IrreversibleEntry {
        detail: Some(format!("job {id}")),
        ..IrreversibleEntry::new(kind.irreversible())
    }
}

/// Effect record of an action on one job.
pub(crate) fn effect(command: &str, job: Job, entry: IrreversibleEntry) -> Effect {
    Effect {
        status: EffectStatus::Ok,
        kind: command.to_owned(),
        targets: Some(1),
        irreversible: Some(vec![entry]),
        target: Some(format!("job:{}", job.id)),
        action: Some(command.to_owned()),
        job: Some(job),
        ..Effect::default()
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Value;

    use super::*;

    fn signal_of(name: Option<&str>) -> Result<Signal> {
        let mut args = BoundArgs::new("stop");
        if let Some(name) = name {
            args.insert("signal", ArgValue::Value(Value::Enum(name.to_owned())), false);
        }
        Control::signal(&args)
    }

    #[test]
    fn the_signal_defaults_to_terminate_and_must_have_a_known_name() {
        assert_eq!(signal_of(None).unwrap(), Signal::Terminate);
        assert_eq!(signal_of(Some("kill")).unwrap(), Signal::Kill);
        assert_eq!(signal_of(Some("interrupt")).unwrap(), Signal::Interrupt);
        assert_eq!(signal_of(Some("hangup")).unwrap(), Signal::Hangup);
        let error = signal_of(Some("bogus")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn every_command_has_its_name_action_and_irreversible_marker() {
        assert_eq!(Kind::Stop.name(), "stop");
        assert_eq!(Kind::Suspend.name(), "suspend");
        assert_eq!(Kind::Resume.name(), "resume");
        assert_eq!(Kind::Stop.action(), Action::Stop);
        assert_eq!(Kind::Suspend.action(), Action::Suspend);
        assert_eq!(Kind::Resume.action(), Action::Resume);
        assert_eq!(Kind::Stop.irreversible(), Irreversible::ProcessStopped);
        assert_eq!(
            Kind::Suspend.irreversible(),
            Irreversible::ProcessStateChanged
        );
        assert_eq!(
            Kind::Resume.irreversible(),
            Irreversible::ProcessStateChanged
        );
    }

    #[test]
    fn the_marker_and_the_effect_name_the_job() {
        let entry = marker(Kind::Stop, "j7");
        assert_eq!(entry.detail.as_deref(), Some("job j7"));
        assert_eq!(entry.reason, Irreversible::ProcessStopped);
        let job = Job {
            id: "j7".to_owned(),
            ..Job::default()
        };
        let record = effect("stop", job, entry);
        assert_eq!(record.target.as_deref(), Some("job:j7"));
        assert_eq!(record.action.as_deref(), Some("stop"));
        assert_eq!(record.kind, "stop");
        assert_eq!(record.targets, Some(1));
        assert_eq!(record.status, EffectStatus::Ok);
        assert_eq!(record.irreversible.map(|entries| entries.len()), Some(1));
        assert_eq!(record.job.map(|job| job.id), Some("j7".to_owned()));
    }

    #[test]
    fn the_descriptors_accept_only_job_records() {
        for kind in [Kind::Stop, Kind::Suspend, Kind::Resume] {
            let descriptor = StageImpl::descriptor(&Control::new(kind, crate::api::components()));
            assert_eq!(descriptor.command, kind.name());
            assert!(!descriptor.source);
            assert_eq!(descriptor.input, InputKinds::Only(vec![RecordKind::Job]));
        }
    }
}

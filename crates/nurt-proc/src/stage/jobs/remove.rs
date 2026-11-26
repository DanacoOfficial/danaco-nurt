// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `remove` command on job records: deletes finished jobs with their
//! kept output from the state directory.
//!
//! A job entry and its output are internal state of the console, not user
//! files, so the removal adds nothing to the journal transaction and marks no
//! irreversible effect. A job that still runs is refused with `E_JOB_STATE`.

use nurt_contracts::error::Result;
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{Effect, EffectStatus, Job};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind};

use super::args::input_jobs;
use crate::iface::Components;
use crate::jobs::{Action, JobStore, check_action};

/// Name of the command and of the action of its effect records.
const REMOVE: &str = "remove";

/// `remove` on job records.
pub(crate) struct Remove {
    components: Components,
}

impl Remove {
    pub(crate) fn new(components: Components) -> Self {
        Self { components }
    }
}

impl StageImpl for Remove {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: REMOVE.to_owned(),
            source: false,
            input: InputKinds::Only(vec![RecordKind::Job]),
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Remove {
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
            check_action(file.id(), file.record.state, Action::Remove)?;
            plan.preview.push(Record::from(store.record(&file)));
            ids.push(job.id);
        }
        plan.bulk = ids.len() > 1;
        Ok(plan.with_state(ids))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let ids: Vec<String> = plan.take_state().unwrap_or_default();
        let store = JobStore::open(ctx, &self.components)?;
        let mut records = Vec::with_capacity(ids.len());
        for id in &ids {
            records.push(Record::from(removed(store.remove(id)?)));
        }
        Ok(EffectOutcome::new(StageOutput::records(
            RecordStream::from_records(records),
        )))
    }
}

/// Effect record of a removed job, holding its last record.
fn removed(job: Job) -> Effect {
    Effect {
        status: EffectStatus::Ok,
        kind: REMOVE.to_owned(),
        targets: Some(1),
        target: Some(format!("job:{}", job.id)),
        action: Some(REMOVE.to_owned()),
        job: Some(job),
        ..Effect::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_descriptor_accepts_only_job_records_and_the_stage_changes_state() {
        let stage = Remove::new(crate::api::components());
        let descriptor = stage.descriptor();
        assert_eq!(descriptor.command, "remove");
        assert!(!descriptor.source);
        assert_eq!(descriptor.input, InputKinds::Only(vec![RecordKind::Job]));
        assert!(stage.as_effect().is_some());
    }

    #[test]
    fn the_effect_record_holds_the_last_record_of_the_job() {
        let job = Job {
            id: "j9".to_owned(),
            ..Job::default()
        };
        let record = removed(job);
        assert_eq!(record.status, EffectStatus::Ok);
        assert_eq!(record.kind, "remove");
        assert_eq!(record.action.as_deref(), Some("remove"));
        assert_eq!(record.target.as_deref(), Some("job:j9"));
        assert_eq!(record.targets, Some(1));
        assert!(record.irreversible.is_none());
        assert_eq!(record.job.map(|job| job.id), Some("j9".to_owned()));
    }
}

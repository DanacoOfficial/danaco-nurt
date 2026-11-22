// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `remove` command on job records: deletes finished jobs with their
//! kept output. The output is not kept in the journal, so the effect is
//! irreversible.

use nurt_contracts::error::Result;
use nurt_contracts::journal::MutationTx;
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind};

use super::args::input_jobs;
use super::control::effect;
use crate::iface::Components;
use crate::jobs::{Action, JobStore, check_action};

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
            command: "remove".to_owned(),
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
            plan.irreversible.push(marker(&job.id));
            ids.push(job.id);
        }
        plan.bulk = ids.len() > 1;
        Ok(plan.with_state(ids))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let ids: Vec<String> = plan.take_state().unwrap_or_default();
        let store = JobStore::open(ctx, &self.components)?;
        let mut records = Vec::with_capacity(ids.len());
        let mut irreversible = Vec::with_capacity(ids.len());
        for id in &ids {
            let job = store.remove(id)?;
            let entry = marker(id);
            tx.mark_irreversible(entry.clone());
            irreversible.push(entry.clone());
            records.push(Record::from(effect("remove", job, entry)));
        }
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
        outcome.irreversible = irreversible;
        Ok(outcome)
    }
}

fn marker(id: &str) -> IrreversibleEntry {
    IrreversibleEntry {
        detail: Some(format!("job {id}")),
        ..IrreversibleEntry::new(Irreversible::JobOutputRemoved)
    }
}

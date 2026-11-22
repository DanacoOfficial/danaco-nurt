// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `retry` command: runs the command of a finished job again.

use nurt_contracts::error::Result;
use nurt_contracts::journal::MutationTx;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, RecordKind};

use super::args::{Wait, duration, input_jobs, wait};
use crate::iface::{Components, RunWait};
use crate::jobs::{Action, JobStore, check_action, exec_record, run_host};

/// The `retry` command on job records.
pub(crate) struct Retry {
    components: Components,
}

impl Retry {
    pub(crate) fn new(components: Components) -> Self {
        Self { components }
    }
}

impl StageImpl for Retry {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "retry".to_owned(),
            source: false,
            input: InputKinds::Only(vec![RecordKind::Job]),
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        duration(args, "timeout")?;
        let waits = !matches!(wait(args)?, Wait::For(limit) if limit.is_zero());
        Ok(StaticInfo {
            waits,
            ..StaticInfo::default()
        })
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Retry {
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
            check_action(file.id(), file.record.state, Action::Retry)?;
            plan.preview.push(Record::from(store.record(&file)));
            ids.push(job.id);
        }
        plan.bulk = ids.len() > 1;
        Ok(plan.with_state(ids))
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let ids: Vec<String> = plan.take_state().unwrap_or_default();
        let store = JobStore::open(ctx, &self.components)?;
        let timeout = duration(args, "timeout")?;
        let wait = match wait(args)? {
            Wait::Default => Some(ctx.config().run.wait.as_std()),
            Wait::For(limit) => Some(limit),
            Wait::Forever => None,
        };
        let host = run_host(ctx, self.components.clone());
        let mut records = Vec::with_capacity(ids.len());
        let mut notable = Vec::new();
        let mut detached = false;
        for id in &ids {
            let request = store.retry_request(id, timeout)?;
            match host.run(ctx, request.clone(), wait)? {
                RunWait::Finished(result) => {
                    if let Some(code) = result.end.exit_code.filter(|code| *code != 0) {
                        notable.push(NotableEntry {
                            count: u64::try_from(code).ok(),
                            detail: Some(format!("retry of job {id}")),
                            ..NotableEntry::new(Notable::ExitNonzero)
                        });
                    }
                    notable.extend(result.track.notes.iter().cloned());
                    records.push(Record::from(exec_record(&request, &result)));
                }
                RunWait::Background(job) => {
                    detached = true;
                    records.push(Record::from(*job));
                }
            }
        }
        if detached {
            notable.push(NotableEntry::new(Notable::PostStagesDeferred));
        }
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
        outcome.notable = notable;
        outcome.detached = detached;
        Ok(outcome)
    }
}

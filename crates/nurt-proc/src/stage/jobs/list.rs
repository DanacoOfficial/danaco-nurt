// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `jobs` command: state, output and result of background jobs.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Job, JobState};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::Record;

use super::args::{count, duration, flag, pattern, word};
use crate::iface::{Components, OutputView, ShowMode};
use crate::jobs::JobStore;

/// The `jobs` command.
pub(crate) struct Jobs {
    components: Components,
}

impl Jobs {
    pub(crate) fn new(components: Components) -> Self {
        Self { components }
    }
}

/// Part of a job returned by `jobs` with an identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Status,
    Output,
    Result,
}

impl StageImpl for Jobs {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "jobs".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        view(args)?;
        state_filter(args)?;
        Ok(StaticInfo {
            waits: duration(args, "wait")?.is_some_and(|wait| !wait.is_zero()),
            ..StaticInfo::default()
        })
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Jobs {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let store = JobStore::open(ctx, &self.components)?;
        let Some(id) = word(args, "id")? else {
            let wanted = state_filter(args)?;
            let records: Vec<Record> = store
                .list()?
                .into_iter()
                .filter(|job| wanted.is_none_or(|state| job.state == state))
                .map(Record::from)
                .collect();
            return Ok(StageOutput::records(RecordStream::from_records(records)));
        };
        let file = match duration(args, "wait")? {
            Some(limit) if !limit.is_zero() => store.wait(id, limit)?,
            _ => store.get(id)?,
        };
        let output_view = output_view(args)?;
        let (job, header): (Job, Option<Record>) = match view(args)? {
            View::Status => (store.status(&file)?, None),
            View::Output => (
                store.output(&file, word(args, "since")?, &output_view)?,
                None,
            ),
            View::Result => store.result(&file, &output_view)?,
        };
        let output = StageOutput::records(RecordStream::once(Record::from(job)));
        Ok(match header {
            Some(header) => output.with_header(header),
            None => output,
        })
    }
}

fn view(args: &BoundArgs) -> Result<View> {
    match word(args, "view")?.unwrap_or("status") {
        "status" => Ok(View::Status),
        "output" => Ok(View::Output),
        "result" => Ok(View::Result),
        other => Err(Error::new(
            ErrorCode::InvalidValue,
            "`jobs` accepts `status`, `output` or `result` for `view`",
        )
        .with_found(other)),
    }
}

fn state_filter(args: &BoundArgs) -> Result<Option<JobState>> {
    match word(args, "state")?.unwrap_or("any") {
        "any" => Ok(None),
        other => other.parse::<JobState>().map(Some).map_err(|_| {
            Error::new(
                ErrorCode::InvalidValue,
                "`jobs` accepts `any` or a job state for `state`",
            )
            .with_found(other)
        }),
    }
}

fn output_view(args: &BoundArgs) -> Result<OutputView> {
    let show = match word(args, "stream")?.unwrap_or("both") {
        "stdout" => ShowMode::Stdout,
        "stderr" => ShowMode::Stderr,
        _ => ShowMode::Both,
    };
    Ok(OutputView {
        show,
        head: count(args, "head")?,
        tail: count(args, "tail")?,
        filter: pattern(args, "filter")?,
        context: count(args, "context")?.unwrap_or(0),
        dedupe: flag(args, "dedupe")?,
        ..OutputView::default()
    })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `test.show` command.

use nurt_contracts::error::{Candidate, Error, ErrorCode, Result};
use nurt_contracts::record::{TestResult, TestStatus};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use super::args;
use super::launch::Shape;
use super::output;
use super::run::{DEFAULT_FAILURES, DEFAULT_MESSAGE_LINES};
use crate::iface::{EntryKind, StoredEntry, VerifyStore, verify_store};

/// Values of the `status` parameter.
const STATUSES: [&str; 3] = ["failed", "passed", "skipped"];

pub(super) struct TestShow;

impl StageImpl for TestShow {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "test.show".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

/// The named run or the newest one.
fn run_entry(ctx: &ExecCtx, store: &dyn VerifyStore, run_id: Option<&str>) -> Result<StoredEntry> {
    let ids = store.ids(ctx, EntryKind::TestRun)?;
    let id = match run_id {
        Some(id) => id.to_owned(),
        None => ids.first().cloned().ok_or_else(|| {
            Error::new(ErrorCode::UnknownRun, "there are no test runs").with_fix("test.run")
        })?,
    };
    store.load(ctx, EntryKind::TestRun, &id)?.ok_or_else(|| {
        let mut error = Error::new(ErrorCode::UnknownRun, format!("unknown test run `{id}`"))
            .with_fix("test.list");
        for (rank, known) in (1_u32..).zip(ids.iter().take(3)) {
            error = error.with_candidate(Candidate {
                rank,
                reference: None,
                program: format!("test.show {known}"),
                why: None,
            });
        }
        error
    })
}

fn selected(status: Option<&str>, result: &TestResult) -> bool {
    match status {
        None => true,
        Some("failed") => matches!(result.status, TestStatus::Failed | TestStatus::Error),
        Some("passed") => result.status == TestStatus::Passed,
        Some("skipped") => matches!(result.status, TestStatus::Skipped | TestStatus::Todo),
        Some(_) => false,
    }
}

fn matches_test(needle: Option<&str>, result: &TestResult) -> bool {
    needle.is_none_or(|needle| {
        let needle = needle.to_lowercase();
        result.id.to_lowercase().contains(&needle) || result.name.to_lowercase().contains(&needle)
    })
}

impl ReadStage for TestShow {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let store = verify_store();
        let entry = run_entry(ctx, store.as_ref(), args::text(args, "run_id")?.as_deref())?;
        let status = args::text(args, "status")?;
        if let Some(unknown) = status
            .as_deref()
            .filter(|status| !STATUSES.contains(status))
        {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("unknown status `{unknown}`; use {}", STATUSES.join(", ")),
            )
            .with_fix("status: failed"));
        }
        let test = args::text(args, "test")?;
        let shape = Shape {
            failures: args::count(args, "failures", DEFAULT_FAILURES)?,
            message_lines: args::count(args, "message_lines", DEFAULT_MESSAGE_LINES)?,
            slowest: 0,
        };
        let results: Vec<TestResult> = entry
            .items
            .iter()
            .filter_map(|record| TestResult::try_from(record.clone()).ok())
            .filter(|result| selected(status.as_deref(), result))
            .filter(|result| matches_test(test.as_deref(), result))
            .collect();
        let records: Vec<Record> = output::shape(&results, shape)
            .into_iter()
            .map(Record::from)
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)).with_header(entry.header))
    }
}

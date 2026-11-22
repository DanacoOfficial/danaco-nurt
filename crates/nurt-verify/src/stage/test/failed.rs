// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `test.failed` command.

use nurt_contracts::error::{Candidate, Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{TestResult, TestStatus};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, Value};

use super::args;
use super::launch::{self, Prepared, Request};
use super::run::{DEFAULT_TIMEOUT, shape};
use crate::iface::{EntryKind, StoredEntry, VerifyStore, verify_store};
use crate::test::Framework;

pub(super) struct TestFailed;

impl StageImpl for TestFailed {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "test.failed".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

/// What the plan decided: run again, or report that nothing failed.
enum Rerun {
    Run(Prepared),
    Nothing(Record),
}

/// The run to repeat: the named one, or the newest run of the project with
/// failures that no later run repeated.
fn source_run(
    ctx: &ExecCtx,
    store: &dyn VerifyStore,
    run_id: Option<&str>,
    dir: Option<&str>,
) -> Result<StoredEntry> {
    let ids = store.ids(ctx, EntryKind::TestRun)?;
    if let Some(id) = run_id {
        return store
            .load(ctx, EntryKind::TestRun, id)?
            .ok_or_else(|| unknown_run(id, &ids));
    }
    let mut repeated: Vec<String> = Vec::new();
    for id in &ids {
        let Some(entry) = store.load(ctx, EntryKind::TestRun, id)? else {
            continue;
        };
        if dir.is_some_and(|dir| entry.header.get("cwd").and_then(Value::as_str) != Some(dir)) {
            continue;
        }
        if let Some(earlier) = entry.header.get("retry_of").and_then(Value::as_str) {
            repeated.push(earlier.to_owned());
        }
        let failed = entry.header.get("failed").and_then(Value::as_int).unwrap_or(0)
            + entry.header.get("errors").and_then(Value::as_int).unwrap_or(0);
        if failed > 0 && !repeated.contains(&entry.id) {
            return Ok(entry);
        }
    }
    Err(Error::new(ErrorCode::UnknownRun, "no test run with failures was found")
        .with_fix("test.list"))
}

fn unknown_run(id: &str, known: &[String]) -> Error {
    let mut error = Error::new(ErrorCode::UnknownRun, format!("unknown test run `{id}`"))
        .with_fix("test.list");
    for (rank, known) in (1_u32..).zip(known.iter().take(3)) {
        error = error.with_candidate(Candidate {
            rank,
            reference: None,
            program: format!("test.failed {known}"),
            why: None,
        });
    }
    error
}

fn failed_results(entry: &StoredEntry) -> Vec<TestResult> {
    entry
        .items
        .iter()
        .filter_map(|record| TestResult::try_from(record.clone()).ok())
        .filter(|result| matches!(result.status, TestStatus::Failed | TestStatus::Error))
        .collect()
}

fn rerun(ctx: &ExecCtx, args: &BoundArgs) -> Result<Rerun> {
    let store = verify_store();
    let run_id = args::text(args, "run_id")?;
    let dir_argument = args::path(args, "path")?;
    let dir_text = dir_argument
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let entry = source_run(ctx, store.as_ref(), run_id.as_deref(), dir_text.as_deref())?;
    let only = failed_results(&entry);
    if only.is_empty() {
        return Ok(Rerun::Nothing(entry.header));
    }
    let header = &entry.header;
    let stored_dir = header
        .get("cwd")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from);
    let dir = match (dir_argument, stored_dir) {
        (Some(dir), _) | (None, Some(dir)) => dir,
        (None, None) => args::base_dir(ctx)?,
    };
    let framework = header
        .get("framework")
        .and_then(Value::as_str)
        .and_then(Framework::from_name)
        .unwrap_or(Framework::Custom);
    let command: Vec<String> = header
        .get("command")
        .and_then(Value::as_list)
        .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let request = Request {
        dir,
        framework: Some(framework),
        targets: Vec::new(),
        filter: None,
        bail: false,
        only,
        command: (framework == Framework::Custom && !command.is_empty()).then_some(command),
        coverage: false,
        coverage_file: None,
        timeout: args::duration(args, "timeout")?.unwrap_or(DEFAULT_TIMEOUT),
        wait: args::duration(args, "wait")?,
        shape: shape(args, 0)?,
        retry_of: Some(entry.id),
    };
    Ok(Rerun::Run(launch::prepare(request)?))
}

impl EffectStage for TestFailed {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        match rerun(ctx, args)? {
            Rerun::Run(prepared) => Ok(launch::preview(ctx, &prepared)),
            Rerun::Nothing(header) => Ok(EffectPlan::new().with_state(header)),
        }
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        if let Some(prepared) = plan.take_state::<Prepared>() {
            return launch::execute(ctx, prepared);
        }
        let header = plan.take_state::<Record>().ok_or_else(|| {
            Error::new(ErrorCode::Internal, "the test rerun has no prepared state")
        })?;
        let output = StageOutput::records(RecordStream::empty()).with_header(header);
        Ok(EffectOutcome::new(output))
    }
}

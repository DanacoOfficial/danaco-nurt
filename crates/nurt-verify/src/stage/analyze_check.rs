// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `analyze.check` command.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{FindingKind, FindingSeverity};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use crate::analyze::check::{CheckRequest, perform};
use crate::analyze::scope::resolve;
use crate::analyze::verdict::{FailOn, Filter, select};
use crate::complexity::COMPLEXITY_MAX_DEFAULT;
use crate::iface::{EntryKind, StoredEntry, verify_store};
use crate::stage::analyze::{base_dir, bool_arg, count_arg, list_arg, severity_arg, text_arg};

/// Default number of findings returned.
const MAX_FINDINGS_DEFAULT: usize = 100;

/// Stage of `analyze.check`.
pub(crate) struct Check;

impl StageImpl for Check {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "analyze.check".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

fn request(args: &BoundArgs, dir: std::path::PathBuf, paths: Vec<String>) -> Result<CheckRequest> {
    let checks = list_arg(args, "checks");
    let checks = if checks.is_empty() {
        None
    } else {
        let parsed: std::result::Result<Vec<FindingKind>, _> =
            checks.iter().map(|name| name.parse()).collect();
        Some(parsed.map_err(|_| {
            Error::new(ErrorCode::Arg, "unknown check")
                .with_fix("use types, lint, format, complexity or duplication")
        })?)
    };
    let fail_on =
        text_arg(args, "fail_on").map_or(Some(FailOn::Error), |text| FailOn::parse(&text));
    Ok(CheckRequest {
        dir,
        paths,
        changed_only: bool_arg(args, "changed_only", false),
        checks,
        include_tests: bool_arg(args, "include_tests", true),
        complexity_max: u32::try_from(count_arg(
            args,
            "complexity_max",
            COMPLEXITY_MAX_DEFAULT as usize,
        ))
        .unwrap_or(COMPLEXITY_MAX_DEFAULT),
        fail_on: fail_on.ok_or_else(|| {
            Error::new(ErrorCode::Arg, "unknown fail_on value").with_fix("use error or warning")
        })?,
    })
}

impl EffectStage for Check {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        Ok(EffectPlan::new())
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        _plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let dir = base_dir(ctx)?;
        let paths = list_arg(args, "path");
        let request = request(args, dir, paths)?;
        let scope = resolve(ctx, &request.dir, &request.paths, request.changed_only)?;
        let store = verify_store();
        let id = store.next_id(ctx, EntryKind::Analysis)?;
        let outcome = perform(ctx, &request, &scope, &id)?;
        let header = Record::from(outcome.analysis.clone());
        store.save(
            ctx,
            EntryKind::Analysis,
            &StoredEntry {
                id,
                header: header.clone(),
                items: outcome.findings.iter().cloned().map(Record::from).collect(),
            },
        )?;
        let filter = Filter {
            min_severity: severity_arg(args, "min_severity", FindingSeverity::Info),
            check: None,
            file: None,
            offset: 0,
            limit: count_arg(args, "max_findings", MAX_FINDINGS_DEFAULT),
        };
        let (page, _) = select(&outcome.findings, &filter);
        let records: Vec<Record> = page.into_iter().cloned().map(Record::from).collect();
        let mut output = StageOutput::records(RecordStream::new(records.into_iter().map(Ok)));
        output.header = Some(header);
        let mut result = EffectOutcome::new(output);
        result.notable = outcome.notes;
        Ok(result)
    }
}

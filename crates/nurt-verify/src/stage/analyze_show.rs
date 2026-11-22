// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `analyze.show` command.

use nurt_contracts::error::{Candidate, Error, ErrorCode, Result};
use nurt_contracts::record::FindingSeverity;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use crate::analyze::verdict::{Filter, select};
use crate::iface::{EntryKind, verify_store};
use crate::stage::analyze::{count_arg, severity_arg, text_arg};

/// Default number of findings returned.
const MAX_FINDINGS_DEFAULT: usize = 100;

/// Most identifiers offered as candidates of an unknown analysis.
const CANDIDATES_MAX: usize = 5;

/// Stage of `analyze.show`.
pub(crate) struct Show;

impl StageImpl for Show {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "analyze.show".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Show {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let store = verify_store();
        let id = match text_arg(args, "analysis_id") {
            Some(id) => id,
            None => latest_id(ctx)?,
        };
        let Some(entry) = store.load(ctx, EntryKind::Analysis, &id)? else {
            return Err(unknown(ctx, &id));
        };
        let filter = Filter {
            min_severity: severity_arg(args, "min_severity", FindingSeverity::Info),
            check: text_arg(args, "check"),
            file: text_arg(args, "file"),
            offset: count_arg(args, "offset", 0),
            limit: count_arg(args, "max_findings", MAX_FINDINGS_DEFAULT),
        };
        let records: Vec<Record> = {
            let findings: Vec<_> = entry
                .items
                .iter()
                .filter_map(|record| nurt_contracts::record::Finding::try_from(record.clone()).ok())
                .collect();
            let (page, _) = select(&findings, &filter);
            page.into_iter().cloned().map(Record::from).collect()
        };
        let mut output = StageOutput::records(RecordStream::new(records.into_iter().map(Ok)));
        output.header = Some(entry.header);
        Ok(output)
    }
}

fn latest_id(ctx: &ExecCtx) -> Result<String> {
    let headers = verify_store().list(ctx, EntryKind::Analysis, 1)?;
    headers
        .first()
        .and_then(|record| record.get("id"))
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| {
            Error::new(ErrorCode::UnknownAnalysis, "no analysis has been stored")
                .with_fix("analyze.check")
        })
}

fn unknown(ctx: &ExecCtx, id: &str) -> Error {
    let mut error =
        Error::new(ErrorCode::UnknownAnalysis, format!("unknown analysis {id}")).with_found(id);
    let ids = verify_store()
        .ids(ctx, EntryKind::Analysis)
        .unwrap_or_default();
    for (rank, known) in ids.iter().rev().take(CANDIDATES_MAX).enumerate() {
        error = error.with_candidate(Candidate {
            rank: u32::try_from(rank + 1).unwrap_or(u32::MAX),
            reference: Some(known.clone()),
            program: format!("analyze.show {known}"),
            why: None,
        });
    }
    error
}

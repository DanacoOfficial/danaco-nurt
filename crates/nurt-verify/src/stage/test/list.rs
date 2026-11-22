// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `test.list` command.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};

use super::args;
use crate::iface::{EntryKind, verify_store};

/// Default number of runs listed.
const DEFAULT_LIMIT: usize = 20;

pub(super) struct TestList;

impl StageImpl for TestList {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "test.list".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for TestList {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let limit = args::count(args, "limit", DEFAULT_LIMIT)?;
        let limit = u32::try_from(limit).map_err(|_| {
            Error::new(ErrorCode::InvalidValue, "`limit` is too large").with_fix("limit: 20")
        })?;
        let headers = verify_store().list(ctx, EntryKind::TestRun, limit)?;
        Ok(StageOutput::records(RecordStream::from_records(headers)))
    }
}

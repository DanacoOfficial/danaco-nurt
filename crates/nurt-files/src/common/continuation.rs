// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Continuation of lines that a command cut to `max_line` characters.
//!
//! The command tells the continuation sink of the execution which lines of
//! which files it cut; `next full` reads those lines whole from the files.

use nurt_contracts::continuation::{
    ContinuationRequest, ContinuationSink, ContinuationSource, ContinuationUnit, CutLine,
};
use nurt_contracts::stage::ExecCtx;

/// Describes the cut lines to the sink of the execution, when it has one.
pub fn offer_cut_lines(ctx: &ExecCtx, cut: Vec<CutLine>) {
    if cut.is_empty() {
        return;
    }
    let Some(sink) = ctx.extensions().get::<ContinuationSink>() else {
        return;
    };
    sink.push(ContinuationRequest {
        id: None,
        cut_lines: u64::try_from(cut.len()).unwrap_or(u64::MAX),
        source: ContinuationSource::Records {
            rows: Vec::new(),
            cut,
            resume: None,
        },
        unit: ContinuationUnit::Records,
        shown: 0,
        remaining: 0,
    });
}

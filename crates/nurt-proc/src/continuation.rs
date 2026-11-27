// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Continuation of the shortened output of a run.
//!
//! The complete output stays in the spool file. A response that leaves lines
//! out or cuts long lines describes the remainder to the continuation sink of
//! the execution, and `next` reads it from the same file.

use nurt_contracts::continuation::{
    ContinuationRequest, ContinuationSink, ContinuationSource, ContinuationUnit, LineRange,
};
use nurt_contracts::stage::ExecCtx;

use crate::iface::RenderedOutput;

/// Describes what the response leaves out of `output` to the sink of the
/// execution, when the execution has one.
pub(crate) fn offer(ctx: &ExecCtx, output: &RenderedOutput) {
    let Some(cut) = &output.continuation else {
        return;
    };
    let Some(sink) = ctx.extensions().get::<ContinuationSink>() else {
        return;
    };
    let left = |first: u64, last: u64| last.saturating_add(1).saturating_sub(first);
    let remaining = cut
        .more
        .iter()
        .fold(left(cut.first_line, cut.last_line), |sum, part| {
            sum + left(part.first_line, part.last_line)
        });
    let cut_total = cut.cut.len() + cut.more.iter().map(|part| part.cut.len()).sum::<usize>();
    sink.push(ContinuationRequest {
        id: None,
        source: ContinuationSource::Lines {
            path: cut.path.display().to_string(),
            first_line: cut.first_line,
            last_line: cut.last_line,
            cut: cut.cut.clone(),
            next: cut
                .more
                .iter()
                .map(|part| LineRange {
                    path: part.path.display().to_string(),
                    first_line: part.first_line,
                    last_line: part.last_line,
                    cut: part.cut.clone(),
                })
                .collect(),
        },
        unit: ContinuationUnit::Lines,
        shown: cut.shown,
        remaining: i64::try_from(remaining).unwrap_or(i64::MAX),
        cut_lines: u64::try_from(cut_total).unwrap_or(u64::MAX),
    });
}

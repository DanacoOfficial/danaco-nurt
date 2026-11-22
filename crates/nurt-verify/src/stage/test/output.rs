// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Shaping of stored results for the output of a command.

use nurt_contracts::record::{TestResult, TestStatus};

use super::launch::Shape;

/// Results as shown: the failures beyond the detail limit lose their
/// message, and every message is cut to the line limit.
pub(super) fn shape(results: &[TestResult], shape: Shape) -> Vec<TestResult> {
    let mut detailed = 0_usize;
    results
        .iter()
        .map(|result| {
            let mut shown = result.clone();
            let failed = matches!(result.status, TestStatus::Failed | TestStatus::Error);
            if failed && detailed < shape.failures {
                detailed += 1;
                shown.message = shown.message.map(|text| head_lines(&text, shape.message_lines));
                shown.stack = shown.stack.map(|text| head_lines(&text, shape.message_lines));
                shown.stdout = shown.stdout.map(|text| head_lines(&text, shape.message_lines));
                shown.stderr = shown.stderr.map(|text| head_lines(&text, shape.message_lines));
            } else {
                shown.message = None;
                shown.stack = None;
                shown.stdout = None;
                shown.stderr = None;
            }
            shown
        })
        .collect()
}

/// First lines of a text.
pub(super) fn head_lines(text: &str, limit: usize) -> String {
    text.lines().take(limit).collect::<Vec<_>>().join("\n")
}

/// Identifiers and times of the slowest tests, slowest first.
pub(super) fn slowest(results: &[TestResult], count: usize) -> Vec<String> {
    let mut timed: Vec<(&TestResult, i64)> = results
        .iter()
        .filter_map(|result| Some((result, result.duration?.as_millis())))
        .filter(|(_, millis)| *millis > 0)
        .collect();
    timed.sort_by(|left, right| right.1.cmp(&left.1));
    timed
        .into_iter()
        .take(count)
        .map(|(result, millis)| format!("{}: {millis}ms", result.id))
        .collect()
}

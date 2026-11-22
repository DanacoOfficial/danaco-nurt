// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Shaping of stored results for the output of a command.

use nurt_contracts::record::{TestResult, TestStatus};

use super::finish::RUN_MARKER;
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
                let head_lines = if result.id == RUN_MARKER {
                    edge_lines
                } else {
                    head_lines
                };
                shown.message = shown
                    .message
                    .map(|text| head_lines(&text, shape.message_lines));
                shown.stack = shown
                    .stack
                    .map(|text| head_lines(&text, shape.message_lines));
                shown.stdout = shown
                    .stdout
                    .map(|text| head_lines(&text, shape.message_lines));
                shown.stderr = shown
                    .stderr
                    .map(|text| head_lines(&text, shape.message_lines));
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
/// Keeps the beginning and the end of a text of more than `limit` lines and
/// replaces the lines between them with a count. The end of the output of a
/// failed start holds the final error line.
pub(super) fn edge_lines(text: &str, limit: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= limit {
        return lines.join("\n");
    }
    let tail = (limit / 2).clamp(1, limit.max(1));
    let head = limit.saturating_sub(tail);
    let omitted = lines.len() - head - tail;
    let mut kept: Vec<String> = lines[..head]
        .iter()
        .map(|line| (*line).to_owned())
        .collect();
    kept.push(format!("[{omitted} lines omitted]"));
    kept.extend(
        lines[lines.len() - tail..]
            .iter()
            .map(|line| (*line).to_owned()),
    );
    kept.join("\n")
}

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
    timed.sort_by_key(|(_, millis)| std::cmp::Reverse(*millis));
    timed
        .into_iter()
        .take(count)
        .map(|(result, millis)| format!("{}: {millis}ms", result.id))
        .collect()
}

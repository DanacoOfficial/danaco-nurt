// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Renderer of the Markdown format.

use nurt_contracts::error::Error;
use nurt_contracts::report::{RecordsPage, Report};
use nurt_contracts::value::{Record, RecordKind};

use crate::facts::{
    SEPARATOR, bulk_directories, bulk_extensions, bulk_sample, change_cells, correction_line,
    error_facts, error_head, field_str, field_texts, header_parts, hunk_location,
    irreversible_parts, json_array, notable_parts, record_text, targets_parts, timing_parts,
    value_text,
};
use crate::lines::single_line;
use crate::shape::Shaped;
use crate::tree;
use crate::units::plural;

const FENCE: &str = "```";

/// Renders a shaped report as Markdown.
pub(crate) fn render(shaped: &Shaped) -> String {
    let report = &shaped.report;
    let mut blocks: Vec<String> = Vec::new();
    blocks.push(format!("### {}", header_parts(report).join(" ")));
    if !report.corrections.is_empty() {
        blocks.push(list(report.corrections.iter().map(correction_line)));
    }
    if let Some(ran) = &report.ran {
        blocks.push(format!("ran: {}", code(ran)));
    }
    if let Some(targets) = &report.targets {
        blocks.push(targets_parts(targets, report.summary.as_ref()).join(SEPARATOR));
    }
    changes_block(&mut blocks, shaped);
    diff_block(&mut blocks, shaped);
    bulk_block(&mut blocks, report);
    exec_block(&mut blocks, report);
    records_block(&mut blocks, report);
    for (label, record) in [
        ("job", &report.job),
        ("test", &report.test),
        ("analysis", &report.analysis),
        ("git", &report.git),
    ] {
        if let Some(record) = record {
            blocks.push(format!("**{label}** {}", code(&record_text(record))));
        }
    }
    if !report.notable.is_empty() {
        blocks.push(format!(
            "**notable**\n{}",
            list(
                report.notable.iter().map(|entry| {
                    format!("{} {}", entry.severity, notable_parts(entry).join(" "))
                })
            )
        ));
    }
    if !report.irreversible.is_empty() {
        blocks.push(format!(
            "**irreversible**\n{}",
            list(
                report
                    .irreversible
                    .iter()
                    .map(|entry| irreversible_parts(entry).join(" "))
            )
        ));
    }
    if let Some(error) = &report.error {
        blocks.push(error_block(error, "error"));
    }
    if let Some(cursor) = &report.next_cursor {
        blocks.push(format!("next cursor {}", code(cursor)));
    }
    if let Some(path) = &report.result_file {
        blocks.push(format!("result file {}", code(path)));
    }
    let timing = report.timing.map(timing_parts).unwrap_or_default();
    if !timing.is_empty() {
        blocks.push(format!("timing {}", timing.join(SEPARATOR)));
    }
    blocks.join("\n\n")
}

/// Bulleted list; an item is written on one line, so that a line break in a
/// value cannot end the item.
fn list(items: impl Iterator<Item = String>) -> String {
    items
        .map(|item| format!("- {}", single_line(&item)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Table cell: the text on one line, line breaks and control characters as
/// escape sequences, and the pipes escaped.
fn cell(text: &str) -> String {
    single_line(text).replace('|', "\\|")
}

/// Inline code span of a text written on one line, delimited by more
/// backticks than the longest run in the text.
fn code(text: &str) -> String {
    let text = single_line(text);
    let fence = "`".repeat(longest_backtick_run(&text) + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

fn table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut lines = vec![
        format!("| {} |", header.join(" | ")),
        format!("|{}", "---|".repeat(header.len())),
    ];
    for row in rows {
        let cells: Vec<String> = row.iter().map(|text| cell(text)).collect();
        lines.push(format!("| {} |", cells.join(" | ")));
    }
    lines.join("\n")
}

fn changes_block(blocks: &mut Vec<String>, shaped: &Shaped) {
    let report = &shaped.report;
    if report.changes.is_empty() {
        return;
    }
    let rows: Vec<Vec<String>> = report
        .changes
        .iter()
        .map(|change| {
            let cells = change_cells(change);
            vec![
                code(change.path.as_str()),
                cells.detail,
                cells.sizes,
                cells.hashes,
            ]
        })
        .collect();
    blocks.push(table(&["path", "change", "size", "hash"], &rows));
    if shaped.omitted.changes > 0 {
        blocks.push(format!(
            "{} not shown",
            plural(shaped.omitted.changes as u64, "change", "changes")
        ));
    }
}

fn diff_block(blocks: &mut Vec<String>, shaped: &Shaped) {
    let report = &shaped.report;
    if report.diff.is_empty() {
        return;
    }
    let shown = report.diff.len() as u64;
    let total = shown + shaped.omitted.hunks as u64;
    blocks.push(format!(
        "**diff** ({shown} of {})",
        plural(total, "hunk", "hunks")
    ));
    for hunk in &report.diff {
        let body = hunk.lines.join("\n");
        let fence = fence_for(&body);
        blocks.push(format!(
            "{}\n{fence}diff\n{body}\n{fence}",
            code(&hunk_location(hunk))
        ));
    }
}

/// Fence of a fenced block that holds `text`: at least three backticks and
/// longer than every run of backticks in the text, so the text cannot close
/// the block.
fn fence_for(text: &str) -> String {
    "`".repeat((longest_backtick_run(text) + 1).max(FENCE.len()))
}

/// Length of the longest run of backticks in `text`.
fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for character in text.chars() {
        if character == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

fn bulk_block(blocks: &mut Vec<String>, report: &Report) {
    let (Some(bulk), Some(targets)) = (&report.bulk, &report.targets) else {
        return;
    };
    let total = targets.files.unwrap_or(targets.count);
    let mut items = Vec::new();
    if let Some((_, text)) = bulk_directories(bulk, total) {
        items.push(format!("by directory: {text}"));
    }
    if let Some((_, text)) = bulk_extensions(bulk, total) {
        items.push(format!("by extension: {text}"));
    }
    if let Some((_, _, text)) = bulk_sample(bulk, total) {
        items.push(format!("sample: {text}"));
    }
    if !items.is_empty() {
        blocks.push(list(items.into_iter()));
    }
}

fn exec_block(blocks: &mut Vec<String>, report: &Report) {
    let Some(exec) = &report.exec else {
        return;
    };
    let mut items = Vec::new();
    if let Some(cwd) = field_str(exec, "cwd") {
        items.push(format!("cwd {}", code(cwd)));
    }
    items.push(format!(
        "command {}",
        code(&json_array(&field_texts(exec, "command")))
    ));
    for name in ["exit_code", "elapsed", "pid"] {
        if let Some(value) = exec.get(name) {
            items.push(format!("{name} {}", value_text(value)));
        }
    }
    blocks.push(list(items.into_iter()));
    for stream in ["stdout", "stderr"] {
        if let Some(text) = field_str(exec, stream).filter(|text| !text.is_empty()) {
            let body = text.trim_end_matches('\n');
            let fence = fence_for(body);
            blocks.push(format!("**{stream}**\n{fence}\n{body}\n{fence}"));
        }
    }
    if !report.changes.is_empty() {
        let tracking = field_str(exec, "tracking").unwrap_or("none");
        blocks.push(format!(
            "changes detected by {tracking}: {}",
            plural(report.changes.len() as u64, "file", "files")
        ));
    }
}

fn records_block(blocks: &mut Vec<String>, report: &Report) {
    let Some(page) = &report.records else {
        return;
    };
    let mut summary = vec![format!("**{}**", report.kind)];
    summary.extend(page.matched.map(|matched| format!("matched {matched}")));
    summary.push(format!("shown {}", page.rows.len()));
    blocks.push(summary.join(SEPARATOR));
    if let Some(header) = &page.header {
        blocks.push(code(&record_text(header)));
    }
    if page.rows.is_empty() {
        return;
    }
    if page.record_type == RecordKind::Entry && tree::is_tree(&page.rows) {
        let body = tree::draw(&page.rows).join("\n");
        let fence = fence_for(&body);
        blocks.push(format!("{fence}\n{body}\n{fence}"));
        return;
    }
    blocks.push(rows_table(page));
}

fn rows_table(page: &RecordsPage) -> String {
    let mut names: Vec<&str> = Vec::new();
    for row in &page.rows {
        for name in row.field_names() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    let rows: Vec<Vec<String>> = page.rows.iter().map(|row| row_cells(row, &names)).collect();
    table(&names, &rows)
}

fn row_cells(row: &Record, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .map(|name| row.get(name).map(value_text).unwrap_or_default())
        .collect()
}

fn error_block(error: &Error, label: &str) -> String {
    let mut text = format!("**{label} {}**", error_head(error).join(" "));
    for (name, value) in error_facts(error) {
        text.push_str(&format!("\n- {name}: {value}"));
    }
    if let Some(cause) = &error.cause {
        text.push_str("\n\n");
        text.push_str(&error_block(cause, "cause"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_has_three_backticks_for_text_without_backticks() {
        assert_eq!(fence_for(""), "```");
        assert_eq!(fence_for("a\nb"), "```");
        assert_eq!(fence_for("`code` and ``more``"), "```");
    }

    #[test]
    fn fence_is_longer_than_every_run_of_backticks_in_the_text() {
        assert_eq!(fence_for("before ``` after"), "````");
        assert_eq!(fence_for("`` a ````` b"), "``````");
    }

    #[test]
    fn pipes_and_line_breaks_of_a_cell_stay_in_the_cell() {
        assert_eq!(cell("a|b\nc\r\nd"), "a\\|b\\nc\\r\\nd");
    }

    #[test]
    fn a_code_span_is_one_line_and_outlasts_the_backticks_inside() {
        assert_eq!(code("a\nb"), "`a\\nb`");
        assert_eq!(code("a`b"), "``a`b``");
        assert_eq!(code("`a"), "`` `a ``");
    }
}

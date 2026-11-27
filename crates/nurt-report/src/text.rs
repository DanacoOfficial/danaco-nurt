// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Renderer of the text format.

use nurt_contracts::envelope::Verbosity;
use nurt_contracts::error::Error;
use nurt_contracts::record::Change;
use nurt_contracts::report::{RecordsPage, Report, ReportStatus, Severity};
use nurt_contracts::value::{Record, RecordKind};

use crate::facts::{
    SEPARATOR, bulk_directories, bulk_extensions, bulk_sample, change_cells, change_marker,
    correction_line, error_facts, error_head, field_millis, field_str, field_texts, field_u64,
    header_parts, hunk_location, irreversible_parts, json_array, marker_counts, notable_parts,
    record_text, targets_parts, timing_parts, value_text,
};
use crate::lines::{Lines, Style, single_line, visible_controls};
use crate::options::RenderOptions;
use crate::shape::Shaped;
use crate::tree;
use crate::units::{format_count, format_duration, format_size, plural};

const ELLIPSIS: &str = "\u{2026}";
const INDENT: &str = "  ";
const OUTPUT_HEAD_LINES: usize = 10;
const OUTPUT_TAIL_LINES: usize = 10;

/// Renders a shaped report as text.
pub(crate) fn render(shaped: &Shaped, options: &RenderOptions) -> String {
    let report = &shaped.report;
    let mut out = Lines::new();
    if !report.corrections.is_empty() {
        out.plain(format!("corrected: {}", report.corrections.len()));
    }
    for correction in &report.corrections {
        out.plain(format!(
            "{INDENT}{}",
            single_line(&correction_line(correction))
        ));
    }
    if let Some(ran) = &report.ran {
        out.plain(format!("ran: {ran}"));
    }
    header(&mut out, report);
    targets(&mut out, report);
    exec_block(&mut out, report, options);
    job_block(&mut out, report);
    records(&mut out, report, options);
    changes(&mut out, shaped, options.verbosity);
    diff(&mut out, shaped);
    bulk(&mut out, report);
    for (label, record) in [
        ("test", &report.test),
        ("analysis", &report.analysis),
        ("git", &report.git),
    ] {
        if let Some(record) = record {
            out.plain(format!("{label}  {}", single_line(&record_text(record))));
        }
    }
    notable(&mut out, report);
    irreversible(&mut out, report);
    if let Some(error) = &report.error {
        error_lines(&mut out, error, "", "error");
    }
    if report.records.is_none()
        && let Some(cursor) = &report.next_cursor
    {
        out.plain(single_line(&format!("next cursor {cursor}")));
    }
    if report.records.is_none()
        && let Some(continuation) = report.continuation.as_ref().filter(|c| c.has_more())
    {
        out.plain(continuation.more_text());
    }
    if let Some(path) = &report.result_file {
        out.plain(single_line(&format!("result file {path}")));
    }
    let timing = report.timing.map(timing_parts).unwrap_or_default();
    if !timing.is_empty() {
        out.plain(format!("timing {}", timing.join(SEPARATOR)));
    }
    out.finish(options.width, options.color)
}

fn status_style(status: ReportStatus) -> Option<Style> {
    match status {
        ReportStatus::Ok => Some(Style::Green),
        ReportStatus::Failed | ReportStatus::RolledBack => Some(Style::Red),
        ReportStatus::Planned | ReportStatus::Running | ReportStatus::Partial => {
            Some(Style::Yellow)
        }
        _ => None,
    }
}

fn header(out: &mut Lines, report: &Report) {
    let style = status_style(report.status);
    if let Some(page) = report
        .records
        .as_ref()
        .filter(|_| report.status == ReportStatus::Ok)
    {
        let line = format!("{}  {}", report.kind, read_summary(page, report));
        out.plain(line);
        return;
    }
    if report.status == ReportStatus::Failed && report.op.is_none() && report.error.is_some() {
        return;
    }
    let mut parts = header_parts(report);
    if let Some(exec) = &report.exec {
        parts.extend(exec_outcome(exec));
    }
    if report.status == ReportStatus::Running
        && let Some(elapsed) = report
            .job
            .as_ref()
            .and_then(|job| field_millis(job, "elapsed"))
    {
        parts.push(format!("(wait {} elapsed)", format_duration(elapsed)));
    }
    let text = parts.join("  ");
    match style {
        Some(style) => out.styled(text, style),
        None => out.plain(text),
    }
}

fn read_summary(page: &RecordsPage, report: &Report) -> String {
    let mut parts = Vec::new();
    if let Some(matched) = page.matched {
        parts.push(format!("matched {}", format_count(matched)));
    }
    parts.push(format!("shown {}", format_count(page.rows.len() as u64)));
    if report.continuation.is_none()
        && let Some(cursor) = &report.next_cursor
    {
        parts.push(format!("next cursor {cursor}"));
    }
    if let Some(continuation) = report.continuation.as_ref().filter(|c| c.has_more()) {
        parts.push(continuation.more_text());
    }
    parts.join(SEPARATOR)
}

fn exec_outcome(exec: &Record) -> Vec<String> {
    let mut parts = Vec::new();
    match (field_str(exec, "signal"), exec.get("exit_code")) {
        (Some(signal), _) => parts.push(format!("signal {signal}")),
        (None, Some(code)) => parts.push(format!("exit_code {}", value_text(code))),
        (None, None) => {}
    }
    for flag in ["timed_out", "stalled"] {
        if exec
            .get(flag)
            .is_some_and(|value| value_text(value) == "true")
        {
            parts.push(flag.to_owned());
        }
    }
    if let Some(elapsed) = field_millis(exec, "elapsed") {
        parts.push(format_duration(elapsed));
    }
    if let Some(pid) = field_u64(exec, "pid") {
        parts.push(format!("pid {pid}"));
    }
    parts
}

fn targets(out: &mut Lines, report: &Report) {
    if let Some(targets) = &report.targets
        && !(report.status == ReportStatus::Failed && report.op.is_none())
    {
        out.plain(targets_parts(targets, report.summary.as_ref()).join(SEPARATOR));
    }
}

fn exec_block(out: &mut Lines, report: &Report, options: &RenderOptions) {
    let Some(exec) = &report.exec else {
        return;
    };
    let mut place = Vec::new();
    if let Some(cwd) = field_str(exec, "cwd") {
        place.push(format!("cwd {}", single_line(cwd)));
    }
    place.push(format!(
        "command {}",
        json_array(&field_texts(exec, "command"))
    ));
    if let Some(encoding) = field_str(exec, "decoded_as") {
        place.push(format!("decoded {encoding}"));
    }
    out.plain(place.join("   "));
    out.plain(
        [
            stream_summary(exec, "stdout", true),
            stream_summary(exec, "stderr", false),
        ]
        .join(SEPARATOR),
    );
    let trim = options.verbosity != Verbosity::Detailed;
    for stream in ["stdout", "stderr"] {
        output_block(out, exec, stream, trim);
    }
    run_changes(out, report, exec);
}

fn stream_summary(exec: &Record, stream: &str, with_size: bool) -> String {
    let lines = field_u64(exec, &format!("{stream}_lines")).unwrap_or(0);
    let mut text = format!("{stream} {}", plural(lines, "line", "lines"));
    if with_size && let Some(bytes) = field_u64(exec, &format!("{stream}_bytes")) {
        text.push(' ');
        text.push_str(&format_size(bytes));
    }
    text
}

fn output_block(out: &mut Lines, exec: &Record, stream: &str, trim: bool) {
    let Some(text) = field_str(exec, stream).filter(|text| !text.is_empty()) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    let cut = trim && lines.len() > OUTPUT_HEAD_LINES + OUTPUT_TAIL_LINES;
    let mut caption = if cut {
        format!("{stream} head {OUTPUT_HEAD_LINES} / tail {OUTPUT_TAIL_LINES}")
    } else {
        stream.to_owned()
    };
    let stdout_empty = field_str(exec, "stdout").is_none_or(str::is_empty);
    if stream == "stdout" || stdout_empty {
        let mut sources = Vec::new();
        if let Some(file) = field_str(exec, "output_file") {
            sources.push(format!("full: {file}"));
        }
        if let Some(cursor) = field_str(exec, "output_cursor") {
            sources.push(format!("next page: cursor {cursor}"));
        }
        if !sources.is_empty() {
            caption.push_str(&format!(" ({})", sources.join("; ")));
        }
    }
    out.plain(format!("{INDENT}{caption}"));
    let shown: Vec<&str> = if cut {
        let mut kept = lines[..OUTPUT_HEAD_LINES].to_vec();
        kept.push(ELLIPSIS);
        kept.extend_from_slice(&lines[lines.len() - OUTPUT_TAIL_LINES..]);
        kept
    } else {
        lines
    };
    for line in shown {
        out.plain(format!("{INDENT}{INDENT}{line}"));
    }
}

fn run_changes(out: &mut Lines, report: &Report, exec: &Record) {
    if report.changes.is_empty() {
        return;
    }
    let mut head = String::from("changes");
    let mut scope = Vec::new();
    if let Some(tracking) = field_str(exec, "tracking") {
        scope.push(tracking.to_owned());
    }
    let watch = field_texts(exec, "watch");
    if !watch.is_empty() {
        scope.push(format!("watch {}", watch.join(", ")));
    }
    if !scope.is_empty() {
        head.push_str(&format!(" ({})", scope.join("; ")));
    }
    let count = report.changes.len() as u64;
    out.plain(
        [
            head,
            plural(count, "file", "files"),
            marker_counts(&report.changes),
        ]
        .join(SEPARATOR),
    );
    let rows: Vec<Vec<String>> = report.changes.iter().map(marker_row).collect();
    out.table(INDENT, &rows);
}

fn marker_row(change: &Change) -> Vec<String> {
    let cells = change_cells(change);
    vec![
        format!("{} {}", change_marker(change.action), change.path),
        cells.sizes,
        cells.hashes,
    ]
}

fn job_block(out: &mut Lines, report: &Report) {
    let Some(job) = &report.job else {
        return;
    };
    let mut parts = Vec::new();
    if let Some(id) = field_str(job, "id") {
        parts.push(format!("job {id}"));
    }
    if let Some(pid) = field_u64(job, "pid") {
        parts.push(format!("pid {pid}"));
    }
    parts.push(format!(
        "command {}",
        json_array(&field_texts(job, "command"))
    ));
    for (label, name) in [("cwd", "cwd"), ("started", "started")] {
        if let Some(value) = job.get(name) {
            parts.push(format!("{label} {}", value_text(value)));
        }
    }
    out.plain(single_line(&parts.join("  ")));
    if report.status == ReportStatus::Running
        && let Some(id) = field_str(job, "id")
    {
        out.plain(format!("read the output once with: jobs \"{id}\""));
    }
}

/// Columns of the table of a record type in the text view, in order. The
/// other fields of the records stay in the JSON form and appear in the text
/// with verbosity `detailed` or when the request names the `fields`.
fn default_columns(kind: RecordKind) -> Option<&'static [&'static str]> {
    match kind {
        RecordKind::Operation => Some(&[
            "id", "time", "kind", "state", "files", "bytes", "summary", "items",
        ]),
        RecordKind::Match => Some(&["path", "no", "column", "text", "count"]),
        RecordKind::Response => Some(&["url", "status", "bytes", "content_type", "elapsed"]),
        _ => None,
    }
}

fn records(out: &mut Lines, report: &Report, options: &RenderOptions) {
    let Some(page) = &report.records else {
        return;
    };
    if let Some(header) = &page.header {
        out.plain(single_line(&record_text(header)));
    }
    if page.rows.is_empty() {
        return;
    }
    if page.record_type == nurt_contracts::value::RecordKind::Entry && tree::is_tree(&page.rows) {
        for line in tree::draw(&page.rows) {
            out.plain(line);
        }
        return;
    }
    let mut names: Vec<&str> = Vec::new();
    for row in &page.rows {
        for name in row.field_names() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    let narrowed = options.fields.is_none() && options.verbosity != Verbosity::Detailed;
    if narrowed && let Some(columns) = default_columns(page.record_type) {
        let kept: Vec<&str> = columns
            .iter()
            .copied()
            .filter(|column| names.contains(column))
            .collect();
        if !kept.is_empty() {
            names = kept;
        }
    }
    let mut table = vec![
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>(),
    ];
    for row in &page.rows {
        table.push(
            names
                .iter()
                .map(|name| row.get(name).map(value_text).unwrap_or_default())
                .collect(),
        );
    }
    out.table("", &table);
}

fn changes(out: &mut Lines, shaped: &Shaped, verbosity: Verbosity) {
    let report = &shaped.report;
    if report.exec.is_some() {
        return;
    }
    if !report.changes.is_empty() {
        let rows: Vec<Vec<String>> = report
            .changes
            .iter()
            .map(|change| {
                let cells = change_cells(change);
                vec![
                    change.path.to_string(),
                    cells.detail,
                    cells.sizes,
                    cells.hashes,
                ]
            })
            .collect();
        out.table(INDENT, &rows);
    }
    if shaped.omitted.changes > 0 && verbosity != Verbosity::Quiet {
        let omitted = shaped.omitted.changes as u64;
        let noun = if omitted == 1 { "change" } else { "changes" };
        out.plain(format!(
            "{INDENT}{ELLIPSIS} {} more {noun}",
            format_count(omitted)
        ));
    }
}

fn diff(out: &mut Lines, shaped: &Shaped) {
    let report = &shaped.report;
    if report.diff.is_empty() {
        return;
    }
    let shown = report.diff.len() as u64;
    let total = shown + shaped.omitted.hunks as u64;
    if total > shown {
        out.plain(format!(
            "diff ({shown} of {})",
            plural(total, "hunk", "hunks")
        ));
    } else {
        out.plain(format!("diff ({})", plural(shown, "hunk", "hunks")));
    }
    for hunk in &report.diff {
        out.plain(format!("{INDENT}{}", single_line(&hunk_location(hunk))));
        for line in &hunk.lines {
            let mut characters = line.chars();
            let marker = characters.next().unwrap_or(' ');
            out.plain(format!(
                "{INDENT}{INDENT}{marker} {}",
                visible_controls(characters.as_str())
            ));
        }
    }
}

fn bulk(out: &mut Lines, report: &Report) {
    let (Some(bulk), Some(targets)) = (&report.bulk, &report.targets) else {
        return;
    };
    if let Some((count, text)) = bulk_directories(bulk, targets.files.unwrap_or(targets.count)) {
        out.plain(format!(
            "by directory (top {count}):  {}",
            single_line(&text)
        ));
    }
    if let Some((count, text)) = bulk_extensions(bulk, targets.files.unwrap_or(targets.count)) {
        out.plain(format!("by extension (top {count}):  {text}"));
    }
    if let Some((head, tail, text)) = bulk_sample(bulk, targets.files.unwrap_or(targets.count)) {
        out.plain(format!(
            "sample (first {head} / last {tail}): {}",
            single_line(&text)
        ));
    }
}

fn notable(out: &mut Lines, report: &Report) {
    if report.notable.is_empty() {
        return;
    }
    out.plain("notable");
    for entry in &report.notable {
        let (mark, style) = match entry.severity {
            Severity::Alert => ("!!", Some(Style::Red)),
            Severity::Warning => ("!", Some(Style::Yellow)),
            Severity::Notice => ("!", None),
            Severity::Info => ("-", Some(Style::Dim)),
        };
        let text = format!(
            "{INDENT}{mark} {}",
            single_line(&notable_parts(entry).join("  "))
        );
        match style {
            Some(style) => out.styled(text, style),
            None => out.plain(text),
        }
    }
}

fn irreversible(out: &mut Lines, report: &Report) {
    if report.irreversible.is_empty() {
        return;
    }
    out.plain("irreversible");
    for entry in &report.irreversible {
        out.plain(format!(
            "{INDENT}{}",
            single_line(&irreversible_parts(entry).join("  "))
        ));
    }
}

fn error_lines(out: &mut Lines, error: &Error, indent: &str, label: &str) {
    out.styled(
        single_line(&format!("{indent}{label} {}", error_head(error).join("  "))),
        Style::Red,
    );
    for (name, text) in error_facts(error) {
        out.plain(single_line(&format!("{indent}{INDENT}{name} {text}")));
    }
    if let Some(cause) = &error.cause {
        error_lines(out, cause, &format!("{indent}{INDENT}"), "cause");
    }
}

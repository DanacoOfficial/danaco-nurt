// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Statements about a report that the text, Markdown and compact renderers
//! have in common.

use nurt_contracts::error::Error;
use nurt_contracts::record::{Change, ChangeAction, Hunk};
use nurt_contracts::report::{
    Bulk, Correction, IrreversibleEntry, NotableEntry, Report, ReportStatus, Summary, Targets,
    Timing,
};
use nurt_contracts::value::{Record, Value};

use crate::units::{format_count, format_duration, format_size, plural};

/// Separator of the parts of a line.
pub(crate) const SEPARATOR: &str = " \u{b7} ";
const ARROW: &str = "\u{2192}";
const MINUS: char = '\u{2212}';
const ELLIPSIS: &str = "\u{2026}";
const HASH_DIGITS: usize = 8;

/// Writes a value the way it appears in a table cell or a statement.
pub(crate) fn value_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        Value::Int(number) => number.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Text(text)
        | Value::BigInt(text)
        | Value::Decimal(text)
        | Value::Path(text)
        | Value::Glob(text)
        | Value::Key(text)
        | Value::Enum(text) => text.clone(),
        Value::Size(size) => format_size(size.bytes()),
        Value::Duration(duration) => format_duration(duration.as_millis()),
        Value::Instant(instant) => instant.to_string(),
        Value::Date(date) => date.text.clone(),
        Value::Regex(regex) => regex.to_string(),
        Value::Range(range) => range.to_string(),
        Value::Bytes(bytes) => format!(
            "<{}>",
            plural(bytes.as_slice().len() as u64, "byte", "bytes")
        ),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(value_text).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Map(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(name, item)| format!("{name}: {}", value_text(item)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Record(record) => record_text(record),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|json| json.get("v").map(ToString::to_string))
            .unwrap_or_default(),
    }
}

/// Writes a record as its kind followed by `name value` pairs.
pub(crate) fn record_text(record: &Record) -> String {
    let parts: Vec<String> = record
        .fields
        .iter()
        .map(|(name, value)| format!("{name} {}", value_text(value)))
        .collect();
    format!("{}({})", record.kind, parts.join(", "))
}

/// Writes a JSON value as a statement: text unquoted, other values compact.
pub(crate) fn json_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Reads an unsigned count field of a record.
pub(crate) fn field_u64(record: &Record, name: &str) -> Option<u64> {
    match record.get(name)? {
        Value::Int(number) => u64::try_from(*number).ok(),
        Value::Size(size) => Some(size.bytes()),
        _ => None,
    }
}

/// Reads a text-like field of a record.
pub(crate) fn field_str<'a>(record: &'a Record, name: &str) -> Option<&'a str> {
    match record.get(name)? {
        Value::Text(text) | Value::Path(text) | Value::Enum(text) | Value::Key(text) => Some(text),
        _ => None,
    }
}

/// Reads a duration field of a record in milliseconds.
pub(crate) fn field_millis(record: &Record, name: &str) -> Option<i64> {
    match record.get(name)? {
        Value::Duration(duration) => Some(duration.as_millis()),
        _ => None,
    }
}

/// Reads a field that holds a list of texts.
pub(crate) fn field_texts(record: &Record, name: &str) -> Vec<String> {
    match record.get(name) {
        Some(Value::List(items)) => items.iter().map(value_text).collect(),
        _ => Vec::new(),
    }
}

/// Writes a list of texts as a JSON array, for example `["go","test"]`.
pub(crate) fn json_array(items: &[String]) -> String {
    serde_json::to_string(items).unwrap_or_default()
}

/// Shortens a hash such as `sha256:1a2b3c4d5e` to its first digits.
pub(crate) fn short_hash(hash: &str) -> &str {
    let digest = hash.split_once(':').map_or(hash, |(_, rest)| rest);
    digest.get(..HASH_DIGITS).unwrap_or(digest)
}

/// Writes the header line without the colors: `op 18  replace  ok`.
pub(crate) fn header_parts(report: &Report) -> Vec<String> {
    let mut parts = Vec::new();
    if report.status == ReportStatus::Planned {
        parts.push(format!("preview {}", report.kind));
        parts.push("(nothing changed)".to_owned());
        return parts;
    }
    if let Some(op) = report.op {
        parts.push(format!("op {op}"));
    }
    parts.push(report.kind.clone());
    parts.push(report.status.to_string());
    if report.bulk.is_some() {
        parts.push("(bulk)".to_owned());
    }
    parts
}

/// Writes the parts of the `targets` line.
pub(crate) fn targets_parts(targets: &Targets, summary: Option<&Summary>) -> Vec<String> {
    let mut parts = Vec::new();
    match targets.files {
        Some(files) => parts.push(format!("targets {}", plural(files, "file", "files"))),
        None => parts.push(format!("targets {}", format_count(targets.count))),
    }
    if let Some(directories) = targets.directories {
        parts.push(plural(directories, "dir", "dirs"));
    }
    if let Some(summary) = summary {
        parts.extend(summary_parts(summary));
    }
    parts
}

fn summary_parts(summary: &Summary) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(replacements) = summary.replacements {
        parts.push(format!("replacements {}", format_count(replacements)));
    }
    if summary.lines_added.is_some() || summary.lines_removed.is_some() {
        parts.push(format!(
            "lines +{} {MINUS}{}",
            format_count(summary.lines_added.unwrap_or(0)),
            format_count(summary.lines_removed.unwrap_or(0))
        ));
    }
    if let (Some(before), Some(after)) = (summary.bytes_before, summary.bytes_after) {
        parts.push(format!(
            "{} {ARROW} {}",
            format_size(before),
            format_size(after)
        ));
    }
    if let Some(freed) = summary.bytes_freed {
        parts.push(format!("{} freed", format_size(freed)));
    }
    if parts.is_empty() {
        parts.extend(count_parts(summary));
    }
    parts
}

/// Writes the counts of files by kind of change; used when the summary has
/// no other quantity to show.
fn count_parts(summary: &Summary) -> Vec<String> {
    let counts = [
        ("created", summary.files_created),
        ("modified", summary.files_modified),
        ("renamed", summary.files_renamed),
        ("deleted", summary.files_deleted),
        ("dirs created", summary.directories_created),
        ("dirs deleted", summary.directories_deleted),
    ];
    counts
        .into_iter()
        .filter_map(|(label, count)| count.map(|count| format!("{label} {}", format_count(count))))
        .collect()
}

/// Cells that describe one change in a table.
pub(crate) struct ChangeCells {
    /// Short statement of what happened, such as `3 repl` or `create`.
    pub(crate) detail: String,
    /// Sizes before and after.
    pub(crate) sizes: String,
    /// Hashes before and after.
    pub(crate) hashes: String,
}

/// Describes a change in cells.
pub(crate) fn change_cells(change: &Change) -> ChangeCells {
    let mut detail: Vec<String> = Vec::new();
    match change.action {
        ChangeAction::Modify => {}
        ChangeAction::Rename => detail.push(match &change.from {
            Some(from) => format!("rename from {from}"),
            None => "rename".to_owned(),
        }),
        action => detail.push(action.as_str().to_owned()),
    }
    if let Some(to) = &change.to {
        detail.push(format!("to {to}"));
    }
    if let Some(replacements) = change.replacements {
        detail.push(format!("{} repl", format_count(replacements)));
    } else if change.added.is_some() || change.removed.is_some() {
        detail.push(format!(
            "+{} {MINUS}{}",
            change.added.unwrap_or(0),
            change.removed.unwrap_or(0)
        ));
    }
    if let (Some(before), Some(after)) = (&change.before, &change.after) {
        detail.push(format!("{before} {ARROW} {after}"));
    }
    ChangeCells {
        detail: detail.join(" "),
        sizes: change_sizes(change),
        hashes: change_hashes(change),
    }
}

fn change_sizes(change: &Change) -> String {
    match (change.size_before, change.size_after) {
        (Some(before), Some(after)) => {
            format!(
                "{} {ARROW} {}",
                format_size(before.bytes()),
                format_size(after.bytes())
            )
        }
        (Some(before), None) => format_size(before.bytes()),
        (None, Some(after)) => format!("0 {ARROW} {}", format_size(after.bytes())),
        (None, None) => String::new(),
    }
}

fn change_hashes(change: &Change) -> String {
    match (&change.hash_before, &change.hash_after) {
        (Some(before), Some(after)) => {
            format!("{} {ARROW} {}", short_hash(before), short_hash(after))
        }
        (Some(hash), None) | (None, Some(hash)) => short_hash(hash).to_owned(),
        (None, None) => String::new(),
    }
}

/// Marker of a change in the list of changes made by a program.
pub(crate) fn change_marker(action: ChangeAction) -> &'static str {
    match action {
        ChangeAction::Create | ChangeAction::Link => "+",
        ChangeAction::Delete => "-",
        ChangeAction::Rename => ">",
        _ => "~",
    }
}

/// Writes the counts of the markers of a list of changes: `+1 ~1`.
pub(crate) fn marker_counts(changes: &[Change]) -> String {
    ["+", "~", "-", ">"]
        .into_iter()
        .filter_map(|marker| {
            let count = changes
                .iter()
                .filter(|change| change_marker(change.action) == marker)
                .count();
            (count > 0).then(|| format!("{marker}{count}"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Writes the location of a hunk: `path:line`.
pub(crate) fn hunk_location(hunk: &Hunk) -> String {
    format!("{}:{}", hunk.path, hunk.new_start)
}

/// Writes the parts of a line of a bulk summary and whether the list is cut.
pub(crate) fn bulk_directories(bulk: &Bulk, total: u64) -> Option<(usize, String)> {
    if bulk.by_directory.is_empty() {
        return None;
    }
    let mut counted = 0;
    let items: Vec<String> = bulk
        .by_directory
        .iter()
        .map(|entry| {
            counted += entry.count;
            let size = entry.bytes.map(|bytes| format!(" {}", format_size(bytes)));
            format!(
                "{} {}{}",
                entry.directory,
                format_count(entry.count),
                size.unwrap_or_default()
            )
        })
        .collect();
    Some((items.len(), join_cut(items, counted < total)))
}

/// Writes the line of the extensions of a bulk summary.
pub(crate) fn bulk_extensions(bulk: &Bulk, total: u64) -> Option<(usize, String)> {
    if bulk.by_extension.is_empty() {
        return None;
    }
    let mut counted = 0;
    let items: Vec<String> = bulk
        .by_extension
        .iter()
        .map(|entry| {
            counted += entry.count;
            let name = if entry.extension.is_empty() {
                "(none)"
            } else {
                entry.extension.as_str()
            };
            let size = entry.bytes.map(|bytes| format!(" {}", format_size(bytes)));
            format!(
                "{name} {}{}",
                format_count(entry.count),
                size.unwrap_or_default()
            )
        })
        .collect();
    Some((items.len(), join_cut(items, counted < total)))
}

/// Writes the sample of a bulk summary: head, a gap when targets are left
/// out, and tail.
pub(crate) fn bulk_sample(bulk: &Bulk, total: u64) -> Option<(usize, usize, String)> {
    if bulk.sample_head.is_empty() && bulk.sample_tail.is_empty() {
        return None;
    }
    let shown = (bulk.sample_head.len() + bulk.sample_tail.len()) as u64;
    let mut items = bulk.sample_head.clone();
    if shown < total {
        items.push(ELLIPSIS.to_owned());
    }
    items.extend(bulk.sample_tail.iter().cloned());
    Some((
        bulk.sample_head.len(),
        bulk.sample_tail.len(),
        items.join(SEPARATOR),
    ))
}

fn join_cut(mut items: Vec<String>, cut: bool) -> String {
    if cut {
        items.push(ELLIPSIS.to_owned());
    }
    items.join(SEPARATOR)
}

/// Writes the parts of the line of a notable effect.
pub(crate) fn notable_parts(entry: &NotableEntry) -> Vec<String> {
    let mut parts = vec![entry.code.to_string()];
    parts.extend(entry.path.clone());
    if let Some(count) = entry.count {
        parts.push(format_count(count));
    }
    parts.extend(entry.detail.clone());
    parts
}

/// Writes the parts of the line of an irreversible effect.
pub(crate) fn irreversible_parts(entry: &IrreversibleEntry) -> Vec<String> {
    let mut parts = vec![entry.reason.to_string()];
    parts.extend(entry.path.clone());
    if let Some(count) = entry.count {
        parts.push(format_count(count));
    }
    parts.extend(entry.detail.as_ref().map(|detail| format!("({detail})")));
    parts
}

/// Writes a correction: `C1 L01  1:1  remvoe -> remove`.
pub(crate) fn correction_line(correction: &Correction) -> String {
    let place = correction
        .at
        .map(|at| format!("{}:{}", at.line, at.column))
        .unwrap_or_default();
    let alternatives = if correction.alternatives.is_empty() {
        String::new()
    } else {
        format!("  (or: {})", correction.alternatives.join(", "))
    };
    format!(
        "{} {}  {place}  {} {ARROW} {}{alternatives}",
        correction.id, correction.rule, correction.from, correction.to
    )
}

/// Labelled statements about an error, in the order of display.
pub(crate) fn error_facts(error: &Error) -> Vec<(String, String)> {
    let mut facts = Vec::new();
    if let Some(location) = &error.location {
        let mut place = location.path.clone();
        for part in [location.line, location.column].into_iter().flatten() {
            place.push_str(&format!(":{part}"));
        }
        facts.push(("location".to_owned(), place));
    }
    let singles = [
        ("found", &error.found),
        ("problem", &error.problem),
        ("help", &error.help),
    ];
    for (label, value) in singles {
        facts.extend(value.iter().map(|value| (label.to_owned(), value.clone())));
    }
    for (label, value) in [("expected", &error.expected), ("actual", &error.actual)] {
        facts.extend(
            value
                .iter()
                .map(|value| (label.to_owned(), json_text(value))),
        );
    }
    facts.extend(error.fix.iter().map(|fix| ("fix".to_owned(), fix.clone())));
    facts.extend(guidance_facts(error));
    facts
}

/// Writes the facts of an error about candidates, further calls and the
/// state of the effect.
fn guidance_facts(error: &Error) -> Vec<(String, String)> {
    let mut facts = Vec::new();
    if !error.sample.is_empty() {
        facts.push((
            format!("sample ({})", error.sample.len()),
            error.sample.join(SEPARATOR),
        ));
    }
    for candidate in &error.candidates {
        let reference = candidate.reference.as_deref().unwrap_or("-");
        let why = candidate
            .why
            .as_ref()
            .map(|why| format!("  ({why})"))
            .unwrap_or_default();
        facts.push((
            "candidate".to_owned(),
            format!("{} {reference}  {}{why}", candidate.rank, candidate.program),
        ));
    }
    facts.extend(
        error
            .next_steps
            .iter()
            .map(|step| ("next".to_owned(), step.clone())),
    );
    if error.effect_applied {
        facts.push(("effect".to_owned(), "applied".to_owned()));
    }
    for (name, value) in &error.details {
        facts.push((format!("detail {name}"), json_text(value)));
    }
    facts
}

/// Writes the durations of the processing steps, for example `parse 1ms`.
///
/// Returns no part for a timing without any step.
pub(crate) fn timing_parts(timing: Timing) -> Vec<String> {
    [
        ("parse", timing.parse_ms),
        ("read", timing.read_ms),
        ("plan", timing.plan_ms),
        ("apply", timing.apply_ms),
    ]
    .into_iter()
    .filter_map(|(label, millis)| {
        millis.map(|millis| {
            let millis = i64::try_from(millis).unwrap_or(i64::MAX);
            format!("{label} {}", format_duration(millis))
        })
    })
    .collect()
}

/// Writes the header of an error: `E_ASSERT  stage 3  at 1:50  message`.
pub(crate) fn error_head(error: &Error) -> Vec<String> {
    let mut parts = vec![error.code.to_string()];
    if let Some(at) = error.at {
        if let Some(stage) = at.stage {
            parts.push(format!("stage {stage}"));
        }
        if let (Some(line), Some(column)) = (at.line, at.column) {
            parts.push(format!("at {line}:{column}"));
        }
    }
    parts.push(error.message.clone());
    parts
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::{ErrorCode, Stage};
    use nurt_contracts::value::{Duration, Size};

    use super::*;

    #[test]
    fn writes_typed_values_in_notation() {
        assert_eq!(value_text(&Value::Size(Size::from_bytes(1127))), "1.1kib");
        assert_eq!(
            value_text(&Value::Duration(Duration::from_millis(12_400))),
            "12.4s"
        );
        assert_eq!(value_text(&Value::Null), "null");
        let list = Value::List(vec![Value::Int(1), Value::Text("a".to_owned())]);
        assert_eq!(value_text(&list), "[1, a]");
    }

    #[test]
    fn shortens_hashes_to_eight_digits() {
        assert_eq!(short_hash("sha256:1a2b3c4d5e6f"), "1a2b3c4d");
        assert_eq!(short_hash("ab12"), "ab12");
    }

    #[test]
    fn describes_a_replacement_change() {
        let mut change = Change::new("/a.go", ChangeAction::Modify);
        change.replacements = Some(3);
        change.size_before = Some(Size::from_bytes(1127));
        change.size_after = Some(Size::from_bytes(1131));
        change.hash_before = Some("sha256:1a2b3c4d".to_owned());
        change.hash_after = Some("sha256:9f8e7d6c".to_owned());
        let cells = change_cells(&change);
        assert_eq!(cells.detail, "3 repl");
        assert_eq!(cells.sizes, "1.1kib \u{2192} 1.1kib");
        assert_eq!(cells.hashes, "1a2b3c4d \u{2192} 9f8e7d6c");
    }

    #[test]
    fn describes_creation_and_deletion_sizes() {
        let mut created = Change::new("/a", ChangeAction::Create);
        created.size_after = Some(Size::from_bytes(2150));
        assert_eq!(change_cells(&created).sizes, "0 \u{2192} 2.1kib");
        let mut deleted = Change::new("/b", ChangeAction::Delete);
        deleted.size_before = Some(Size::from_bytes(12));
        let cells = change_cells(&deleted);
        assert_eq!(cells.detail, "delete");
        assert_eq!(cells.sizes, "12b");
    }

    #[test]
    fn counts_markers_of_changes() {
        let changes = [
            Change::new("/a", ChangeAction::Create),
            Change::new("/b", ChangeAction::Modify),
            Change::new("/c", ChangeAction::Modify),
        ];
        assert_eq!(marker_counts(&changes), "+1 ~2");
    }

    #[test]
    fn writes_error_head_with_stage_and_place() {
        let error = Error::new(ErrorCode::Assert, "assert failed")
            .with_at(nurt_contracts::error::At::new(1, 50, 16).with_stage(Stage::Check));
        assert_eq!(
            error_head(&error),
            ["E_ASSERT", "stage 3", "at 1:50", "assert failed"]
        );
    }

    #[test]
    fn lists_error_facts_in_display_order() {
        let error = Error::new(ErrorCode::Assert, "m")
            .with_expected(serde_json::json!(12))
            .with_actual(serde_json::json!("14 files"))
            .with_sample("/a")
            .with_next_step("narrow the glob")
            .with_effect_applied(true);
        let labels: Vec<String> = error_facts(&error)
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        assert_eq!(
            labels,
            ["expected", "actual", "sample (1)", "next", "effect"]
        );
    }
}

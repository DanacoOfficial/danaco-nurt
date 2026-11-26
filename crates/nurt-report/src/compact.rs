// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Renderer of the compact format: one record per line, no headings.

use nurt_contracts::envelope::Verbosity;
use nurt_contracts::error::Error;
use nurt_contracts::value::Record;
use serde::Serialize;

use crate::facts::{error_facts, json_text, short_hash, value_text};
use crate::lines::single_line;
use crate::options::RenderOptions;
use crate::shape::Shaped;

/// Renders a shaped report as one line per record.
pub(crate) fn render(shaped: &Shaped, options: &RenderOptions) -> String {
    let report = &shaped.report;
    let mut lines: Vec<String> = Vec::new();
    let mut head = vec![
        "report".to_owned(),
        pair("kind", &report.kind),
        pair("status", report.status.as_str()),
    ];
    head.extend(report.op.map(|op| pair("op", &op.to_string())));
    head.extend(report.ran.as_ref().map(|ran| pair("ran", ran)));
    lines.push(head.join(" "));
    for correction in &report.corrections {
        lines.push(object_line("correction", correction));
    }
    lines.extend(
        report
            .targets
            .map(|targets| object_line("targets", &targets)),
    );
    lines.extend(
        report
            .summary
            .map(|summary| object_line("summary", &summary)),
    );
    body(&mut lines, shaped, options);
    for entry in &report.notable {
        lines.push(object_line("notable", entry));
    }
    for entry in &report.irreversible {
        lines.push(object_line("irreversible", entry));
    }
    if let Some(error) = &report.error {
        error_block(&mut lines, error, "error");
    }
    lines.extend(
        report
            .next_cursor
            .as_ref()
            .map(|cursor| format!("next {}", pair("cursor", cursor))),
    );
    lines.extend(
        report
            .result_file
            .as_ref()
            .map(|path| format!("result {}", pair("file", path))),
    );
    lines.extend(report.timing.map(|timing| object_line("timing", &timing)));
    lines.join("\n")
}

fn body(lines: &mut Vec<String>, shaped: &Shaped, options: &RenderOptions) {
    let report = &shaped.report;
    for change in &report.changes {
        lines.push(change_line(change));
    }
    if options.verbosity != Verbosity::Quiet && shaped.omitted != crate::shape::Omitted::default() {
        lines.push(format!(
            "omitted changes={} hunks={} rows={}",
            shaped.omitted.changes, shaped.omitted.hunks, shaped.omitted.rows
        ));
    }
    for hunk in &report.diff {
        for line in &hunk.lines {
            lines.push(format!(
                "diff {}:{} {line}",
                single_line(hunk.path.as_str()),
                hunk.new_start
            ));
        }
    }
    lines.extend(report.bulk.as_ref().map(|bulk| object_line("bulk", bulk)));
    for record in [
        &report.exec,
        &report.job,
        &report.test,
        &report.analysis,
        &report.git,
    ]
    .into_iter()
    .flatten()
    {
        lines.push(record_line(record));
    }
    if let Some(page) = &report.records {
        lines.extend(page.header.iter().map(record_line));
        lines.extend(page.rows.iter().map(record_line));
    }
}

fn change_line(change: &nurt_contracts::record::Change) -> String {
    let mut parts = vec![
        "change".to_owned(),
        change.action.as_str().to_owned(),
        token(change.path.as_str()),
    ];
    if let Some(from) = &change.from {
        parts.push(pair("from", from.as_str()));
    }
    if let Some(to) = &change.to {
        parts.push(pair("to", to.as_str()));
    }
    let size = |value: Option<nurt_contracts::value::Size>| {
        value.map_or_else(|| "-".to_owned(), |size| size.bytes().to_string())
    };
    if change.size_before.is_some() || change.size_after.is_some() {
        parts.push(format!(
            "size={}>{}",
            size(change.size_before),
            size(change.size_after)
        ));
    }
    let hash = |value: &Option<String>| value.as_deref().map_or("-", short_hash).to_owned();
    if change.hash_before.is_some() || change.hash_after.is_some() {
        parts.push(format!(
            "hash={}>{}",
            hash(&change.hash_before),
            hash(&change.hash_after)
        ));
    }
    for (label, count) in [
        ("repl", change.replacements),
        ("added", change.added),
        ("removed", change.removed),
    ] {
        parts.extend(count.map(|count| format!("{label}={count}")));
    }
    parts.join(" ")
}

fn record_line(record: &Record) -> String {
    let mut parts = vec![record.kind.to_string()];
    parts.extend(
        record
            .fields
            .iter()
            .map(|(name, value)| pair(name, &value_text(value))),
    );
    parts.join(" ")
}

fn error_block(lines: &mut Vec<String>, error: &Error, label: &str) {
    let mut parts = vec![label.to_owned(), pair("code", error.code.as_str())];
    if let Some(stage) = error.stage() {
        parts.push(pair("stage", stage.as_str()));
    }
    parts.push(pair("message", &error.message));
    lines.push(parts.join(" "));
    for (name, text) in error_facts(error) {
        lines.push(format!(
            "{label}.{} {}",
            name.replace(' ', "_"),
            token(&text)
        ));
    }
    if let Some(cause) = &error.cause {
        error_block(lines, cause, "cause");
    }
}

/// Writes a serializable object as a tag followed by `key=value` pairs.
fn object_line<T: Serialize>(tag: &str, value: &T) -> String {
    let mut parts = vec![tag.to_owned()];
    if let Ok(serde_json::Value::Object(map)) = serde_json::to_value(value) {
        for (key, item) in &map {
            parts.push(pair(key, &flatten(item)));
        }
    }
    parts.join(" ")
}

fn flatten(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(items) => items.iter().map(flatten).collect::<Vec<_>>().join(","),
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(key, item)| format!("{key}:{}", flatten(item)))
            .collect::<Vec<_>>()
            .join(","),
        other => json_text(other),
    }
}

fn pair(key: &str, value: &str) -> String {
    format!("{key}={}", token(value))
}

/// Quotes a value that is empty or contains a space, a control character, a
/// quote or an equals sign.
fn token(value: &str) -> String {
    let plain = !value.is_empty()
        && !value.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || character == '"'
                || character == '='
        });
    if plain {
        value.to_owned()
    } else {
        serde_json::to_string(value).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_values_with_spaces_quotes_or_equals_signs() {
        assert_eq!(token("plain"), "plain");
        assert_eq!(token("two words"), "\"two words\"");
        assert_eq!(token("a=b"), "\"a=b\"");
        assert_eq!(token(""), "\"\"");
        assert_eq!(token("a\u{1}b"), "\"a\\u0001b\"");
    }

    #[test]
    fn flattens_nested_values() {
        let value = serde_json::json!({"a": [1, 2], "b": "x"});
        assert_eq!(flatten(&value), "a:1,2,b:x");
    }
}

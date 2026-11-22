// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Output of an analyzer and construction of findings shared by the parsers.

use std::path::Path;
use std::sync::LazyLock;

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};
use nurt_contracts::value::Path as RecordPath;

/// Line of the form `file:line:column: severity: message`.
#[allow(
    clippy::expect_used,
    reason = "the pattern is a constant checked by the tests"
)]
static COLON_LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"^(.+?):(\d+):(?:(\d+):)?\s*(?:(error|warning|note)(?:\[[^\]]*\])?:\s*)?(.+)$",
    )
    .expect("constant pattern")
});

/// Rule name in square brackets at the end of a message.
#[allow(
    clippy::expect_used,
    reason = "the pattern is a constant checked by the tests"
)]
static RULE_SUFFIX: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\s*\[([\w-]+)\]\s*$").expect("constant pattern"));

/// Longest message kept in a finding, in characters.
const MESSAGE_MAX_CHARS: usize = 300;

/// Captured output of an analyzer run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ToolOutput {
    /// Standard output, decoded as text.
    pub(crate) stdout: String,
    /// Standard error, decoded as text.
    pub(crate) stderr: String,
    /// Exit code; `None` when the program ended without one.
    pub(crate) exit_code: Option<i32>,
}

impl ToolOutput {
    /// Standard output followed by standard error.
    pub(crate) fn combined(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

/// Position of a finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    /// File relative to the project directory.
    pub(crate) file: String,
    /// Line from 1; zero when the finding concerns the whole file.
    pub(crate) line: u64,
    /// Column from 1.
    pub(crate) column: u64,
}

impl Place {
    /// Position in a file reported by a tool.
    pub(crate) fn new(file: String, line: u64, column: u64) -> Self {
        Self { file, line, column }
    }

    /// Whole file.
    pub(crate) fn file(file: String) -> Self {
        Self {
            file,
            line: 0,
            column: 1,
        }
    }
}

/// Builds a finding of an analyzer, normalizing the message.
pub(crate) fn finding(
    kind: FindingKind,
    tool: &str,
    severity: FindingSeverity,
    place: Place,
    rule: &str,
    message: &str,
) -> Finding {
    let collapsed = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let message: String = collapsed.chars().take(MESSAGE_MAX_CHARS).collect();
    Finding {
        path: RecordPath::new(place.file),
        kind,
        severity,
        message,
        line: (place.line > 0).then_some(place.line),
        column: (place.line > 0).then_some(place.column.max(1)),
        tool: Some(tool.to_owned()),
        rule: (!rule.is_empty()).then(|| rule.to_owned()),
        ..Finding::default()
    }
}

/// Finding for a file that a formatter would change.
pub(crate) fn unformatted(tool: &str, file: String) -> Finding {
    finding(
        FindingKind::Format,
        tool,
        FindingSeverity::Warning,
        Place::file(file),
        "format",
        "file needs formatting",
    )
}

/// Path of `file` relative to `dir`, with forward slashes and without a
/// leading `./`.
pub(crate) fn local_path(file: &str, dir: &Path) -> String {
    let normalized = file.replace('\\', "/");
    let base = dir.to_string_lossy().replace('\\', "/");
    let base = base.trim_end_matches('/');
    let relative = match normalized.strip_prefix(base) {
        Some(rest) if !base.is_empty() && rest.starts_with('/') => rest.trim_start_matches('/'),
        _ => normalized.as_str(),
    };
    relative.strip_prefix("./").unwrap_or(relative).to_owned()
}

/// Files listed one per line, or matched by `pattern` (first participating
/// group), that exist in `dir`, as findings of a format check.
pub(crate) fn file_list_findings(
    tool: &str,
    output: &ToolOutput,
    dir: &Path,
    pattern: Option<&regex::Regex>,
) -> Vec<Finding> {
    let mut files: Vec<String> = Vec::new();
    for raw in output.combined().lines() {
        let line = raw.trim();
        let named = match pattern {
            None => line,
            Some(pattern) => pattern
                .captures(line)
                .and_then(|found| found.iter().skip(1).flatten().next())
                .map_or("", |group| group.as_str()),
        };
        if named.is_empty() {
            continue;
        }
        let file = local_path(named, dir);
        if dir.join(&file).exists() && !files.contains(&file) {
            files.push(file);
        }
    }
    files
        .into_iter()
        .map(|file| unformatted(tool, file))
        .collect()
}

/// Lines `file:line:column: [severity:] message` of go vet, mypy and similar
/// tools.
pub(crate) fn colon_line_findings(
    text: &str,
    dir: &Path,
    kind: FindingKind,
    tool: &str,
) -> Vec<Finding> {
    let pattern = &*COLON_LINE;
    let rule_suffix = &*RULE_SUFFIX;
    let mut found = Vec::new();
    for line in text.lines() {
        let Some(captures) = pattern.captures(line.trim()) else {
            continue;
        };
        let file = &captures[1];
        let level = captures.get(4).map_or("", |level| level.as_str());
        if level == "note" || file.starts_with('#') {
            continue;
        }
        let message = &captures[5];
        let rule = rule_suffix
            .captures(message)
            .map_or("", |rule| rule.get(1).map_or("", |group| group.as_str()));
        let message = rule_suffix.replace(message, "");
        let severity = if level == "warning" {
            FindingSeverity::Warning
        } else {
            FindingSeverity::Error
        };
        let line_no = captures[2].parse().unwrap_or(0);
        let column = captures
            .get(3)
            .and_then(|group| group.as_str().parse().ok())
            .unwrap_or(1);
        found.push(finding(
            kind,
            tool,
            severity,
            Place::new(local_path(file, dir), line_no, column),
            rule,
            &message,
        ));
    }
    found
}

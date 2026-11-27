// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-difftest::output
// Responsibility:   Comparison of the output records of a command with the output of a `git`.
// Dependencies:     nurt_contracts, serde, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Comparison of the output records of a command with the output of a `git`
//! command.
//!
//! Both outputs are reduced to lines of text. The `git` side runs the command
//! of the scenario on copy A after the operation; the Nurt side formats each
//! output record. A `status` comparison reads `git status --porcelain=v2 -z`
//! and formats `git_status` records as `XY <path>`, with ` <- <source>` for
//! renames; a `lines` comparison formats records with a template whose
//! `{field}` placeholders name fields of the plain JSON form of the record.

use std::fmt;
use std::path::Path;

use nurt_contracts::value::{Record, RecordKind};
use serde::Deserialize;

use crate::normalize::ROOT_PLACEHOLDER;

/// Arguments of the `git` command of a `status` comparison.
pub const STATUS_ARGS: &[&str] = &["status", "--porcelain=v2", "-z", "--untracked-files=all"];

/// Largest number of differing lines quoted in a finding.
const QUOTED_LINES: usize = 20;

/// Output comparison as written in a scenario: the name `"status"` or a
/// table.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum RawOutput {
    Preset(String),
    Table(RawOutputTable),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawOutputTable {
    format: Option<String>,
    git: Option<Vec<String>>,
    record: Option<String>,
    kind: Option<String>,
    sort: Option<bool>,
}

/// How records and `git` output become lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    /// `git status --porcelain=v2 -z` against `git_status` records.
    Status,
    /// Lines of the `git` output against records formatted by a template.
    Lines {
        /// Template with `{field}` placeholders; `{{` and `}}` stand for
        /// braces.
        record: String,
    },
}

/// Comparison of the output of the command with the output of `git`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputSpec {
    /// Arguments of the `git` command, without the program name.
    pub git: Vec<String>,
    /// Formatting of both sides.
    pub format: OutputFormat,
    /// Only records of this kind are formatted.
    pub kind: Option<RecordKind>,
    /// Lines are compared as sorted lists.
    pub sort: bool,
}

impl OutputSpec {
    /// Validates the comparison as written in a scenario.
    pub(crate) fn from_raw(raw: RawOutput) -> Result<Self, String> {
        let table = match raw {
            RawOutput::Preset(name) if name == "status" => RawOutputTable {
                format: Some(name),
                git: None,
                record: None,
                kind: None,
                sort: None,
            },
            RawOutput::Preset(name) => {
                return Err(format!(
                    "[compare] output `{name}` is unknown, use \"status\" or a table with git and record"
                ));
            }
            RawOutput::Table(table) => table,
        };
        let kind = table
            .kind
            .map(|name| {
                name.parse::<RecordKind>()
                    .map_err(|_| format!("[compare.output] kind `{name}` is not a record kind"))
            })
            .transpose()?;
        let format = table.format.as_deref().unwrap_or("lines");
        match format {
            "status" => {
                if table.record.is_some() {
                    return Err(
                        "[compare.output] record is not used by the status format, remove it"
                            .to_owned(),
                    );
                }
                let git = table
                    .git
                    .unwrap_or_else(|| STATUS_ARGS.iter().map(|arg| (*arg).to_owned()).collect());
                Ok(Self {
                    git: check_args(git)?,
                    format: OutputFormat::Status,
                    kind: Some(kind.unwrap_or(RecordKind::GitStatus)),
                    sort: table.sort.unwrap_or(true),
                })
            }
            "lines" => {
                let git = table.git.ok_or_else(|| {
                    "[compare.output] needs git = [<arguments>] for the lines format".to_owned()
                })?;
                let record = table.record.ok_or_else(|| {
                    "[compare.output] needs record = \"<template>\" for the lines format".to_owned()
                })?;
                Template::parse(&record)?;
                Ok(Self {
                    git: check_args(git)?,
                    format: OutputFormat::Lines { record },
                    kind,
                    sort: table.sort.unwrap_or(false),
                })
            }
            other => Err(format!(
                "[compare.output] format `{other}` is unknown, use status or lines"
            )),
        }
    }

    /// Lines of the output of the `git` command; the case directory
    /// `case_root` of the `git` side reads as `$ROOT`.
    #[must_use]
    pub fn git_lines(&self, stdout: &[u8], case_root: &Path) -> Vec<String> {
        let lines = match self.format {
            OutputFormat::Status => porcelain_v2_lines(stdout),
            OutputFormat::Lines { .. } => text_lines(stdout),
        };
        let roots = Roots::of(case_root);
        lines.into_iter().map(|line| roots.replace(&line)).collect()
    }

    /// Lines of the output records; the case directory `case_root` of the
    /// Nurt side reads as `$ROOT`.
    #[must_use]
    pub fn record_lines(&self, records: &[Record], case_root: &Path) -> Vec<String> {
        let roots = Roots::of(case_root);
        let selected = records
            .iter()
            .filter(|record| self.kind.is_none_or(|kind| record.kind == kind));
        match &self.format {
            OutputFormat::Status => selected
                .map(|record| roots.replace(&status_line(&record.to_plain_json())))
                .collect(),
            OutputFormat::Lines { record } => {
                let Ok(template) = Template::parse(record) else {
                    return Vec::new();
                };
                selected
                    .map(|item| roots.replace(&template.render(&item.to_plain_json())))
                    .collect()
            }
        }
    }

    /// Compares the lines of both sides; returns a description of the
    /// difference.
    #[must_use]
    pub fn compare(&self, git: &[String], nurt: &[String]) -> Option<String> {
        compare_lines(git, nurt, self.sort)
    }
}

fn check_args(args: Vec<String>) -> Result<Vec<String>, String> {
    match args.first().map(String::as_str) {
        None => Err("[compare.output] git is empty, give the git arguments".to_owned()),
        Some("git") => Err(
            "[compare.output] git starts with `git`, give the arguments without the program name"
                .to_owned(),
        ),
        Some(_) => Ok(args),
    }
}

/// The case directory in native and forward-slash spelling.
struct Roots {
    native: String,
    forward: String,
}

impl Roots {
    fn of(case_root: &Path) -> Self {
        let native = case_root.to_string_lossy().into_owned();
        let forward = native.replace('\\', "/");
        Self { native, forward }
    }

    fn replace(&self, text: &str) -> String {
        text.replace(&self.native, ROOT_PLACEHOLDER)
            .replace(&self.forward, ROOT_PLACEHOLDER)
    }
}

/// Lines of a text output: line ends are LF or CRLF; the empty text after a
/// final line end is not a line.
fn text_lines(stdout: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(stdout);
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// Number of fields before the path of the porcelain v2 entry types `1`,
/// `2` and `u`, the type letter included.
const ORDINARY_FIELDS: usize = 8;
const RENAMED_FIELDS: usize = 9;
const UNMERGED_FIELDS: usize = 10;

/// Lines `XY <path>` of `git status --porcelain=v2 -z`; renames and copies
/// get ` <- <source>`, untracked paths `??` and ignored paths `!!`. Header
/// lines (`#`) are left out.
fn porcelain_v2_lines(stdout: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(stdout);
    let mut fields = text.split('\0').filter(|entry| !entry.is_empty());
    let mut lines = Vec::new();
    while let Some(entry) = fields.next() {
        let line = match entry.as_bytes().first() {
            Some(b'1') => path_after(entry, ORDINARY_FIELDS).map(|path| xy_line(entry, path)),
            Some(b'u') => path_after(entry, UNMERGED_FIELDS).map(|path| xy_line(entry, path)),
            Some(b'2') => path_after(entry, RENAMED_FIELDS).map(|path| {
                let source = fields.next().unwrap_or_default();
                format!("{} <- {source}", xy_line(entry, path))
            }),
            Some(b'?') => entry.get(2..).map(|path| format!("?? {path}")),
            Some(b'!') => entry.get(2..).map(|path| format!("!! {path}")),
            _ => None,
        };
        lines.extend(line);
    }
    lines
}

/// The path of an entry with `count` space-separated fields before it.
fn path_after(entry: &str, count: usize) -> Option<&str> {
    entry.splitn(count + 1, ' ').nth(count)
}

fn xy_line(entry: &str, path: &str) -> String {
    let xy = entry.split(' ').nth(1).unwrap_or("??");
    format!("{xy} {path}")
}

/// Line of a `git_status` record in the form of [`porcelain_v2_lines`].
fn status_line(record: &serde_json::Value) -> String {
    let field = |name: &str| record.get(name).and_then(serde_json::Value::as_str);
    let path = field("path").unwrap_or_default();
    let xy = match field("kind") {
        Some("untracked") => "??".to_owned(),
        Some("ignored") => "!!".to_owned(),
        Some("conflict") => conflict_xy(field("conflict")).to_owned(),
        _ => format!(
            "{}{}",
            state_letter(field("index")),
            state_letter(field("worktree"))
        ),
    };
    match field("renamed_from") {
        Some(source) => format!("{xy} {path} <- {source}"),
        None => format!("{xy} {path}"),
    }
}

/// Letter of a file state in `git status --porcelain`.
fn state_letter(state: Option<&str>) -> char {
    match state {
        Some("modified") => 'M',
        Some("added") => 'A',
        Some("deleted") => 'D',
        Some("renamed") => 'R',
        Some("copied") => 'C',
        Some("type_changed") => 'T',
        Some("unmerged") => 'U',
        Some("untracked") => '?',
        Some("ignored") => '!',
        _ => '.',
    }
}

/// `XY` of an unmerged path; conflict kinds without a porcelain code give
/// `U?`, which differs from every `git` line.
fn conflict_xy(kind: Option<&str>) -> &'static str {
    match kind {
        Some("both_modified") => "UU",
        Some("both_added") => "AA",
        Some("both_deleted") => "DD",
        Some("added_by_us") => "AU",
        Some("added_by_them") => "UA",
        Some("deleted_by_us") => "DU",
        Some("deleted_by_them") => "UD",
        _ => "U?",
    }
}

/// Parsed record template.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Template {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Field(Vec<String>),
}

impl Template {
    fn parse(text: &str) -> Result<Self, String> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '{' => {
                    let name: String = chars.by_ref().take_while(|ch| *ch != '}').collect();
                    if name.is_empty() || name.contains('{') {
                        return Err(format!(
                            "[compare.output] record `{text}` has an empty or unclosed placeholder"
                        ));
                    }
                    if !literal.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut literal)));
                    }
                    parts.push(Part::Field(name.split('.').map(str::to_owned).collect()));
                }
                '}' => {
                    return Err(format!(
                        "[compare.output] record `{text}` has `}}` without `{{`, write `}}}}` for a brace"
                    ));
                }
                other => literal.push(other),
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Text(literal));
        }
        Ok(Self { parts })
    }

    fn render(&self, record: &serde_json::Value) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Field(path) => {
                    let value = path
                        .iter()
                        .try_fold(record, |value, name| value.get(name.as_str()));
                    out.push_str(&value.map(render_value).unwrap_or_default());
                }
            }
        }
        out
    }
}

/// Text of a field value: texts as written, lists joined by commas, `null`
/// as nothing, objects as JSON.
fn render_value(value: &serde_json::Value) -> String {
    use serde_json::Value as Json;
    match value {
        Json::Null => String::new(),
        Json::String(text) => text.clone(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::Array(items) => items.iter().map(render_value).collect::<Vec<_>>().join(","),
        Json::Object(_) => value.to_string(),
    }
}

/// Difference between the lines of `git` and of the Nurt side.
#[derive(Debug)]
struct LineDifference<'a> {
    git_only: Vec<&'a str>,
    nurt_only: Vec<&'a str>,
    first_reordered: Option<usize>,
}

impl fmt::Display for LineDifference<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (label, lines) in [("git only", &self.git_only), ("nurt only", &self.nurt_only)] {
            for line in lines.iter().take(QUOTED_LINES) {
                writeln!(f, "  {label}: {line}")?;
            }
            if lines.len() > QUOTED_LINES {
                writeln!(f, "  {label}: {} more lines", lines.len() - QUOTED_LINES)?;
            }
        }
        if let Some(index) = self.first_reordered {
            writeln!(
                f,
                "  the same lines in another order, first difference at line {}",
                index + 1
            )?;
        }
        Ok(())
    }
}

/// Compares two lists of lines, as sorted lists when `sort` is set.
fn compare_lines(git: &[String], nurt: &[String], sort: bool) -> Option<String> {
    if git == nurt {
        return None;
    }
    let mut left: Vec<&str> = git.iter().map(String::as_str).collect();
    let mut right: Vec<&str> = nurt.iter().map(String::as_str).collect();
    left.sort_unstable();
    right.sort_unstable();
    if left == right && sort {
        return None;
    }
    let difference = LineDifference {
        git_only: multiset_minus(&left, &right),
        nurt_only: multiset_minus(&right, &left),
        first_reordered: (left == right)
            .then(|| git.iter().zip(nurt).position(|(a, b)| a != b))
            .flatten(),
    };
    Some(difference.to_string())
}

/// Elements of sorted `left` not matched by an element of sorted `right`.
fn multiset_minus<'a>(left: &[&'a str], right: &[&str]) -> Vec<&'a str> {
    let mut rest = Vec::new();
    let mut other = right.iter().peekable();
    for item in left {
        while other.next_if(|candidate| *candidate < item).is_some() {}
        if other.next_if(|candidate| *candidate == item).is_none() {
            rest.push(*item);
        }
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::value::Value;

    /// Comparison written as a quoted preset name or as the body of a table.
    fn spec(text: &str) -> Result<OutputSpec, String> {
        let raw = match text
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
        {
            Some(name) => RawOutput::Preset(name.to_owned()),
            None => RawOutput::Table(toml::from_str(text).map_err(|error| error.to_string())?),
        };
        OutputSpec::from_raw(raw)
    }

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn reads_every_porcelain_v2_entry_type() {
        let output = b"1 M. N... 100644 100644 100644 aaa bbb a b.txt\0\
2 R. N... 100644 100644 100644 aaa aaa R100 new name.txt\0old.txt\0\
u UU N... 100644 100644 100644 100644 a b c both.txt\0\
? new dir/x.txt\0! build/out\0# branch.oid abc\0";
        assert_eq!(
            porcelain_v2_lines(output),
            lines(&[
                "M. a b.txt",
                "R. new name.txt <- old.txt",
                "UU both.txt",
                "?? new dir/x.txt",
                "!! build/out",
            ])
        );
    }

    #[test]
    fn formats_status_records_like_porcelain() {
        let status = |pairs: &[(&str, &str)]| {
            let mut record = Record::new(RecordKind::GitStatus);
            for &(name, value) in pairs {
                record = record.with(name, Value::from(value));
            }
            record
        };
        let records = [
            status(&[
                ("path", "a b.txt"),
                ("kind", "tracked"),
                ("index", "modified"),
                ("worktree", "unmodified"),
            ]),
            status(&[
                ("path", "new name.txt"),
                ("kind", "tracked"),
                ("index", "renamed"),
                ("worktree", "unmodified"),
                ("renamed_from", "old.txt"),
            ]),
            status(&[
                ("path", "both.txt"),
                ("kind", "conflict"),
                ("conflict", "both_modified"),
            ]),
            status(&[("path", "x"), ("kind", "untracked")]),
            Record::new(RecordKind::Finding).with("path", Value::from("ignored")),
        ];
        let spec = spec("\"status\"").unwrap();
        assert_eq!(
            spec.record_lines(&records, Path::new("/case/b")),
            lines(&[
                "M. a b.txt",
                "R. new name.txt <- old.txt",
                "UU both.txt",
                "?? x"
            ])
        );
        assert_eq!(conflict_xy(Some("rename_rename")), "U?");
    }

    #[test]
    fn templates_render_fields_lists_and_nested_values() {
        let template = Template::parse("{name} {{{parents}}} {stats.added}{missing}").unwrap();
        let json = serde_json::json!({
            "name": "main",
            "parents": ["a", "b"],
            "stats": { "added": 3 },
        });
        assert_eq!(template.render(&json), "main {a,b} 3");
        assert!(Template::parse("{").is_err());
        assert!(Template::parse("{}").is_err());
        assert!(Template::parse("a } b").is_err());
    }

    #[test]
    fn lines_format_replaces_the_case_directory() {
        let spec = spec(
            "git = [\"worktree\", \"list\", \"--porcelain\"]\nrecord = \"worktree {path}\"\nkind = \"git_worktree\"",
        )
        .unwrap();
        let record = Record::new(RecordKind::GitWorktree)
            .with("path", Value::Text("/case/b/repo".to_owned()));
        assert_eq!(
            spec.record_lines(&[record], Path::new("/case/b")),
            lines(&["worktree $ROOT/repo"])
        );
        assert_eq!(
            spec.git_lines(b"worktree /case/a/repo\r\nHEAD x\n\n", Path::new("/case/a")),
            lines(&["worktree $ROOT/repo", "HEAD x", ""])
        );
    }

    #[test]
    fn rejects_incomplete_or_unknown_comparisons() {
        assert!(spec("\"porcelain\"").unwrap_err().contains("unknown"));
        assert!(
            spec("record = \"{name}\"")
                .unwrap_err()
                .contains("needs git")
        );
        assert!(
            spec("git = [\"log\"]")
                .unwrap_err()
                .contains("needs record")
        );
        assert!(
            spec("format = \"status\"\nrecord = \"x\"")
                .unwrap_err()
                .contains("not used")
        );
        assert!(
            spec("git = [\"git\", \"log\"]\nrecord = \"x\"")
                .unwrap_err()
                .contains("program name")
        );
        assert!(
            spec("git = [\"log\"]\nrecord = \"x\"\nkind = \"nope\"")
                .unwrap_err()
                .contains("record kind")
        );
        assert!(
            spec("format = \"csv\"\ngit = [\"log\"]")
                .unwrap_err()
                .contains("status or lines")
        );
    }

    #[test]
    fn compares_lines_in_order_or_sorted() {
        let git = lines(&["a", "b", "b"]);
        assert!(compare_lines(&git, &git, false).is_none());
        let reordered = lines(&["b", "a", "b"]);
        assert!(compare_lines(&git, &reordered, true).is_none());
        let message = compare_lines(&git, &reordered, false).unwrap();
        assert!(message.contains("another order, first difference at line 1"));
        let message = compare_lines(&git, &lines(&["a", "b", "c"]), true).unwrap();
        assert!(message.contains("git only: b"), "{message}");
        assert!(message.contains("nurt only: c"), "{message}");
        let many: Vec<String> = (0..30).map(|n| format!("line {n:02}")).collect();
        let message = compare_lines(&many, &[], true).unwrap();
        assert!(message.contains("git only: 10 more lines"), "{message}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

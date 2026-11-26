// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Checks of the changed files of a review.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::record::{Finding, FindingSeverity, FindingTarget};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::{ReviewRuleProvider, RuleSeverity};
use nurt_contracts::value::Path as RecordPath;
use regex::bytes::Regex;

use super::Check;
use super::collect::Changed;
use crate::diff::{Hunk, LineKind};

/// Files above this size in bytes are reported by `large_files`; untracked
/// files above it are treated as binary and not read.
pub const LARGE_FILE_BYTES: u64 = 512 * 1024;

/// Extensions of the source files checked by `debug_statements` and `tests`.
pub const CODE_EXTENSIONS: [&str; 24] = [
    "c", "cc", "cjs", "cpp", "cs", "cxx", "go", "h", "hpp", "java", "js", "jsx", "kt", "kts",
    "mjs", "php", "py", "rb", "rs", "scala", "swift", "ts", "tsx", "vue",
];

/// File stems that name no unit; the `tests` check uses the directory name
/// instead.
pub const GENERIC_STEMS: [&str; 7] = ["index", "main", "mod", "lib", "__init__", "utils", "common"];

/// Directory names that hold tests.
pub const TEST_DIRECTORIES: [&str; 6] = ["test", "tests", "spec", "specs", "__tests__", "testing"];

/// Debugging calls reported by `debug_statements`.
pub const DEBUG_PATTERN: &str = r"\bconsole\.(?:log|debug)\s*\(|\bdebugger\b|\bdbg!\s*\(|\bpdb\.set_trace\s*\(|\bbreakpoint\s*\(\s*\)";

/// Length of a conflict marker.
const MARKER_LEN: usize = 7;

/// Extensions of Markdown files, where trailing spaces end a line.
const MARKDOWN_EXTENSIONS: [&str; 2] = ["md", "markdown"];

#[expect(
    clippy::expect_used,
    reason = "the pattern is a constant checked by the tests of this module"
)]
static DEBUG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(DEBUG_PATTERN).expect("valid debug pattern"));

/// A rule of the review rule service with its compiled pattern.
#[derive(Debug, Clone)]
pub(crate) struct SecretRule {
    id: String,
    pattern: Regex,
    severity: FindingSeverity,
    summary: String,
}

/// Compiles the rules of `provider`; a missing service and invalid patterns
/// are reported as `source_unavailable` notes.
pub(crate) fn secret_rules(
    provider: Option<&dyn ReviewRuleProvider>,
    notes: &mut Vec<NotableEntry>,
) -> Vec<SecretRule> {
    let unavailable = |detail: String| {
        let mut note = NotableEntry::new(Notable::SourceUnavailable);
        note.detail = Some(detail);
        note
    };
    let Some(provider) = provider else {
        notes.push(unavailable(
            "no review rule service is available, so the `secrets` check had no rules".to_owned(),
        ));
        return Vec::new();
    };
    let mut rules = Vec::new();
    for rule in provider.rules() {
        match Regex::new(&rule.pattern) {
            Ok(pattern) => rules.push(SecretRule {
                id: rule.id,
                pattern,
                severity: severity_of(rule.severity),
                summary: rule.summary,
            }),
            Err(_) => notes.push(unavailable(format!(
                "review rule `{}` has an invalid pattern and was not applied",
                rule.id
            ))),
        }
    }
    rules
}

fn severity_of(severity: RuleSeverity) -> FindingSeverity {
    match severity {
        RuleSeverity::Error => FindingSeverity::Error,
        RuleSeverity::Warning => FindingSeverity::Warning,
        _ => FindingSeverity::Info,
    }
}

/// Settings shared by the checks of one review.
pub(crate) struct Settings<'a> {
    /// Selected checks.
    pub(crate) checks: &'a BTreeSet<Check>,
    /// Rules of the `secrets` check.
    pub(crate) secrets: &'a [SecretRule],
    /// Tracked files of the new side.
    pub(crate) tracked: &'a [BString],
}

/// Runs the selected checks on `files`.
pub(crate) fn check_files(files: &[Changed], settings: &Settings<'_>) -> Vec<(Check, Finding)> {
    let mut out = Vec::new();
    for file in files {
        check_file(file, settings, &mut out);
    }
    if settings.checks.contains(&Check::Tests) {
        tests(files, settings.tracked, &mut out);
    }
    out
}

/// An added line with its number in the new content.
struct AddedLine<'a> {
    number: u64,
    text: &'a BStr,
    hunk: &'a Hunk,
}

fn added_lines(file: &Changed) -> Vec<AddedLine<'_>> {
    let mut lines = Vec::new();
    for hunk in &file.hunks {
        let mut number = u64::from(hunk.new_start);
        for line in &hunk.lines {
            match line.kind {
                LineKind::Context => number += 1,
                LineKind::Added => {
                    lines.push(AddedLine {
                        number,
                        text: line.text.as_bstr(),
                        hunk,
                    });
                    number += 1;
                }
                LineKind::Removed | LineKind::NoNewline => {}
            }
        }
    }
    lines
}

fn check_file(file: &Changed, settings: &Settings<'_>, out: &mut Vec<(Check, Finding)>) {
    let enabled = |check: Check| settings.checks.contains(&check);
    if enabled(Check::LargeFiles) && file.size > LARGE_FILE_BYTES {
        out.push((Check::LargeFiles, large_file(file)));
    }
    if file.binary {
        if enabled(Check::Binary) {
            out.push((
                Check::Binary,
                finding(
                    file,
                    Check::Binary,
                    FindingSeverity::Warning,
                    "binary",
                    "the file has binary content; the line checks do not apply, review it separately"
                        .to_owned(),
                ),
            ));
        }
        return;
    }
    let added = added_lines(file);
    if enabled(Check::Secrets) {
        secrets(file, &added, settings.secrets, out);
    }
    if enabled(Check::ConflictMarkers) {
        conflict_markers(file, &added, out);
    }
    if enabled(Check::Whitespace) {
        whitespace(file, &added, out);
    }
    if enabled(Check::DebugStatements) && is_code(file.path.as_bstr()) {
        debug_statements(file, &added, out);
    }
    if enabled(Check::FinalNewline) {
        final_newline(file, out);
    }
}

fn finding(
    file: &Changed,
    check: Check,
    severity: FindingSeverity,
    rule: &str,
    message: String,
) -> Finding {
    Finding {
        path: RecordPath::new(file.path.to_str_lossy().into_owned()),
        kind: check.kind(),
        severity,
        message,
        rule: Some(rule.to_owned()),
        target: Some(FindingTarget::File),
        ..Finding::default()
    }
}

/// Sets the line and, for a canonical difference, the hunk of `finding`.
fn at(mut finding: Finding, file: &Changed, line: u64, hunk: Option<&Hunk>) -> Finding {
    finding.line = Some(line);
    finding.hunk = hunk.filter(|_| file.canonical).map(hunk_id);
    finding
}

fn hunk_id(hunk: &Hunk) -> String {
    let mut id = hunk.id();
    id.fingerprint = Some(hunk.fingerprint());
    id.to_string()
}

fn large_file(file: &Changed) -> Finding {
    let mut found = finding(
        file,
        Check::LargeFiles,
        FindingSeverity::Warning,
        "large_files",
        format!(
            "the file has {} bytes, more than {LARGE_FILE_BYTES}; keep large files out of the repository",
            file.size
        ),
    );
    found.value = Some(file.size as f64);
    found.threshold = Some(LARGE_FILE_BYTES as f64);
    found
}

fn secrets(
    file: &Changed,
    added: &[AddedLine<'_>],
    rules: &[SecretRule],
    out: &mut Vec<(Check, Finding)>,
) {
    for line in added {
        for rule in rules {
            if rule.pattern.is_match(line.text) {
                let found = finding(
                    file,
                    Check::Secrets,
                    rule.severity,
                    &rule.id,
                    format!("{}; remove the value from the change", rule.summary),
                );
                out.push((
                    Check::Secrets,
                    at(found, file, line.number, Some(line.hunk)),
                ));
            }
        }
    }
}

/// The marker of a line that starts with seven `<` or `>` and a space.
fn conflict_marker(text: &[u8]) -> Option<&str> {
    if text.get(MARKER_LEN) != Some(&b' ') {
        return None;
    }
    let head = &text[..MARKER_LEN];
    if head.iter().all(|byte| *byte == b'<') {
        Some("<<<<<<<")
    } else if head.iter().all(|byte| *byte == b'>') {
        Some(">>>>>>>")
    } else {
        None
    }
}

fn conflict_markers(file: &Changed, added: &[AddedLine<'_>], out: &mut Vec<(Check, Finding)>) {
    for line in added {
        if let Some(marker) = conflict_marker(line.text) {
            let found = finding(
                file,
                Check::ConflictMarkers,
                FindingSeverity::Error,
                "conflict_markers",
                format!(
                    "leftover conflict marker `{marker}`; resolve the conflict and remove the marker"
                ),
            );
            out.push((
                Check::ConflictMarkers,
                at(found, file, line.number, Some(line.hunk)),
            ));
        }
    }
}

/// One finding for every added line that matches `test`, with the count
/// and the first line.
fn aggregate(
    file: &Changed,
    added: &[AddedLine<'_>],
    rule: &str,
    describe: impl Fn(usize) -> String,
    test: impl Fn(&[u8]) -> bool,
) -> Option<Finding> {
    let matching: Vec<&AddedLine<'_>> = added.iter().filter(|line| test(line.text)).collect();
    let first = matching.first()?;
    let found = finding(
        file,
        Check::Whitespace,
        FindingSeverity::Warning,
        rule,
        describe(matching.len()),
    );
    let mut found = at(found, file, first.number, Some(first.hunk));
    found.count = u64::try_from(matching.len()).ok();
    Some(found)
}

fn lines_text(count: usize) -> String {
    if count == 1 {
        "1 added line".to_owned()
    } else {
        format!("{count} added lines")
    }
}

fn whitespace(file: &Changed, added: &[AddedLine<'_>], out: &mut Vec<(Check, Finding)>) {
    if !has_extension(file.path.as_bstr(), &MARKDOWN_EXTENSIONS)
        && let Some(found) = aggregate(
            file,
            added,
            "whitespace",
            |count| {
                format!(
                    "{} {} with white space; remove the trailing spaces, tabs and carriage returns",
                    lines_text(count),
                    if count == 1 { "ends" } else { "end" }
                )
            },
            ends_with_white_space,
        )
    {
        out.push((Check::Whitespace, found));
    }
    if let Some(found) = aggregate(
        file,
        added,
        "space_before_tab",
        |count| {
            format!(
                "{} {} a space before a tab in the indentation; indent with tabs or with spaces only",
                lines_text(count),
                if count == 1 { "has" } else { "have" }
            )
        },
        |text| {
            let indent = text
                .iter()
                .position(|byte| *byte != b' ' && *byte != b'\t')
                .unwrap_or(text.len());
            text[..indent].find(b" \t").is_some()
        },
    ) {
        out.push((Check::Whitespace, found));
    }
    if let Some(line) = blank_at_eof(file, added) {
        let found = finding(
            file,
            Check::Whitespace,
            FindingSeverity::Warning,
            "blank_at_eof",
            "the change adds blank lines at the end of the file; remove them".to_owned(),
        );
        out.push((
            Check::Whitespace,
            at(found, file, line.number, Some(line.hunk)),
        ));
    }
}

/// Whether a line ends with white space as `git diff --check` counts it by
/// default: a space, a tab or the carriage return of a CRLF line ending.
fn ends_with_white_space(text: &[u8]) -> bool {
    matches!(text.last(), Some(b' ' | b'\t' | b'\r'))
}

/// Number of lines of `content`; a last line without line feed counts.
fn line_count(content: &[u8]) -> u64 {
    let feeds = content.iter().filter(|byte| **byte == b'\n').count();
    let partial = usize::from(!content.is_empty() && !content.ends_with(b"\n"));
    u64::try_from(feeds + partial).unwrap_or(u64::MAX)
}

/// The first of the added blank lines that end the file, as `git diff
/// --check` reports it.
fn blank_at_eof<'a>(file: &Changed, added: &'a [AddedLine<'a>]) -> Option<&'a AddedLine<'a>> {
    let mut expected = line_count(&file.new);
    let mut first = None;
    for line in added.iter().rev() {
        if line.number != expected || !line.text.trim().is_empty() {
            break;
        }
        first = Some(line);
        expected = expected.saturating_sub(1);
    }
    first
}

fn final_newline(file: &Changed, out: &mut Vec<(Check, Finding)>) {
    let introduced = file.old.is_empty() || file.old.ends_with(b"\n");
    if file.new.is_empty() || file.new.ends_with(b"\n") || !introduced {
        return;
    }
    let found = finding(
        file,
        Check::FinalNewline,
        FindingSeverity::Warning,
        "final_newline",
        "the file does not end with a line feed; add one after the last line".to_owned(),
    );
    out.push((
        Check::FinalNewline,
        at(found, file, line_count(&file.new), file.hunks.last()),
    ));
}

fn debug_statements(file: &Changed, added: &[AddedLine<'_>], out: &mut Vec<(Check, Finding)>) {
    for line in added {
        if let Some(found) = DEBUG.find(line.text) {
            let call = found
                .as_bytes()
                .to_str_lossy()
                .trim_end_matches('(')
                .trim()
                .to_owned();
            let found = finding(
                file,
                Check::DebugStatements,
                FindingSeverity::Warning,
                "debug_statements",
                format!("debugging call `{call}` in an added line; remove it before the commit"),
            );
            out.push((
                Check::DebugStatements,
                at(found, file, line.number, Some(line.hunk)),
            ));
        }
    }
}

fn file_name(path: &BStr) -> &BStr {
    path.rsplit_str("/")
        .next()
        .unwrap_or(path.as_bytes())
        .as_bstr()
}

fn has_extension(path: &BStr, extensions: &[&str]) -> bool {
    let name = file_name(path);
    name.rfind_byte(b'.').is_some_and(|dot| {
        let extension = name[dot + 1..].to_str_lossy().to_ascii_lowercase();
        extensions.contains(&extension.as_str())
    })
}

fn is_code(path: &BStr) -> bool {
    has_extension(path, &CODE_EXTENSIONS)
}

/// The path names a test: it lies in a test directory or its name follows a
/// test naming convention (`test_*`, `*_test.*`, `*.test.*`, `*.spec.*`,
/// `*_spec.*`, `*Test.*`, `*Tests.*`).
fn is_test_path(path: &BStr) -> bool {
    let text = path.to_str_lossy();
    let mut parts: Vec<&str> = text.split('/').collect();
    let name = parts.pop().unwrap_or_default();
    if parts
        .iter()
        .any(|dir| TEST_DIRECTORIES.contains(&dir.to_ascii_lowercase().as_str()))
    {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    let stem = name.split('.').next().unwrap_or_default();
    lower.starts_with("test_")
        || ["_test.", ".test.", ".spec.", "_spec."]
            .iter()
            .any(|marker| lower.contains(marker))
        || stem.ends_with("Test")
        || stem.ends_with("Tests")
}

/// The name a test of `path` contains: the file stem, or the directory name
/// for a generic stem such as `mod` or `index`.
fn test_key(path: &BStr) -> Option<String> {
    let text = path.to_str_lossy().to_ascii_lowercase();
    let (dir, name) = match text.rsplit_once('/') {
        Some((dir, name)) => (Some(dir), name),
        None => (None, text.as_str()),
    };
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    if !GENERIC_STEMS.contains(&stem) {
        return Some(stem.to_owned());
    }
    dir.and_then(|dir| dir.rsplit('/').next())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn tests(files: &[Changed], tracked: &[BString], out: &mut Vec<(Check, Finding)>) {
    let changed_tests: Vec<String> = files
        .iter()
        .filter(|file| is_test_path(file.path.as_bstr()))
        .map(|file| file.path.to_str_lossy().to_ascii_lowercase())
        .collect();
    let tracked_tests: Vec<&BString> = tracked
        .iter()
        .filter(|path| is_test_path(path.as_bstr()))
        .collect();
    for file in files {
        let path = file.path.as_bstr();
        if !is_code(path) || is_test_path(path) {
            continue;
        }
        let Some(key) = test_key(path) else {
            continue;
        };
        if changed_tests.iter().any(|test| test.contains(&key)) {
            continue;
        }
        let existing = tracked_tests
            .iter()
            .find(|test| test.to_str_lossy().to_ascii_lowercase().contains(&key));
        let (rule, message) = match existing {
            Some(test) => (
                "tests_unchanged",
                format!(
                    "the tests of `{path}` were not changed: `{test}`; update them for the change"
                ),
            ),
            None => (
                "tests_missing",
                format!("no test changes accompany `{path}`; add or update its tests"),
            ),
        };
        out.push((
            Check::Tests,
            finding(file, Check::Tests, FindingSeverity::Warning, rule, message),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_markers_need_seven_signs_and_a_space() {
        assert_eq!(conflict_marker(b"<<<<<<< HEAD"), Some("<<<<<<<"));
        assert_eq!(conflict_marker(b">>>>>>> topic"), Some(">>>>>>>"));
        assert_eq!(conflict_marker(b"======="), None);
        assert_eq!(conflict_marker(b"<<<<<<<"), None);
        assert_eq!(conflict_marker(b"<<<<<< x"), None);
        assert_eq!(conflict_marker(b"<<<>>>> x"), None);
    }

    #[test]
    fn debug_pattern_finds_the_calls_of_the_supported_languages() {
        for line in [
            "console.log(value)",
            "  console.debug (x);",
            "debugger;",
            "let x = dbg!(y);",
            "import pdb; pdb.set_trace()",
            "breakpoint()",
        ] {
            assert!(DEBUG.is_match(line.as_bytes()), "{line}");
        }
        for line in [
            "logger.log(x)",
            "debuggerEnabled = true",
            "my_breakpoint(1)",
        ] {
            assert!(!DEBUG.is_match(line.as_bytes()), "{line}");
        }
    }

    #[test]
    fn recognizes_test_paths_by_directory_and_name() {
        for path in [
            "tests/parser.rs",
            "src/__tests__/app.tsx",
            "pkg/parser_test.go",
            "web/app.test.ts",
            "web/app.spec.js",
            "test_parser.py",
            "src/ParserTest.java",
            "src/ParserTests.cs",
        ] {
            assert!(is_test_path(path.into()), "{path}");
        }
        for path in ["src/parser.rs", "src/latest.rs", "docs/contest/notes.md"] {
            assert!(!is_test_path(path.into()), "{path}");
        }
    }

    #[test]
    fn test_keys_use_the_directory_for_generic_stems() {
        assert_eq!(test_key("src/Parser.rs".into()).as_deref(), Some("parser"));
        assert_eq!(
            test_key("src/lexer/mod.rs".into()).as_deref(),
            Some("lexer")
        );
        assert_eq!(test_key("app/index.ts".into()).as_deref(), Some("app"));
        assert_eq!(test_key("main.go".into()), None);
    }

    #[test]
    fn carriage_return_before_the_line_feed_is_trailing_white_space() {
        assert!(ends_with_white_space(b"text\r"));
        assert!(ends_with_white_space(b"text "));
        assert!(ends_with_white_space(b"text\t"));
        assert!(!ends_with_white_space(b"text"));
        assert!(!ends_with_white_space(b""));
    }

    #[test]
    fn counts_lines_with_and_without_a_final_line_feed() {
        assert_eq!(line_count(b""), 0);
        assert_eq!(line_count(b"a\n"), 1);
        assert_eq!(line_count(b"a\nb"), 2);
    }

    #[test]
    fn maps_rule_severities() {
        assert_eq!(severity_of(RuleSeverity::Error), FindingSeverity::Error);
        assert_eq!(severity_of(RuleSeverity::Warning), FindingSeverity::Warning);
        assert_eq!(severity_of(RuleSeverity::Info), FindingSeverity::Info);
    }
}

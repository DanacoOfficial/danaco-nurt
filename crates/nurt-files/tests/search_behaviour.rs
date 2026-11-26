// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Behaviour of `search`: patterns, options, outputs, inputs, encodings and
//! the use of the code service.

// Test helpers unwrap results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::any::Any;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::{Content, Entry, Line, Match, SymbolKind};
use nurt_contracts::report::Notable;
use nurt_contracts::services::{
    ByteRange, ClassifiedSpan, CodeService, FunctionMetrics, LanguageId, LanguageInfo, Services,
    SpanClass, SymbolInfo, SymbolReference, SyntaxCapture, SyntaxIssue, SyntaxMatch, SyntaxPattern,
    SyntaxToken, TextPosition, TextSpan,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, Demand, ExecCtx, Limits, RecordStream, StageData, StageFrame, StageInput,
};
use nurt_contracts::value::{Range, Record, RecordKind, Regex, RegexFlags, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::test_path;

fn abs(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn language(path: &str) -> String {
    abs(path).to_language_string()
}

fn write(test: &TestContext, path: &str, content: &[u8]) {
    let path = abs(path);
    if let Some(parent) = path.as_path().parent() {
        test.fs()
            .create_dir_all(&AbsPath::new(parent.to_path_buf()).unwrap())
            .unwrap();
    }
    test.fs().write_file(&path, content).unwrap();
}

fn tree(files: &[(&str, &[u8])]) -> TestContext {
    let test = TestContext::new();
    for (path, content) in files {
        write(&test, path, content);
    }
    test
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn choice(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

fn regex(pattern: &str) -> Value {
    Value::Regex(Regex {
        pattern: pattern.to_owned(),
        flags: RegexFlags::default(),
    })
}

fn bound(values: Vec<(&str, Value)>) -> BoundArgs {
    let mut args = BoundArgs::new("search");
    for (name, value) in values {
        args.insert(name, ArgValue::Value(value), false);
    }
    args
}

fn execute(ctx: &ExecCtx, args: &BoundArgs, input: StageInput<'_>) -> Result<Vec<Record>> {
    let stage = nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "search")
        .unwrap();
    let output = stage.as_read().unwrap().execute(args, input, ctx)?;
    let StageData::Records(stream) = output.data else {
        panic!("search produces records");
    };
    stream.collect()
}

fn search(
    ctx: &ExecCtx,
    pattern: Value,
    root: &str,
    extra: Vec<(&str, Value)>,
) -> Result<Vec<Record>> {
    let mut values = vec![("pattern", pattern), ("path", Value::Path(language(root)))];
    values.extend(extra);
    execute(ctx, &bound(values), StageInput::none())
}

fn matches(records: Vec<Record>) -> Vec<Match> {
    records
        .into_iter()
        .map(|record| Match::try_from(record).unwrap())
        .collect()
}

fn found(ctx: &ExecCtx, pattern: Value, root: &str, extra: Vec<(&str, Value)>) -> Vec<Match> {
    matches(search(ctx, pattern, root, extra).unwrap())
}

fn positions(found: &[Match]) -> Vec<(u64, u64)> {
    found.iter().map(|found| (found.no, found.column)).collect()
}

fn names(found: &[Match]) -> Vec<String> {
    found
        .iter()
        .map(|found| found.path.as_str().to_owned())
        .collect()
}

#[test]
fn literal_hits_report_file_offsets_and_columns() {
    let test = tree(&[("/t/a.txt", "x\n  \u{17c}\u{f3}\u{142}w abc\n".as_bytes())]);
    let found = found(&test.ctx(), text("abc"), "/t", vec![]);
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].no, found[0].column, found[0].start, found[0].end),
        (2, 8, 12, 15)
    );
    assert_eq!(found[0].line, "  \u{17c}\u{f3}\u{142}w abc");
    assert_eq!(found[0].regex_engine, None);
    assert_eq!(found[0].pattern, None);
}

#[test]
fn whole_word_respects_unicode_letters() {
    let test = tree(&[("/t/a.txt", "\u{17c}abc abc\n".as_bytes())]);
    let found = found(
        &test.ctx(),
        text("abc"),
        "/t",
        vec![("whole_word", Value::Bool(true))],
    );
    assert_eq!(positions(&found), [(1, 6)]);
}

#[test]
fn case_insensitive_matching_folds_unicode() {
    let test = tree(&[("/t/a.txt", "\u{17b}\u{d3}\u{141}W\n".as_bytes())]);
    let found = found(
        &test.ctx(),
        text("\u{17c}\u{f3}\u{142}w"),
        "/t",
        vec![("case_sensitive", Value::Bool(false))],
    );
    assert_eq!(positions(&found), [(1, 1)]);
}

#[test]
fn regular_expressions_report_flags_and_groups() {
    let test = tree(&[("/t/a.txt", b"id 12-ab\nHELLO\n")]);
    let ctx = test.ctx();
    let grouped = found(&ctx, regex(r"(\d+)-(?P<tail>\w+)"), "/t", vec![]);
    assert_eq!(
        grouped[0].groups,
        Some(vec![
            Some("12-ab".to_owned()),
            Some("12".to_owned()),
            Some("ab".to_owned())
        ])
    );
    let named = grouped[0].named.as_ref().unwrap();
    assert_eq!(named.get("tail"), Some(&Some("ab".to_owned())));
    let insensitive = Value::Regex(Regex {
        pattern: "hello".to_owned(),
        flags: RegexFlags {
            case_insensitive: true,
            ..RegexFlags::default()
        },
    });
    assert_eq!(positions(&found(&ctx, insensitive, "/t", vec![])), [(2, 1)]);
}

#[test]
fn back_references_use_the_backtracking_engine() {
    let test = tree(&[("/t/a.txt", b"xaay\n")]);
    let found = found(&test.ctx(), regex(r"(\w)\1"), "/t", vec![]);
    assert_eq!(found[0].text, "aa");
    assert_eq!(
        found[0].regex_engine.map(|engine| engine.as_str()),
        Some("backtracking")
    );
}

#[test]
fn text_patterns_in_regex_mode_are_regular_expressions() {
    let test = tree(&[("/t/a.txt", b"a1 b22\n")]);
    let ctx = test.ctx();
    assert!(found(&ctx, text(r"\d+"), "/t", vec![]).is_empty());
    let found = found(
        &ctx,
        text(r"\d+"),
        "/t",
        vec![
            ("mode", choice("regex")),
            ("only_matching", Value::Bool(true)),
        ],
    );
    assert_eq!(positions(&found), [(1, 2), (1, 5)]);
}

#[test]
fn regular_expressions_in_literal_mode_are_text() {
    let test = tree(&[("/t/a.txt", b"a.b axb\n")]);
    let found = found(
        &test.ctx(),
        regex("a.b"),
        "/t",
        vec![
            ("mode", choice("literal")),
            ("only_matching", Value::Bool(true)),
        ],
    );
    assert_eq!(positions(&found), [(1, 1)]);
}

#[test]
fn invalid_regular_expressions_fail_with_their_pattern() {
    let test = tree(&[("/t/a.txt", b"x\n")]);
    let error = search(&test.ctx(), regex("(unclosed"), "/t", vec![]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Regex);
    assert_eq!(error.found.as_deref(), Some("(unclosed"));
}

#[test]
fn multiline_matches_report_their_end() {
    let test = tree(&[("/t/a.txt", b"a\nfoo\nbar\nz\n")]);
    let found = found(
        &test.ctx(),
        regex(r"foo\nbar"),
        "/t",
        vec![("multiline", Value::Bool(true))],
    );
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].no, found[0].end_line, found[0].end_column),
        (2, Some(3), Some(4))
    );
    assert_eq!(found[0].line, "foo\nbar");
}

#[test]
fn literals_with_line_breaks_match_across_lines() {
    let test = tree(&[("/t/a.txt", b"a\nfoo\r\nbar\n")]);
    let found = found(
        &test.ctx(),
        text("foo\nbar"),
        "/t",
        vec![("whitespace", choice("trailing"))],
    );
    assert_eq!(positions(&found), [(2, 1)]);
}

#[test]
fn context_stops_at_the_file_boundaries() {
    let test = tree(&[("/t/a.txt", b"m\nx\n")]);
    let with_context = found(
        &test.ctx(),
        text("m"),
        "/t",
        vec![("context", Value::Int(2))],
    );
    assert_eq!(with_context[0].before, Some(Vec::new()));
    assert_eq!(with_context[0].after, Some(vec!["x".to_owned()]));
    let without = found(&test.ctx(), text("m"), "/t", vec![]);
    assert_eq!(without[0].before, None);
}

#[test]
fn before_and_after_override_context() {
    let test = tree(&[("/t/a.txt", b"1\n2\nm\n4\n5\n")]);
    let found = found(
        &test.ctx(),
        text("m"),
        "/t",
        vec![("context", Value::Int(2)), ("after", Value::Int(0))],
    );
    assert_eq!(found[0].before, Some(vec!["1".to_owned(), "2".to_owned()]));
    assert_eq!(found[0].after, None);
}

#[test]
fn invert_reports_the_lines_without_matches() {
    let test = tree(&[("/t/a.txt", b"a\nb\na\n")]);
    let found = found(
        &test.ctx(),
        text("a"),
        "/t",
        vec![("invert", Value::Bool(true))],
    );
    assert_eq!(positions(&found), [(2, 1)]);
    assert_eq!(found[0].line, "b");
}

#[test]
fn only_matching_and_max_per_file_limit_the_records() {
    let test = tree(&[("/t/a.txt", b"ab ab ab\nab\n")]);
    let ctx = test.ctx();
    let each = found(
        &ctx,
        text("ab"),
        "/t",
        vec![
            ("only_matching", Value::Bool(true)),
            ("max_per_file", Value::Int(2)),
        ],
    );
    assert_eq!(positions(&each), [(1, 1), (1, 4)]);
    assert_eq!(each[0].line, "ab");
    let lines = found(
        &ctx,
        text("ab"),
        "/t",
        vec![("max_per_file", Value::Int(1))],
    );
    assert_eq!(positions(&lines), [(1, 1)]);
    let all = found(&ctx, text("ab"), "/t", vec![]);
    assert_eq!(positions(&all), [(1, 1), (2, 1)]);
}

#[test]
fn trim_and_dedupe_shorten_the_result() {
    let test = tree(&[("/t/a.txt", b"  same  \n\tsame\nother same\n")]);
    let found = found(
        &test.ctx(),
        text("same"),
        "/t",
        vec![("trim", Value::Bool(true)), ("dedupe", Value::Bool(true))],
    );
    let lines: Vec<&str> = found.iter().map(|found| found.line.as_str()).collect();
    assert_eq!(lines, ["same", "other same"]);
}

#[test]
fn long_lines_are_shortened_and_counted() {
    let line = format!("{}needle{}\n", "a".repeat(30), "b".repeat(30));
    let test = tree(&[("/t/a.txt", line.as_bytes())]);
    let ctx = test.ctx();
    let found = found(
        &ctx,
        text("needle"),
        "/t",
        vec![("max_line", Value::Int(10))],
    );
    assert!(found[0].line.contains("characters omitted"));
    assert!(found[0].line.contains("needle"));
    let notes = ctx.take_notes();
    assert!(
        notes
            .iter()
            .any(|note| note.code == Notable::TruncatedLines && note.count == Some(1))
    );
}

#[test]
fn a_pattern_that_exceeds_the_backtracking_limit_is_reported() {
    // The nested repetition before the back-reference makes the number of
    // steps of the backtracking engine grow exponentially with the run of `a`.
    let line = format!("{}c\n", "a".repeat(28));
    let test = tree(&[("/t/a.txt", line.as_bytes())]);
    let ctx = test.ctx();
    let found = found(&ctx, regex(r"(a*)*\1b"), "/t", vec![]);
    assert!(found.is_empty());
    let notes = ctx.take_notes();
    assert!(
        notes
            .iter()
            .any(|note| note.code == Notable::RegexBacktracking && note.count == Some(1))
    );
}

#[test]
fn binary_files_are_skipped_or_searched_as_text() {
    let test = tree(&[
        ("/t/a.bin", b"\x00\x01needle\x00\x02"),
        ("/t/b.txt", b"needle\n"),
    ]);
    let ctx = test.ctx();
    let skipped = found(&ctx, text("needle"), "/t", vec![]);
    assert_eq!(names(&skipped), [language("/t/b.txt")]);
    let notes = ctx.take_notes();
    assert!(
        notes
            .iter()
            .any(|note| note.code == Notable::BinarySkipped && note.count == Some(1))
    );
    let searched = found(&ctx, text("needle"), "/t", vec![("binary", choice("text"))]);
    assert_eq!(
        names(&searched),
        [language("/t/a.bin"), language("/t/b.txt")]
    );
    assert_eq!(searched[0].start, 2);
}

#[test]
fn legacy_encodings_are_decoded_with_offsets_in_the_file() {
    let test = tree(&[(
        "/t/legacy.txt",
        b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\n",
    )]);
    let found = found(&test.ctx(), text("\u{17c}\u{f3}\u{142}ty"), "/t", vec![]);
    assert_eq!(
        (found[0].no, found[0].column, found[0].start, found[0].end),
        (1, 16, 15, 20)
    );
    assert_eq!(found[0].text, "\u{17c}\u{f3}\u{142}ty");
    assert!(found[0].line.starts_with("Zielona \u{142}\u{105}ka"));
}

#[test]
fn byte_order_marks_are_not_part_of_the_first_line() {
    let test = tree(&[("/t/a.txt", b"\xef\xbb\xbfabc\n")]);
    let found = found(&test.ctx(), regex("^abc"), "/t", vec![]);
    assert_eq!((found[0].column, found[0].start, found[0].end), (1, 3, 6));
}

#[test]
fn utf16_files_are_searched_with_byte_offsets() {
    let mut content = vec![0xff, 0xfe];
    content.extend("a\nkey\n".encode_utf16().flat_map(u16::to_le_bytes));
    let test = tree(&[("/t/wide.txt", &content)]);
    let found = found(&test.ctx(), text("key"), "/t", vec![]);
    assert_eq!(
        (found[0].no, found[0].column, found[0].start, found[0].end),
        (2, 1, 6, 12)
    );
}

#[test]
fn globs_and_file_types_select_files() {
    let test = tree(&[
        ("/t/a.rs", b"x\n"),
        ("/t/b.py", b"x\n"),
        ("/t/Makefile", b"x\n"),
        ("/t/c.md", b"x\n"),
    ]);
    let ctx = test.ctx();
    let globbed = found(
        &ctx,
        text("x"),
        "/t",
        vec![(
            "glob",
            Value::List(vec![
                Value::Glob("*.rs".to_owned()),
                Value::Glob("*.py".to_owned()),
            ]),
        )],
    );
    assert_eq!(names(&globbed), [language("/t/a.rs"), language("/t/b.py")]);
    let make = found(
        &ctx,
        text("x"),
        "/t",
        vec![("file_type", Value::List(vec![text("make")]))],
    );
    assert_eq!(names(&make), [language("/t/Makefile")]);
    let no_docs = found(
        &ctx,
        text("x"),
        "/t",
        vec![("exclude_file_type", Value::List(vec![text("md")]))],
    );
    assert_eq!(
        names(&no_docs),
        [
            language("/t/Makefile"),
            language("/t/a.rs"),
            language("/t/b.py")
        ]
    );
    let unknown = search(
        &ctx,
        text("x"),
        "/t",
        vec![("file_type", Value::List(vec![text("nope")]))],
    )
    .unwrap_err();
    assert_eq!(unknown.code(), ErrorCode::UnknownValue);
}

#[test]
fn hidden_files_can_be_left_out() {
    let test = tree(&[("/t/.env", b"x\n"), ("/t/a", b"x\n")]);
    let ctx = test.ctx();
    assert_eq!(found(&ctx, text("x"), "/t", vec![]).len(), 2);
    let visible = found(&ctx, text("x"), "/t", vec![("hidden", Value::Bool(false))]);
    assert_eq!(names(&visible), [language("/t/a")]);
}

#[test]
fn sort_by_modified_puts_recent_files_first() {
    let test = TestContext::new();
    write(&test, "/t/a.txt", b"x\n");
    test.clock().advance(Duration::from_secs(5));
    write(&test, "/t/b.txt", b"x\n");
    let found = found(
        &test.ctx(),
        text("x"),
        "/t",
        vec![("sort", choice("modified"))],
    );
    assert_eq!(names(&found), [language("/t/b.txt"), language("/t/a.txt")]);
}

fn with_demand(test: &TestContext, demand: Demand) -> ExecCtx {
    test.builder()
        .frame(StageFrame {
            demand,
            ..StageFrame::default()
        })
        .build()
}

#[test]
fn adaptive_results_count_the_files_that_do_not_fit() {
    let test = tree(&[("/t/a.txt", b"x\nx\nx\n"), ("/t/b.txt", b"x\n")]);
    let limited = |limit| {
        with_demand(
            &test,
            Demand {
                limit: Some(limit),
                ..Demand::default()
            },
        )
    };
    let summary = |found: Vec<Match>| -> Vec<(String, Option<u64>, Option<bool>)> {
        found
            .into_iter()
            .map(|found| {
                (
                    found.path.as_str().to_owned(),
                    found.count,
                    found.summarized,
                )
            })
            .collect()
    };
    let adaptive = vec![("adaptive", Value::Bool(true))];
    assert_eq!(
        summary(found(&limited(2), text("x"), "/t", adaptive.clone())),
        [
            (language("/t/a.txt"), Some(3), Some(true)),
            (language("/t/b.txt"), None, None)
        ]
    );
    let full = summary(found(&limited(4), text("x"), "/t", adaptive.clone()));
    assert_eq!(full.len(), 4);
    assert!(
        full.iter()
            .all(|(_, count, summarized)| count.is_none() && summarized.is_none())
    );
    assert_eq!(
        found(
            &with_demand(&test, Demand::default()),
            text("x"),
            "/t",
            adaptive
        )
        .len(),
        4
    );
}

#[test]
fn whitespace_tolerance_applies_to_literals() {
    let test = tree(&[("/t/a.txt", b"let  x = 1;\n")]);
    let ctx = test.ctx();
    let count = |pattern: &str, mode: &str| {
        found(
            &ctx,
            text(pattern),
            "/t",
            vec![("whitespace", choice(mode))],
        )
        .len()
    };
    assert_eq!(count("let x = 1;", "exact"), 0);
    assert_eq!(count("let x = 1;", "normalize"), 1);
    assert_eq!(count("x = 1;  ", "exact"), 0);
    assert_eq!(count("x = 1;  ", "trailing"), 1);
    assert_eq!(count("    let", "exact"), 0);
    assert_eq!(count("    let", "indent"), 1);
}

#[test]
fn counts_files_and_unmatched_outputs() {
    let test = tree(&[("/t/a.txt", b"x x\n"), ("/t/b.txt", b"y\n")]);
    let ctx = test.ctx();
    let counts = found(&ctx, text("x"), "/t", vec![("output", choice("counts"))]);
    assert_eq!(counts.len(), 1);
    assert_eq!((counts[0].count, counts[0].no), (Some(2), 1));
    let entries = |output: &str| -> Vec<(String, Option<u64>)> {
        search(&ctx, text("x"), "/t", vec![("output", choice(output))])
            .unwrap()
            .into_iter()
            .map(|record| Entry::try_from(record).unwrap())
            .map(|entry| (entry.path.as_str().to_owned(), entry.match_count))
            .collect()
    };
    assert_eq!(entries("files"), [(language("/t/a.txt"), Some(2))]);
    assert_eq!(entries("unmatched"), [(language("/t/b.txt"), Some(0))]);
}

#[test]
fn stage_form_searches_entries_content_and_lines() {
    let test = tree(&[("/t/d/a.txt", b"needle\n"), ("/t/b.txt", b"none\nneedle\n")]);
    let ctx = test.ctx();
    let entry =
        |path: &str| Record::new(RecordKind::Entry).with("path", Value::Path(language(path)));
    let args = bound(vec![("pattern", text("needle"))]);
    let input = RecordStream::from_records(vec![entry("/t/d"), entry("/t/b.txt")]);
    let from_entries = matches(execute(&ctx, &args, StageInput::records(input)).unwrap());
    assert_eq!(positions(&from_entries), [(1, 1), (2, 1)]);
    assert_eq!(
        names(&from_entries),
        [language("/t/d/a.txt"), language("/t/b.txt")]
    );

    let content = Content {
        path: nurt_contracts::value::Path::new(language("/t/b.txt")),
        text: "none\nneedle\n".to_owned(),
        etag: "content-tag".to_owned(),
        encoding: "utf-8".to_owned(),
        range: Some(Range::new(Some(10), Some(11)).unwrap()),
        ..Content::default()
    };
    let input = RecordStream::from_records(vec![Record::from(content)]);
    let from_content = matches(execute(&ctx, &args, StageInput::records(input)).unwrap());
    assert_eq!(positions(&from_content), [(11, 1)]);
    assert_eq!(from_content[0].etag, "content-tag");
}

fn line_record(path: &str, no: u64, text: &str, start: u64) -> Record {
    Record::from(Line {
        path: nurt_contracts::value::Path::new(language(path)),
        no,
        text: text.to_owned(),
        etag: "line-tag".to_owned(),
        start: Some(start),
        ..Line::default()
    })
}

#[test]
fn lines_of_one_file_are_counted_together() {
    let test = TestContext::new();
    let ctx = test.ctx();
    let records = vec![
        line_record("/t/a.txt", 5, "two", 20),
        line_record("/t/a.txt", 6, "two two", 24),
        line_record("/t/b.txt", 1, "two", 0),
    ];
    let args = bound(vec![("pattern", text("two")), ("output", choice("counts"))]);
    let counts = matches(
        execute(
            &ctx,
            &args,
            StageInput::records(RecordStream::from_records(records.clone())),
        )
        .unwrap(),
    );
    let summary: Vec<(String, Option<u64>)> = counts
        .iter()
        .map(|found| (found.path.as_str().to_owned(), found.count))
        .collect();
    assert_eq!(
        summary,
        [
            (language("/t/a.txt"), Some(3)),
            (language("/t/b.txt"), Some(1))
        ]
    );
    let args = bound(vec![
        ("pattern", text("two")),
        ("only_matching", Value::Bool(true)),
    ]);
    let each = matches(
        execute(
            &ctx,
            &args,
            StageInput::records(RecordStream::from_records(records)),
        )
        .unwrap(),
    );
    let offsets: Vec<(u64, u64, u64)> = each
        .iter()
        .map(|found| (found.no, found.column, found.start))
        .collect();
    assert_eq!(offsets, [(5, 1, 20), (6, 1, 24), (6, 5, 28), (1, 1, 0)]);
}

#[test]
fn offsets_in_a_line_of_a_legacy_file_count_the_stored_bytes() {
    let test = TestContext::new();
    let ctx = test.ctx();
    // In windows-1250 every Polish letter is one byte; in the UTF-8 text of
    // the record it is two.
    let mut record =
        Line::try_from(line_record("/t/a.txt", 3, "\u{142}\u{f3}d\u{17a} x", 100)).unwrap();
    record.encoding = Some("windows-1250".to_owned());
    let args = bound(vec![("pattern", text("x"))]);
    let found = matches(
        execute(
            &ctx,
            &args,
            StageInput::records(RecordStream::from_records(vec![Record::from(record)])),
        )
        .unwrap(),
    );
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].no, found[0].column), (3, 6));
    assert_eq!((found[0].start, found[0].end), (105, 106));
}

#[test]
fn other_input_records_fail_with_a_type_error() {
    let test = TestContext::new();
    let args = bound(vec![("pattern", text("x"))]);
    let input = RecordStream::from_records(vec![Record::new(RecordKind::Hunk)]);
    let error = execute(&test.ctx(), &args, StageInput::records(input)).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Type);
}

#[test]
fn threads_do_not_change_the_order_of_results() {
    let test = TestContext::new();
    for index in 0..40 {
        write(
            &test,
            &format!("/t/f{index:02}.txt"),
            format!("x {index}\n").as_bytes(),
        );
    }
    let sequential = found(&test.ctx(), text("x"), "/t", vec![]);
    let parallel_ctx = test
        .builder()
        .limits(Limits {
            threads: 4,
            ..Limits::default()
        })
        .build();
    let parallel = found(&parallel_ctx, text("x"), "/t", vec![]);
    assert_eq!(names(&sequential), names(&parallel));
    assert_eq!(sequential.len(), 40);
}

#[test]
fn missing_roots_and_parameters_fail() {
    let test = TestContext::new();
    let ctx = test.ctx();
    let missing = search(&ctx, text("x"), "/none", vec![]).unwrap_err();
    assert_eq!(missing.code(), ErrorCode::NotFound);
    let no_path = execute(
        &ctx,
        &bound(vec![("pattern", text("x"))]),
        StageInput::none(),
    )
    .unwrap_err();
    assert_eq!(no_path.code(), ErrorCode::MissingParam);
    let no_pattern = execute(
        &ctx,
        &bound(vec![("path", Value::Path(language("/t")))]),
        StageInput::none(),
    )
    .unwrap_err();
    assert_eq!(no_pattern.code(), ErrorCode::MissingParam);
}

#[test]
fn invalid_parameter_values_are_rejected() {
    let test = tree(&[("/t/a.txt", b"x\n")]);
    let ctx = test.ctx();
    let negative = search(&ctx, text("x"), "/t", vec![("context", Value::Int(-1))]).unwrap_err();
    assert_eq!(negative.code(), ErrorCode::InvalidValue);
    let unknown = search(&ctx, text("x"), "/t", vec![("output", choice("lines"))]).unwrap_err();
    assert_eq!(unknown.code(), ErrorCode::UnknownValue);
    let wrong = search(&ctx, Value::Int(3), "/t", vec![]).unwrap_err();
    assert_eq!(wrong.code(), ErrorCode::Type);
    let empty = search(&ctx, Value::List(Vec::new()), "/t", vec![]).unwrap_err();
    assert_eq!(empty.code(), ErrorCode::InvalidValue);
}

#[test]
fn etags_carry_a_hash_only_when_demanded() {
    let test = tree(&[("/t/a.txt", b"x\n")]);
    let hashed = found(&test.ctx(), text("x"), "/t", vec![]);
    assert!(hashed[0].etag.contains("sha256:"));
    let plain_ctx = with_demand(
        &test,
        Demand {
            etag: false,
            ..Demand::default()
        },
    );
    let plain = found(&plain_ctx, text("x"), "/t", vec![]);
    assert!(!plain[0].etag.is_empty());
    assert!(!plain[0].etag.contains("sha256:"));
}

/// Code service of the tests: files ending in `.rs` are Rust, `//` starts a
/// comment, `"` encloses a string, the whole file is the function
/// `app.main`, and a syntax pattern matches its text literally.
struct FakeCode;

#[derive(Debug)]
struct FakePattern {
    language: LanguageId,
    source: String,
}

impl SyntaxPattern for FakePattern {
    fn language(&self) -> &LanguageId {
        &self.language
    }

    fn source(&self) -> &str {
        &self.source
    }

    fn metavariables(&self) -> Vec<String> {
        Vec::new()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn position(bytes: &[u8], offset: usize) -> TextPosition {
    let before = &bytes[..offset];
    let line_start = before
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |at| at + 1);
    let column = String::from_utf8_lossy(&before[line_start..])
        .chars()
        .count();
    TextPosition {
        line: u32::try_from(before.iter().filter(|&&byte| byte == b'\n').count() + 1).unwrap(),
        column: u32::try_from(column + 1).unwrap(),
    }
}

fn span(bytes: &[u8], start: usize, end: usize) -> TextSpan {
    TextSpan {
        bytes: ByteRange::new(start as u64, end as u64),
        start: position(bytes, start),
        end: position(bytes, end),
    }
}

impl CodeService for FakeCode {
    fn languages(&self) -> Vec<LanguageInfo> {
        Vec::new()
    }

    fn language_of(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
        let rust = path?.extension()? == "rs";
        rust.then(|| LanguageId::new("rust").unwrap())
    }

    fn outline(
        &self,
        _bytes: &[u8],
        _language: &LanguageId,
        _depth: Option<u32>,
    ) -> Result<Vec<SymbolInfo>> {
        Ok(Vec::new())
    }

    fn find_symbol(
        &self,
        _bytes: &[u8],
        _language: &LanguageId,
        _name: &str,
    ) -> Result<Vec<SymbolInfo>> {
        Ok(Vec::new())
    }

    fn enclosing(
        &self,
        bytes: &[u8],
        _language: &LanguageId,
        _offset: u64,
    ) -> Result<Option<SymbolInfo>> {
        let whole = span(bytes, 0, bytes.len().saturating_sub(1));
        Ok(Some(SymbolInfo {
            name: "main".to_owned(),
            kind: nurt_contracts::services::SymbolKind::Function,
            container: Some("app".to_owned()),
            depth: 0,
            span: whole,
            name_span: whole,
            signature: None,
            exported: None,
        }))
    }

    fn classify_spans(&self, bytes: &[u8], _language: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        let mut spans = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let rest = &bytes[at..];
            let (class, length) = if rest.starts_with(b"//") {
                (
                    SpanClass::Comment,
                    rest.iter()
                        .position(|&byte| byte == b'\n')
                        .unwrap_or(rest.len()),
                )
            } else if rest[0] == b'"' {
                (
                    SpanClass::String,
                    rest[1..]
                        .iter()
                        .position(|&byte| byte == b'"')
                        .map_or(rest.len(), |end| end + 2),
                )
            } else {
                at += 1;
                continue;
            };
            spans.push(ClassifiedSpan {
                bytes: ByteRange::new(at as u64, (at + length) as u64),
                class,
            });
            at += length;
        }
        Ok(spans)
    }

    fn check_syntax(&self, _bytes: &[u8], _language: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        Ok(Vec::new())
    }

    fn compile_pattern(
        &self,
        pattern: &str,
        language: &LanguageId,
    ) -> Result<Box<dyn SyntaxPattern>> {
        Ok(Box::new(FakePattern {
            language: language.clone(),
            source: pattern.to_owned(),
        }))
    }

    fn syntax_search(&self, bytes: &[u8], pattern: &dyn SyntaxPattern) -> Result<Vec<SyntaxMatch>> {
        let needle = pattern.source().as_bytes();
        let found = (0..bytes.len().saturating_sub(needle.len()) + 1)
            .filter(|&start| bytes[start..].starts_with(needle))
            .map(|start| {
                let text = pattern.source().to_owned();
                SyntaxMatch {
                    span: span(bytes, start, start + needle.len()),
                    node_kind: "call_expression".to_owned(),
                    captures: vec![SyntaxCapture {
                        name: "ALL".to_owned(),
                        spans: Vec::new(),
                        text: text.clone(),
                    }],
                    text,
                }
            })
            .collect();
        Ok(found)
    }

    fn render_template(&self, _found: &SyntaxMatch, _template: &str) -> Result<String> {
        Ok(String::new())
    }

    fn references(
        &self,
        _bytes: &[u8],
        _language: &LanguageId,
        _name: &str,
    ) -> Result<Vec<SymbolReference>> {
        Ok(Vec::new())
    }

    fn function_metrics(
        &self,
        _bytes: &[u8],
        _language: &LanguageId,
    ) -> Result<Vec<FunctionMetrics>> {
        Ok(Vec::new())
    }

    fn tokens(&self, _bytes: &[u8], _language: &LanguageId) -> Result<Vec<SyntaxToken>> {
        Ok(Vec::new())
    }
}

const RUST_SOURCE: &[u8] = b"fn main() {\n    let a = \"needle\"; // needle\n    needle();\n}\n";

fn with_code() -> TestContext {
    let test = TestContext::new().with_services(Services::new().with_code(Arc::new(FakeCode)));
    write(&test, "/t/main.rs", RUST_SOURCE);
    write(&test, "/t/notes.txt", b"needle()\n");
    test
}

#[test]
fn scope_selects_code_comments_or_strings() {
    let test = with_code();
    let ctx = test.ctx();
    let scoped = |scope: &str| {
        let found = found(
            &ctx,
            text("needle"),
            "/t/main.rs",
            vec![("scope", choice(scope))],
        );
        positions(&found)
    };
    assert_eq!(scoped("code"), [(3, 5)]);
    assert_eq!(scoped("comments"), [(2, 26)]);
    assert_eq!(scoped("strings"), [(2, 14)]);
    assert_eq!(scoped("any"), [(2, 14), (3, 5)]);
}

#[test]
fn files_of_unknown_language_count_as_code() {
    let test = with_code();
    let ctx = test.ctx();
    let code = found(
        &ctx,
        text("needle"),
        "/t/notes.txt",
        vec![("scope", choice("code"))],
    );
    assert_eq!(positions(&code), [(1, 1)]);
    let comments = found(
        &ctx,
        text("needle"),
        "/t/notes.txt",
        vec![("scope", choice("comments"))],
    );
    assert!(comments.is_empty());
}

#[test]
fn enclosing_adds_the_container_of_each_match() {
    let test = with_code();
    let found = found(
        &test.ctx(),
        text("needle();"),
        "/t/main.rs",
        vec![("enclosing", Value::Bool(true))],
    );
    let container = found[0].container.as_ref().unwrap();
    assert_eq!(container.name, "app.main");
    assert_eq!(container.kind, SymbolKind::Function);
    assert_eq!((container.line, container.end_line), (1, Some(4)));
}

#[test]
fn enclosing_without_the_code_service_adds_nothing() {
    let test = TestContext::new().with_services(Services::new());
    write(&test, "/t/main.rs", RUST_SOURCE);
    let found = found(
        &test.ctx(),
        text("needle();"),
        "/t",
        vec![("enclosing", Value::Bool(true))],
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].container, None);
}

#[test]
fn scope_and_syntax_need_the_code_service() {
    let test = TestContext::new().with_services(Services::new());
    write(&test, "/t/main.rs", RUST_SOURCE);
    let ctx = test.ctx();
    let scope = search(&ctx, text("x"), "/t", vec![("scope", choice("code"))]).unwrap_err();
    assert_eq!(scope.code(), ErrorCode::Unsupported);
    let syntax = search(&ctx, text("x"), "/t", vec![("mode", choice("syntax"))]).unwrap_err();
    assert_eq!(syntax.code(), ErrorCode::Unsupported);
}

#[test]
fn invert_cannot_be_combined_with_scope() {
    let test = with_code();
    let error = search(
        &test.ctx(),
        text("x"),
        "/t",
        vec![("scope", choice("code")), ("invert", Value::Bool(true))],
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::ParamConflict);
}

#[test]
fn syntax_patterns_search_files_of_known_languages() {
    let test = with_code();
    let found = found(
        &test.ctx(),
        text("needle()"),
        "/t",
        vec![("mode", choice("syntax")), ("context", Value::Int(1))],
    );
    assert_eq!(names(&found), [language("/t/main.rs")]);
    assert_eq!(positions(&found), [(3, 5)]);
    assert_eq!(found[0].text, "needle()");
    assert_eq!(found[0].line, "    needle();");
    assert_eq!(found[0].before.as_ref().map(Vec::len), Some(1));
    assert_eq!(found[0].after, Some(vec!["}".to_owned()]));
    let named = found[0].named.as_ref().unwrap();
    assert_eq!(named.get("ALL"), Some(&Some("needle()".to_owned())));
    assert_eq!(found[0].start, 48);
}

#[test]
fn syntax_patterns_follow_the_given_language() {
    let test = with_code();
    let found = found(
        &test.ctx(),
        text("needle()"),
        "/t/notes.txt",
        vec![("mode", choice("syntax")), ("language", text("rust"))],
    );
    assert_eq!(positions(&found), [(1, 1)]);
}

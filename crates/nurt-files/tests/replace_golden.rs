// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of the page of `replace` and the behaviour of `replace` on
//! the virtual file system: every form, the parameters of the catalog,
//! preserved encodings and line endings, and the transaction protocol.

// Helpers outside the test functions unwrap results of the virtual file
// system and the stage, whose failure fails the test.
#![allow(clippy::unwrap_used)]

mod support_report;

use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, RollbackReason, TargetIndex, TargetPath};
use nurt_contracts::record::{self, Change};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{ArgValue, BoundArgs, RecordStream, StageImpl, StageInput};
use nurt_contracts::value::{self, Range, Record, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

const COMMAND: &str = "replace";

/// Windows-1250 text with Polish letters and CRLF line endings, long enough
/// for detection to tell it from double-byte encodings.
const CP1250_CRLF: &[u8] = b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\n";

fn abs(path: &str) -> AbsPath {
    test_path(path).unwrap()
}

fn stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == COMMAND)
        .unwrap()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn choice(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

fn regex(pattern: &str) -> Value {
    Value::Regex(value::Regex::new(pattern, "").unwrap())
}

fn files(paths: &[&AbsPath]) -> Value {
    Value::List(
        paths
            .iter()
            .map(|path| Value::Path(path.to_language_string()))
            .collect(),
    )
}

fn args(params: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(COMMAND);
    for (name, value) in params {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

#[derive(Debug)]
struct Outcome {
    changes: Vec<Change>,
    notable: Vec<NotableEntry>,
}

fn run(test: &TestContext, args: &BoundArgs, input: StageInput<'_>) -> Result<Outcome> {
    let stage = stage();
    let effect = stage.as_effect().unwrap();
    let ctx = test.ctx();
    let plan = effect.plan(args, input, &ctx)?;
    support_report::assert_plan_valid(args.command(), &plan.changes);
    let notable = plan.notable.clone();
    let mut tx = test.journal().begin(begin_request(COMMAND));
    match effect.apply(args, plan, tx.as_mut(), &ctx) {
        Ok(outcome) => {
            let changes = outcome.changes;
            support_report::assert_changes_valid(args.command(), &changes);
            tx.commit()?;
            Ok(Outcome { changes, notable })
        }
        Err(error) => {
            tx.rollback(RollbackReason::TargetFailed)?;
            Err(error)
        }
    }
}

fn source(test: &TestContext, params: &[(&str, Value)]) -> Result<Outcome> {
    run(test, &args(params), StageInput::none())
}

fn content(test: &TestContext, path: &AbsPath) -> String {
    String::from_utf8(test.fs().read_file(path).unwrap()).unwrap()
}

fn with_files(entries: &[(&str, &str)]) -> TestContext {
    let test = TestContext::new();
    for (path, body) in entries {
        test.fs().write_file(&abs(path), body.as_bytes()).unwrap();
    }
    test
}

/// Declared targets and successfully applied target indexes of the journal.
fn journal(test: &TestContext) -> (Vec<TargetPath>, Vec<TargetIndex>) {
    let mut declared = Vec::new();
    let mut applied = Vec::new();
    for call in test.journal().calls() {
        match call {
            JournalCall::Preserve { targets, .. } => declared.extend(targets),
            JournalCall::Apply {
                index, ok: true, ..
            } => applied.push(index),
            _ => {}
        }
    }
    (declared, applied)
}

fn assert_every_change_declared(test: &TestContext, changed: &[&AbsPath]) {
    let (declared, applied) = journal(test);
    let expected: Vec<TargetPath> = changed
        .iter()
        .map(|path| TargetPath::Fs((*path).clone()))
        .collect();
    assert_eq!(declared, expected);
    let indexes: Vec<TargetIndex> = (0..declared.len())
        .map(|index| TargetIndex(u32::try_from(index).unwrap()))
        .collect();
    assert_eq!(applied, indexes);
}

#[test]
fn example_replaces_one_occurrence_in_one_file() {
    let test = with_files(&[("/home/user/app/a.go", "x := oldName(1)\ny := oldName(2)\n")]);
    let path = abs("/home/user/app/a.go");
    let outcome = source(
        &test,
        &[
            ("pattern", text("oldName(")),
            ("with", text("newName(")),
            ("in", files(&[&path])),
            ("occurrence", Value::Int(1)),
        ],
    )
    .unwrap();
    assert_eq!(content(&test, &path), "x := newName(1)\ny := oldName(2)\n");
    assert_eq!(outcome.changes.len(), 1);
    assert_eq!(outcome.changes[0].replacements, Some(1));
    assert_eq!(outcome.changes[0].added, Some(1));
    assert_eq!(outcome.changes[0].removed, Some(1));
    assert_every_change_declared(&test, &[&path]);
}

#[test]
fn example_rewrites_calls_with_groups_in_a_directory() {
    let test = with_files(&[
        ("/home/user/app/src/main.rs", "let a = load().unwrap();\n"),
        (
            "/home/user/app/src/io/read.rs",
            "let b = open(p).unwrap();\n",
        ),
        ("/home/user/app/README.md", "call().unwrap()\n"),
    ]);
    let root = abs("/home/user/app");
    let outcome = source(
        &test,
        &[
            ("pattern", regex(r"(\w+\([^)]*\))\.unwrap\(\)")),
            ("with", text("$1?")),
            ("in", files(&[&root])),
            ("glob", Value::Glob("**/*.rs".to_owned())),
        ],
    )
    .unwrap();
    assert_eq!(
        content(&test, &abs("/home/user/app/src/main.rs")),
        "let a = load()?;\n"
    );
    assert_eq!(
        content(&test, &abs("/home/user/app/src/io/read.rs")),
        "let b = open(p)?;\n"
    );
    assert_eq!(
        content(&test, &abs("/home/user/app/README.md")),
        "call().unwrap()\n"
    );
    assert_eq!(outcome.changes.len(), 2);
    assert_every_change_declared(
        &test,
        &[
            &abs("/home/user/app/src/io/read.rs"),
            &abs("/home/user/app/src/main.rs"),
        ],
    );
}

#[test]
fn example_matches_a_block_with_other_indentation() {
    let before = "def run():\n    if ready:\n        start()\n        report()\n";
    let test = with_files(&[("/home/user/app/run.py", before)]);
    let path = abs("/home/user/app/run.py");
    source(
        &test,
        &[
            ("pattern", text("start()\nreport()")),
            ("with", text("prepare()\nstart()\nreport()")),
            ("in", files(&[&path])),
        ],
    )
    .unwrap();
    assert_eq!(
        content(&test, &path),
        "def run():\n    if ready:\n        prepare()\n        start()\n        report()\n"
    );
}

#[test]
fn example_replaces_inside_input_lines() {
    let test = with_files(&[(
        "/home/user/notes.txt",
        "draft one\ndraft two\ndraft three\n",
    )]);
    let path = abs("/home/user/notes.txt");
    let lines: Vec<Record> = [(2, "draft two"), (3, "draft three")]
        .into_iter()
        .map(|(no, line)| {
            Record::from(record::Line {
                path: value::Path::new(path.to_language_string()),
                no,
                text: line.to_owned(),
                ..record::Line::default()
            })
        })
        .collect();
    run(
        &test,
        &args(&[("pattern", text("draft")), ("with", text("final"))]),
        StageInput::records(RecordStream::from_records(lines)),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "draft one\nfinal two\nfinal three\n");
}

#[test]
fn example_replaces_search_matches_with_their_groups() {
    let test = with_files(&[("/home/user/app/config.ini", "port=8080\nhost=example.com\n")]);
    let path = abs("/home/user/app/config.ini");
    let matches: Vec<Record> = [
        (1, "port=8080", "port", "8080", 0),
        (2, "host=example.com", "host", "example.com", 10),
    ]
    .into_iter()
    .map(|(no, found, key, value, start)| {
        Record::from(record::Match {
            path: value::Path::new(path.to_language_string()),
            no,
            column: 1,
            start,
            end: start + found.len() as u64,
            text: found.to_owned(),
            line: found.to_owned(),
            groups: Some(vec![
                Some(found.to_owned()),
                Some(key.to_owned()),
                Some(value.to_owned()),
            ]),
            ..record::Match::default()
        })
    })
    .collect();
    let outcome = run(
        &test,
        &args(&[("with", text("$1 = $2"))]),
        StageInput::records(RecordStream::from_records(matches)),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "port = 8080\nhost = example.com\n");
    assert_eq!(outcome.changes[0].replacements, Some(2));
}

#[test]
fn example_requires_the_expected_number_of_replacements() {
    let test = with_files(&[
        (
            "/home/user/site/a.html",
            "http://example.com http://example.com\n",
        ),
        ("/home/user/site/b.html", "http://example.com\n"),
    ]);
    let root = abs("/home/user/site");
    let too_many = source(
        &test,
        &[
            ("pattern", text("http://")),
            ("with", text("https://")),
            ("in", files(&[&root])),
            ("expect", Value::Int(2)),
        ],
    )
    .unwrap_err();
    assert_eq!(too_many.code(), ErrorCode::Assert);
    assert_eq!(too_many.sample.len(), 3);
    assert_eq!(
        content(&test, &abs("/home/user/site/b.html")),
        "http://example.com\n"
    );
    assert!(journal(&test).0.is_empty());

    source(
        &test,
        &[
            ("pattern", text("http://")),
            ("with", text("https://")),
            ("in", files(&[&root])),
            ("expect", Value::Int(3)),
        ],
    )
    .unwrap();
    assert_eq!(
        content(&test, &abs("/home/user/site/a.html")),
        "https://example.com https://example.com\n"
    );
}

#[test]
fn an_empty_replacement_deletes_the_match() {
    let test = with_files(&[("/data/a.txt", "keep remove keep\n")]);
    let path = abs("/data/a.txt");
    source(
        &test,
        &[
            ("pattern", text(" remove")),
            ("with", text("")),
            ("in", files(&[&path])),
        ],
    )
    .unwrap();
    assert_eq!(content(&test, &path), "keep keep\n");
}

#[test]
fn every_occurrence_is_replaced_by_default_and_whole_word_limits_matches() {
    let test = with_files(&[("/data/a.txt", "id idx id_2 id\n")]);
    let path = abs("/data/a.txt");
    source(
        &test,
        &[
            ("pattern", text("id")),
            ("with", text("key")),
            ("in", files(&[&path])),
            ("whole_word", Value::Bool(true)),
        ],
    )
    .unwrap();
    assert_eq!(content(&test, &path), "key idx id_2 key\n");
    source(
        &test,
        &[
            ("pattern", text("id")),
            ("with", text("ID")),
            ("in", files(&[&path])),
        ],
    )
    .unwrap();
    assert_eq!(content(&test, &path), "key IDx ID_2 key\n");
}

#[test]
fn case_insensitive_matching_finds_every_spelling() {
    let test = with_files(&[("/data/a.txt", "Error error ERROR\n")]);
    let path = abs("/data/a.txt");
    source(
        &test,
        &[
            ("pattern", text("error")),
            ("with", text("issue")),
            ("in", files(&[&path])),
            ("case_sensitive", Value::Bool(false)),
        ],
    )
    .unwrap();
    assert_eq!(content(&test, &path), "issue issue issue\n");
}

#[test]
fn whitespace_settings_control_the_tolerance() {
    let body = "fn f() {\n\tlet a = 1;   \n}\n";
    for (setting, replaced) in [
        ("exact", false),
        ("trailing", false),
        ("indent", true),
        ("normalize", true),
    ] {
        let test = with_files(&[("/data/a.rs", body)]);
        let path = abs("/data/a.rs");
        let outcome = source(
            &test,
            &[
                ("pattern", text("    let a = 1;\n")),
                ("with", text("    let a = 2;\n")),
                ("in", files(&[&path])),
                ("whitespace", choice(setting)),
            ],
        )
        .unwrap();
        assert_eq!(!outcome.changes.is_empty(), replaced, "{setting}");
        if replaced {
            assert_eq!(
                content(&test, &path),
                "fn f() {\n\tlet a = 2;\n}\n",
                "{setting}"
            );
        }
    }
    let test = with_files(&[("/data/b.txt", "value = 1   \n")]);
    let path = abs("/data/b.txt");
    source(
        &test,
        &[
            ("pattern", text("value = 1\n")),
            ("with", text("value = 2\n")),
            ("in", files(&[&path])),
            ("whitespace", choice("trailing")),
        ],
    )
    .unwrap();
    assert_eq!(content(&test, &path), "value = 2\n");
}

#[test]
fn mixed_indentation_is_converted_to_the_unit_of_the_file() {
    let test = with_files(&[("/data/a.py", "def f():\n\tif x:\n\t\tpass\n")]);
    let path = abs("/data/a.py");
    source(
        &test,
        &[
            ("pattern", text("if x:\n    pass")),
            ("with", text("if x:\n    run()\n    pass")),
            ("in", files(&[&path])),
        ],
    )
    .unwrap();
    assert_eq!(
        content(&test, &path),
        "def f():\n\tif x:\n\t\trun()\n\t\tpass\n"
    );
}

#[test]
fn nothing_matched_is_notable_or_an_error() {
    let test = with_files(&[("/data/a.txt", "alpha beta\n")]);
    let path = abs("/data/a.txt");
    let skipped = source(
        &test,
        &[
            ("pattern", text("gamma")),
            ("with", text("delta")),
            ("in", files(&[&path])),
        ],
    )
    .unwrap();
    assert!(skipped.changes.is_empty());
    assert_eq!(skipped.notable[0].code, Notable::NothingMatched);
    let error = source(
        &test,
        &[
            ("pattern", text("gamma beta")),
            ("with", text("delta")),
            ("in", files(&[&path])),
            ("on_missing", choice("error")),
        ],
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NoMatch);
    assert_eq!(error.sample, [format!("{path}:1: alpha beta")]);
}

#[test]
fn backtracking_patterns_and_named_groups_are_supported() {
    let test = with_files(&[("/data/a.txt", "foobar foobaz\n")]);
    let path = abs("/data/a.txt");
    source(
        &test,
        &[
            ("pattern", regex(r"(?<stem>foo)(?=bar)")),
            ("with", text("${stem}d")),
            ("in", files(&[&path])),
        ],
    )
    .unwrap();
    assert_eq!(content(&test, &path), "foodbar foobaz\n");
}

#[test]
fn invalid_templates_fail_with_e_regex() {
    let test = with_files(&[("/data/a.txt", "a\n")]);
    let path = abs("/data/a.txt");
    let error = source(
        &test,
        &[
            ("pattern", regex(r"(a)")),
            ("with", text("${missing}")),
            ("in", files(&[&path])),
        ],
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Regex);
    assert_eq!(content(&test, &path), "a\n");
}

#[test]
fn legacy_encodings_and_line_endings_survive_a_replacement() {
    let test = TestContext::new();
    let path = abs("/data/legacy.txt");
    test.fs().write_file(&path, CP1250_CRLF).unwrap();
    let outcome = source(
        &test,
        &[
            ("pattern", text("koniec")),
            ("with", text("\u{17c}\u{f3}\u{142}w\nnowy")),
            ("in", files(&[&path])),
        ],
    )
    .unwrap();
    assert_eq!(
        test.fs().read_file(&path).unwrap(),
        b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\n\xbf\xf3\xb3w\r\nnowy\r\n"
    );
    assert_eq!(
        outcome.changes[0].encoding_after.as_deref(),
        Some("windows-1250")
    );
    let codes: Vec<Notable> = outcome.notable.iter().map(|entry| entry.code).collect();
    assert!(codes.contains(&Notable::EncodingPreserved));
    assert!(codes.contains(&Notable::LineEndingPreserved));

    let wide = abs("/data/wide.txt");
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend("a\r\nb\r\n".encode_utf16().flat_map(u16::to_le_bytes));
    test.fs().write_file(&wide, &bytes).unwrap();
    source(
        &test,
        &[
            ("pattern", text("b")),
            ("with", text("c")),
            ("in", files(&[&wide])),
        ],
    )
    .unwrap();
    let mut expected = vec![0xff, 0xfe];
    expected.extend("a\r\nc\r\n".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(test.fs().read_file(&wide).unwrap(), expected);
}

#[test]
fn unrepresentable_replacements_change_nothing() {
    let test = TestContext::new();
    let path = abs("/data/legacy.txt");
    test.fs().write_file(&path, CP1250_CRLF).unwrap();
    let error = source(
        &test,
        &[
            ("pattern", text("koniec")),
            ("with", text("\u{4e2d}")),
            ("in", files(&[&path])),
        ],
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Encoding);
    assert_eq!(test.fs().read_file(&path).unwrap(), CP1250_CRLF);
}

#[test]
fn if_match_with_another_etag_fails_with_stale_and_changes_nothing() {
    let test = with_files(&[("/data/a.txt", "old\n")]);
    let path = abs("/data/a.txt");
    let error = source(
        &test,
        &[
            ("pattern", text("old")),
            ("with", text("new")),
            ("in", files(&[&path])),
            ("if_match", text("4-1-1-1-0")),
        ],
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(content(&test, &path), "old\n");
    assert!(journal(&test).0.is_empty());
}

#[test]
fn a_file_changed_after_the_plan_is_not_overwritten() {
    let test = with_files(&[("/data/a.txt", "old\n")]);
    let path = abs("/data/a.txt");
    let stage = stage();
    let effect = stage.as_effect().unwrap();
    let ctx = test.ctx();
    let given = args(&[
        ("pattern", text("old")),
        ("with", text("new")),
        ("in", files(&[&path])),
    ]);
    let plan = effect.plan(&given, StageInput::none(), &ctx).unwrap();
    test.fs().write_file(&path, b"changed meanwhile\n").unwrap();
    let mut tx = test.journal().begin(begin_request(COMMAND));
    let error = effect.apply(&given, plan, tx.as_mut(), &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(content(&test, &path), "changed meanwhile\n");
}

#[test]
fn several_files_are_one_bulk_operation() {
    let test = with_files(&[
        ("/data/a.txt", "x\n"),
        ("/data/b.txt", "x\n"),
        ("/data/c.txt", "y\n"),
    ]);
    let stage = stage();
    let effect = stage.as_effect().unwrap();
    let ctx = test.ctx();
    let given = args(&[
        ("pattern", text("x")),
        ("with", text("z")),
        ("in", files(&[&abs("/data")])),
    ]);
    let plan = effect.plan(&given, StageInput::none(), &ctx).unwrap();
    assert!(plan.bulk);
    assert_eq!(plan.changes.len(), 2);
    assert!(
        plan.changes
            .iter()
            .all(|change| change.diff.as_ref().is_some_and(|diff| diff.len() == 1))
    );
}

#[test]
fn replacing_and_replacing_back_restores_the_text() {
    let samples = [
        "alpha beta\ngamma\n",
        "one\r\ntwo\r\nthree",
        "\tindented line\n    spaced line\n",
        "x = old_value;\ny = old_value + 1;\n",
    ];
    let pairs = [
        ("beta", "BETA"),
        ("two", "zwei"),
        ("line", "row"),
        ("old_value", "new_value"),
    ];
    for (body, (from, to)) in samples.iter().zip(pairs) {
        let test = with_files(&[("/data/a.txt", body)]);
        let path = abs("/data/a.txt");
        for (pattern, with) in [(from, to), (to, from)] {
            source(
                &test,
                &[
                    ("pattern", text(pattern)),
                    ("with", text(with)),
                    ("in", files(&[&path])),
                    ("whitespace", choice("exact")),
                ],
            )
            .unwrap();
        }
        assert_eq!(content(&test, &path), *body);
    }
}

#[test]
fn input_content_with_a_range_limits_the_lines() {
    let test = with_files(&[("/data/a.txt", "a\na\na\n")]);
    let path = abs("/data/a.txt");
    let record = Record::from(record::Content {
        path: value::Path::new(path.to_language_string()),
        range: Some(Range::new(Some(3), None).unwrap()),
        ..record::Content::default()
    });
    run(
        &test,
        &args(&[("pattern", text("a")), ("with", text("b"))]),
        StageInput::records(RecordStream::once(record)),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "a\na\nb\n");
}

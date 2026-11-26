// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Examples of the page of `edit` and the behaviour of `edit` on the virtual
//! file system: lists of changes, all or none, validation of the result and
//! the transaction protocol.

// Helpers outside the test functions unwrap results of the virtual file
// system and the stage, whose failure fails the test.
#![allow(clippy::unwrap_used)]

mod support_report;

use std::path::Path;
use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, RollbackReason, TargetIndex, TargetPath};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::record::Change;
use nurt_contracts::report::Notable;
use nurt_contracts::services::{
    ByteRange, DocFormatId, DocFormatInfo, DocItem, DocService, IssueSeverity, SerializeOptions,
    Services, SyntaxIssue, TextPosition, TextSpan,
};
use nurt_contracts::stage::{ArgValue, BoundArgs, Event, StageImpl, StageInput};
use nurt_contracts::value::{self, Range, Record, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

const COMMAND: &str = "edit";

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

fn lines(first: i64, last: i64) -> Value {
    Value::Range(Range::new(Some(first), Some(last)).unwrap())
}

fn change(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn args(path: &AbsPath, changes: Vec<Value>, params: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(COMMAND)
        .with(
            "path",
            ArgValue::Value(Value::Path(path.to_language_string())),
            false,
        )
        .with("changes", ArgValue::Value(Value::List(changes)), false);
    for (name, value) in params {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn run_in(test: &TestContext, args: &BoundArgs) -> Result<Vec<Change>> {
    let stage = stage();
    let effect = stage.as_effect().unwrap();
    let ctx = test.ctx();
    let plan = effect.plan(args, StageInput::none(), &ctx)?;
    support_report::assert_plan_valid(args.command(), &plan.changes);
    let mut tx = test.journal().begin(begin_request(COMMAND));
    match effect.apply(args, plan, tx.as_mut(), &ctx) {
        Ok(outcome) => {
            let changes = outcome.changes;
            support_report::assert_changes_valid(args.command(), &changes);
            tx.commit()?;
            Ok(changes)
        }
        Err(error) => {
            tx.rollback(RollbackReason::ValidateFailed)?;
            Err(error)
        }
    }
}

fn content(test: &TestContext, path: &AbsPath) -> String {
    String::from_utf8(test.fs().read_file(path).unwrap()).unwrap()
}

fn with_file(test: TestContext, path: &str, body: &str) -> (TestContext, AbsPath) {
    let path = abs(path);
    test.fs().write_file(&path, body.as_bytes()).unwrap();
    (test, path)
}

fn applied(test: &TestContext) -> (Vec<TargetPath>, Vec<TargetIndex>) {
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

/// Document service that knows `.json` files and reports one syntax error
/// when the braces are unbalanced.
struct BraceChecker;

impl DocService for BraceChecker {
    fn formats(&self) -> Vec<DocFormatInfo> {
        Vec::new()
    }

    fn detect_format(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<DocFormatId> {
        path.filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .map(|_| DocFormatId::JSON)
    }

    fn read_keys(
        &self,
        _bytes: &[u8],
        _format: &DocFormatId,
        _key: &KeyPath,
        _depth: Option<u32>,
    ) -> Result<Vec<DocItem>> {
        Ok(Vec::new())
    }

    fn check_syntax(&self, bytes: &[u8], _format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
        let open = bytes.iter().filter(|byte| **byte == b'{').count();
        let close = bytes.iter().filter(|byte| **byte == b'}').count();
        if open == close {
            return Ok(Vec::new());
        }
        let position = TextPosition { line: 1, column: 1 };
        Ok(vec![SyntaxIssue {
            severity: IssueSeverity::Error,
            message: "unbalanced braces".to_owned(),
            span: TextSpan {
                bytes: ByteRange::new(0, 1),
                start: position,
                end: position,
            },
            kind: Some("unbalanced".to_owned()),
        }])
    }

    fn serialize_records(
        &self,
        _records: &[Record],
        _format: &DocFormatId,
        _options: &SerializeOptions,
    ) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

fn checked() -> TestContext {
    TestContext::new().with_services(Services::new().with_doc(Arc::new(BraceChecker)))
}

#[test]
fn example_changes_a_value_and_adds_a_line() {
    let (test, path) = with_file(
        TestContext::new(),
        "/home/user/app/config.toml",
        "[server]\ndebug = true\nport = 8080\n",
    );
    let changes = run_in(
        &test,
        &args(
            &path,
            vec![
                change(&[
                    ("replace", text("debug = true")),
                    ("with", text("debug = false")),
                ]),
                change(&[
                    ("insert", text("log = \"info\"")),
                    ("line", Value::Int(2)),
                    ("position", choice("after")),
                ]),
            ],
            &[],
        ),
    )
    .unwrap();
    assert_eq!(
        content(&test, &path),
        "[server]\ndebug = false\nlog = \"info\"\nport = 8080\n"
    );
    assert_eq!(changes[0].replacements, Some(1));
    let (declared, indexes) = applied(&test);
    assert_eq!(declared, [TargetPath::Fs(path)]);
    assert_eq!(indexes, [TargetIndex(0)]);
}

#[test]
fn example_replaces_a_range_of_lines() {
    let (test, path) = with_file(
        TestContext::new(),
        "/home/user/app/util.py",
        "def add(a, b):\n    total = a + b\n    print(total)\n    return total\n",
    );
    run_in(
        &test,
        &args(
            &path,
            vec![
                change(&[("delete_lines", lines(2, 4))]),
                change(&[
                    ("insert", text("    return a + b")),
                    ("line", Value::Int(2)),
                    ("position", choice("before")),
                ]),
            ],
            &[],
        ),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "def add(a, b):\n    return a + b\n");
}

#[test]
fn example_bumps_one_version_with_a_regular_expression() {
    let (test, path) = with_file(
        TestContext::new(),
        "/home/user/app/Cargo.toml",
        "version = \"1.4.2\"\nedition = \"2024\"\n",
    );
    run_in(
        &test,
        &args(
            &path,
            vec![change(&[
                (
                    "replace",
                    Value::Regex(
                        value::Regex::new(r#"version = "(\d+)\.(\d+)\.\d+""#, "").unwrap(),
                    ),
                ),
                ("with", text("version = \"$1.${2}.3\"")),
                ("occurrence", Value::Int(1)),
            ])],
            &[("expect", Value::Int(1))],
        ),
    )
    .unwrap();
    assert_eq!(
        content(&test, &path),
        "version = \"1.4.3\"\nedition = \"2024\"\n"
    );
}

#[test]
fn a_failing_change_leaves_the_file_untouched() {
    let (test, path) = with_file(TestContext::new(), "/data/a.txt", "one\ntwo\n");
    let error = run_in(
        &test,
        &args(
            &path,
            vec![
                change(&[("replace", text("one")), ("with", text("1"))]),
                change(&[
                    ("replace", text("three")),
                    ("with", text("3")),
                    ("on_missing", choice("error")),
                ]),
            ],
            &[],
        ),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NoMatch);
    assert!(error.message.contains("change 2"));
    assert_eq!(content(&test, &path), "one\ntwo\n");
    assert!(applied(&test).0.is_empty());
}

#[test]
fn conflicting_ranges_are_rejected_before_any_change() {
    let (test, path) = with_file(TestContext::new(), "/data/a.txt", "a\nb\nc\nd\n");
    let error = run_in(
        &test,
        &args(
            &path,
            vec![
                change(&[("delete_lines", lines(1, 3))]),
                change(&[("delete_lines", lines(3, 4))]),
            ],
            &[],
        ),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(error.message.contains("changes 1 and 2 overlap"));
    assert_eq!(content(&test, &path), "a\nb\nc\nd\n");
}

#[test]
fn ambiguous_replacements_list_the_candidates() {
    let (test, path) = with_file(TestContext::new(), "/data/a.rs", "let a = 1;\nlet b = 1;\n");
    let error = run_in(
        &test,
        &args(
            &path,
            vec![change(&[
                ("replace", text("= 1;")),
                ("with", text("= 2;")),
                ("expect", Value::Int(1)),
            ])],
            &[],
        ),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Assert);
    assert_eq!(error.sample, ["1:7: let a = 1;", "2:7: let b = 1;"]);
}

#[test]
fn whitespace_of_the_command_and_of_a_change() {
    let (test, path) = with_file(TestContext::new(), "/data/a.py", "if x:\n\tgo()\n");
    let exact = args(
        &path,
        vec![change(&[
            ("replace", text("    go()")),
            ("with", text("    stop()")),
        ])],
        &[("whitespace", choice("exact"))],
    );
    let ctx = test.ctx();
    let stage = stage();
    let plan = stage
        .as_effect()
        .unwrap()
        .plan(&exact, StageInput::none(), &ctx)
        .unwrap();
    assert!(plan.changes.is_empty());
    assert_eq!(plan.notable[0].code, Notable::NothingMatched);
    run_in(
        &test,
        &args(
            &path,
            vec![change(&[
                ("replace", text("    go()")),
                ("with", text("    stop()")),
                ("whitespace", choice("indent")),
            ])],
            &[("whitespace", choice("exact"))],
        ),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "if x:\n\tstop()\n");
}

#[test]
fn whole_word_changes_skip_longer_names() {
    let (test, path) = with_file(TestContext::new(), "/data/a.js", "count += counter;\n");
    run_in(
        &test,
        &args(
            &path,
            vec![change(&[
                ("replace", text("count")),
                ("with", text("total")),
                ("whole_word", Value::Bool(true)),
            ])],
            &[],
        ),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "total += counter;\n");
}

#[test]
fn a_new_syntax_error_rolls_the_edit_back() {
    let (test, path) = with_file(checked(), "/data/config.json", "{\"a\": {\"b\": 1}}\n");
    let error = run_in(
        &test,
        &args(
            &path,
            vec![change(&[("replace", text("1}}")), ("with", text("1}"))])],
            &[],
        ),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::RolledBack);
    assert_eq!(
        error.sample,
        [format!("{path}:1:1 unbalanced braces [json]")]
    );
    // The recording journal keeps no copies, so the restored content is the
    // concern of the journal; the stage declares the file before writing it
    // and the failed apply ends in a rollback of that target.
    let (declared, indexes) = applied(&test);
    assert_eq!(declared, [TargetPath::Fs(path)]);
    assert_eq!(indexes, [TargetIndex(0)]);
    assert!(test.journal().calls().iter().any(|call| matches!(
        call,
        JournalCall::Rollback {
            reason: Some(RollbackReason::ValidateFailed),
            ..
        }
    )));
}

#[test]
fn an_error_present_before_the_edit_does_not_block_it() {
    let (test, path) = with_file(checked(), "/data/config.json", "{\"a\": 1\n");
    run_in(
        &test,
        &args(
            &path,
            vec![change(&[("replace", text("1")), ("with", text("2"))])],
            &[],
        ),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "{\"a\": 2\n");
}

#[test]
fn validate_warn_and_off_keep_the_change() {
    for (mode, events) in [("warn", 1), ("off", 0)] {
        let (test, path) = with_file(checked(), "/data/config.json", "{}\n");
        run_in(
            &test,
            &args(
                &path,
                vec![change(&[("replace", text("{}")), ("with", text("{"))])],
                &[("validate", choice(mode))],
            ),
        )
        .unwrap();
        assert_eq!(content(&test, &path), "{\n", "{mode}");
        let logged = test
            .events()
            .take()
            .into_iter()
            .filter(|event| matches!(event, Event::Log(_)))
            .count();
        assert_eq!(logged, events, "{mode}");
    }
}

#[test]
fn if_match_with_another_etag_fails_with_stale_and_changes_nothing() {
    let (test, path) = with_file(TestContext::new(), "/data/a.txt", "a\n");
    let error = run_in(
        &test,
        &args(
            &path,
            vec![change(&[("replace", text("a")), ("with", text("b"))])],
            &[("if_match", text("2-0-0-0-0"))],
        ),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(content(&test, &path), "a\n");
    assert!(applied(&test).0.is_empty());
}

#[test]
fn deleting_and_inserting_back_restores_the_file() {
    let body = "first\nsecond\nthird\n";
    let (test, path) = with_file(TestContext::new(), "/data/a.txt", body);
    run_in(
        &test,
        &args(&path, vec![change(&[("delete_lines", Value::Int(2))])], &[]),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "first\nthird\n");
    run_in(
        &test,
        &args(
            &path,
            vec![change(&[
                ("insert", text("second")),
                ("line", Value::Int(1)),
            ])],
            &[],
        ),
    )
    .unwrap();
    assert_eq!(content(&test, &path), body);
}

#[test]
fn byte_order_mark_and_crlf_are_kept() {
    let test = TestContext::new();
    let path = abs("/data/a.txt");
    test.fs()
        .write_file(&path, b"\xef\xbb\xbfone\r\ntwo\r\n")
        .unwrap();
    let changes = run_in(
        &test,
        &args(
            &path,
            vec![
                change(&[("replace", text("two")), ("with", text("2"))]),
                change(&[("insert", text("zero")), ("position", choice("start"))]),
            ],
            &[],
        ),
    )
    .unwrap();
    assert_eq!(
        test.fs().read_file(&path).unwrap(),
        b"\xef\xbb\xbfzero\r\none\r\n2\r\n"
    );
    assert_eq!(changes[0].bom_after, Some(true));
}

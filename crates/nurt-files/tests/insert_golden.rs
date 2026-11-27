// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/insert_golden.rs
// Responsibility:   Examples of the page of `insert` and the behaviour of `insert` on the.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Examples of the page of `insert` and the behaviour of `insert` on the
//! virtual file system: every form, line endings, entity tags and the
//! transaction protocol.

// Helpers outside the test functions unwrap results of the virtual file
// system and the stage, whose failure fails the test.
#![allow(clippy::unwrap_used)]

mod support_report;

use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, RollbackReason, TargetIndex, TargetPath};
use nurt_contracts::record::{self, Change};
use nurt_contracts::stage::{ArgValue, BoundArgs, RecordStream, StageImpl, StageInput};
use nurt_contracts::value::{self, Record, RecordKind, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

const COMMAND: &str = "insert";

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

fn file(path: &AbsPath) -> Value {
    Value::Path(path.to_language_string())
}

fn args(params: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(COMMAND);
    for (name, value) in params {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn run(test: &TestContext, args: &BoundArgs, input: StageInput<'_>) -> Result<Vec<Change>> {
    let stage = stage();
    let effect = stage.as_effect().unwrap();
    let ctx = test.ctx();
    let plan = effect.plan(args, input, &ctx)?;
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
            tx.rollback(RollbackReason::TargetFailed)?;
            Err(error)
        }
    }
}

fn content(test: &TestContext, path: &AbsPath) -> String {
    String::from_utf8(test.fs().read_file(path).unwrap()).unwrap()
}

fn with_file(path: &str, body: &str) -> (TestContext, AbsPath) {
    let test = TestContext::new();
    let path = abs(path);
    test.fs().write_file(&path, body.as_bytes()).unwrap();
    (test, path)
}

fn assert_every_change_declared(test: &TestContext, changed: &[&AbsPath]) {
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

fn line_record(path: &AbsPath, no: u64) -> Record {
    Record::from(record::Line {
        path: value::Path::new(path.to_language_string()),
        no,
        ..record::Line::default()
    })
}

#[test]
fn example_inserts_an_import_before_the_first_line() {
    let (test, path) = with_file("/home/user/app/src/lib.rs", "pub mod parse;\n");
    let changes = run(
        &test,
        &args(&[
            ("text", text("use std::fmt;")),
            ("in", file(&path)),
            ("line", Value::Int(1)),
            ("position", choice("before")),
        ]),
        StageInput::none(),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "use std::fmt;\npub mod parse;\n");
    assert_eq!(changes[0].added, Some(1));
    assert_every_change_declared(&test, &[&path]);
}

#[test]
fn example_inserts_a_comment_before_every_search_match() {
    let (test, path) = with_file(
        "/home/user/app/src/main.rs",
        "fn helper() {}\n\nfn main() {\n    helper();\n}\n",
    );
    let found = Record::from(record::Match {
        path: value::Path::new(path.to_language_string()),
        no: 3,
        column: 1,
        start: 16,
        end: 23,
        text: "fn main".to_owned(),
        line: "fn main() {".to_owned(),
        ..record::Match::default()
    });
    run(
        &test,
        &args(&[
            ("text", text("// Entry point of the program.")),
            ("position", choice("before")),
        ]),
        StageInput::records(RecordStream::once(found)),
    )
    .unwrap();
    assert_eq!(
        content(&test, &path),
        "fn helper() {}\n\n// Entry point of the program.\nfn main() {\n    helper();\n}\n"
    );
}

#[test]
fn example_appends_a_line_at_the_end_of_a_file() {
    let (test, path) = with_file("/home/user/.profile", "umask 022\n");
    run(
        &test,
        &args(&[
            ("text", text("export PATH=\"$HOME/bin:$PATH\"")),
            ("in", file(&path)),
            ("position", choice("end")),
        ]),
        StageInput::none(),
    )
    .unwrap();
    assert_eq!(
        content(&test, &path),
        "umask 022\nexport PATH=\"$HOME/bin:$PATH\"\n"
    );
}

#[test]
fn example_adds_a_header_to_every_listed_file() {
    let test = TestContext::new();
    let first = abs("/home/user/app/src/a.rs");
    let second = abs("/home/user/app/src/b.rs");
    test.fs().write_file(&first, b"fn a() {}\n").unwrap();
    test.fs().write_file(&second, b"fn b() {}\n").unwrap();
    let entries: Vec<Record> = [&first, &second]
        .into_iter()
        .map(|path| Record::new(RecordKind::Entry).with("path", file(path)))
        .collect();
    let changes = run(
        &test,
        &args(&[
            ("text", text("// Part of the parser.")),
            ("position", choice("start")),
        ]),
        StageInput::records(RecordStream::from_records(entries)),
    )
    .unwrap();
    assert_eq!(
        content(&test, &first),
        "// Part of the parser.\nfn a() {}\n"
    );
    assert_eq!(
        content(&test, &second),
        "// Part of the parser.\nfn b() {}\n"
    );
    assert_eq!(changes.len(), 2);
    assert_every_change_declared(&test, &[&first, &second]);
}

#[test]
fn inserting_after_a_line_of_a_crlf_file_keeps_crlf() {
    let (test, path) = with_file("/data/a.ini", "[a]\r\nx=1\r\n");
    run(
        &test,
        &args(&[
            ("text", text("y=2\nz=3")),
            ("in", file(&path)),
            ("line", Value::Int(2)),
        ]),
        StageInput::none(),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "[a]\r\nx=1\r\ny=2\r\nz=3\r\n");
}

#[test]
fn the_end_of_a_file_without_a_final_line_break_stays_open() {
    let (test, path) = with_file("/data/a.txt", "last");
    run(
        &test,
        &args(&[("text", text("more")), ("in", file(&path))]),
        StageInput::none(),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "last\nmore");
    run(
        &test,
        &args(&[
            ("text", text("end")),
            ("in", file(&path)),
            ("ensure_newline", Value::Bool(true)),
        ]),
        StageInput::none(),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "last\nmore\nend\n");
}

#[test]
fn line_numbers_of_the_input_refer_to_the_file_as_read() {
    let (test, path) = with_file("/data/a.txt", "1\n2\n3\n4\n");
    let records = vec![
        line_record(&path, 1),
        line_record(&path, 2),
        line_record(&path, 4),
    ];
    run(
        &test,
        &args(&[("text", text("-"))]),
        StageInput::records(RecordStream::from_records(records)),
    )
    .unwrap();
    assert_eq!(content(&test, &path), "1\n-\n2\n-\n3\n4\n-\n");
}

#[test]
fn if_match_with_another_etag_fails_with_stale_and_changes_nothing() {
    let (test, path) = with_file("/data/a.txt", "a\n");
    let error = run(
        &test,
        &args(&[
            ("text", text("b")),
            ("in", file(&path)),
            ("if_match", text("9-9-9-9-0")),
        ]),
        StageInput::none(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Stale);
    assert_eq!(content(&test, &path), "a\n");
    assert_every_change_declared(&test, &[]);
}

#[test]
fn binary_files_are_rejected() {
    let test = TestContext::new();
    let path = abs("/data/a.bin");
    test.fs()
        .write_file(&path, b"\x7fELF\x00\x01\x02\x00")
        .unwrap();
    let error = run(
        &test,
        &args(&[("text", text("x")), ("in", file(&path))]),
        StageInput::none(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Binary);
}

#[test]
fn a_line_before_the_first_line_is_out_of_range() {
    let (test, path) = with_file("/data/a.txt", "a\n");
    let error = run(
        &test,
        &args(&[
            ("text", text("b")),
            ("in", file(&path)),
            ("line", Value::Int(0)),
            ("position", choice("before")),
        ]),
        StageInput::none(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::OutOfRange);
    assert_eq!(content(&test, &path), "a\n");
    assert_every_change_declared(&test, &[]);
}

#[test]
fn a_line_record_after_the_last_line_is_out_of_range() {
    let (test, path) = with_file("/data/a.txt", "a\nb\n");
    let records = vec![line_record(&path, 1), line_record(&path, 7)];
    let error = run(
        &test,
        &args(&[("text", text("-"))]),
        StageInput::records(RecordStream::from_records(records)),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::OutOfRange);
    assert!(error.message.contains("line 7"));
    assert_eq!(content(&test, &path), "a\nb\n");
    assert_every_change_declared(&test, &[]);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

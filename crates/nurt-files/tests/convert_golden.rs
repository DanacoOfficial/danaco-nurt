// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/convert_golden.rs
// Responsibility:   Examples of `docs/commands/convert.md`, run on the virtual file system.
// Dependencies:     nurt_contracts, nurt_testkit, nurt_text
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Examples of `docs/commands/convert.md`, run on the virtual file system
//! with the recording journal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

mod support_report;

use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{JournalStore, TargetIndex, TargetPath};
use nurt_contracts::record::{Effect, LineEnding};
use nurt_contracts::report::Notable;
use nurt_contracts::stage::{ArgValue, BoundArgs, RecordStream, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::fixtures::begin_request;
use nurt_testkit::journal::JournalCall;
use nurt_testkit::vfs::test_path;

/// Windows-1250 text with Polish letters and CRLF line endings.
const CP1250_CRLF: &[u8] = b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\n";

/// The same text in UTF-8 with LF line endings.
const UTF8_LF: &[u8] =
    b"Zielona \xc5\x82\xc4\x85ka i \xc5\xbc\xc3\xb3\xc5\x82ty \xc5\x9bwierszcz\nkoniec\n";

fn convert_stage() -> Arc<dyn StageImpl> {
    nurt_files::stages()
        .into_iter()
        .find(|stage| stage.descriptor().command == "convert")
        .expect("convert is registered")
}

fn abs(text: &str) -> AbsPath {
    test_path(text).unwrap()
}

fn language(text: &str) -> String {
    abs(text).to_language_string()
}

fn with_files(files: &[(&str, &[u8])]) -> TestContext {
    let test = TestContext::new();
    for (path, content) in files {
        test.fs().write_file(&abs(path), content).unwrap();
    }
    test
}

fn args(paths: &[&str], params: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new("convert");
    if !paths.is_empty() {
        let list = paths
            .iter()
            .map(|path| Value::Path(language(path)))
            .collect();
        args.insert("path", ArgValue::Value(Value::List(list)), false);
    }
    for (name, value) in params {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn choice(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

/// Plan and apply in one transaction, as the executor does.
fn run(
    test: &TestContext,
    args: &BoundArgs,
    input: StageInput<'_>,
) -> Result<(Effect, Vec<Notable>)> {
    let stage = convert_stage();
    let effect_stage = stage.as_effect().expect("convert is an effect stage");
    let ctx = test.ctx();
    let plan = effect_stage.plan(args, input, &ctx)?;
    support_report::assert_plan_valid(args.command(), &plan.changes);
    let notable = plan.notable.iter().map(|entry| entry.code).collect();
    let mut tx = test.journal().begin(begin_request("convert"));
    let outcome = effect_stage.apply(args, plan, tx.as_mut(), &ctx)?;
    support_report::assert_changes_valid(args.command(), &outcome.changes);
    let StageData::Records(stream) = outcome.output.data else {
        panic!("expected the effect record");
    };
    let records: Vec<Record> = stream.collect::<Result<_>>()?;
    tx.commit()?;
    Ok((Effect::try_from(records[0].clone()).unwrap(), notable))
}

fn declared(test: &TestContext) -> Vec<TargetPath> {
    test.journal()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            JournalCall::Preserve { targets, .. } => Some(targets),
            _ => None,
        })
        .flatten()
        .collect()
}

fn applied_ok(test: &TestContext) -> Vec<TargetIndex> {
    test.journal()
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            JournalCall::Apply {
                index, ok: true, ..
            } => Some(index),
            _ => None,
        })
        .collect()
}

/// Example `convert "/srv/legacy/notes.txt" encoding: "utf-8" line_ending: "lf"`
/// converts a Windows-1250 file with CRLF line endings.
#[test]
fn converts_a_legacy_file_to_utf8_with_lf() {
    let test = with_files(&[("/srv/legacy/notes.txt", CP1250_CRLF)]);
    let given = args(
        &["/srv/legacy/notes.txt"],
        &[("encoding", text("utf-8")), ("line_ending", choice("lf"))],
    );
    let (effect, notable) = run(&test, &given, StageInput::none()).unwrap();
    assert_eq!(
        test.fs().read_file(&abs("/srv/legacy/notes.txt")).unwrap(),
        UTF8_LF
    );
    assert_eq!(
        notable,
        [Notable::EncodingChanged, Notable::LineEndingChanged]
    );
    let changes = effect.changes.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path.as_str(), language("/srv/legacy/notes.txt"));
    assert_eq!(changes[0].encoding_before.as_deref(), Some("windows-1250"));
    assert_eq!(changes[0].encoding_after.as_deref(), Some("utf-8"));
    assert_eq!(changes[0].line_ending_before, Some(LineEnding::Crlf));
    assert_eq!(changes[0].line_ending_after, Some(LineEnding::Lf));
    assert_eq!(declared(&test).len(), applied_ok(&test).len());
}

/// Example `find "/srv/app/scripts" glob: "*.cmd" type: file |> convert line_ending: "crlf"`
/// converts the line endings of the listed files; a file that already has
/// CRLF line endings is reported as unchanged.
#[test]
fn converts_line_endings_of_listed_files() {
    let test = with_files(&[
        ("/srv/app/scripts/build.cmd", b"@echo off\nmsbuild\n"),
        ("/srv/app/scripts/test.cmd", b"@echo off\r\nvstest\r\n"),
    ]);
    let records = ["/srv/app/scripts/build.cmd", "/srv/app/scripts/test.cmd"]
        .iter()
        .map(|path| Record::new(RecordKind::Entry).with("path", Value::Path(language(path))))
        .collect();
    let input = StageInput::records(RecordStream::from_records(records));
    let given = args(&[], &[("line_ending", choice("crlf"))]);
    let (effect, notable) = run(&test, &given, input).unwrap();
    assert_eq!(
        test.fs()
            .read_file(&abs("/srv/app/scripts/build.cmd"))
            .unwrap(),
        b"@echo off\r\nmsbuild\r\n"
    );
    assert_eq!(notable, [Notable::LineEndingChanged, Notable::Unchanged]);
    assert_eq!(effect.targets, Some(1));
    assert_eq!(
        declared(&test),
        [TargetPath::Fs(abs("/srv/app/scripts/build.cmd"))]
    );
    assert_eq!(applied_ok(&test), [TargetIndex(0)]);
}

/// Example `convert "/srv/app/data.csv" bom: false` removes the byte order
/// mark and keeps the encoding and the line endings.
#[test]
fn removes_the_byte_order_mark() {
    let test = with_files(&[("/srv/app/data.csv", b"\xef\xbb\xbfid;name\r\n1;a\r\n")]);
    let given = args(&["/srv/app/data.csv"], &[("bom", Value::Bool(false))]);
    let (effect, notable) = run(&test, &given, StageInput::none()).unwrap();
    assert_eq!(
        test.fs().read_file(&abs("/srv/app/data.csv")).unwrap(),
        b"id;name\r\n1;a\r\n"
    );
    assert_eq!(notable, [Notable::BomChanged]);
    let change = &effect.changes.unwrap()[0];
    assert_eq!(change.bom_before, Some(true));
    assert_eq!(change.bom_after, Some(false));
    assert_eq!(change.line_ending_after, Some(LineEnding::Crlf));
}

/// Example `convert "/srv/app/notes.txt" encoding: "iso-8859-2"` fails with
/// `E_ENCODING` at the first character Latin-2 cannot represent, and the
/// file is not changed.
#[test]
fn rejects_characters_the_target_cannot_represent() {
    let content = "first line\nsecond \u{2192} line\n".as_bytes();
    let test = with_files(&[("/srv/app/notes.txt", content)]);
    let given = args(&["/srv/app/notes.txt"], &[("encoding", text("iso-8859-2"))]);
    let error = run(&test, &given, StageInput::none()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Encoding);
    let location = error.location.clone().unwrap();
    assert_eq!(location.line, Some(2));
    assert_eq!(location.column, Some(8));
    assert_eq!(
        error.details.get("character"),
        Some(&serde_json::Value::from("U+2192"))
    );
    assert_eq!(
        test.fs().read_file(&abs("/srv/app/notes.txt")).unwrap(),
        content
    );
    assert!(declared(&test).is_empty());
}

/// Example `convert "/srv/dos/menu.txt" encoding: "utf-8"` decodes a file of
/// the DOS code page 852, detected through `text.legacy_encodings`.
#[test]
fn converts_dos_code_page_852_to_utf8() {
    let expected =
        "\u{179}r\u{f3}d\u{142}o: \u{17c}\u{f3}\u{142}w, g\u{119}\u{15b} i \u{142}\u{105}ka.\r\n";
    let codec = nurt_text::codec::Codec::Ibm852;
    let bytes = nurt_text::codec::encode(expected, &codec.text_encoding(false)).unwrap();
    let test = with_files(&[("/srv/dos/menu.txt", &bytes)]);
    let mut config = nurt_contracts::config::Config::default();
    config.text.legacy_encodings = vec!["ibm852".to_owned()];
    let ctx = test.builder().config(Arc::new(config)).build();
    let stage = convert_stage();
    let effect_stage = stage.as_effect().unwrap();
    let given = args(&["/srv/dos/menu.txt"], &[("encoding", text("utf-8"))]);
    let plan = effect_stage.plan(&given, StageInput::none(), &ctx).unwrap();
    assert_eq!(plan.changes[0].encoding_before.as_deref(), Some("ibm852"));
    let mut tx = test.journal().begin(begin_request("convert"));
    effect_stage.apply(&given, plan, tx.as_mut(), &ctx).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        test.fs().read_file(&abs("/srv/dos/menu.txt")).unwrap(),
        expected.as_bytes()
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

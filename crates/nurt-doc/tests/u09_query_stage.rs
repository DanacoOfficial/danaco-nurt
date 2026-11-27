// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u09_query_stage.rs
// Responsibility:   Tests of the `query` stage: the descriptor and the static check, response.
// Dependencies:     nurt_contracts, nurt_testkit, u09_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the `query` stage: the descriptor and the static check, response
//! records, documents read without a document service, the document size
//! limit and the errors of the stage form.

#![cfg(feature = "testing")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

mod u09_support;

use std::sync::Arc;

use nurt_contracts::config::Config;
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::{Content, Entry, EntryType, Item};
use nurt_contracts::spec::StageSpec;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CheckCtx, ExecCtx, InputKinds, Limits, RecordStream, StageData, StageImpl,
    StageInput, StaticInfo,
};
use nurt_contracts::value::{Path, Record, RecordKind, Value};
use nurt_doc::commands::query::QueryStage;
use nurt_doc::registry::DocOptions;
use nurt_doc::service::DocumentService;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::services::test_services;

use u09_support::{Fixture, args, etag, failure, items, path, query_all, registry, text};

const CONFIG: &str = "/data/config.json";

const CONFIG_TEXT: &[u8] = b"{\n  \"name\": \"web\",\n  \"ports\": [80, 443]\n}\n";

/// Catalog entry of `query` with the fields the specification requires.
const SPEC: &str = "name = \"query\"\ncategory = \"transform\"\nreversibility = \"read\"\neffect = \"read\"\nimplementation = \"builtin\"\ninput = [\"none\", \"content\", \"entry\", \"item\", \"response\"]\noutput = [\"item\"]\nsummary = \"Select and transform document values with a jq filter.\"\n";

fn check(args: &BoundArgs) -> Result<StaticInfo> {
    let spec: StageSpec = toml::from_str(SPEC).unwrap();
    let test = TestContext::new();
    let config = Config::default();
    QueryStage::default().check(args, &CheckCtx::new(&spec, &**test.host(), &config))
}

/// Every element of the output of `query` in the given context.
fn run(args: &BoundArgs, input: StageInput<'_>, ctx: &ExecCtx) -> Vec<Result<Record>> {
    let stage = QueryStage::default();
    match stage.as_read().unwrap().execute(args, input, ctx) {
        Ok(output) => match output.data {
            StageData::Records(stream) => stream.collect(),
            other => panic!("`query` returned {other:?} instead of records"),
        },
        Err(error) => vec![Err(error)],
    }
}

/// Context with the document service of the crate and the given limits.
fn context_with_limits(test: &TestContext, limits: Limits) -> ExecCtx {
    let service = DocumentService::new(registry(), DocOptions::default());
    let services = test_services().with_doc(Arc::new(service));
    test.builder()
        .services(Arc::new(services))
        .limits(limits)
        .build()
}

fn records(input: Vec<Record>) -> StageInput<'static> {
    StageInput::records(RecordStream::new(input.into_iter().map(Ok)))
}

fn content(path: &str, text: &str) -> Record {
    Record::from(Content {
        path: Path::new(path),
        text: text.to_owned(),
        etag: etag(text.as_bytes()),
        ..Content::default()
    })
}

fn item(key: &str, value: Value) -> Record {
    Record::from(Item {
        path: Path::new("/data/deploy.json"),
        format: "json".to_owned(),
        key: key.to_owned(),
        value,
        etag: etag(b"{}"),
        ..Item::default()
    })
}

fn expression(filter: &str) -> BoundArgs {
    args("query", &[("expression", text(filter))])
}

#[test]
fn descriptor_names_the_command_and_its_input_kinds() {
    let descriptor = QueryStage::default().descriptor();
    assert_eq!(descriptor.command, "query");
    assert!(descriptor.source);
    let InputKinds::Only(kinds) = descriptor.input else {
        panic!("`query` accepts a fixed set of input kinds");
    };
    assert_eq!(
        kinds,
        [
            RecordKind::Content,
            RecordKind::Entry,
            RecordKind::Item,
            RecordKind::Response
        ]
    );
}

#[test]
fn check_reports_the_path_and_accepts_a_valid_filter() {
    let info = check(&args(
        "query",
        &[("expression", text(".name")), ("path", path(CONFIG))],
    ))
    .unwrap();
    assert_eq!(info.paths, [CONFIG]);
    let info = check(&expression(".")).unwrap();
    assert!(info.paths.is_empty());
}

#[test]
fn check_rejects_a_filter_that_does_not_compile_at_binding() {
    let error = check(&expression(".ports |")).unwrap_err();
    assert_eq!(error.code, ErrorCode::Query);
    assert_eq!(error.stage(), Some(Stage::Bind));
}

#[test]
fn response_records_are_read_as_documents_of_their_media_type() {
    let body = "{\"port\": 8080}";
    let record = Record::new(RecordKind::Response)
        .with("url", text("https://example.test/config"))
        .with("body", text(body))
        .with("content_type", text("application/json; charset=utf-8"))
        .with("etag", text(&etag(body.as_bytes())));
    let fixture = Fixture::new();
    let output = query_all(&fixture, &expression(".port"), Some(vec![record]));
    let found = items(&output.into_iter().collect::<Result<Vec<_>>>().unwrap());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].key, "port");
    assert_eq!(found[0].value, Value::Int(8080));
    assert_eq!(found[0].format, "json");
    assert_eq!(found[0].path, Path::new("https://example.test/config"));
    assert_eq!(found[0].etag, etag(body.as_bytes()));
}

#[test]
fn content_is_read_as_json_without_a_document_service() {
    let test = TestContext::new();
    let ctx = test.ctx();
    let output = run(
        &expression(".a[1]"),
        records(vec![content("/data/notes.txt", "{\"a\": [1, 2]}")]),
        &ctx,
    );
    let found = items(&output.into_iter().collect::<Result<Vec<_>>>().unwrap());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].key, "a[1]");
    assert_eq!(found[0].value, Value::Int(2));
    assert_eq!(found[0].format, "json");
}

#[test]
fn content_that_is_not_json_fails_with_its_path_without_a_document_service() {
    let test = TestContext::new();
    let ctx = test.ctx();
    let output = run(
        &expression("."),
        records(vec![content("/data/notes.txt", "{oops")]),
        &ctx,
    );
    let error = failure(output.into_iter().collect::<Result<Vec<_>>>());
    assert_eq!(error.code, ErrorCode::DocParse);
    assert!(error.message.contains("nor JSON"), "{}", error.message);
    assert_eq!(
        error
            .location
            .as_ref()
            .map(|location| location.path.as_str()),
        Some("/data/notes.txt")
    );
}

#[test]
fn entry_records_of_directories_are_rejected() {
    let record = Record::from(Entry {
        path: Path::new("/data"),
        name: "data".to_owned(),
        kind: EntryType::Directory,
        ..Entry::default()
    });
    let output = query_all(&Fixture::new(), &expression("."), Some(vec![record]));
    let error = failure(output.into_iter().collect::<Result<Vec<_>>>());
    assert_eq!(error.code, ErrorCode::Type);
    assert!(error.message.contains("type: file"), "{}", error.message);
}

#[test]
fn an_entry_whose_name_is_not_valid_unicode_is_read_through_its_raw_path() {
    let fixture = Fixture::new();
    let abs = AbsPath::from_raw_bytes(b"/data/n\xffotes.json").unwrap();
    let fs = fixture.test.fs();
    fs.create_dir_all(&AbsPath::for_test("/data")).unwrap();
    fs.write_file(&abs, b"{\"a\": 7}").unwrap();
    let record = Record::from(Entry {
        path: Path::new(abs.to_language_string()),
        raw_path: abs.raw_bytes(),
        name: "n\u{fffd}otes.json".to_owned(),
        kind: EntryType::File,
        ..Entry::default()
    });
    let output = query_all(&fixture, &expression(".a"), Some(vec![record.clone()]));
    let found = items(&output.into_iter().collect::<Result<Vec<_>>>().unwrap());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].value, Value::Int(7));
    let mut other = Entry::try_from(record).unwrap();
    other.path = Path::new("/data/other.json");
    let output = query_all(&fixture, &expression(".a"), Some(vec![Record::from(other)]));
    let error = failure(output.into_iter().collect::<Result<Vec<_>>>());
    assert_eq!(
        error.code,
        ErrorCode::NotFound,
        "a path that is not the display form of the raw path is read as written"
    );
}

#[test]
fn files_larger_than_the_document_limit_are_refused() {
    let fixture = Fixture::new();
    fixture.file(CONFIG, CONFIG_TEXT);
    let ctx = context_with_limits(
        &fixture.test,
        Limits {
            max_file_bytes_text: 4,
            ..Limits::default()
        },
    );
    let output = run(
        &args(
            "query",
            &[("expression", text(".")), ("path", path(CONFIG))],
        ),
        StageInput::none(),
        &ctx,
    );
    let error = failure(output.into_iter().collect::<Result<Vec<_>>>());
    assert_eq!(error.code, ErrorCode::Limit);
}

#[test]
fn a_filter_that_is_not_text_is_a_type_error() {
    let fixture = Fixture::new();
    fixture.file(CONFIG, CONFIG_TEXT);
    let call = BoundArgs::new("query")
        .with("expression", ArgValue::Value(Value::Int(1)), false)
        .with("path", ArgValue::Value(path(CONFIG)), false);
    let error = failure(
        query_all(&fixture, &call, None)
            .into_iter()
            .collect::<Result<Vec<_>>>(),
    );
    assert_eq!(error.code, ErrorCode::Type);
}

#[test]
fn an_error_of_the_input_ends_the_output() {
    let test = TestContext::new();
    let ctx = test.ctx();
    let input = StageInput::records(RecordStream::new(std::iter::once(Err::<Record, _>(
        Error::new(ErrorCode::Io, "reading the input failed"),
    ))));
    let output = run(&expression("."), input, &ctx);
    assert_eq!(output.len(), 1);
    assert_eq!(
        failure(output.into_iter().next().unwrap()).code,
        ErrorCode::Io
    );
}

#[test]
fn one_filter_serves_every_input_record() {
    let output = query_all(
        &Fixture::new(),
        &expression("."),
        Some(vec![item("a", Value::Int(1)), item("b", Value::Int(2))]),
    );
    let found = items(&output.into_iter().collect::<Result<Vec<_>>>().unwrap());
    let keys: Vec<&str> = found.iter().map(|item| item.key.as_str()).collect();
    assert_eq!(keys, ["a", "b"]);
    let values: Vec<&Value> = found.iter().map(|item| &item.value).collect();
    assert_eq!(values, [&Value::Int(1), &Value::Int(2)]);
}

#[test]
fn runtime_errors_name_the_key_of_the_input_value_when_it_has_one() {
    let fixture = Fixture::new();
    let filter = expression("error(\"boom\")");
    let keyed = failure(
        query_all(&fixture, &filter, Some(vec![item("spec", Value::Int(1))]))
            .into_iter()
            .collect::<Result<Vec<_>>>(),
    );
    assert_eq!(keyed.code, ErrorCode::Query);
    assert_eq!(
        keyed.details.get("key").and_then(serde_json::Value::as_str),
        Some("spec")
    );
    let unkeyed = failure(
        query_all(&fixture, &filter, Some(vec![item("a[", Value::Int(1))]))
            .into_iter()
            .collect::<Result<Vec<_>>>(),
    );
    assert_eq!(unkeyed.code, ErrorCode::Query);
    assert!(unkeyed.details.get("key").is_none());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

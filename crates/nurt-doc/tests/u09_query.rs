// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the `query` stage: the source form, the input record kinds,
//! keys of results, errors and limits.

#![cfg(feature = "testing")]

mod u09_support;

use std::path::PathBuf;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{Content, Entry, EntryType, Hunk, Item};
use nurt_contracts::value::{Path, Record, Value};
use nurt_doc::jq::value_type;

use u09_support::{
    Fixture, args, etag, failure, items, map, path, query, query_all, query_file, text,
};

const CONFIG: &str = "/data/config.json";

/// A JSON document with a scalar, a list and the layout of a hand-written
/// file.
const CONFIG_TEXT: &[u8] = b"{\n  \"name\": \"web\",\n  \"ports\": [80, 443]\n}\n";

fn fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.file(CONFIG, CONFIG_TEXT);
    fixture
}

fn keys(items: &[Item]) -> Vec<&str> {
    items.iter().map(|item| item.key.as_str()).collect()
}

fn stage_query(
    fixture: &Fixture,
    expression: &str,
    input: Record,
) -> nurt_contracts::error::Result<Vec<Item>> {
    let args = args("query", &[("expression", text(expression))]);
    query(fixture, &args, Some(vec![input])).map(|records| items(&records))
}

#[test]
fn path_results_get_the_key_of_their_position() {
    let found = query_file(&fixture(), CONFIG, ".ports[]").unwrap();
    assert_eq!(keys(&found), ["ports[0]", "ports[1]"]);
    assert_eq!(found[0].value, Value::Int(80));
    assert_eq!(found[1].value, Value::Int(443));
    assert_eq!(found[0].value_type, value_type(&Value::Int(80)));
    assert_eq!(found[0].format, "json");
    assert_eq!(found[0].path, Path::new(CONFIG));
    assert_eq!(found[0].etag, etag(CONFIG_TEXT));
}

#[test]
fn computed_results_get_the_dot_key() {
    let found = query_file(&fixture(), CONFIG, ".ports | length").unwrap();
    assert_eq!(keys(&found), ["."]);
    assert_eq!(found[0].value, Value::Int(2));
    let built = query_file(&fixture(), CONFIG, "{n: .name}").unwrap();
    assert_eq!(keys(&built), ["."]);
    assert_eq!(built[0].value, map(&[("n", text("web"))]));
}

#[test]
fn identity_gets_the_root_key() {
    let found = query_file(&fixture(), CONFIG, ".").unwrap();
    assert_eq!(keys(&found), ["$"]);
    assert_eq!(found[0].value.get("name"), Some(&text("web")));
}

#[test]
fn filters_without_results_produce_no_records() {
    let found = query_file(&fixture(), CONFIG, ".ports[] | select(. > 1000)").unwrap();
    assert!(found.is_empty());
}

#[test]
fn content_without_a_known_format_is_read_as_json() {
    let record = Record::from(Content {
        path: Path::new("/data/notes.txt"),
        text: "{\"a\": [1, 2]}".to_owned(),
        etag: etag(b"{\"a\": [1, 2]}"),
        ..Content::default()
    });
    let found = stage_query(&Fixture::new(), ".a[1]", record).unwrap();
    assert_eq!(keys(&found), ["a[1]"]);
    assert_eq!(found[0].value, Value::Int(2));
    assert_eq!(found[0].format, "json");
}

#[test]
fn content_of_a_known_format_is_read_by_the_document_service() {
    let record = Record::from(Content {
        path: Path::new("/data/app.kv"),
        text: "port = 8080\nname = web\n".to_owned(),
        etag: etag(b"port = 8080\nname = web\n"),
        ..Content::default()
    });
    let found = stage_query(&Fixture::new(), ".port", record).unwrap();
    assert_eq!(keys(&found), ["port"]);
    assert_eq!(found[0].value, Value::Int(8080));
    assert_eq!(found[0].format, "kv");
}

#[test]
fn binary_content_is_rejected() {
    let record = Record::from(Content {
        path: Path::new("/data/image.bin"),
        binary: Some(true),
        ..Content::default()
    });
    let error = failure(stage_query(&Fixture::new(), ".", record));
    assert_eq!(error.code, ErrorCode::Binary);
}

#[test]
fn entry_records_read_their_files() {
    let record = Record::from(Entry {
        path: Path::new(CONFIG),
        name: "config.json".to_owned(),
        kind: EntryType::File,
        ..Entry::default()
    });
    let found = stage_query(&fixture(), ".name", record).unwrap();
    assert_eq!(keys(&found), ["name"]);
    assert_eq!(found[0].value, text("web"));
    assert_eq!(found[0].etag, etag(CONFIG_TEXT));
}

#[test]
fn item_records_extend_their_key() {
    let record = Record::from(Item {
        path: Path::new("/data/deploy.json"),
        format: "json".to_owned(),
        key: "spec".to_owned(),
        value: map(&[("replicas", Value::Int(3))]),
        etag: etag(b"{}"),
        ..Item::default()
    });
    let found = stage_query(&Fixture::new(), ".replicas", record).unwrap();
    assert_eq!(keys(&found), ["spec.replicas"]);
    assert_eq!(found[0].value, Value::Int(3));
    assert_eq!(found[0].format, "json");
    assert_eq!(found[0].path, Path::new("/data/deploy.json"));
}

#[test]
fn other_record_kinds_are_rejected() {
    let error = failure(stage_query(
        &Fixture::new(),
        ".",
        Record::from(Hunk::default()),
    ));
    assert_eq!(error.code, ErrorCode::Type);
}

#[test]
fn filters_that_do_not_compile_fail_with_e_query() {
    let error = failure(query_file(&fixture(), CONFIG, ".ports |"));
    assert_eq!(error.code, ErrorCode::Query);
    let undefined = failure(query_file(&fixture(), CONFIG, "no_such_function(1)"));
    assert_eq!(undefined.code, ErrorCode::Query);
}

#[test]
fn runtime_errors_follow_the_earlier_results() {
    let args = args(
        "query",
        &[
            (
                "expression",
                text(".ports[] | if . > 100 then error(\"too large\") else . end"),
            ),
            ("path", path(CONFIG)),
        ],
    );
    let output = query_all(&fixture(), &args, None);
    assert_eq!(output.len(), 2);
    let first = Item::try_from(output[0].as_ref().unwrap().clone()).unwrap();
    assert_eq!(first.value, Value::Int(80));
    let error = output[1].as_ref().unwrap_err();
    assert_eq!(error.code, ErrorCode::Query);
    assert!(error.message.contains("too large"), "{}", error.message);
    assert_eq!(
        error
            .location
            .as_ref()
            .map(|location| location.path.as_str()),
        Some(CONFIG)
    );
}

#[test]
fn the_record_limit_ends_endless_filters() {
    let fixture = fixture().with_max_records(5);
    let error = failure(query_file(&fixture, CONFIG, "repeat(1)"));
    assert_eq!(error.code, ErrorCode::Limit);
}

#[test]
fn missing_files_are_not_found() {
    let error = failure(query_file(&Fixture::new(), "/data/none.json", "."));
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn the_source_form_needs_a_path() {
    let args = args("query", &[("expression", text("."))]);
    let error = failure(query(&Fixture::new(), &args, None));
    assert_eq!(error.code, ErrorCode::MissingParam);
}

#[test]
fn large_integers_keep_their_digits() {
    let fixture = Fixture::new();
    fixture.file("/data/big.json", b"{\"n\": 12345678901234567890}\n");
    let found = query_file(&fixture, "/data/big.json", ".n").unwrap();
    assert_eq!(
        found[0].value,
        Value::BigInt("12345678901234567890".to_owned())
    );
}

#[test]
fn documents_are_opened_with_the_path_of_their_file() {
    let fixture = Fixture::new();
    fixture.file("/data/app.kvp", b"name = web\n");
    let found = query_file(&fixture, "/data/app.kvp", ".name").unwrap();
    assert_eq!(keys(&found), ["name"]);
    let opened = fixture.opened.lock().unwrap();
    assert_eq!(opened.as_slice(), [Some(PathBuf::from("/data/app.kvp"))]);
}

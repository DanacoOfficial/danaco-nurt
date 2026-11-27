// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u02_json.rs
// Responsibility:   JSON and JSONC documents through the format registry and the document.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! JSON and JSONC documents through the format registry and the document
//! handle: detection, encodings, syntax errors and edits that keep layout,
//! comments and number lexemes.

// Test code fails through `unwrap` and `panic`; the failure is the result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::report::Notable;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::{Bytes, Duration, Size, Value};
use nurt_doc::formats::json::{self, Dialect};
use nurt_doc::model::SetOptions;
use nurt_doc::registry::{DocOptions, OpenDocument, Registry};

const CREATE: SetOptions = SetOptions { create: true };
const NO_CREATE: SetOptions = SetOptions { create: false };

/// Compiler settings in the style of `tsconfig.json`: comments, a
/// single-line object inside a multi-line one, trailing commas and a number
/// whose lexeme a binary form would change.
const TSCONFIG: &str = "{\n  // Compiler settings.\n  \"compilerOptions\": {\n    \"target\": \"ES2022\", // output level\n    \"strict\": false,\n    \"paths\": {\"@/*\": [\"src/*\"]},\n  },\n  \"version\": 1.10,\n}\n";

fn registry() -> Registry {
    Registry::with(json::formats())
}

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn open(id: &DocFormatId, bytes: &[u8]) -> OpenDocument {
    registry().open(id, bytes, &DocOptions::default()).unwrap()
}

fn set(id: &DocFormatId, source: &str, path: &str, value: Value) -> String {
    let mut document = open(id, source.as_bytes());
    document.handle.set(&key(path), &value, &CREATE).unwrap();
    String::from_utf8(document.bytes()).unwrap()
}

fn remove(id: &DocFormatId, source: &str, path: &str) -> String {
    let mut document = open(id, source.as_bytes());
    document.handle.remove(&key(path)).unwrap();
    String::from_utf8(document.bytes()).unwrap()
}

fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn detects_json_and_jsonc_from_names_extensions_and_content() {
    let registry = registry();
    let detect = |name: Option<&str>, content: &[u8]| registry.detect(name.map(Path::new), content);
    assert_eq!(detect(Some("package.json"), b"{}"), Some(DocFormatId::JSON));
    assert_eq!(
        detect(Some("settings.jsonc"), b"{}"),
        Some(DocFormatId::JSONC)
    );
    for name in [
        "tsconfig.json",
        "tsconfig.build.json",
        "jsconfig.json",
        "devcontainer.json",
        ".devcontainer.json",
        ".eslintrc.json",
        ".babelrc.json",
    ] {
        assert_eq!(
            detect(Some(name), b"{}"),
            Some(DocFormatId::JSONC),
            "{name}"
        );
    }
    assert_eq!(
        detect(Some("config.json5"), b"{unquoted: 1, list: [1,],}"),
        None
    );
    assert_eq!(detect(None, b"{\"a\": [1, 2]}"), Some(DocFormatId::JSON));
    assert_eq!(
        detect(None, b"// settings\n{\"a\": 1,}"),
        Some(DocFormatId::JSONC)
    );
    assert_eq!(
        Dialect::detect(Path::new(".vscode/launch.json")),
        Some(Dialect::Jsonc)
    );
}

#[test]
fn keeps_the_byte_order_mark_line_endings_and_utf16_encoding() {
    let source = "\u{FEFF}{\r\n  \"a\": 1\r\n}\r\n";
    assert_eq!(
        set(&DocFormatId::JSON, source, "b", Value::Bool(true)),
        "\u{FEFF}{\r\n  \"a\": 1,\r\n  \"b\": true\r\n}\r\n"
    );
    let original = utf16le("\u{FEFF}{\"a\": \"\u{e9}\"}");
    let mut document = open(&DocFormatId::JSON, &original);
    assert_eq!(document.bytes(), original);
    document
        .handle
        .set(&key("a"), &Value::Text("\u{fc}".to_owned()), &CREATE)
        .unwrap();
    assert_eq!(document.bytes(), utf16le("\u{FEFF}{\"a\": \"\u{fc}\"}"));
}

#[test]
fn rejects_comments_and_trailing_commas_in_strict_json_with_their_location() {
    let options = DocOptions::default();
    let error = registry()
        .open(&DocFormatId::JSON, b"{\n  \"a\": 1 // note\n}", &options)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DocParse);
    let location = error.location.clone().unwrap();
    assert_eq!(
        (location.line, location.column, location.byte),
        (Some(2), Some(10), Some(11))
    );
    assert!(error.message.contains("format: jsonc"), "{}", error.message);
    let error = registry()
        .open(&DocFormatId::JSON, b"[1, 2,]", &options)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DocParse);
    assert!(
        error.message.contains("trailing commas"),
        "{}",
        error.message
    );
    assert!(
        registry()
            .open(&DocFormatId::JSONC, b"{\n  \"a\": 1 // note\n}", &options)
            .is_ok()
    );
    assert!(
        registry()
            .open(&DocFormatId::JSONC, b"[1, 2,]", &options)
            .is_ok()
    );
}

#[test]
fn replaces_and_inserts_values_keeping_comments_and_trailing_commas() {
    assert_eq!(
        set(
            &DocFormatId::JSONC,
            TSCONFIG,
            "compilerOptions.strict",
            Value::Bool(true)
        ),
        TSCONFIG.replace("\"strict\": false", "\"strict\": true")
    );
    assert_eq!(
        set(
            &DocFormatId::JSONC,
            TSCONFIG,
            "compilerOptions.noEmit",
            Value::Bool(true)
        ),
        TSCONFIG.replace("[\"src/*\"]},\n", "[\"src/*\"]},\n    \"noEmit\": true,\n")
    );
    assert_eq!(
        set(
            &DocFormatId::JSONC,
            TSCONFIG,
            "compilerOptions.paths[\"@/*\"][+]",
            Value::Text("lib/*".to_owned())
        ),
        TSCONFIG.replace("[\"src/*\"]", "[\"src/*\", \"lib/*\"]")
    );
}

#[test]
fn changes_the_type_of_a_value_in_the_layout_of_its_object() {
    let source = "{\n  \"name\": \"app\",\n  \"scripts\": \"build\"\n}";
    let scripts = Value::Map(
        [("build".to_owned(), Value::Text("tsc".to_owned()))]
            .into_iter()
            .collect(),
    );
    assert_eq!(
        set(&DocFormatId::JSON, source, "scripts", scripts),
        "{\n  \"name\": \"app\",\n  \"scripts\": {\n    \"build\": \"tsc\"\n  }\n}"
    );
    assert_eq!(
        set(&DocFormatId::JSON, "{\"a\": [1, 2]}", "a", Value::Null),
        "{\"a\": null}"
    );
}

#[test]
fn removes_a_member_with_its_comment_lines_and_keeps_comments_after_a_blank_line() {
    let source = "{\n  \"a\": 1,\n\n  // kept: separated by a blank line\n\n  // about b\n  \"b\": 2, // b\n  \"c\": 3\n}";
    assert_eq!(
        remove(&DocFormatId::JSONC, source, "b"),
        "{\n  \"a\": 1,\n\n  // kept: separated by a blank line\n\n  \"c\": 3\n}"
    );
    assert_eq!(
        remove(&DocFormatId::JSONC, TSCONFIG, "version"),
        "{\n  // Compiler settings.\n  \"compilerOptions\": {\n    \"target\": \"ES2022\", // output level\n    \"strict\": false,\n    \"paths\": {\"@/*\": [\"src/*\"]},\n  },\n}\n"
    );
    assert_eq!(
        remove(&DocFormatId::JSON, "{\n  \"a\": 1,\n  \"b\": 2\n}", "b"),
        "{\n  \"a\": 1\n}"
    );
}

#[test]
fn edits_single_line_documents_on_one_line_without_adding_a_final_newline() {
    assert_eq!(
        set(&DocFormatId::JSON, "{\"a\":1}", "b", Value::Int(2)),
        "{\"a\":1,\"b\":2}"
    );
    assert_eq!(
        set(
            &DocFormatId::JSON,
            "{\"list\": [1, 2]}",
            "list[+]",
            Value::Int(3)
        ),
        "{\"list\": [1, 2, 3]}"
    );
    assert_eq!(
        remove(&DocFormatId::JSON, "{\"a\": 1, \"b\": 2}", "a"),
        "{\"b\": 2}"
    );
}

#[test]
fn keeps_number_lexemes_and_escapes_of_unchanged_values() {
    let source = "{\"big\": 18446744073709551616, \"exp\": 1e400, \"s\": \"\\u00e9\\/\", \"n\": 1}";
    assert_eq!(
        set(&DocFormatId::JSON, source, "n", Value::Int(2)),
        source.replace("\"n\": 1", "\"n\": 2")
    );
    let document = open(&DocFormatId::JSON, source.as_bytes());
    let value = document.handle.to_value().unwrap();
    assert_eq!(
        value.get("big"),
        Some(&Value::BigInt("18446744073709551616".to_owned()))
    );
    assert_eq!(value.get("exp"), Some(&Value::Decimal("1e400".to_owned())));
    assert_eq!(value.get("s"), Some(&Value::Text("\u{e9}/".to_owned())));
}

#[test]
fn changes_the_last_of_repeated_members_and_reports_it() {
    let mut document = open(&DocFormatId::JSON, b"{\"a\": 1, \"a\": 2}");
    let report = document
        .handle
        .set(&key("a"), &Value::Int(3), &CREATE)
        .unwrap();
    assert!(
        report
            .notable
            .iter()
            .any(|entry| entry.code == Notable::DuplicateKey)
    );
    assert_eq!(document.bytes(), b"{\"a\": 1, \"a\": 3}");
    let report = document.handle.remove(&key("a")).unwrap();
    assert!(
        report
            .notable
            .iter()
            .any(|entry| entry.code == Notable::DuplicateKey)
    );
    assert_eq!(document.bytes(), b"{}");
}

#[test]
fn writes_values_of_the_language_as_document_values() {
    let source = "{\"limit\": 0, \"timeout\": \"\"}";
    let mut document = open(&DocFormatId::JSON, source.as_bytes());
    document
        .handle
        .set(&key("limit"), &Value::Size(Size::from_bytes(512)), &CREATE)
        .unwrap();
    document
        .handle
        .set(
            &key("timeout"),
            &Value::Duration(Duration::from_millis(1500)),
            &CREATE,
        )
        .unwrap();
    assert_eq!(
        document.bytes(),
        b"{\"limit\": 512, \"timeout\": \"1500ms\"}"
    );
    let error = document
        .handle
        .set(
            &key("limit"),
            &Value::Bytes(Bytes::new(vec![1, 2])),
            &CREATE,
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidValue);
    assert_eq!(
        document.bytes(),
        b"{\"limit\": 512, \"timeout\": \"1500ms\"}"
    );
}

#[test]
fn creates_missing_keys_only_when_asked() {
    let mut document = open(&DocFormatId::JSON, b"{}");
    let error = document
        .handle
        .set(&key("a.b"), &Value::Int(1), &NO_CREATE)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyNotFound);
    let report = document
        .handle
        .set(&key("a.b"), &Value::Int(1), &CREATE)
        .unwrap();
    assert!(
        report
            .notable
            .iter()
            .any(|entry| entry.code == Notable::KeysCreated)
    );
    assert_eq!(report.flags, ["keys_created"]);
    assert_eq!(document.bytes(), b"{\"a\": {\"b\": 1}}");
}

#[test]
fn rejects_paths_through_values_of_another_kind() {
    let mut document = open(&DocFormatId::JSON, b"{\"a\": 1, \"l\": [1]}");
    for path in ["a.b", "l.x", "l[5]"] {
        let error = document
            .handle
            .set(&key(path), &Value::Int(2), &CREATE)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict, "{path}");
    }
    let error = document.handle.remove(&key("missing")).unwrap_err();
    assert_eq!(error.code, ErrorCode::KeyNotFound);
    assert_eq!(document.bytes(), b"{\"a\": 1, \"l\": [1]}");
}

#[test]
fn reads_nodes_with_positions_comments_and_canonical_keys() {
    let document = open(&DocFormatId::JSONC, TSCONFIG.as_bytes());
    let found = document.handle.get(&key("compilerOptions.target")).unwrap();
    let node = &document.handle.tree()[found[0].node];
    assert_eq!(node.comment.as_deref(), Some("output level"));
    assert_eq!((node.start.line, node.start.column), (4, 15));
    assert_eq!(node.value, Some(Value::Text("ES2022".to_owned())));
    let options = document.handle.get(&key("compilerOptions")).unwrap();
    assert_eq!(
        document.handle.tree()[options[0].node].comment.as_deref(),
        Some("Compiler settings.")
    );
    let paths = document
        .handle
        .get(&key("compilerOptions.paths.*"))
        .unwrap();
    assert_eq!(paths.len(), 1);
}

fn member_of(document: &OpenDocument, name: &str) -> Value {
    document
        .handle
        .to_value()
        .unwrap()
        .get(name)
        .cloned()
        .unwrap()
}

#[test]
fn removes_elements_of_a_list_by_position_and_keeps_the_others() {
    let mut document = open(&DocFormatId::JSON, b"{\"l\": [1, 2, 3, 4]}");
    document.handle.remove(&key("l[1]")).unwrap();
    assert_eq!(
        member_of(&document, "l"),
        Value::List(vec![Value::Int(1), Value::Int(3), Value::Int(4)])
    );
    document.handle.remove(&key("l[-1]")).unwrap();
    assert_eq!(
        member_of(&document, "l"),
        Value::List(vec![Value::Int(1), Value::Int(3)])
    );
    serde_json::from_slice::<serde_json::Value>(&document.bytes()).unwrap();
}

#[test]
fn appends_to_an_empty_list() {
    let mut document = open(&DocFormatId::JSON, b"{\"l\": []}");
    document
        .handle
        .set(&key("l[+]"), &Value::Int(7), &CREATE)
        .unwrap();
    assert_eq!(member_of(&document, "l"), Value::List(vec![Value::Int(7)]));
    serde_json::from_slice::<serde_json::Value>(&document.bytes()).unwrap();
}

#[test]
fn writes_multi_byte_text_and_escapes_that_read_back_unchanged() {
    let text = "za\u{17c}\u{f3}\u{142}\u{107} \"q\" \\ \n\t \u{1F600}";
    let mut document = open(&DocFormatId::JSON, "{\"a\": \"x\"}".as_bytes());
    document
        .handle
        .set(&key("a"), &Value::Text(text.to_owned()), &CREATE)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&document.bytes()).unwrap();
    assert_eq!(parsed["a"], text);
    assert_eq!(member_of(&document, "a"), Value::Text(text.to_owned()));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

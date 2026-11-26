// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Round trip of the golden trees through the JSON codec, the canonical
//! encoding behind `ast_hash` and the decoding errors.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{AST_HASH_PREFIX, strip_for_hash};
use nurt_contracts::error::{Error, ErrorCode, Stage};
use nurt_syntax::json::{self, MAX_JSON_DEPTH};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden(directory: &str) -> Vec<PathBuf> {
    let directory = workspace().join(directory);
    let mut paths: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            name.ends_with(".json") && !name.starts_with("schema")
        })
        .collect();
    paths.sort();
    paths
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn hash(value: &Value) -> String {
    let input = json::encode_canonical(&strip_for_hash(value)).expect("canonical JSON");
    format!(
        "{AST_HASH_PREFIX}{}",
        hex::encode(Sha256::digest(input.as_bytes()))
    )
}

#[test]
fn surface_trees_survive_encode_and_decode_with_their_hash() {
    let trees = golden("spec/golden/ast");
    assert!(trees.len() >= 60, "found {} trees", trees.len());
    for path in trees {
        let document = json::decode_surface(&read(&path))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        for encoded in [json::encode(&document), json::encode_pretty(&document)] {
            let encoded = encoded.expect("encodes");
            assert_eq!(
                json::decode_surface(&encoded).expect("decodes"),
                document,
                "{}",
                path.display()
            );
        }
        let value = serde_json::to_value(&document).expect("serializes");
        let recorded = path.with_extension("hash");
        if recorded.exists() {
            assert_eq!(
                hash(&value),
                read(&recorded).trim_end(),
                "{}",
                path.display()
            );
        }
    }
}

#[test]
fn bound_trees_survive_encode_and_decode() {
    for path in golden("spec/golden/bound") {
        let document = json::decode_bound(&read(&path))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let encoded = json::encode(&document).expect("encodes");
        assert_eq!(json::decode_bound(&encoded).expect("decodes"), document);
        let recorded = path.with_extension("hash");
        if recorded.exists() {
            let value = serde_json::to_value(&document).expect("serializes");
            assert_eq!(
                hash(&value),
                read(&recorded).trim_end(),
                "{}",
                path.display()
            );
        }
    }
}

fn minimal(nurt: &str) -> Value {
    json!({"nurt": nurt, "program": {"statements": [{"t": "pipeline",
        "head": {"t": "stage", "name": "first", "args": [{"value": {"t": "int", "v": 20}}]},
        "stages": []}]}})
}

fn decode_error(value: Value) -> nurt_syntax::error::SyntaxError {
    let text = value.to_string();
    let from_value = json::decode_surface_value(value).expect_err("invalid tree");
    let from_text = json::decode_surface(&text).expect_err("invalid tree");
    assert_eq!(from_value.code, from_text.code);
    from_value
}

#[test]
fn accepts_an_older_or_equal_minor_version() {
    assert!(json::decode_surface_value(minimal("1.0")).is_ok());
}

#[test]
fn rejects_a_newer_minor_version_with_the_unknown_node_kinds() {
    let mut value = minimal("1.3");
    value["program"]["statements"][0]["head"]["args"][0]["value"] = json!({"t": "uuid", "v": "x"});
    let error = decode_error(value);
    assert_eq!(error.code, ErrorCode::AstVersion);
    assert_eq!(error.found.as_deref(), Some("/nurt"));
    assert_eq!(error.expected, ["uuid"]);
    assert_eq!(error.problem.as_deref(), Some("unknown node kinds: uuid"));
}

#[test]
fn rejects_another_major_version() {
    let error = decode_error(minimal("2.0"));
    assert_eq!(error.code, ErrorCode::AstVersion);
    assert!(
        error.message.contains("major version 2"),
        "{}",
        error.message
    );
}

#[test]
fn reports_a_minor_version_of_five_digits_as_newer() {
    let error = decode_error(minimal("1.70000"));
    assert_eq!(error.code, ErrorCode::AstVersion);
    assert_eq!(error.found.as_deref(), Some("/nurt"));
}

#[test]
fn rejects_a_malformed_version() {
    let error = decode_error(minimal("1.01"));
    assert_eq!(error.code, ErrorCode::AstSchema);
    assert_eq!(error.found.as_deref(), Some("/nurt"));
}

#[test]
fn rejects_an_unknown_member_with_its_pointer() {
    let mut value = minimal("1.0");
    value["program"]["statements"][0]["head"]["effect"] = json!("read");
    let error = decode_error(value);
    assert_eq!(error.code, ErrorCode::AstSchema);
    assert_eq!(
        error.found.as_deref(),
        Some("/program/statements/0/head/effect")
    );
    let mut value = minimal("1.0");
    value["program"]["statements"][0]["head"]["args"][0]["value"]["w"] = json!(1);
    assert_eq!(
        decode_error(value).found.as_deref(),
        Some("/program/statements/0/head/args/0/value/w")
    );
    let mut value = minimal("1.0");
    value["bound"] = json!("1.0");
    assert_eq!(decode_error(value).found.as_deref(), Some("/bound"));
}

#[test]
fn escapes_the_member_name_in_the_pointer_of_an_unknown_member() {
    let mut value = minimal("1.0");
    value["program"]["a/b~c"] = json!(1);
    let error = decode_error(value);
    assert_eq!(error.code, ErrorCode::AstSchema);
    assert_eq!(error.found.as_deref(), Some("/program/a~1b~0c"));
}

#[test]
fn rejects_a_node_kind_of_the_bound_tree() {
    let mut value = minimal("1.0");
    value["program"]["statements"][0]["head"]["args"][0]["value"] = json!({"t": "path", "v": "/a"});
    let error = decode_error(value);
    assert_eq!(error.code, ErrorCode::AstSchema);
    assert_eq!(
        error.found.as_deref(),
        Some("/program/statements/0/head/args/0/value/t")
    );
}

#[test]
fn locates_a_value_of_the_wrong_type() {
    let mut value = minimal("1.0");
    value["program"]["statements"][0]["head"]["args"][0]["value"]["v"] = json!("twenty");
    let error = decode_error(value);
    assert_eq!(error.code, ErrorCode::AstSchema);
    assert_eq!(
        error.found.as_deref(),
        Some("/program/statements/0/head/args/0/value")
    );
}

#[test]
fn rejects_nesting_deeper_than_the_limit() {
    let mut nested = json!({"t": "int", "v": 1});
    for _ in 0..MAX_JSON_DEPTH {
        nested = json!({"t": "list", "items": [nested]});
    }
    let mut value = minimal("1.0");
    value["program"]["statements"][0]["head"]["args"][0]["value"] = nested;
    let error = decode_error(value);
    assert_eq!(error.code, ErrorCode::AstSchema);
    assert!(
        error.message.contains("deeper than 128"),
        "{}",
        error.message
    );
}

#[test]
fn rejects_invalid_json_and_non_objects() {
    assert_eq!(
        json::decode_surface("{").expect_err("invalid").code,
        ErrorCode::AstSchema
    );
    assert_eq!(
        json::decode_surface("[]").expect_err("invalid").code,
        ErrorCode::AstSchema
    );
    assert_eq!(
        json::decode_bound("1").expect_err("invalid").code,
        ErrorCode::AstSchema
    );
}

#[test]
fn rejects_a_metadata_integer_that_canonical_json_cannot_represent() {
    let mut value = minimal("1.0");
    value["meta"] = json!({"source": 9_007_199_254_740_993_i64});
    let error = decode_error(value);
    assert_eq!(error.code, ErrorCode::AstSchema);
    assert_eq!(error.found.as_deref(), Some("/meta"));
}

#[test]
fn lists_ident_among_the_unknown_node_kinds_of_a_newer_bound_tree() {
    let value = json!({"nurt": "1.0", "bound": "1.9", "catalog_rev": "cat-1", "corr_rev": "corr-1",
        "program": {"statements": [{"t": "pipeline",
            "head": {"t": "stage", "name": "list", "effect": "read", "phase": "source",
                "args": [{"name": "value", "value": {"t": "ident", "v": "x"}}]},
            "stages": []}]}});
    let error = json::decode_bound_value(value).expect_err("newer minor version");
    assert_eq!(error.code, ErrorCode::AstVersion);
    assert_eq!(error.found.as_deref(), Some("/bound"));
    assert_eq!(error.expected, ["ident"]);
}

#[test]
fn decoding_errors_map_to_the_parse_stage() {
    let error = Error::from(decode_error(minimal("2.0")));
    assert_eq!(error.code, ErrorCode::AstVersion);
    assert_eq!(error.stage(), Some(Stage::Parse));
}

/// Code column of `expected.tsv` for a tree that the decoder admits.
const ACCEPTED: &str = "-";

#[test]
fn invalid_golden_trees_give_their_codes() {
    let directory = workspace().join("tests/d1/golden/invalid");
    let index = read(&directory.join("expected.tsv"));
    let mut checked = 0;
    for row in index.lines().skip(1) {
        let columns: Vec<&str> = row.split('\t').collect();
        let [file, kind, code, ..] = columns.as_slice() else {
            continue;
        };
        let name = file.trim_start_matches("invalid/");
        let text = read(&directory.join(name));
        let decoded = match *kind {
            "ast" => json::decode_surface(&text).map(drop),
            _ => json::decode_bound(&text).map(drop),
        };
        match (*code, decoded) {
            (ACCEPTED, Ok(())) => {}
            (ACCEPTED, Err(error)) => panic!("{name}: admissible tree rejected: {error}"),
            (_, Ok(())) => panic!("{name}: invalid tree accepted"),
            (code, Err(error)) => {
                assert_eq!(error.code.to_string(), code, "{name}");
                assert!(error.found.is_some(), "{name}: no JSON Pointer");
            }
        }
        checked += 1;
    }
    assert!(checked >= 16, "checked {checked} invalid trees");
}

#[test]
fn canonical_encoding_sorts_members_and_drops_insignificant_white_space() {
    let value = json!({"v": 1.5, "t": "num", "span": [1, 1, 1, 4]});
    assert_eq!(
        json::encode_canonical(&value).expect("encodes"),
        r#"{"span":[1,1,1,4],"t":"num","v":1.5}"#
    );
}

#[test]
fn rejects_an_object_that_repeats_a_key_at_any_depth() {
    let program = r#""program":{"statements":[{"t":"pipeline","head":{"t":"stage","name":"count","args":[]},"stages":[]}]}"#;
    let documents = [
        format!(r#"{{"nurt":"1.0","nurt":"1.1",{program}}}"#),
        format!(
            r#"{{"nurt":"1.0",{program},"meta":{{"a":1,"b":{{"c":1,"c":2}}}}}}"#
        ),
    ];
    for text in documents {
        let error = json::decode_surface(&text).unwrap_err();
        assert_eq!(error.code, ErrorCode::AstSchema, "{text}");
        assert!(error.message.contains("repeated"), "{}", error.message);
    }
    let fine = format!(r#"{{"nurt":"1.0",{program},"meta":{{"a":1,"b":{{"c":1}}}}}}"#);
    json::decode_surface(&fine).unwrap();
}

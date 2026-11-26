// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Every format accepts the same tool calls.
//!
//! A call is accepted by a format when the arguments, written the way a
//! model writes them for that format, match the rendered input schema, turn
//! into a request, and the request passes the checks of the instance
//! (`Request::validate` and the schema `nurt-request/1`). A constraint that
//! a format moves into a description is enforced by the last two steps, so
//! the set of accepted calls is the same in every format.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

// Each test crate uses a different part of the shared helpers.
#[allow(dead_code)]
mod common;

use nurt_tooldef::{ToolFormat, definition, request_from_arguments};
use serde_json::{Value, json};

use common::{as_json_schema, encode, input_schema, read_json, validator};

/// Smallest syntax tree of the published examples.
fn tree() -> Value {
    read_json(&common::crate_dir().join("../../spec/golden/ast/example-37-journal-space.json"))
}

/// Arguments and whether the instance accepts them.
fn calls() -> Vec<(&'static str, Value, bool)> {
    vec![
        (
            "program only",
            json!({"program": "find \"/home/user/src\""}),
            true,
        ),
        (
            "program with every option",
            json!({
                "program": "find \"/home/user/src\" |> count",
                "preview": true, "base": "/home/user", "limit": 20,
                "fields": ["path", "size"], "format": "json", "verbosity": "quiet",
                "diff": "stat", "diff_context": 0, "max_chars": 0,
                "save_to": "/home/user/result.json", "timeout": "1m30s",
                "session": "s-1", "language": "pl", "script": true
            }),
            true,
        ),
        ("syntax tree", json!({"ast": tree(), "preview": true}), true),
        ("cursor", json!({"cursor": "c-2", "limit": 100}), true),
        (
            "limit at the lower bound",
            json!({"program": "help", "limit": 1}),
            true,
        ),
        (
            "limit at the upper bound",
            json!({"program": "help", "limit": 5000}),
            true,
        ),
        (
            "multi-byte program",
            json!({"program": "find \"/home/użytkownik/zażółć\""}),
            true,
        ),
        (
            "negative limit",
            json!({"program": "help", "limit": -1}),
            false,
        ),
        ("null program", json!({"program": null}), false),
        ("accepted candidate", json!({"accept": "c1"}), true),
        ("no input", json!({"preview": true}), false),
        (
            "two inputs",
            json!({"program": "help", "cursor": "c-2"}),
            false,
        ),
        (
            "limit below range",
            json!({"program": "help", "limit": 0}),
            false,
        ),
        (
            "limit above range",
            json!({"program": "help", "limit": 5001}),
            false,
        ),
        (
            "unknown format",
            json!({"program": "help", "format": "xml"}),
            false,
        ),
        (
            "empty field name",
            json!({"program": "help", "fields": [""]}),
            false,
        ),
        (
            "malformed timeout",
            json!({"program": "help", "timeout": "soon"}),
            false,
        ),
        (
            "relative base",
            json!({"program": "help", "base": "src"}),
            false,
        ),
        ("empty cursor", json!({"cursor": ""}), false),
        ("program of the wrong type", json!({"program": 1}), false),
        (
            "unknown argument",
            json!({"program": "help", "command": "ls"}),
            false,
        ),
        (
            "host field",
            json!({"program": "help", "nurt": "1.0"}),
            false,
        ),
        ("variant", json!({"program": "help", "variant": "a"}), false),
    ]
}

fn accepted(format: ToolFormat, arguments: &Value, request_schema: &jsonschema::Validator) -> bool {
    let definition = definition(format, &common::options()).unwrap();
    let schema = as_json_schema(format, input_schema(format, &definition));
    let encoded = encode(format, &schema, arguments);
    if !validator(&schema).is_valid(&encoded) {
        return false;
    }
    let Ok(request) = request_from_arguments(format, &encoded) else {
        return false;
    };
    request.validate().is_ok() && request_schema.is_valid(&serde_json::to_value(&request).unwrap())
}

#[test]
fn every_format_accepts_exactly_the_same_calls() {
    let request_schema = validator(&serde_json::from_str(nurt_contracts::schema::REQUEST).unwrap());
    for (name, arguments, expected) in calls() {
        for format in ToolFormat::ALL {
            assert_eq!(
                accepted(*format, &arguments, &request_schema),
                expected,
                "{format}: {name}"
            );
        }
    }
}

#[test]
fn every_format_gives_the_same_request() {
    for (name, arguments, expected) in calls() {
        if !expected {
            continue;
        }
        let neutral = request_from_arguments(ToolFormat::JsonSchema, &arguments).unwrap();
        for format in ToolFormat::ALL {
            let definition = definition(*format, &common::options()).unwrap();
            let schema = as_json_schema(*format, input_schema(*format, &definition));
            let encoded = encode(*format, &schema, &arguments);
            let request = request_from_arguments(*format, &encoded).unwrap();
            assert_eq!(request, neutral, "{format}: {name}");
        }
    }
}

#[test]
fn rendered_definitions_are_deterministic() {
    for format in ToolFormat::ALL {
        let first = nurt_tooldef::render(*format, &common::options()).unwrap();
        let second = nurt_tooldef::render(*format, &common::options().clone()).unwrap();
        assert_eq!(first, second, "{format}");
    }
}

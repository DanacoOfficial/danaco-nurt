// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-cli/tests/serve_selectors.rs
// Responsibility:   Parameters `{what}` of `catalog` and `{query}` of `history`, and the.
// Dependencies:     nurt_contracts, nurt_core, nurt_ffi, nurt_testkit, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parameters `{what}` of `catalog` and `{query}` of `history`, and the
//! check document returned by `check`, through the protocol server.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "the helpers of the tests fail the test on unexpected output"
)]

use std::io::Cursor;
use std::sync::Arc;

use nurt_cli::serve::{self, INVALID_PARAMS, ServeOptions};
use nurt_contracts::schema::{self, ID_BASE};
use nurt_core::config::ConfigSources;
use nurt_core::{Engine, EngineConfig};
use nurt_ffi::e2e::{self, LIST};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use serde_json::{Value, json};

fn plain_engine() -> Engine {
    Engine::open(
        EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir(),
    )
    .unwrap()
}

/// Sends `calls` after `initialize` and returns the reply to each, in order.
fn converse(engine: Engine, calls: &[(&str, Value)]) -> Vec<Value> {
    let mut messages = vec![json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocol": "nurt-rpc/1"}
    })];
    for (index, (method, params)) in calls.iter().enumerate() {
        messages.push(json!({
            "jsonrpc": "2.0", "id": index + 2, "method": method, "params": params
        }));
    }
    let mut input = Vec::new();
    for message in &messages {
        input.extend_from_slice(message.to_string().as_bytes());
        input.push(b'\n');
    }
    let mut output = Vec::new();
    serve::run(
        ServeOptions::engine(engine),
        Cursor::new(input),
        &mut output,
    );
    let replies: Vec<Value> = output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    (0..calls.len())
        .map(|index| {
            replies
                .iter()
                .find(|reply| reply["id"] == index + 2)
                .unwrap()
                .clone()
        })
        .collect()
}

fn validator(name: &str) -> jsonschema::Validator {
    let registry = schema::ALL
        .iter()
        .fold(jsonschema::Registry::new(), |builder, (name, content)| {
            builder
                .add(
                    format!("{ID_BASE}nurt-{name}-1.schema.json"),
                    serde_json::from_str::<Value>(content).unwrap(),
                )
                .unwrap()
        })
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&json!({"$ref": format!("{ID_BASE}nurt-{name}-1.schema.json")}))
        .unwrap()
}

fn assert_valid(validator: &jsonschema::Validator, document: &Value) {
    let errors: Vec<String> = validator
        .iter_errors(document)
        .map(|error| format!("{error} at {}", error.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{document}\n{errors:#?}");
}

#[test]
fn catalog_what_selects_a_group_or_a_command() {
    let calls = [
        ("catalog", Value::Null),
        ("catalog", json!({"what": null})),
        ("catalog", json!({"what": {"group": "core"}})),
    ];
    let replies = converse(plain_engine(), &calls);
    let summary = &replies[0]["result"];
    assert!(summary["groups"].is_array(), "{summary}");
    assert_eq!(&replies[1]["result"], summary);
    let group = &replies[2]["result"];
    assert_eq!(group["group"]["name"], "core");
    let command = group["commands"][0]["name"].as_str().unwrap().to_owned();
    let calls = [
        ("catalog", json!({"what": {"command": command}})),
        ("catalog", json!({"what": {"group": "no-such-group"}})),
        ("catalog", json!({"what": {"command": "no-such-command"}})),
    ];
    let replies = converse(plain_engine(), &calls);
    assert_eq!(replies[0]["result"]["command"]["name"], json!(command));
    assert_eq!(
        replies[1]["result"]["report"]["error"]["code"],
        "E_UNKNOWN_NAME"
    );
    assert_eq!(
        replies[2]["result"]["report"]["error"]["code"],
        "E_UNKNOWN_COMMAND"
    );
    let result = validator("catalog-result");
    assert_valid(&result, summary);
    assert_valid(&result, group);
    assert_valid(&result, &converse(plain_engine(), &calls[..1])[0]["result"]);
}

#[test]
fn catalog_rejects_a_selection_that_is_not_in_what() {
    let calls = [
        ("catalog", json!({"what": "find"})),
        ("catalog", json!({"what": {"group": "a", "command": "b"}})),
        ("catalog", json!({"what": {"name": "a"}})),
        ("catalog", json!({"group": "core"})),
    ];
    for reply in converse(plain_engine(), &calls) {
        assert_eq!(reply["error"]["code"], INVALID_PARAMS, "{reply}");
    }
}

#[test]
fn history_query_is_read_from_query_or_from_the_object() {
    let calls = [
        ("history", json!({"limit": 3})),
        ("history", json!({"query": {"limit": 3}})),
        ("history", json!({"query": "{\"limit\": 3}"})),
        ("history", json!({"query": null})),
        ("history", json!({})),
    ];
    let replies = converse(plain_engine(), &calls);
    assert_eq!(replies[0]["result"], replies[1]["result"]);
    assert_eq!(replies[0]["result"], replies[2]["result"]);
    assert_eq!(replies[3]["result"], replies[4]["result"]);
    let calls = [
        ("history", json!({"query": 7})),
        ("history", json!({"query": "{"})),
        ("history", json!({"query": {"limit": "many"}})),
        ("history", json!({"query": {}, "limit": 3})),
    ];
    for reply in converse(plain_engine(), &calls) {
        assert_eq!(reply["error"]["code"], INVALID_PARAMS, "{reply}");
    }
}

#[test]
fn check_result_conforms_to_the_check_document() {
    let world = e2e::world().unwrap();
    let calls = [("check", json!({"nurt": "1.0", "program": LIST}))];
    let replies = converse(world.engine, &calls);
    let result = &replies[0]["result"];
    assert_eq!(result["format"], "nurt-check/1", "{result}");
    assert_valid(&validator("check"), result);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

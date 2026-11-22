// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conformance of the messages of `nurt serve` with the published schemas
//! `nurt-rpc/1` and `nurt-event/1`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "the helpers of the tests fail the test on unexpected output"
)]

use std::collections::BTreeMap;
use std::io::Cursor;

use nurt_cli::serve::{self, ServeOptions};
use nurt_contracts::schema::{self, ID_BASE};
use serde_json::{Value, json};

fn parse(content: &str) -> Value {
    serde_json::from_str(content).unwrap()
}

/// Validator of the document `{"$ref": reference}` with every published
/// schema registered under its `$id`.
fn validator(reference: &str) -> jsonschema::Validator {
    let registry = schema::ALL
        .iter()
        .fold(jsonschema::Registry::new(), |builder, (name, content)| {
            builder
                .add(
                    format!("{ID_BASE}nurt-{name}-1.schema.json"),
                    parse(content),
                )
                .unwrap()
        })
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&json!({"$ref": format!("{ID_BASE}{reference}")}))
        .unwrap()
}

fn assert_valid(validator: &jsonschema::Validator, message: &Value, what: &str) {
    let errors: Vec<String> = validator
        .iter_errors(message)
        .map(|error| format!("{error} at {}", error.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{what}: {message}\n{errors:#?}");
}

/// Checks every message of the conversation: requests that were answered
/// with a result, replies, results by method and events. The methods in
/// `unchecked` and the methods answered with a failure report (an engine
/// without language front end or journal) are checked as messages only, not
/// as requests and results.
fn assert_conforms(requests: &[Value], output: &[Value], unchecked: &[&str]) {
    let message = validator("nurt-rpc-1.schema.json");
    let event = validator("nurt-event-1.schema.json");
    let mut results = BTreeMap::new();
    let sent: BTreeMap<String, &Value> = requests
        .iter()
        .filter_map(|request| Some((request.get("id")?.to_string(), request)))
        .collect();
    let mut successes = 0;
    for reply in output {
        assert_valid(&message, reply, "server message");
        if reply.get("method").is_some() {
            assert_valid(&event, &reply["params"], "event");
            continue;
        }
        let id = reply["id"].to_string();
        let Some(result) = reply.get("result") else {
            continue;
        };
        successes += 1;
        let request = sent[&id];
        let method = request["method"].as_str().unwrap();
        if unchecked.contains(&method) || result["report"]["status"] == "failed" {
            continue;
        }
        let check = results.entry(method.to_owned()).or_insert_with(|| {
            validator(&format!(
                "nurt-rpc-1.schema.json#/$defs/results/properties/{method}"
            ))
        });
        assert_valid(check, result, &format!("result of {method}"));
        assert_valid(&message, request, &format!("request {method}"));
    }
    assert!(successes > 0, "no successful reply in the conversation");
}

fn ndjson(messages: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for message in messages {
        bytes.extend_from_slice(message.to_string().as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

fn read(output: &[u8]) -> Vec<Value> {
    output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

fn converse(options: ServeOptions, requests: &[Value]) -> Vec<Value> {
    let mut output = Vec::new();
    serve::run(options, Cursor::new(ndjson(requests)), &mut output);
    read(&output)
}

#[test]
fn rejected_requests_are_replies_of_the_schema() {
    let requests = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "info"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "nothing"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "initialize", "params": {"protocol": "nurt-rpc/2"}}),
    ];
    let output = converse(rpc_options(), &requests);
    let message = validator("nurt-rpc-1.schema.json");
    assert_eq!(output.len(), 3);
    for reply in &output {
        assert_valid(&message, reply, "error reply");
        assert!(reply["error"]["code"].is_i64());
    }
}

#[cfg(feature = "stub")]
fn rpc_options() -> ServeOptions {
    ServeOptions::stub(&nurt_ffi::stub::cases_dir()).unwrap()
}

#[cfg(not(feature = "stub"))]
fn rpc_options() -> ServeOptions {
    ServeOptions::engine(engine())
}

#[cfg(not(feature = "stub"))]
fn engine() -> nurt_core::Engine {
    use std::sync::Arc;

    use nurt_core::EngineConfig;
    use nurt_core::config::ConfigSources;
    use nurt_testkit::clock::VirtualClock;
    use nurt_testkit::host::TestHost;

    nurt_core::Engine::open(
        EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir(),
    )
    .unwrap()
}

#[cfg(feature = "stub")]
#[test]
fn stub_transcript_conforms_to_the_schemas() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/serve/methods.requests.ndjson");
    let mut requests = read(&std::fs::read(path).unwrap());
    requests[0]["params"]["capabilities"]["events"] = json!(true);
    let shutdown = requests
        .iter()
        .position(|r| r["method"] == "shutdown")
        .unwrap();
    let shutdown = requests.remove(shutdown);
    requests.push(shutdown.clone());
    let output = converse(rpc_options(), &requests);
    assert_conforms(&requests, &output, &[]);
    assert!(output.iter().any(|m| m["method"] == "nurt/event"));

    // A separate conversation keeps the runs below the limit of requests in flight.
    let mut runs = vec![requests[0].clone()];
    for (id, program) in [
        (100, r#"find "/srv/app" glob: "*.rs""#),
        (101, r#"find "/srv/panic""#),
    ] {
        runs.push(json!({"jsonrpc": "2.0", "id": id, "method": "run", "params": {"nurt": "1.0", "program": program}}));
    }
    runs.push(shutdown);
    let output = converse(rpc_options(), &runs);
    assert_conforms(&runs, &output, &[]);
    assert!(output.iter().any(|m| m["method"] == "nurt/event"));
    assert!(
        output
            .iter()
            .any(|m| m["error"]["data"]["panic"].is_string())
    );
}

#[cfg(not(feature = "stub"))]
#[test]
fn engine_transcript_conforms_to_the_schemas() {
    let call = |id: u64, method: &str, params: Value| json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let requests = vec![
        call(
            1,
            "initialize",
            json!({"protocol": "nurt-rpc/1", "capabilities": {"events": true}}),
        ),
        call(2, "info", Value::Null),
        call(3, "schema", json!({"name": "report"})),
        call(4, "parse", json!({"program": "find \"/srv/app\""})),
        call(5, "check", json!({"nurt": "1.0", "program": "count"})),
        call(6, "run", json!({"nurt": "1.0", "program": "count"})),
        call(7, "describe", json!({"subject": "find"})),
        call(8, "help", json!({"topic": "lists"})),
        call(9, "catalog", Value::Null),
        call(10, "history", json!({})),
        call(11, "space", Value::Null),
        call(12, "cancel", json!({"request_id": 99})),
        call(13, "shutdown", Value::Null),
    ];
    let output = converse(rpc_options(), &requests);
    assert_conforms(&requests, &output, &[]);
    assert!(output.iter().any(|m| m["method"] == "nurt/event"));
}

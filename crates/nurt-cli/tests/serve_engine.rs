// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Protocol tests of `nurt serve` executing requests through the engine.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "the helpers of the tests fail the test on unexpected output"
)]
#![allow(
    clippy::cognitive_complexity,
    reason = "each test is one conversation with the server, read in order"
)]

use std::collections::BTreeMap;
use std::io::Cursor;
use std::process::ExitCode;
use std::sync::Arc;

use nurt_cli::serve::{self, INVALID_PARAMS, ServeOptions};
use nurt_core::config::ConfigSources;
use nurt_core::{Engine, EngineConfig};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use serde_json::{Value, json};

fn engine() -> Engine {
    Engine::open(
        EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir(),
    )
    .unwrap()
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// Serves `messages` to the end of the input; returns the exit code and
/// every message written by the server.
fn serve(messages: &[Value]) -> (ExitCode, Vec<Value>) {
    let mut input = Vec::new();
    for message in messages {
        input.extend_from_slice(message.to_string().as_bytes());
        input.push(b'\n');
    }
    let mut output = Vec::new();
    let code = serve::run(
        ServeOptions::engine(engine()),
        Cursor::new(input),
        &mut output,
    );
    let messages = output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    (code, messages)
}

fn replies(messages: &[Value]) -> BTreeMap<u64, Value> {
    messages
        .iter()
        .filter(|message| message.get("method").is_none())
        .map(|message| (message["id"].as_u64().unwrap(), message.clone()))
        .collect()
}

#[test]
fn every_engine_method_is_answered() {
    let (code, messages) = serve(&[
        request(
            1,
            "initialize",
            json!({"protocol": "nurt-rpc/1", "capabilities": {"events": true}}),
        ),
        request(2, "info", Value::Null),
        request(3, "schema", json!({"name": "report"})),
        request(4, "parse", json!({"program": "find \"/srv/app\""})),
        request(5, "correct", json!({"program": "fnd \"/srv/app\""})),
        request(6, "check", json!({"nurt": "1.0", "program": "count"})),
        request(7, "run", json!({"nurt": "1.0", "program": "count"})),
        request(8, "page", json!({"cursor": "c_0000", "limit": 5})),
        request(9, "describe", json!({"subject": "find"})),
        request(10, "help", json!({"topic": "lists"})),
        request(11, "catalog", Value::Null),
        request(12, "history", json!({})),
        request(13, "space", json!({"volume": "relative/path"})),
        request(14, "space", Value::Null),
        request(15, "cancel", json!({"request_id": 99})),
        request(16, "shutdown", Value::Null),
    ]);
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    let replies = replies(&messages);
    assert_eq!(replies.len(), 16);
    for (id, reply) in &replies {
        assert!(reply.get("result").is_some(), "request {id}: {reply}");
    }
    let initialize = &replies[&1]["result"];
    assert_eq!(initialize["protocol"], "nurt-rpc/1");
    assert_eq!(initialize["abi"], Value::Null);
    assert!(initialize["groups"].is_array());
    assert_eq!(replies[&2]["result"]["formats"]["envelope"], "1.0");
    let report_schema: Value = serde_json::from_str(nurt_contracts::schema::REPORT).unwrap();
    assert_eq!(replies[&3]["result"], report_schema);
    assert!(replies[&7]["result"]["report"]["status"].is_string());
    assert!(replies[&8]["result"]["report"].is_object());
    assert_eq!(
        replies[&13]["result"]["report"]["error"]["code"],
        "E_PATH_RELATIVE"
    );
    assert_eq!(replies[&15]["result"], json!({"cancelled": false}));
    assert_eq!(replies[&16]["result"], Value::Null);

    let events: Vec<&Value> = messages
        .iter()
        .filter(|message| message["method"] == "nurt/event")
        .collect();
    let types: Vec<&Value> = events.iter().map(|event| &event["params"]["t"]).collect();
    assert_eq!(types.first(), Some(&&json!("exec.started")));
    assert_eq!(types.last(), Some(&&json!("exec.finished")));
    assert!(events.iter().all(|event| event["params"]["request"] == "7"));
    let run_reply = messages
        .iter()
        .position(|message| message.get("method").is_none() && message["id"] == 7)
        .unwrap();
    let last_event = messages
        .iter()
        .rposition(|message| message["method"] == "nurt/event")
        .unwrap();
    assert!(last_event < run_reply, "events precede the reply of `run`");
}

#[test]
fn malformed_engine_parameters_are_invalid_params() {
    let (_, messages) = serve(&[
        request(1, "initialize", json!({"protocol": "nurt-rpc/1"})),
        request(2, "print", json!({"ast": {"pipelines": 1}})),
        request(3, "history", json!({"limit": "many"})),
        request(4, "run", Value::Null),
    ]);
    let replies = replies(&messages);
    for id in [2, 3, 4] {
        assert_eq!(
            replies[&id]["error"]["code"], INVALID_PARAMS,
            "request {id}"
        );
    }
}

#[test]
fn a_session_from_initialize_reaches_the_engine() {
    let (_, messages) = serve(&[
        request(
            1,
            "initialize",
            json!({"protocol": "nurt-rpc/1", "session": "s1"}),
        ),
        request(2, "run", json!({"nurt": "1.0", "program": "count"})),
        request(3, "page", json!({"cursor": "c_0000"})),
    ]);
    let replies = replies(&messages);
    assert!(replies[&2]["result"]["report"].is_object());
    assert!(replies[&3]["result"]["report"].is_object());
}

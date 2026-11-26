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

use nurt_cli::serve::{
    self, Framing, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, ServeOptions,
    application_code,
};
use nurt_contracts::error::AbiStatus;
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

fn ndjson(messages: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for message in messages {
        bytes.extend_from_slice(message.to_string().as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// Serves `input` to the end with `options`; returns the exit code and the
/// bytes written by the server.
fn serve_bytes(options: ServeOptions, input: Vec<u8>) -> (ExitCode, Vec<u8>) {
    let mut output = Vec::new();
    let code = serve::run(options, Cursor::new(input), &mut output);
    (code, output)
}

fn lines(output: &[u8]) -> Vec<Value> {
    output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

fn content_length_messages(mut bytes: &[u8]) -> Vec<Value> {
    let mut messages = Vec::new();
    while !bytes.is_empty() {
        let text = std::str::from_utf8(bytes).unwrap();
        let (header, rest) = text.split_once("\r\n\r\n").unwrap();
        let length: usize = header
            .strip_prefix("Content-Length: ")
            .unwrap()
            .parse()
            .unwrap();
        messages.push(serde_json::from_str(&rest[..length]).unwrap());
        bytes = &rest.as_bytes()[length..];
    }
    messages
}

fn is_success(code: ExitCode) -> bool {
    format!("{code:?}") == format!("{:?}", ExitCode::SUCCESS)
}

fn initialize(id: u64) -> Value {
    request(id, "initialize", json!({"protocol": "nurt-rpc/1"}))
}

#[test]
fn invalid_messages_get_error_replies_and_reading_continues() {
    let mut input = ndjson(&[initialize(1)]);
    input.extend_from_slice(b"{not json\n");
    input.extend_from_slice(format!("{{\"pad\":\"{}\"}}\n", "x".repeat(200)).as_bytes());
    input.extend_from_slice(b"[1]\n");
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":1}\n");
    input.extend(ndjson(&[request(2, "info", Value::Null)]));
    let options = ServeOptions::engine(engine()).with_max_message_bytes(160);
    let (code, output) = serve_bytes(options, input);
    assert!(is_success(code));
    let replies = lines(&output);
    assert_eq!(replies.len(), 5, "the reply sent by the client is ignored");
    let codes: Vec<Value> = replies[1..4]
        .iter()
        .map(|reply| reply["error"]["code"].clone())
        .collect();
    assert_eq!(
        codes,
        [
            json!(PARSE_ERROR),
            json!(INVALID_REQUEST),
            json!(INVALID_REQUEST)
        ]
    );
    assert!(replies[1..4].iter().all(|reply| reply["id"].is_null()));
    assert_eq!(replies[4]["id"], 2);
}

#[test]
fn messages_that_are_not_requests_are_answered_with_their_identifier() {
    let input = [
        r#"{"jsonrpc":"1.0","id":4,"method":"info"}"#,
        r#"{"jsonrpc":"2.0","id":{},"method":"info"}"#,
        r#"{"jsonrpc":"2.0","id":5}"#,
        r#"{"jsonrpc":"2.0","id":6,"method":7}"#,
    ]
    .map(|line| format!("{line}\n"))
    .concat();
    let (code, output) = serve_bytes(ServeOptions::engine(engine()), input.into_bytes());
    assert!(is_success(code));
    let replies = lines(&output);
    let ids: Vec<Value> = replies.iter().map(|reply| reply["id"].clone()).collect();
    assert_eq!(ids, [json!(4), Value::Null, json!(5), json!(6)]);
    assert!(
        replies
            .iter()
            .all(|reply| reply["error"]["code"] == INVALID_REQUEST)
    );
}

#[test]
fn a_request_with_a_null_identifier_is_answered_with_a_null_identifier() {
    let input = ndjson(&[
        initialize(1),
        json!({"jsonrpc": "2.0", "id": null, "method": "info"}),
    ]);
    let (_, output) = serve_bytes(ServeOptions::engine(engine()), input);
    let replies = lines(&output);
    assert!(
        replies
            .iter()
            .any(|reply| reply["id"].is_null() && reply["result"].is_object()),
        "{replies:?}"
    );
}

#[test]
fn broken_framing_ends_the_server_with_a_failure() {
    let initialize = initialize(1).to_string();
    let mut input = Vec::new();
    input.extend_from_slice(
        format!("Content-Length: {}\r\n\r\n{initialize}", initialize.len()).as_bytes(),
    );
    input.extend_from_slice(b"Content-Length: many\r\n\r\n{}");
    let options = ServeOptions::engine(engine()).with_framing(Framing::ContentLength);
    let (code, output) = serve_bytes(options, input);
    assert!(!is_success(code));
    let replies = content_length_messages(&output);
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[1]["error"]["code"], PARSE_ERROR);
    assert!(replies[1]["id"].is_null());
}

#[test]
fn shutdown_waits_for_requests_answers_last_and_stops_reading() {
    let input = ndjson(&[
        initialize(1),
        request(2, "schema", json!({"name": "report"})),
        request(3, "shutdown", Value::Null),
        request(4, "info", Value::Null),
    ]);
    let (code, output) = serve_bytes(ServeOptions::engine(engine()), input);
    assert!(is_success(code));
    let replies = lines(&output);
    let ids: Vec<Value> = replies.iter().map(|reply| reply["id"].clone()).collect();
    assert_eq!(ids.last(), Some(&json!(3)));
    assert!(ids.contains(&json!(2)));
    assert!(!ids.contains(&json!(4)));
    assert_eq!(replies.last().unwrap()["result"], Value::Null);
}

#[test]
fn a_shutdown_notification_stops_the_server_without_a_reply() {
    let input = ndjson(&[
        initialize(1),
        json!({"jsonrpc": "2.0", "method": "shutdown"}),
        request(2, "info", Value::Null),
    ]);
    let (code, output) = serve_bytes(ServeOptions::engine(engine()), input);
    assert!(is_success(code));
    assert_eq!(lines(&output).len(), 1);
}

#[test]
fn the_end_of_the_input_ends_the_server_after_the_replies() {
    let (code, output) = serve_bytes(ServeOptions::engine(engine()), Vec::new());
    assert!(is_success(code));
    assert!(output.is_empty());
    let input = ndjson(&[initialize(1), request(2, "info", Value::Null)]);
    let (code, output) = serve_bytes(ServeOptions::engine(engine()), input);
    assert!(is_success(code));
    assert_eq!(replies(&lines(&output)).len(), 2);
}

#[test]
fn notifications_are_executed_without_replies() {
    let input = ndjson(&[
        initialize(1),
        json!({"jsonrpc": "2.0", "method": "info"}),
        json!({"jsonrpc": "2.0", "method": "nothing"}),
        json!({"jsonrpc": "2.0", "method": "parse", "params": {"program": 1}}),
    ]);
    let (code, output) = serve_bytes(ServeOptions::engine(engine()), input);
    assert!(is_success(code));
    let replies = lines(&output);
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["id"], 1);
}

#[test]
fn requests_before_initialize_are_rejected() {
    let input = ndjson(&[
        request(1, "info", Value::Null),
        request(2, "cancel", json!({"request_id": 1})),
        request(3, "nothing", Value::Null),
        initialize(4),
        request(5, "info", Value::Null),
    ]);
    let (_, output) = serve_bytes(ServeOptions::engine(engine()), input);
    let replies = replies(&lines(&output));
    assert_eq!(replies[&1]["error"]["code"], INVALID_REQUEST);
    assert!(
        replies[&1]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("initialize")
    );
    assert_eq!(replies[&2]["error"]["code"], INVALID_REQUEST);
    assert_eq!(replies[&3]["error"]["code"], METHOD_NOT_FOUND);
    assert!(
        replies[&3]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("known: initialize")
    );
    assert!(replies[&4]["result"].is_object());
    assert!(replies[&5]["result"].is_object());
}

#[test]
fn initialize_negotiates_the_major_versions() {
    let abi_error = application_code(AbiStatus::Abi);
    let attempts = [
        (json!({"protocol": "nurt-rpc/2"}), abi_error),
        (json!({"protocol": "lsp/1"}), abi_error),
        (json!({"protocol": "nurt-rpc/1", "ast": "2.0"}), abi_error),
        (
            json!({"protocol": "nurt-rpc/1", "config_overrides": {"output": {}}}),
            INVALID_PARAMS,
        ),
        (json!({"client": {"name": "x"}}), INVALID_PARAMS),
        (json!({"protocol": 1}), INVALID_PARAMS),
    ];
    for (params, code) in attempts {
        let input = ndjson(&[request(1, "initialize", params.clone())]);
        let (_, output) = serve_bytes(ServeOptions::engine(engine()), input);
        assert_eq!(lines(&output)[0]["error"]["code"], code, "{params}");
    }
    let supported = json!({
        "protocol": "nurt-rpc/1.3", "ast": "1.0", "abi": 0x0002_0000,
        "config_overrides": {}, "client": {"name": "x"}, "language": "1.0"
    });
    let input = ndjson(&[
        request(1, "initialize", supported),
        request(2, "initialize", json!({"protocol": "nurt-rpc/1"})),
    ]);
    let (_, output) = serve_bytes(ServeOptions::engine(engine()), input);
    let replies = replies(&lines(&output));
    assert_eq!(replies[&1]["result"]["protocol"], "nurt-rpc/1");
    assert_eq!(replies[&2]["error"]["code"], INVALID_REQUEST);
}

#[test]
fn cancel_names_exactly_one_target_and_job_cancelling_is_not_available() {
    let input = ndjson(&[
        initialize(1),
        request(2, "cancel", json!({"job_id": "j1"})),
        request(3, "cancel", json!({"request_id": 1, "job_id": "j1"})),
        request(4, "cancel", json!({})),
        request(5, "cancel", json!({"request_id": "absent"})),
    ]);
    let (_, output) = serve_bytes(ServeOptions::engine(engine()), input);
    let replies = replies(&lines(&output));
    assert_eq!(replies[&2]["result"]["report"]["status"], "failed");
    assert_eq!(
        replies[&2]["result"]["report"]["error"]["code"],
        "E_UNSUPPORTED"
    );
    assert_eq!(replies[&3]["error"]["code"], INVALID_PARAMS);
    assert_eq!(replies[&4]["error"]["code"], INVALID_PARAMS);
    assert_eq!(replies[&5]["result"], json!({"cancelled": false}));
}

#[test]
fn initialize_ignores_unknown_members_and_is_accepted_once() {
    let input = ndjson(&[
        request(
            1,
            "initialize",
            json!({"protocol": "nurt-rpc/1", "client": {"name": "x"}}),
        ),
        request(2, "history", json!({"limit": 1})),
        initialize(3),
    ]);
    let (_, output) = serve_bytes(ServeOptions::engine(engine()), input);
    let replies = replies(&lines(&output));
    assert!(replies[&1]["result"].is_object());
    assert!(replies[&2].get("result").is_some());
    assert_eq!(replies[&3]["error"]["code"], INVALID_REQUEST);
}

#[test]
fn parameters_of_the_wrong_type_are_invalid_params() {
    let input = ndjson(&[
        initialize(1),
        request(2, "print", json!({"ast": [1, 2]})),
        request(3, "print", json!({})),
        request(4, "parse", json!({"program": "x", "variant": "z"})),
        request(5, "page", json!({"cursor": 7})),
        request(6, "space", json!({"volume": 7})),
    ]);
    let (_, output) = serve_bytes(ServeOptions::engine(engine()), input);
    let replies = replies(&lines(&output));
    for id in 2..=6 {
        assert_eq!(
            replies[&id]["error"]["code"], INVALID_PARAMS,
            "request {id}"
        );
    }
}

#[test]
fn every_method_that_corrects_offers_a_misspelt_changing_command() {
    let program = "remvoe \"/srv/app/a.txt\"";
    let (_, messages) = serve(&[
        request(
            1,
            "initialize",
            json!({"protocol": "nurt-rpc/1", "capabilities": {}}),
        ),
        request(2, "check", json!({"nurt": "1.0", "program": program})),
        request(3, "run", json!({"nurt": "1.0", "program": program})),
        request(4, "correct", json!({"program": program})),
        request(5, "correct", json!({"program": "lsit \"/srv/app\""})),
        request(6, "shutdown", Value::Null),
    ]);
    let replies = replies(&messages);
    for id in [2, 3, 4] {
        let reply = replies[&id].to_string();
        assert!(reply.contains("E_AMBIGUOUS"), "request {id}: {reply}");
        assert!(
            reply.contains("remove \\\"/srv/app/a.txt\\\""),
            "request {id}: {reply}"
        );
    }
    let reading = replies[&5].to_string();
    assert!(!reading.contains("E_AMBIGUOUS"), "{reading}");
    assert!(reading.contains("list"), "{reading}");
}

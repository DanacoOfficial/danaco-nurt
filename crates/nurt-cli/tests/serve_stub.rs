// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-cli/tests/serve_stub.rs
// Responsibility:   Protocol tests of `nurt serve` answering from the recorded stub cases.
// Dependencies:     nurt_contracts, nurt_ffi, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Protocol tests of `nurt serve` answering from the recorded stub cases.

#![cfg(feature = "stub")]
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
use std::fs;
use std::io::{BufReader, Cursor, PipeWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nurt_cli::serve::{
    self, Framing, INVALID_PARAMS, INVALID_REQUEST, PARSE_ERROR, ServeOptions, application_code,
};
use nurt_contracts::error::AbiStatus;
use nurt_contracts::stage::DeliveredEvent;
use serde_json::{Value, json};

/// Longest wait for one message of a running server.
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

const READ_REQUEST: &str = r#"find "/srv/app" glob: "*.go""#;
const EVENTS_REQUEST: &str = r#"find "/srv/app" glob: "*.rs""#;
const SLOW_REQUEST: &str = r#"find "/srv/archive""#;
const PANIC_REQUEST: &str = r#"find "/srv/panic""#;

fn fixtures() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("tests/fixtures/serve")
}

fn options() -> ServeOptions {
    ServeOptions::stub(&nurt_ffi::stub::cases_dir()).unwrap()
}

fn request(id: impl Into<Value>, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id.into(), "method": method, "params": params})
}

fn initialize(events: bool) -> Value {
    request(
        "init",
        "initialize",
        json!({"protocol": "nurt-rpc/1", "capabilities": {"events": events}}),
    )
}

fn run(id: impl Into<Value>, program: &str) -> Value {
    request(id, "run", json!({"nurt": "1.0", "program": program}))
}

fn ndjson(messages: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for message in messages {
        bytes.extend_from_slice(message.to_string().as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

fn read_ndjson(bytes: &[u8]) -> Vec<Value> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

fn read_content_length(mut bytes: &[u8]) -> Vec<Value> {
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

/// Serves `input` to the end and returns the exit code and the output.
fn serve_all(options: ServeOptions, input: Vec<u8>) -> (ExitCode, Vec<u8>) {
    let mut output = Vec::new();
    let code = serve::run(options, Cursor::new(input), &mut output);
    (code, output)
}

fn is_success(code: ExitCode) -> bool {
    format!("{code:?}") == format!("{:?}", ExitCode::SUCCESS)
}

/// Whether `actual` matches `expected`: objects member by member, arrays
/// and scalars exactly, and the string `"*"` matches anything.
fn matches(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::String(wildcard), _) if wildcard == "*" => true,
        (Value::Object(expected), Value::Object(actual)) => expected.iter().all(|(name, value)| {
            actual
                .get(name)
                .is_some_and(|actual| matches(value, actual))
        }),
        (Value::Array(expected), Value::Array(actual)) => {
            expected.len() == actual.len()
                && expected.iter().zip(actual).all(|(e, a)| matches(e, a))
        }
        _ => expected == actual,
    }
}

fn by_id(messages: Vec<Value>) -> BTreeMap<String, Value> {
    messages
        .into_iter()
        .filter(|message| message.get("method").is_none())
        .map(|message| (message["id"].to_string(), message))
        .collect()
}

fn assert_transcript(replies: Vec<Value>) {
    let expected = read_ndjson(&fs::read(fixtures().join("methods.replies.ndjson")).unwrap());
    let replies = by_id(replies);
    assert_eq!(replies.len(), expected.len(), "replies: {replies:#?}");
    for expected in expected {
        let actual = &replies[&expected["id"].to_string()];
        assert!(
            matches(&expected, actual),
            "expected {expected}\nactual   {actual}"
        );
    }
}

/// Output that forwards each complete line to a channel.
struct LineSink {
    pending: Vec<u8>,
    lines: Sender<Value>,
}

impl Write for LineSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let _ = self.lines.send(serde_json::from_slice(&line).unwrap());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Server running on a pipe, for tests that interleave requests and
/// replies.
struct Running {
    input: Option<PipeWriter>,
    output: Receiver<Value>,
    server: JoinHandle<ExitCode>,
}

impl Running {
    fn start(options: ServeOptions) -> Self {
        let (reader, writer) = std::io::pipe().unwrap();
        let (lines, output) = mpsc::channel();
        let server = thread::spawn(move || {
            let sink = LineSink {
                pending: Vec::new(),
                lines,
            };
            serve::run(options, BufReader::new(reader), sink)
        });
        Self {
            input: Some(writer),
            output,
            server,
        }
    }

    fn send(&mut self, message: &Value) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{message}").unwrap();
    }

    fn next(&self) -> Value {
        self.output.recv_timeout(REPLY_TIMEOUT).unwrap()
    }

    /// Messages up to and including the reply to `id`.
    fn until_reply(&self, id: &Value) -> Vec<Value> {
        let mut messages = Vec::new();
        loop {
            let message = self.next();
            let done = message.get("method").is_none() && message["id"] == *id;
            messages.push(message);
            if done {
                return messages;
            }
        }
    }

    /// Messages up to and including the replies to every one of `ids`, in
    /// whatever order they arrive.
    fn until_replies(&self, ids: &[Value]) -> Vec<Value> {
        let mut messages = Vec::new();
        let mut missing: Vec<&Value> = ids.iter().collect();
        while !missing.is_empty() {
            let message = self.next();
            if message.get("method").is_none() {
                missing.retain(|id| message["id"] != **id);
            }
            messages.push(message);
        }
        messages
    }

    fn reply(&self, id: impl Into<Value>) -> Value {
        let id = id.into();
        self.until_reply(&id).pop().unwrap()
    }

    /// Closes the input and returns the exit code and the remaining output.
    fn close(mut self) -> (ExitCode, Vec<Value>) {
        self.input = None;
        let code = self.server.join().unwrap();
        (code, self.output.try_iter().collect())
    }
}

#[test]
fn every_method_is_answered_from_the_recorded_cases() {
    let input = fs::read(fixtures().join("methods.requests.ndjson")).unwrap();
    let (code, output) = serve_all(options(), input);
    assert!(is_success(code));
    let replies = read_ndjson(&output);
    assert!(replies.iter().all(|reply| reply["jsonrpc"] == "2.0"));
    assert_eq!(
        replies.last().unwrap(),
        &json!({"jsonrpc": "2.0", "id": 20, "result": null}),
        "shutdown is answered last"
    );
    assert_transcript(replies);
}

#[test]
fn content_length_framing_carries_the_same_session() {
    let requests = read_ndjson(&fs::read(fixtures().join("methods.requests.ndjson")).unwrap());
    let mut input = Vec::new();
    for message in &requests {
        let body = message.to_string();
        input.extend_from_slice(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
    }
    let options = options().with_framing(Framing::ContentLength);
    let (code, output) = serve_all(options, input);
    assert!(is_success(code));
    assert_transcript(read_content_length(&output));
}

#[test]
fn published_schemas_are_returned_by_name() {
    let input = ndjson(&[
        initialize(false),
        request(1, "schema", json!({"name": "request"})),
        request(2, "schema", json!({"name": "tool"})),
        request(3, "schema", json!({})),
    ]);
    let replies = by_id(read_ndjson(&serve_all(options(), input).1));
    let request_schema: Value = serde_json::from_str(nurt_contracts::schema::REQUEST).unwrap();
    assert_eq!(replies["1"]["result"], request_schema);
    assert!(replies["2"]["result"].is_object());
    assert_eq!(replies["3"]["error"]["code"], INVALID_PARAMS);
}

#[test]
fn requests_before_initialize_are_rejected() {
    let input = ndjson(&[
        request(1, "info", Value::Null),
        request(2, "cancel", json!({"request_id": 1})),
        request(3, "nothing", Value::Null),
        initialize(false),
        request(4, "info", Value::Null),
    ]);
    let replies = by_id(read_ndjson(&serve_all(options(), input).1));
    assert_eq!(replies["1"]["error"]["code"], INVALID_REQUEST);
    assert!(
        replies["1"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("initialize")
    );
    assert_eq!(replies["2"]["error"]["code"], INVALID_REQUEST);
    assert_eq!(replies["3"]["error"]["code"], -32601);
    assert_eq!(replies["4"]["result"]["features"], json!(["stub"]));
}

#[test]
fn initialize_negotiates_the_major_versions() {
    let abi_error = application_code(AbiStatus::Abi);
    let attempts = [
        (json!({"protocol": "nurt-rpc/2"}), abi_error),
        (json!({"protocol": "lsp/1"}), abi_error),
        (json!({"protocol": "nurt-rpc/1", "ast": "2.0"}), abi_error),
        (
            json!({"protocol": "nurt-rpc/1", "abi": 0x0002_0000}),
            abi_error,
        ),
        (
            json!({"protocol": "nurt-rpc/1", "config_overrides": {"output": {}}}),
            INVALID_PARAMS,
        ),
        (json!({"client": {"name": "x"}}), INVALID_PARAMS),
    ];
    for (params, code) in attempts {
        let input = ndjson(&[request(1, "initialize", params.clone())]);
        let reply = &read_ndjson(&serve_all(options(), input).1)[0];
        assert_eq!(reply["error"]["code"], code, "parameters {params}");
    }
    let input = ndjson(&[
        request(
            1,
            "initialize",
            json!({"protocol": "nurt-rpc/1.3", "ast": "1.0", "abi": 0x0001_0000, "config_overrides": {}, "client": {"name": "x"}, "language": "1.0"}),
        ),
        request(2, "initialize", json!({"protocol": "nurt-rpc/1"})),
    ]);
    let replies = by_id(read_ndjson(&serve_all(options(), input).1));
    assert_eq!(replies["1"]["result"]["protocol"], "nurt-rpc/1");
    assert_eq!(replies["2"]["error"]["code"], INVALID_REQUEST);
}

#[test]
fn events_are_sent_only_after_they_are_requested() {
    let input = ndjson(&[initialize(false), run(1, EVENTS_REQUEST)]);
    let quiet = read_ndjson(&serve_all(options(), input).1);
    assert!(quiet.iter().all(|message| message.get("method").is_none()));

    let input = ndjson(&[initialize(true), run(1, EVENTS_REQUEST)]);
    let messages = read_ndjson(&serve_all(options(), input).1);
    let events: Vec<DeliveredEvent> = messages
        .iter()
        .filter(|message| message["method"] == "nurt/event")
        .map(|message| serde_json::from_value(message["params"].clone()).unwrap())
        .collect();
    let types: Vec<Value> = events
        .iter()
        .map(|event| serde_json::to_value(event).unwrap()["t"].clone())
        .collect();
    assert_eq!(
        types,
        vec![
            json!("exec.started"),
            json!("log"),
            json!("progress"),
            json!("progress"),
            json!("exec.finished")
        ]
    );
    let seqs: Vec<u64> = events.iter().map(|event| event.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
    assert!(
        events
            .iter()
            .all(|event| event.request.as_deref() == Some("1"))
    );
    let reply = messages
        .iter()
        .position(|message| message["id"] == 1)
        .unwrap();
    assert_eq!(reply, messages.len() - 1, "events precede the reply");
}

#[test]
fn replies_follow_completion_and_cancel_ends_a_request() {
    let mut server = Running::start(options());
    server.send(&initialize(true));
    server.reply("init");
    server.send(&run("slow", SLOW_REQUEST));
    let started = server.next();
    assert_eq!(started["params"]["t"], "exec.started");
    server.send(&request("fast", "info", Value::Null));
    let before = server.until_reply(&json!("fast"));
    assert!(before.iter().all(|message| message["id"] != "slow"));

    server.send(&request(3, "cancel", json!({"request_id": "slow"})));
    let messages = server.until_replies(&[json!(3), json!("slow")]);
    let cancel = messages.iter().find(|message| message["id"] == 3).unwrap();
    assert_eq!(cancel["result"], json!({"cancelled": true}));
    let slow = messages
        .iter()
        .find(|message| message["id"] == "slow")
        .unwrap();
    assert_eq!(
        slow["error"]["code"],
        application_code(AbiStatus::Cancelled)
    );
    assert_eq!(slow["error"]["data"]["status"], "NURT_E_CANCELLED");
    assert_eq!(
        slow["error"]["data"]["response"]["report"]["error"]["code"],
        "E_CANCELLED"
    );
    let finished = messages
        .iter()
        .find(|message| message["params"]["t"] == "exec.finished")
        .unwrap();
    assert_eq!(finished["params"]["error"], "E_CANCELLED");

    server.send(&request(4, "cancel", json!({"request_id": "slow"})));
    assert_eq!(server.reply(4)["result"], json!({"cancelled": false}));
    let (code, rest) = server.close();
    assert!(is_success(code));
    assert!(rest.is_empty());
}

#[test]
fn requests_in_flight_are_limited_and_identified() {
    let mut server = Running::start(options().with_max_inflight(1));
    server.send(&initialize(false));
    assert_eq!(server.reply("init")["result"]["limits"]["max_inflight"], 1);
    server.send(&run(1, SLOW_REQUEST));
    server.send(&request(2, "info", Value::Null));
    let rejected = server.reply(2);
    assert_eq!(rejected["error"]["code"], INVALID_REQUEST);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("limit is 1")
    );
    server.send(&request(3, "cancel", json!({"request_id": 1})));
    let messages = by_id(server.until_replies(&[json!(3), json!(1)]));
    assert_eq!(messages["3"]["result"]["cancelled"], true);
    assert_eq!(
        messages["1"]["error"]["code"],
        application_code(AbiStatus::Cancelled)
    );
    server.close();

    let mut server = Running::start(options());
    server.send(&initialize(false));
    server.reply("init");
    server.send(&run(1, SLOW_REQUEST));
    server.send(&run(1, READ_REQUEST));
    let duplicate = server.reply(1);
    assert_eq!(duplicate["error"]["code"], INVALID_REQUEST);
    assert!(
        duplicate["error"]["message"]
            .as_str()
            .unwrap()
            .contains("in flight")
    );
    server.send(&request(2, "cancel", json!({"request_id": 1})));
    server.send(&request(
        3,
        "cancel",
        json!({"request_id": 1, "job_id": "j"}),
    ));
    server.send(&request(4, "cancel", json!({})));
    let messages = by_id(server.until_reply(&json!(4)));
    assert_eq!(messages["3"]["error"]["code"], INVALID_PARAMS);
    assert_eq!(messages["4"]["error"]["code"], INVALID_PARAMS);
    let (code, rest) = server.close();
    assert!(is_success(code));
    let rest = by_id(rest.into_iter().chain(messages.into_values()).collect());
    assert_eq!(rest["2"]["result"]["cancelled"], true);
    assert_eq!(
        rest["1"]["error"]["code"],
        application_code(AbiStatus::Cancelled)
    );
}

#[test]
fn a_panic_in_a_request_is_an_internal_error_and_the_server_continues() {
    let input = ndjson(&[
        initialize(false),
        run(1, PANIC_REQUEST),
        run(2, "count"),
        request(3, "info", Value::Null),
    ]);
    let (code, output) = serve_all(options(), input);
    assert!(is_success(code));
    let replies = by_id(read_ndjson(&output));
    assert_eq!(
        replies["1"]["error"]["code"],
        application_code(AbiStatus::Internal)
    );
    assert_eq!(
        replies["1"]["error"]["data"]["panic"],
        "the stub case raised a panic"
    );
    assert_eq!(
        replies["2"]["error"]["code"],
        application_code(AbiStatus::Internal),
        "a call without a case"
    );
    assert!(replies["2"]["error"]["data"]["key"].is_string());
    assert!(replies["3"]["result"].is_object());
}

#[test]
fn shutdown_waits_for_requests_and_stops_reading() {
    let input = ndjson(&[
        initialize(false),
        run(1, READ_REQUEST),
        request(2, "shutdown", Value::Null),
        request(3, "info", Value::Null),
    ]);
    let (code, output) = serve_all(options(), input);
    assert!(is_success(code));
    let replies = read_ndjson(&output);
    let ids: Vec<Value> = replies.iter().map(|reply| reply["id"].clone()).collect();
    assert_eq!(ids, vec![json!("init"), json!(1), json!(2)]);

    let input = ndjson(&[
        initialize(false),
        json!({"jsonrpc": "2.0", "method": "shutdown"}),
        request(3, "info", Value::Null),
    ]);
    let replies = read_ndjson(&serve_all(options(), input).1);
    assert_eq!(replies.len(), 1, "a shutdown notification has no reply");
}

#[test]
fn end_of_input_ends_the_server_after_the_replies() {
    let input = ndjson(&[initialize(false), run(1, READ_REQUEST)]);
    let (code, output) = serve_all(options(), input);
    assert!(is_success(code));
    let replies = by_id(read_ndjson(&output));
    assert_eq!(replies["1"]["result"]["report"]["status"], "ok");
    let (code, output) = serve_all(options(), Vec::new());
    assert!(is_success(code));
    assert!(output.is_empty());
}

#[test]
fn invalid_messages_get_error_replies_and_reading_continues() {
    let mut input = ndjson(&[initialize(false)]);
    input.extend_from_slice(b"{not json\n");
    input.extend_from_slice(format!("{{\"pad\":\"{}\"}}\n", "x".repeat(200)).as_bytes());
    input.extend_from_slice(b"[1]\n");
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":1}\n");
    input.extend(ndjson(&[request(1, "info", Value::Null)]));
    let options = options().with_max_message_bytes(160);
    let (code, output) = serve_all(options, input);
    assert!(is_success(code));
    let replies = read_ndjson(&output);
    let codes: Vec<Value> = replies[1..4]
        .iter()
        .map(|reply| reply["error"]["code"].clone())
        .collect();
    assert_eq!(
        codes,
        vec![
            json!(PARSE_ERROR),
            json!(INVALID_REQUEST),
            json!(INVALID_REQUEST)
        ]
    );
    assert!(replies[1..4].iter().all(|reply| reply["id"].is_null()));
    assert_eq!(replies[4]["id"], 1);
    assert_eq!(replies.len(), 5, "the reply sent by the client is ignored");
}

#[test]
fn broken_framing_ends_the_server_with_a_failure() {
    let mut input = Vec::new();
    let initialize = initialize(false).to_string();
    input.extend_from_slice(
        format!("Content-Length: {}\r\n\r\n{initialize}", initialize.len()).as_bytes(),
    );
    input.extend_from_slice(b"Content-Length: many\r\n\r\n{}");
    let options = options().with_framing(Framing::ContentLength);
    let (code, output) = serve_all(options, input);
    assert!(!is_success(code));
    let replies = read_content_length(&output);
    assert_eq!(replies[1]["error"]["code"], PARSE_ERROR);
}

#[test]
fn notifications_run_without_replies() {
    let input = ndjson(&[
        initialize(true),
        json!({"jsonrpc": "2.0", "method": "run", "params": {"nurt": "1.0", "program": EVENTS_REQUEST}}),
        json!({"jsonrpc": "2.0", "method": "nothing"}),
        json!({"jsonrpc": "2.0", "method": "parse", "params": {"program": 1}}),
    ]);
    let messages = read_ndjson(&serve_all(options(), input).1);
    let replies: Vec<&Value> = messages
        .iter()
        .filter(|message| message.get("method").is_none())
        .collect();
    assert_eq!(replies.len(), 1);
    let events: Vec<&Value> = messages
        .iter()
        .filter(|message| message["method"] == "nurt/event")
        .collect();
    assert_eq!(events.len(), 5);
    assert!(
        events
            .iter()
            .all(|event| event["params"]["request"].is_null())
    );
}

#[test]
fn a_session_given_in_initialize_is_accepted() {
    let input = ndjson(&[
        request(
            1,
            "initialize",
            json!({"protocol": "nurt-rpc/1", "session": "client-1"}),
        ),
        run(2, READ_REQUEST),
    ]);
    let replies = by_id(read_ndjson(&serve_all(options(), input).1));
    assert_eq!(replies["2"]["result"]["report"]["status"], "ok");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

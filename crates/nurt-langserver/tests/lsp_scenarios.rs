// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Protocol scenarios of `tests/d1/lsp/scenarios` and the end of sessions.
//!
//! A scenario file holds the `initialize` parameters, the expected subset of
//! the `initialize` result and a list of steps: `open`, `change` and `close`
//! send document notifications, `notification` waits for a notification of
//! the server and `request` sends a request and checks its `result` or its
//! `error`. Expected values follow the matcher of `lsp_support`. A scenario
//! with `"catalog": true` binds programs to the catalog of the workspace and
//! takes hover texts from its help.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod lsp_support;

use std::fs;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_support::{Client, assert_matches, catalog, did_open, full_options, workspace};
use nurt_langserver::{Options, ServerError, serve};
use serde_json::{Value, json};

/// Directory of the scenario files, relative to the workspace.
const SCENARIOS: &str = "tests/d1/lsp/scenarios";

fn scenario_files() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(workspace().join(SCENARIOS))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
}

fn run(path: &Path, commands: &[nurt_contracts::spec::StageSpec]) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let scenario: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let initialize = scenario
        .get("initialize")
        .cloned()
        .unwrap_or_else(|| json!({"capabilities": {}}));
    let options = if scenario.get("catalog") == Some(&Value::Bool(true)) {
        full_options()
    } else {
        Options::new(commands.to_vec())
    };
    let (mut client, result) = Client::start(options, initialize);
    if let Some(expected) = scenario.get("capabilities") {
        assert_matches(&result, expected, &format!("{name}: initialize"));
    }
    for (index, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
        step_through(&mut client, step, &format!("{name}: step {index}"));
    }
    client.shutdown().unwrap();
}

fn step_through(client: &mut Client, step: &Value, at: &str) {
    if let Some(open) = step.get("open") {
        let text = match open.get("file") {
            Some(file) => fs::read_to_string(workspace().join(file.as_str().unwrap())).unwrap(),
            None => open["text"].as_str().unwrap().to_owned(),
        };
        let version = open.get("version").and_then(Value::as_i64).unwrap_or(1);
        client.notify(
            "textDocument/didOpen",
            did_open(open["uri"].as_str().unwrap(), &text, version),
        );
    } else if let Some(change) = step.get("change") {
        let changes = change
            .get("contentChanges")
            .cloned()
            .unwrap_or_else(|| json!([{"text": change["text"]}]));
        let params = json!({
            "textDocument": {"uri": change["uri"], "version": change["version"]},
            "contentChanges": changes,
        });
        client.notify("textDocument/didChange", params);
    } else if let Some(uri) = step.get("close") {
        client.notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri}}),
        );
    } else if let Some(method) = step.get("notification") {
        let params = client.notification(method.as_str().unwrap());
        assert_matches(&params, &step["params"], at);
    } else if let Some(method) = step.get("request") {
        let response = client.request(method.as_str().unwrap(), step["params"].clone());
        match (step.get("result"), step.get("error"), response) {
            (Some(expected), _, Ok(result)) => assert_matches(&result, expected, at),
            (_, Some(expected), Err(error)) => assert_matches(&error, expected, at),
            (_, _, response) => panic!("{at}: unexpected response {response:?}"),
        }
    } else {
        panic!("{at}: unknown step {step}");
    }
}

#[test]
fn protocol_scenarios_produce_the_expected_messages() {
    let files = scenario_files();
    assert!(!files.is_empty(), "no scenario files in {SCENARIOS}");
    let commands = catalog();
    for path in &files {
        run(path, &commands);
    }
}

#[test]
fn rejects_invalid_initialize_parameters() {
    let (server, client) = Connection::memory();
    let session = std::thread::spawn(move || serve(server, &Options::default()));
    let params = json!({"capabilities": 7});
    client
        .sender
        .send(Request::new(RequestId::from(1), "initialize".to_owned(), params).into())
        .unwrap();
    let Message::Response(response) = client.receiver.recv().unwrap() else {
        panic!("expected a response");
    };
    assert_eq!(response.response_result.unwrap_err().code, -32602);
    assert!(matches!(
        session.join().unwrap(),
        Err(ServerError::Initialize(_))
    ));
}

#[test]
fn ends_with_an_error_on_exit_without_shutdown() {
    let (client, _) = Client::start(Options::default(), json!({"capabilities": {}}));
    client.notify("exit", Value::Null);
    let ended = client.join();
    assert!(
        matches!(ended, Err(ServerError::ExitWithoutShutdown)),
        "{ended:?}"
    );
}

#[test]
fn ends_with_an_error_when_the_client_disconnects() {
    let (client, _) = Client::start(Options::default(), json!({"capabilities": {}}));
    let ended = client.join();
    assert!(matches!(ended, Err(ServerError::Disconnected)), "{ended:?}");
    assert_eq!(
        ServerError::Disconnected.to_string(),
        "the client closed the connection before shutdown"
    );
}

#[test]
fn ignores_notifications_with_invalid_parameters() {
    let (mut client, _) = Client::start(Options::default(), json!({"capabilities": {}}));
    client.notify("textDocument/didOpen", json!({"textDocument": 3}));
    let log = client.notification("window/logMessage");
    assert_eq!(log["type"], 1);
    assert!(
        log["message"]
            .as_str()
            .unwrap()
            .starts_with("invalid parameters of textDocument/didOpen")
    );
    client.send(Notification::new("$/cancelRequest".to_owned(), json!({"id": 1})).into());
    let response = client.request("textDocument/hover", json!({"position": 1}));
    assert_eq!(response.unwrap_err()["code"], -32602);
    client.shutdown().unwrap();
}

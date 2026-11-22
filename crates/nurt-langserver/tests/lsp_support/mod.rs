// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Client side of in-memory language server sessions and a matcher for
//! expected protocol messages.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Each test file uses a part of the helpers.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use nurt_contracts::spec::{CatalogFile, StageSpec};
use nurt_langserver::{Options, ServerError, serve};
use serde_json::{Value, json};

/// Longest wait for a message of the server.
const TIMEOUT: Duration = Duration::from_secs(20);

/// File name of the catalog index, which lists the catalog files.
const CATALOG_INDEX: &str = "_index.toml";

/// Root of the workspace.
pub(crate) fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Command signatures of the catalog of the workspace.
pub(crate) fn catalog() -> Vec<StageSpec> {
    let mut paths: Vec<PathBuf> = fs::read_dir(workspace().join("catalog"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .filter(|path| path.file_name().is_some_and(|name| name != CATALOG_INDEX))
        .collect();
    paths.sort();
    paths
        .iter()
        .flat_map(|path| {
            let text = fs::read_to_string(path).unwrap();
            let mut file: CatalogFile =
                toml::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            file.resolve_groups();
            file.commands
        })
        .collect()
}

/// Client of a session that runs on another thread.
pub(crate) struct Client {
    connection: Connection,
    session: JoinHandle<Result<(), ServerError>>,
    next_id: i32,
    pending: Vec<Notification>,
}

impl Client {
    /// Starts a session, sends `initialize` and `initialized` and returns the
    /// client with the result of `initialize`.
    pub(crate) fn start(options: Options, initialize: Value) -> (Self, Value) {
        let (server, connection) = Connection::memory();
        let session = thread::spawn(move || serve(server, &options));
        let mut client = Self {
            connection,
            session,
            next_id: 0,
            pending: Vec::new(),
        };
        let result = client
            .request("initialize", initialize)
            .unwrap_or_else(|error| panic!("initialize failed: {error}"));
        client.notify("initialized", json!({}));
        (client, result)
    }

    /// Sends a request and returns its result, or its error object.
    pub(crate) fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.send(Request::new(id.clone(), method.to_owned(), params).into());
        loop {
            match self.receive() {
                Message::Response(response) if response.id == id => {
                    return response
                        .response_result
                        .map_err(|error| serde_json::to_value(error).unwrap());
                }
                Message::Notification(notification) => self.pending.push(notification),
                other => panic!("unexpected message {other:?}"),
            }
        }
    }

    /// Sends a notification.
    pub(crate) fn notify(&self, method: &str, params: Value) {
        self.send(Notification::new(method.to_owned(), params).into());
    }

    /// Returns the parameters of the next notification `method`, keeping
    /// other notifications for later.
    pub(crate) fn notification(&mut self, method: &str) -> Value {
        if let Some(index) = self
            .pending
            .iter()
            .position(|pending| pending.method == method)
        {
            return self.pending.remove(index).params;
        }
        loop {
            match self.receive() {
                Message::Notification(notification) if notification.method == method => {
                    return notification.params;
                }
                Message::Notification(notification) => self.pending.push(notification),
                other => panic!("unexpected message {other:?} while waiting for {method}"),
            }
        }
    }

    /// Sends `shutdown` and `exit` and returns the result of the session.
    pub(crate) fn shutdown(mut self) -> Result<(), ServerError> {
        assert_eq!(self.request("shutdown", Value::Null), Ok(Value::Null));
        self.notify("exit", Value::Null);
        self.session.join().unwrap()
    }

    /// Sends a raw message.
    pub(crate) fn send(&self, message: Message) {
        self.connection.sender.send(message).unwrap();
    }

    /// Receives the next raw message.
    pub(crate) fn receive(&self) -> Message {
        self.connection
            .receiver
            .recv_timeout(TIMEOUT)
            .expect("a message of the server")
    }

    /// Waits for the end of the session without `shutdown`.
    pub(crate) fn join(self) -> Result<(), ServerError> {
        drop(self.connection);
        self.session.join().unwrap()
    }
}

/// Parameters of `textDocument/didOpen`.
pub(crate) fn did_open(uri: &str, text: &str, version: i64) -> Value {
    json!({"textDocument": {"uri": uri, "languageId": "nurt", "version": version, "text": text}})
}

/// Panics when `actual` does not match `expected`.
///
/// An expected object matches an object with at least its keys; an expected
/// array matches an array of the same length element by element. An object
/// whose keys all start with `$` is a set of operators: `$includes` (the
/// text contains a fragment), `$contains` (each listed value matches an
/// element of the array), `$excludes` (no listed value matches an element)
/// and `$length` (the array has the length). Other values must be equal.
pub(crate) fn assert_matches(actual: &Value, expected: &Value, at: &str) {
    if let Some(problem) = mismatch(actual, expected, "") {
        panic!("{at}: {problem}\nactual: {actual:#}");
    }
}

fn mismatch(actual: &Value, expected: &Value, path: &str) -> Option<String> {
    match expected {
        Value::Object(map) if !map.is_empty() && map.keys().all(|key| key.starts_with('$')) => map
            .iter()
            .find_map(|(operator, operand)| apply(operator, operand, actual, path)),
        Value::Object(map) => map.iter().find_map(|(key, value)| match actual.get(key) {
            Some(found) => mismatch(found, value, &format!("{path}.{key}")),
            None => Some(format!("{path}.{key} is missing")),
        }),
        Value::Array(items) => {
            let Some(found) = actual.as_array() else {
                return Some(format!("{path} is not an array"));
            };
            if found.len() != items.len() {
                return Some(format!(
                    "{path} has {} elements, expected {}",
                    found.len(),
                    items.len()
                ));
            }
            found
                .iter()
                .zip(items)
                .enumerate()
                .find_map(|(index, (found, item))| {
                    mismatch(found, item, &format!("{path}[{index}]"))
                })
        }
        _ if actual == expected => None,
        _ => Some(format!("{path} is {actual}, expected {expected}")),
    }
}

fn apply(operator: &str, operand: &Value, actual: &Value, path: &str) -> Option<String> {
    let items = || actual.as_array().cloned().unwrap_or_default();
    match operator {
        "$includes" => {
            let fragment = operand.as_str().unwrap();
            let included = actual.as_str().is_some_and(|text| text.contains(fragment));
            (!included).then(|| format!("{path} does not include {fragment:?}"))
        }
        "$contains" => operand.as_array().unwrap().iter().find_map(|wanted| {
            let found = items()
                .iter()
                .any(|item| mismatch(item, wanted, path).is_none());
            (!found).then(|| format!("{path} has no element matching {wanted}"))
        }),
        "$excludes" => operand.as_array().unwrap().iter().find_map(|unwanted| {
            let found = items()
                .iter()
                .any(|item| mismatch(item, unwanted, path).is_none());
            found.then(|| format!("{path} has an element matching {unwanted}"))
        }),
        "$length" => {
            let length = actual.as_array().map(Vec::len);
            let expected = operand
                .as_u64()
                .and_then(|value| usize::try_from(value).ok());
            (length != expected)
                .then(|| format!("{path} has length {length:?}, expected {operand}"))
        }
        _ => panic!("unknown operator {operator}"),
    }
}

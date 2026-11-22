// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Replays the golden transcripts of `tests/transcripts/`.
//!
//! Every line of a transcript is a JSON object with one member:
//!
//! - `client`: a message sent to the server;
//! - `raw`: text sent to the server as one line, verbatim;
//! - `server`: the next message expected from the server.
//!
//! In an expected message the string `"*"` matches any value; this marks the
//! fields that vary between engines and builds. After the last line the
//! client closes its input and the server must end without further output.

mod common;

use std::path::{Path, PathBuf};

use common::{Client, MockBackend};
use serde_json::Value;

/// Value that matches any value in an expected message.
const ANY: &str = "*";

fn matches(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::String(text), _) if text == ANY => true,
        (Value::Object(expected), Value::Object(actual)) => {
            expected.len() == actual.len()
                && expected.iter().all(|(key, value)| {
                    actual.get(key).is_some_and(|actual| matches(value, actual))
                })
        }
        (Value::Array(expected), Value::Array(actual)) => {
            expected.len() == actual.len()
                && expected
                    .iter()
                    .zip(actual)
                    .all(|(value, actual)| matches(value, actual))
        }
        _ => expected == actual,
    }
}

fn transcripts() -> Vec<PathBuf> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/transcripts");
    let mut files: Vec<PathBuf> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect();
    files.sort();
    files
}

async fn replay(path: &Path) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let mut client = Client::start(MockBackend::new());
    let text = std::fs::read_to_string(path).unwrap();
    for (number, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let step: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("{name}:{}: {error}", number + 1));
        if let Some(message) = step.get("client") {
            client.send(message).await;
        } else if let Some(raw) = step.get("raw").and_then(Value::as_str) {
            client.send_raw(raw).await;
        } else if let Some(expected) = step.get("server") {
            let actual = client.recv().await;
            assert!(
                matches(expected, &actual),
                "{name}:{}: expected {expected}, got {actual}",
                number + 1
            );
        } else {
            panic!("{name}:{}: unknown step {step}", number + 1);
        }
    }
    let (result, rest) = client.close().await;
    assert!(result.is_ok(), "{name}: {result:?}");
    assert!(rest.is_empty(), "{name}: unexpected output {rest:?}");
}

#[tokio::test]
async fn golden_transcripts_replay() {
    let files = transcripts();
    assert!(!files.is_empty());
    for file in files {
        replay(&file).await;
    }
}

#[test]
fn wildcard_matches_any_value_and_nothing_else_is_loose() {
    let expected = serde_json::json!({"a": "*", "b": [1, "*"]});
    assert!(matches(
        &expected,
        &serde_json::json!({"a": {"x": 1}, "b": [1, null]})
    ));
    assert!(!matches(
        &expected,
        &serde_json::json!({"a": 1, "b": [2, null]})
    ));
    assert!(!matches(&expected, &serde_json::json!({"a": 1, "b": [1]})));
    assert!(!matches(
        &expected,
        &serde_json::json!({"a": 1, "b": [1, 2], "c": 3})
    ));
}

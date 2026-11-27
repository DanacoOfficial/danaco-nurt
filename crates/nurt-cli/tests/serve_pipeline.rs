// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Pipelines `find |> filter |> count` and `find |> remove` through the
//! protocol server, on an engine over a virtual file system.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "the helpers of the tests fail the test on unexpected output"
)]

use std::collections::VecDeque;
use std::io::{self, BufRead, Cursor, Read, Write};
use std::process::ExitCode;
use std::sync::{Arc, Condvar, Mutex};

use nurt_cli::serve::{self, ServeOptions};
use nurt_ffi::e2e::{self, COUNT_LARGE, FILES, LIST, REMOVE_ALL, REMOVE_LARGE, World};
use serde_json::{Value, json};

fn message(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn run_request(id: u64, program: &str, preview: bool) -> Value {
    message(
        id,
        "run",
        json!({
            "nurt": "1.0",
            "program": program,
            "options": {"preview": preview, "verbosity": "detailed"}
        }),
    )
}

/// Output of the server, shared with the input that waits for replies.
#[derive(Clone, Default)]
struct Wire(Arc<(Mutex<Vec<u8>>, Condvar)>);

impl Wire {
    fn messages(&self) -> Vec<Value> {
        let bytes = self.0.0.lock().unwrap();
        bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect()
    }

    fn wait_for_reply(&self, id: u64) {
        let (bytes, changed) = &*self.0;
        let mut guard = bytes.lock().unwrap();
        let replied = |bytes: &[u8]| {
            bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
                .any(|message| message.get("method").is_none() && message["id"] == id)
        };
        while !replied(&guard) {
            guard = changed.wait(guard).unwrap();
        }
    }
}

impl Write for Wire {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let (bytes, changed) = &*self.0;
        bytes.lock().unwrap().extend_from_slice(buf);
        changed.notify_all();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Input that hands over each request after the reply to the previous one,
/// so that the requests run one after another.
struct Lockstep {
    wire: Wire,
    pending: VecDeque<(u64, Vec<u8>)>,
    awaited: Option<u64>,
    current: Cursor<Vec<u8>>,
}

impl Read for Lockstep {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(buf.len());
        buf[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for Lockstep {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.current.position() as usize >= self.current.get_ref().len() {
            if let Some(id) = self.awaited.take() {
                self.wire.wait_for_reply(id);
            }
            if let Some((id, line)) = self.pending.pop_front() {
                self.awaited = Some(id);
                self.current = Cursor::new(line);
            }
        }
        self.current.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.current.consume(amount);
    }
}

/// Serves `programs` in order on the engine of `world`, each after the reply
/// to the previous one, and returns the messages written by the server.
fn serve(world: World, programs: &[(&str, bool)]) -> Vec<Value> {
    let mut input = vec![message(
        1,
        "initialize",
        json!({"protocol": "nurt-rpc/1", "capabilities": {"events": true}}),
    )];
    for (index, (program, preview)) in programs.iter().enumerate() {
        input.push(run_request(index as u64 + 2, program, *preview));
    }
    input.push(message(99, "shutdown", Value::Null));
    let wire = Wire::default();
    let lockstep = Lockstep {
        wire: wire.clone(),
        pending: input
            .iter()
            .map(|value| {
                (
                    value["id"].as_u64().unwrap(),
                    format!("{value}\n").into_bytes(),
                )
            })
            .collect(),
        awaited: None,
        current: Cursor::new(Vec::new()),
    };
    let code = serve::run(ServeOptions::engine(world.engine), lockstep, wire.clone());
    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    wire.messages()
}

fn report(messages: &[Value], id: u64) -> &Value {
    let reply = messages
        .iter()
        .find(|message| message.get("method").is_none() && message["id"] == id)
        .unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    &reply["result"]["report"]
}

fn deletions(report: &Value) -> usize {
    report["changes"].as_array().map_or(0, |changes| {
        changes
            .iter()
            .filter(|change| change["action"] == "delete")
            .count()
    })
}

fn paths(report: &Value) -> Vec<&str> {
    report["records"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["fields"]["path"]["v"].as_str().unwrap())
        .collect()
}

fn counted(report: &Value) -> Vec<i64> {
    report["records"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["fields"]["value"]["v"].as_i64().unwrap())
        .collect()
}

fn world() -> World {
    e2e::world().unwrap()
}

#[test]
fn counts_the_entries_that_pass_the_filter() {
    let messages = serve(world(), &[(COUNT_LARGE, false)]);
    let report = report(&messages, 2);
    assert_eq!(report["status"], "ok", "{report}");
    assert_eq!(counted(report), [2]);
    assert_eq!(deletions(report), 0);
}

#[test]
fn preview_plans_the_removal_and_keeps_the_files() {
    let world = world();
    let fs = world.fs.clone();
    let journal = world.journal.clone();
    let messages = serve(world, &[(REMOVE_ALL, true), (LIST, false)]);
    let planned = report(&messages, 2);
    assert_eq!(planned["status"], "planned", "{planned}");
    assert_eq!(deletions(planned), FILES.len());
    assert!(planned.get("op").is_none());
    assert_eq!(paths(report(&messages, 3)).len(), FILES.len());
    assert!(journal.committed().is_empty());
    for (path, _) in FILES {
        assert!(
            fs.exists(&nurt_testkit::vfs::test_path(path).unwrap()),
            "{path}"
        );
    }
}

#[test]
fn removal_deletes_the_found_entries_and_reports_the_changes() {
    let world = world();
    let fs = world.fs.clone();
    let journal = world.journal.clone();
    let messages = serve(
        world,
        &[(REMOVE_LARGE, false), (LIST, false), (COUNT_LARGE, false)],
    );
    let removed = report(&messages, 2);
    assert_eq!(removed["status"], "ok", "{removed}");
    assert_eq!(deletions(removed), 2);
    assert!(removed.get("op").is_some());
    assert_eq!(
        paths(report(&messages, 3)),
        [nurt_testkit::paths::abs("/data/a.log")]
    );
    assert_eq!(counted(report(&messages, 4)), [0]);
    assert_eq!(journal.committed().len(), 1);
    assert!(!fs.exists(&nurt_testkit::vfs::test_path("/data/b.log").unwrap()));
}

#[test]
fn every_run_is_bracketed_by_its_events() {
    let messages = serve(world(), &[(COUNT_LARGE, false), (REMOVE_ALL, true)]);
    for (id, request) in [(2, "2"), (3, "3")] {
        let types: Vec<&str> = messages
            .iter()
            .filter(|message| message["method"] == "nurt/event")
            .filter(|message| message["params"]["request"] == request)
            .map(|message| message["params"]["t"].as_str().unwrap())
            .collect();
        assert_eq!(types.first(), Some(&"exec.started"), "request {id}");
        assert_eq!(types.last(), Some(&"exec.finished"), "request {id}");
    }
}

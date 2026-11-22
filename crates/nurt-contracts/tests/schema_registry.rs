// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests that `nurt_contracts::schema` publishes every file of `spec/schemas/`
//! under its naming convention, that every schema compiles with the other
//! published schemas as resources, and that the schemas of catalog query
//! results, file tree snapshots, responses, check results, protocol messages
//! and events accept and reject the expected documents.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::schema::{self, ID_BASE};
use serde_json::{Value, json};

const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

fn directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/schemas")
}

fn file_name(name: &str) -> String {
    format!("nurt-{name}-1.schema.json")
}

fn parse(content: &str) -> Value {
    serde_json::from_str(content).unwrap()
}

/// Validator for `document`, with every published schema registered under
/// its `$id`.
fn validator_for(document: &Value) -> jsonschema::Validator {
    let registry = schema::ALL
        .iter()
        .fold(jsonschema::Registry::new(), |builder, (other, content)| {
            builder
                .add(format!("{ID_BASE}{}", file_name(other)), parse(content))
                .unwrap()
        })
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(document)
        .unwrap_or_else(|error| panic!("schema {document} does not compile: {error}"))
}

/// Validator for the published schema `name`.
fn validator(name: &str) -> jsonschema::Validator {
    validator_for(&parse(schema::by_name(name).unwrap()))
}

#[test]
fn every_schema_file_is_published() {
    let files: BTreeSet<String> = fs::read_dir(directory())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".schema.json"))
        .collect();
    let published: BTreeSet<String> = schema::ALL
        .iter()
        .map(|(name, _)| file_name(name))
        .collect();
    assert_eq!(files, published, "files of spec/schemas and schema::ALL");
    for (name, content) in schema::ALL {
        let on_disk = fs::read_to_string(directory().join(file_name(name))).unwrap();
        assert_eq!(parse(&on_disk), parse(content), "embedded schema {name}");
        assert_eq!(schema::by_name(name), Some(*content));
    }
}

#[test]
fn every_schema_has_draft_and_stable_id() {
    for (name, content) in schema::ALL {
        let value = parse(content);
        assert_eq!(value["$schema"], DRAFT, "$schema of {name}");
        assert_eq!(
            value["$id"],
            format!("{ID_BASE}{}", file_name(name)),
            "$id of {name}"
        );
    }
}

#[test]
fn every_schema_compiles() {
    for (name, _) in schema::ALL {
        validator(name);
    }
}

fn revision() -> String {
    format!("sha256:{}", "0123456789abcdef".repeat(4))
}

fn count_command() -> Value {
    json!({
        "name": "count",
        "category": "transform",
        "reversibility": "read",
        "effect": "read",
        "implementation": "builtin",
        "input": ["any"],
        "output": ["int"],
        "summary": "Count the records."
    })
}

fn core_group() -> Value {
    json!({ "name": "core", "summary": "Core commands.", "origin": "core", "commands": 1 })
}

#[test]
fn catalog_result_accepts_each_form() {
    let validator = validator("catalog-result");
    let documents = [
        json!({
            "format": "nurt-catalog-result/1",
            "catalog_rev": revision(),
            "total": 1,
            "groups": [core_group()]
        }),
        json!({
            "format": "nurt-catalog-result/1",
            "catalog_rev": revision(),
            "group": core_group(),
            "commands": [{
                "name": "count",
                "summary": "Count the records.",
                "category": "transform",
                "effect": "read",
                "reversibility": "read",
                "implementation": "builtin"
            }]
        }),
        json!({
            "format": "nurt-catalog-result/1",
            "catalog_rev": revision(),
            "command": count_command()
        }),
    ];
    for document in &documents {
        assert!(validator.is_valid(document), "rejected {document}");
    }
}

#[test]
fn catalog_result_rejects_invalid_documents() {
    let validator = validator("catalog-result");
    let mut unknown_origin = core_group();
    unknown_origin["origin"] = json!("vendor");
    let mut command_without_output = count_command();
    command_without_output
        .as_object_mut()
        .unwrap()
        .remove("output");
    let documents = [
        json!({ "format": "nurt-catalog-result/1", "total": 0, "groups": [] }),
        json!({ "format": "nurt-catalog-result/1", "catalog_rev": "sha256:00", "total": 0, "groups": [] }),
        json!({ "format": "nurt-catalog-result/2", "catalog_rev": revision(), "total": 0, "groups": [] }),
        json!({ "format": "nurt-catalog-result/1", "catalog_rev": revision(), "total": 1, "groups": [unknown_origin] }),
        json!({ "format": "nurt-catalog-result/1", "catalog_rev": revision(), "command": command_without_output }),
        json!({
            "format": "nurt-catalog-result/1",
            "catalog_rev": revision(),
            "total": 0,
            "groups": [],
            "command": count_command()
        }),
    ];
    for document in &documents {
        assert!(!validator.is_valid(document), "accepted {document}");
    }
}

fn snapshot(entries: Value) -> Value {
    json!({ "format": "nurt-fs-snapshot/1", "entries": entries })
}

#[test]
fn fs_snapshot_accepts_files_directories_and_links() {
    let validator = validator("fs-snapshot");
    let document = snapshot(json!([
        { "path": "src", "type": "dir", "mode": "0755" },
        { "path": "src/main.txt", "type": "file", "content_utf8": "text\n", "mode": "0644" },
        { "path": "data.bin", "type": "file", "content_base64": "AAEC", "mtime": "2026-01-01T00:00:00Z" },
        { "path": "link", "type": "symlink", "target": "src/main.txt" }
    ]));
    assert!(validator.is_valid(&document), "rejected {document}");
    assert!(validator.is_valid(&snapshot(json!([]))));
}

#[test]
fn fs_snapshot_rejects_invalid_entries() {
    let validator = validator("fs-snapshot");
    let entries = [
        json!({ "path": "/absolute", "type": "dir" }),
        json!({ "path": "a//b", "type": "dir" }),
        json!({ "path": "a/../b", "type": "dir" }),
        json!({ "path": "./a", "type": "dir" }),
        json!({ "path": "a/", "type": "dir" }),
        json!({ "path": "", "type": "dir" }),
        json!({ "path": "a", "type": "fifo" }),
        json!({ "path": "a", "type": "file", "content_utf8": "x", "content_base64": "eA==" }),
        json!({ "path": "a", "type": "file", "target": "b" }),
        json!({ "path": "a", "type": "dir", "content_utf8": "x" }),
        json!({ "path": "a", "type": "symlink" }),
        json!({ "path": "a", "type": "symlink", "target": "b", "content_utf8": "x" }),
        json!({ "path": "a", "type": "file", "mode": "644" }),
        json!({ "path": "a", "type": "file", "size": 1 }),
    ];
    for entry in entries {
        let document = snapshot(json!([entry]));
        assert!(!validator.is_valid(&document), "accepted {document}");
    }
    assert!(!validator.is_valid(&json!({ "format": "nurt-fs-snapshot/1" })));
}

#[test]
fn response_schema_equals_the_response_definition_of_the_request_schema() {
    let request = parse(schema::REQUEST);
    let mut response = parse(schema::RESPONSE);
    let object = response.as_object_mut().unwrap();
    object.remove("$schema");
    object.remove("$id");
    let definitions = object.remove("$defs").unwrap();
    assert_eq!(
        definitions,
        json!({ "rendered": request["$defs"]["rendered"] }),
        "definitions of nurt-response/1"
    );
    assert_eq!(response, request["$defs"]["response"]);
}

#[test]
fn golden_responses_match_the_response_schema() {
    let validator = validator("response");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/golden/request/response");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let document = parse(&fs::read_to_string(&path).unwrap());
            assert!(validator.is_valid(&document), "{}", path.display());
            checked += 1;
        }
    }
    assert!(checked > 0, "no golden responses in {}", dir.display());
    assert!(!validator.is_valid(&json!({ "nurt": "1.0" })));
    assert!(!validator.is_valid(&json!({
        "nurt": "1.0",
        "report": { "nurt": "1.0", "status": "ok", "kind": "read" },
        "rendered": { "format": "json", "text": "" }
    })));
}

fn check_summary() -> Value {
    json!({
        "format": "nurt-check/1",
        "effect": "read",
        "reversibility": "read",
        "operations": 1,
        "waits": false,
        "paths": ["/srv/app"],
        "network": [],
        "programs": [],
        "groups": ["core"],
        "corrections": [],
        "details": []
    })
}

#[test]
fn check_schema_accepts_a_summary_and_rejects_invalid_ones() {
    let validator = validator("check");
    let summary = check_summary();
    assert!(validator.is_valid(&summary), "rejected {summary}");
    let mut without_effect = check_summary();
    without_effect.as_object_mut().unwrap().remove("effect");
    let mut unknown_effect = check_summary();
    unknown_effect["effect"] = json!("write");
    let mut negative_operations = check_summary();
    negative_operations["operations"] = json!(-1);
    for document in [without_effect, unknown_effect, negative_operations] {
        assert!(!validator.is_valid(&document), "accepted {document}");
    }
}

/// Delivered events in their JSON form, one of every event type.
fn delivered_events() -> Vec<Value> {
    let common = json!({ "seq": 1, "time": "2026-01-01T00:00:00Z", "request": "r1" });
    let bodies = [
        json!({ "t": "exec.started", "ast_hash": null, "effect": "read" }),
        json!({ "t": "exec.finished", "status": "ok", "error": null }),
        json!({ "t": "progress", "command": "find", "phase": "scan", "done": 10, "total": null, "bytes": null, "message": null }),
        json!({ "t": "job.started", "id": "j1", "label": null, "command": ["make"], "cwd": "/srv/app", "pid": 42, "started": "2026-01-01T00:00:00Z" }),
        json!({ "t": "job.output", "id": "j1", "stream": "stdout", "cursor": "c1", "lines": ["ok"], "bytes": 3 }),
        json!({ "t": "job.progress", "id": "j1", "percent": 50.0, "line": null }),
        json!({ "t": "job.finished", "id": "j1", "state": "exited", "exit_code": 0, "signal": null, "elapsed": 1500, "output_file": null, "changes": 0 }),
        json!({ "t": "log", "level": "warn", "message": "slow volume", "source": null }),
    ];
    bodies
        .into_iter()
        .map(|body| {
            let mut event = common.clone();
            event
                .as_object_mut()
                .unwrap()
                .extend(body.as_object().unwrap().clone());
            event
        })
        .collect()
}

#[test]
fn event_schema_accepts_every_event_of_the_contract() {
    let validator = validator("event");
    for event in delivered_events() {
        let typed: nurt_contracts::stage::DeliveredEvent = serde_json::from_value(event.clone())
            .unwrap_or_else(|error| panic!("{event}: {error}"));
        let serialized = serde_json::to_value(&typed).unwrap();
        assert!(validator.is_valid(&serialized), "rejected {serialized}");
    }
}

#[test]
fn event_schema_rejects_invalid_events() {
    let validator = validator("event");
    let mut without_seq = delivered_events().remove(0);
    without_seq.as_object_mut().unwrap().remove("seq");
    let mut unknown_type = delivered_events().remove(0);
    unknown_type["t"] = json!("exec.paused");
    let mut finished_without_status = delivered_events().remove(1);
    finished_without_status
        .as_object_mut()
        .unwrap()
        .remove("status");
    for document in [without_seq, unknown_type, finished_without_status] {
        assert!(!validator.is_valid(&document), "accepted {document}");
    }
}

#[test]
fn rpc_schema_accepts_requests_replies_and_events() {
    let validator = validator("rpc");
    let mut notification = json!({ "jsonrpc": "2.0", "method": "nurt/event" });
    notification["params"] = delivered_events().remove(0);
    let documents = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocol": "nurt-rpc/1", "capabilities": { "events": true } }
        }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "run", "params": { "nurt": "1.0", "program": "list \"/srv\"" } }),
        json!({ "jsonrpc": "2.0", "id": "3", "method": "cancel", "params": { "request_id": 2 } }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "schema", "params": { "name": "fs-snapshot" } }),
        json!({ "jsonrpc": "2.0", "id": 2, "result": { "nurt": "1.0", "report": { "nurt": "1.0", "status": "ok", "kind": "list" } } }),
        json!({ "jsonrpc": "2.0", "id": 5, "error": { "code": -32601, "message": "unknown method" } }),
        notification,
    ];
    for document in &documents {
        assert!(validator.is_valid(document), "rejected {document}");
    }
}

#[test]
fn rpc_schema_rejects_invalid_messages() {
    let validator = validator("rpc");
    let documents = [
        json!({ "jsonrpc": "1.0", "id": 1, "method": "run", "params": { "nurt": "1.0", "program": "list" } }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "evaluate" }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "run" }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "cancel", "params": { "request_id": 1, "job_id": "j1" } }),
        json!({ "jsonrpc": "2.0", "id": 1, "result": {}, "error": { "code": -32603, "message": "internal" } }),
        json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -31000, "message": "out of range" } }),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "schema", "params": { "name": "unknown" } }),
    ];
    for document in &documents {
        assert!(!validator.is_valid(document), "accepted {document}");
    }
}

#[test]
fn rpc_check_result_is_a_check_summary_with_diagnostics() {
    let validator = validator_for(&json!({
        "$ref": format!("{ID_BASE}{}#/$defs/results/properties/check", file_name("rpc"))
    }));
    let mut result = check_summary();
    result["diagnostics"] = json!([]);
    assert!(validator.is_valid(&result), "rejected {result}");
    let only_diagnostics = json!({ "diagnostics": [] });
    for document in [check_summary(), only_diagnostics] {
        assert!(!validator.is_valid(&document), "accepted {document}");
    }
}

#[test]
fn rpc_schema_names_every_published_schema() {
    let rpc = parse(schema::RPC);
    let named: BTreeSet<&str> = rpc["$defs"]["schema_params"]["properties"]["name"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    let published: BTreeSet<&str> = schema::ALL.iter().map(|(name, _)| *name).collect();
    assert_eq!(named, published);
}

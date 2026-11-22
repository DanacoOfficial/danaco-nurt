// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests that `nurt_contracts::schema` publishes every file of `spec/schemas/`
//! under its naming convention, that every schema compiles with the other
//! published schemas as resources, and that the schemas of catalog query
//! results and file tree snapshots accept and reject the expected documents.

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

/// Validator for the published schema `name`, with every published schema
/// registered under its `$id`.
fn validator(name: &str) -> jsonschema::Validator {
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
        .build(&parse(schema::by_name(name).unwrap()))
        .unwrap_or_else(|error| panic!("schema {name} does not compile: {error}"))
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-tooldef/tests/docs_examples.rs
// Responsibility:   Every JSON example of the integrator guide matches its published schema.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Every JSON example of the integrator guide matches its published schema.
//!
//! A `json` block is preceded by a comment `<!-- schema: NAME -->` or
//! `<!-- schema: NAME#/POINTER -->`. `NAME` is a schema of
//! `nurt_contracts::schema`, or `arguments` for the arguments of a tool call,
//! which must turn into a valid request.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::schema::{self, ID_BASE};
use nurt_tooldef::{ToolFormat, request_from_arguments};
use serde_json::{Value, json};

const MARKER: &str = "<!-- schema: ";
const FENCE: &str = "```";
const ARGUMENTS: &str = "arguments";

struct Example {
    file: String,
    line: usize,
    schema: Option<String>,
    body: String,
}

fn guide_dir() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../docs/integrators")
}

fn file_name(name: &str) -> String {
    format!("nurt-{name}-1.schema.json")
}

fn examples() -> Vec<Example> {
    let mut found = Vec::new();
    let mut files: Vec<PathBuf> = fs::read_dir(guide_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    files.sort();
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let lines: Vec<&str> = text.lines().collect();
        let mut index = 0;
        while index < lines.len() {
            if lines[index].trim() == format!("{FENCE}json") {
                let schema = index
                    .checked_sub(1)
                    .and_then(|previous| lines[previous].trim().strip_prefix(MARKER))
                    .and_then(|rest| rest.strip_suffix(" -->"))
                    .map(str::to_owned);
                let start = index + 1;
                let length = lines[start..]
                    .iter()
                    .position(|line| line.trim() == FENCE)
                    .expect("unterminated code block");
                found.push(Example {
                    file: name.clone(),
                    line: index + 1,
                    schema,
                    body: lines[start..start + length].join("\n"),
                });
                index = start + length;
            }
            index += 1;
        }
    }
    found
}

fn registry() -> jsonschema::Registry<'static> {
    schema::ALL
        .iter()
        .fold(jsonschema::Registry::new(), |builder, (name, content)| {
            let document: Value = serde_json::from_str(content).unwrap();
            builder
                .add(format!("{ID_BASE}{}", file_name(name)), document)
                .unwrap()
        })
        .prepare()
        .unwrap()
}

fn validator(registry: &jsonschema::Registry<'static>, reference: &str) -> jsonschema::Validator {
    let (name, pointer) = reference.split_once('#').unwrap_or((reference, ""));
    let target = format!("{ID_BASE}{}#{pointer}", file_name(name));
    jsonschema::options()
        .with_registry(registry)
        .build(&json!({"$ref": target}))
        .unwrap_or_else(|error| panic!("{reference}: {error}"))
}

fn assert_valid(registry: &jsonschema::Registry<'static>, reference: &str, instance: &Value) {
    let errors: Vec<String> = validator(registry, reference)
        .iter_errors(instance)
        .map(|error| format!("{error} at {}", error.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{reference}: {}", errors.join("; "));
}

#[test]
fn guide_contains_examples() {
    assert!(examples().len() >= 12);
}

#[test]
fn every_json_example_names_a_schema() {
    let unmarked: Vec<String> = examples()
        .iter()
        .filter(|example| example.schema.is_none())
        .map(|example| format!("{}:{}", example.file, example.line))
        .collect();
    assert!(unmarked.is_empty(), "no schema marker: {unmarked:?}");
}

#[test]
fn every_json_example_matches_its_schema() {
    let registry = registry();
    for example in examples() {
        let reference = example.schema.as_deref().unwrap();
        let place = format!("{}:{}", example.file, example.line);
        let instance: Value = serde_json::from_str(&example.body)
            .unwrap_or_else(|error| panic!("{place}: not JSON: {error}"));
        if reference == ARGUMENTS {
            request_from_arguments(ToolFormat::JsonSchema, &instance)
                .unwrap_or_else(|error| panic!("{place}: {error}"));
            continue;
        }
        assert_valid(&registry, reference, &instance);
        if reference == "response" {
            assert_valid(&registry, "report", &instance["report"]);
        }
    }
}

#[test]
fn rejects_an_example_that_does_not_match() {
    let registry = registry();
    let validator = validator(&registry, "event");
    assert!(!validator.is_valid(&json!({"seq": 0, "time": "2026-11-22T10:15:09Z", "t": "log"})));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

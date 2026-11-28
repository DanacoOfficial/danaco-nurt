// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-langserver/tests/lsp_golden.rs
// Responsibility:   Sessions over every program file of the golden sets in three variants.
// Dependencies:     lsp_support, nurt_contracts, nurt_syntax, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Sessions over every program file of the golden sets in three variants.
//!
//! For each file the server publishes no diagnostics, formats the text into
//! the result of the formatter, describes every catalog command at the head
//! of a stage on hover, offers it when its name is completed and classifies
//! it as a function among the semantic tokens.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod lsp_support;

use std::fs;
use std::path::{Path, PathBuf};

use lsp_support::{Client, assert_matches, catalog, did_open, workspace};
use nurt_contracts::ast::{AstDocument, Head, Pipeline, Stage, Statement};
use nurt_langserver::Options;
use nurt_syntax::fmt;
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use serde_json::{Value, json};

/// Golden program files of the specification, named `<name>.nurt`,
/// `<name>.b.nurt` and `<name>.c.nurt`.
const SPEC_GOLDEN: &str = "spec/golden/ast";

/// Golden program files of the language, one directory per variant.
const LANGUAGE_GOLDEN: &str = "tests/d1/golden/ast";

/// Smallest number of golden program files expected in each variant.
const MIN_FILES: usize = 10;

fn program_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "nurt")
        })
        .collect();
    paths.sort();
    paths
}

fn variant_of_name(path: &Path) -> Variant {
    let name = path.file_name().unwrap().to_string_lossy();
    if name.ends_with(".b.nurt") {
        Variant::B
    } else if name.ends_with(".c.nurt") {
        Variant::C
    } else {
        Variant::A
    }
}

fn golden_files(variant: Variant) -> Vec<PathBuf> {
    let directory = match variant {
        Variant::A => "a",
        Variant::B => "b",
        Variant::C => "c",
    };
    let mut files: Vec<PathBuf> = program_files(&workspace().join(SPEC_GOLDEN))
        .into_iter()
        .filter(|path| variant_of_name(path) == variant)
        .collect();
    files.extend(program_files(
        &workspace().join(LANGUAGE_GOLDEN).join(directory),
    ));
    files
}

fn initialize(variant: Variant) -> Value {
    let name = match variant {
        Variant::A => "a",
        Variant::B => "b",
        Variant::C => "c",
    };
    json!({
        "capabilities": {"general": {"positionEncodings": ["utf-32"]}},
        "initializationOptions": {"syntax": {"variant": name}},
    })
}

/// Stages of the top-level pipelines of a document.
fn stages(document: &AstDocument) -> Vec<&Stage> {
    let pipelines = document
        .program
        .statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Let(binding) => Some(&binding.value),
            Statement::Pipeline(pipeline) => Some(pipeline),
            _ => None,
        });
    pipelines
        .flat_map(|pipeline: &Pipeline| {
            let head = match &pipeline.head {
                Head::Stage(stage) => Some(stage),
                _ => None,
            };
            head.into_iter().chain(&pipeline.stages)
        })
        .collect()
}

/// Column (from 0, in Unicode scalar values) of the first `name` on line
/// `line` at or after column `from`.
fn name_column(text: &str, line: u32, from: u32, name: &str) -> u32 {
    let content = text
        .split('\n')
        .nth(usize::try_from(line).unwrap())
        .unwrap();
    let skipped = usize::try_from(from).unwrap();
    let rest: String = content.chars().skip(skipped).collect();
    let index = rest
        .find(name)
        .unwrap_or_else(|| panic!("no {name} on line {line}"));
    from + u32::try_from(rest[..index].chars().count()).unwrap()
}

/// Starts (line and character) of the tokens of type `function`, the first
/// type of the legend, in a semantic tokens result.
fn function_starts(result: &Value) -> Vec<(u32, u32)> {
    let data: Vec<u32> = result["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| u32::try_from(value.as_u64().unwrap()).unwrap())
        .collect();
    let mut starts = Vec::new();
    let (mut line, mut character) = (0, 0);
    for token in data.chunks(5) {
        if token[0] > 0 {
            character = 0;
        }
        line += token[0];
        character += token[1];
        if token[3] == 0 {
            starts.push((line, character));
        }
    }
    starts
}

fn formatted(text: &str, result: &Value) -> String {
    match result.as_array().unwrap().as_slice() {
        [] => text.to_owned(),
        [edit] => {
            assert_eq!(edit["range"]["start"], json!({"line": 0, "character": 0}));
            edit["newText"].as_str().unwrap().to_owned()
        }
        edits => panic!("expected at most one edit: {edits:?}"),
    }
}

/// Checks hover and completion at the command name of every stage and
/// returns the number of stages checked.
///
/// The span of a stage after `|>` starts at the operator, so the name is
/// searched from the start of the span.
fn check_commands(
    client: &mut Client,
    uri: &str,
    text: &str,
    document: &AstDocument,
    names: &[String],
    at: &str,
) -> usize {
    let tokens = client
        .request(
            "textDocument/semanticTokens/full",
            json!({"textDocument": {"uri": uri}}),
        )
        .unwrap();
    let functions = function_starts(&tokens);
    let mut checked = 0;
    for stage in stages(document) {
        if !names.contains(&stage.name) {
            continue;
        }
        let span = stage.span.unwrap();
        let line = span.line - 1;
        let start = name_column(text, line, span.column - 1, &stage.name);
        let end = start + u32::try_from(stage.name.chars().count()).unwrap();
        assert!(
            functions.contains(&(line, start)),
            "{at}: {} is not a function token",
            stage.name
        );
        let position = |character: u32| json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}});
        let hover = client
            .request("textDocument/hover", position(start))
            .unwrap();
        let expected = json!({
            "contents": {"kind": "markdown", "value": {"$includes": format!("```nurt\n{}", stage.name)}},
            "range": {"start": {"line": line, "character": start}, "end": {"line": line, "character": end}},
        });
        assert_matches(&hover, &expected, &format!("{at}: hover on {}", stage.name));
        let items = client
            .request("textDocument/completion", position(end))
            .unwrap();
        let expected = json!({"$contains": [{"label": stage.name, "kind": 3}]});
        assert_matches(
            &items,
            &expected,
            &format!("{at}: completion of {}", stage.name),
        );
        checked += 1;
    }
    checked
}

fn check_variant(variant: Variant) {
    let files = golden_files(variant);
    assert!(
        files.len() >= MIN_FILES,
        "{variant:?}: {} golden files",
        files.len()
    );
    let commands = catalog();
    let names: Vec<String> = commands.iter().map(|spec| spec.name.clone()).collect();
    let (mut client, _) = Client::start(Options::new(commands), initialize(variant));
    let mut checked = 0;
    for path in &files {
        let relative = path
            .strip_prefix(workspace())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let uri = format!("file:///workspace/{relative}");
        let text = fs::read_to_string(path).unwrap();
        client.notify("textDocument/didOpen", did_open(&uri, &text, 1));
        let published = client.notification("textDocument/publishDiagnostics");
        assert_matches(
            &published,
            &json!({"uri": uri, "version": 1, "diagnostics": []}),
            &relative,
        );

        let params =
            json!({"textDocument": {"uri": uri}, "options": {"tabSize": 4, "insertSpaces": true}});
        let response = client.request("textDocument/formatting", params);
        match (fmt::format(&text, variant), response) {
            (Ok(expected), Ok(result)) => {
                assert_eq!(formatted(&text, &result), expected, "{relative}")
            }
            (Err(_), Err(error)) => assert_eq!(error["code"], -32803, "{relative}"),
            (expected, response) => {
                panic!("{relative}: formatter {expected:?}, server {response:?}")
            }
        }

        let document = parse(&text, variant, &ParseOptions::default()).unwrap();
        checked += check_commands(&mut client, &uri, &text, &document, &names, &relative);

        client.notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri}}),
        );
        let closed = client.notification("textDocument/publishDiagnostics");
        assert_matches(&closed, &json!({"uri": uri, "diagnostics": []}), &relative);
    }
    assert!(
        checked >= files.len(),
        "{variant:?}: only {checked} stages checked"
    );
    client.shutdown().unwrap();
}

#[test]
fn golden_programs_of_variant_a_are_served_without_diagnostics() {
    check_variant(Variant::A);
}

#[test]
fn golden_programs_of_variant_b_are_served_without_diagnostics() {
    check_variant(Variant::B);
}

#[test]
fn golden_programs_of_variant_c_are_served_without_diagnostics() {
    check_variant(Variant::C);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-langserver/tests/lsp_correction.rs
// Responsibility:   Sessions that bind programs to the catalog of the workspace: diagnostics of golden programs, fields of input records and help on hover.
// Dependencies:     lsp_support, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Sessions that bind programs to the catalog of the workspace: diagnostics
//! of golden programs, fields of input records and help on hover.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod lsp_support;

use std::fs;
use std::path::{Path, PathBuf};

use lsp_support::{Client, assert_matches, did_open, full_options, workspace};
use serde_json::{Value, json};

/// URI of the document of the single-document sessions.
const URI: &str = "file:///work/program.nurt";

/// Golden program files of the specification, named `<name>.nurt` in
/// variant A.
const SPEC_GOLDEN: &str = "spec/golden/ast";

/// Golden program files of the language in variant A.
const LANGUAGE_GOLDEN: &str = "tests/d1/golden/ast/a";

/// Number of values of one encoded semantic token.
const TOKEN_FIELDS: usize = 5;

fn variant_a_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            name.ends_with(".nurt") && !name.ends_with(".b.nurt") && !name.ends_with(".c.nurt")
        })
        .collect();
    paths.sort();
    paths
}

fn open(client: &mut Client, text: &str) -> Value {
    client.notify("textDocument/didOpen", did_open(URI, text, 1));
    client.notification("textDocument/publishDiagnostics")
}

fn at(line: u32, character: usize) -> Value {
    json!({"textDocument": {"uri": URI}, "position": {"line": line, "character": character}})
}

#[test]
fn golden_programs_of_variant_a_are_bound_and_classified() {
    let mut files = variant_a_files(&workspace().join(SPEC_GOLDEN));
    files.extend(variant_a_files(&workspace().join(LANGUAGE_GOLDEN)));
    assert!(!files.is_empty());
    let (mut client, _) = Client::start(full_options(), json!({"capabilities": {}}));
    let mut clean = 0;
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
        let diagnostics = published["diagnostics"].as_array().unwrap();
        for diagnostic in diagnostics {
            assert_eq!(diagnostic["source"], "nurt", "{relative}");
            assert!(diagnostic["code"].is_string(), "{relative}: {diagnostic}");
        }
        if diagnostics.is_empty() {
            clean += 1;
        }
        let tokens = client
            .request(
                "textDocument/semanticTokens/full",
                json!({"textDocument": {"uri": uri}}),
            )
            .unwrap();
        let data = tokens["data"].as_array().unwrap();
        assert_eq!(data.len() % TOKEN_FIELDS, 0, "{relative}");
        assert!(!data.is_empty(), "{relative}");
        client.notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri}}),
        );
        client.notification("textDocument/publishDiagnostics");
    }
    assert!(clean > 0, "no golden program binds without diagnostics");
    client.shutdown().unwrap();
}

#[test]
fn completes_fields_of_the_records_of_the_previous_stage() {
    let (mut client, _) = Client::start(full_options(), json!({"capabilities": {}}));
    let text = "find \"/srv\" |> filter size > 1mb and ";
    open(&mut client, text);
    let items = client
        .request("textDocument/completion", at(0, text.len()))
        .unwrap();
    let expected = json!({"$contains": [
        {"label": "size", "kind": 5, "detail": "field of entry", "sortText": "~size"},
        {"label": "modified", "kind": 5},
    ]});
    assert_matches(&items, &expected, "fields after and");
    client.shutdown().unwrap();
}

#[test]
fn describes_commands_and_error_codes_from_the_help_on_hover() {
    let (mut client, _) = Client::start(full_options(), json!({"capabilities": {}}));
    let published = open(&mut client, "find \"/srv/app\" |> qwzxv\n");
    let start = published["diagnostics"][0]["range"]["start"].clone();
    assert_eq!(published["diagnostics"][0]["code"], "E_UNKNOWN_COMMAND");
    let error = client
        .request(
            "textDocument/hover",
            json!({"textDocument": {"uri": URI}, "position": start}),
        )
        .unwrap();
    assert_matches(
        &error,
        &json!({"contents": {"kind": "markdown", "value": {"$includes": "E_UNKNOWN_COMMAND — "}}}),
        "error hover",
    );
    let command = client.request("textDocument/hover", at(0, 1)).unwrap();
    assert_matches(
        &command,
        &json!({"contents": {"value": {"$includes": "```text\nfind — "}}}),
        "command hover",
    );
    client.shutdown().unwrap();
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

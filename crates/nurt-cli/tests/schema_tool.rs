// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-cli/tests/schema_tool.rs
// Responsibility:   `nurt schema tool` against its documentation: the page of the tool definition for integrators and the reference of the command line.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `nurt schema tool` against its documentation: the page of the tool
//! definition for integrators and the reference of the command line.
//!
//! The executable runs with the engine of the program, so the description
//! texts and the catalog revision are the real ones.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "assertions of a test file"
)]

use std::collections::BTreeSet;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_nurt");
const TOOL_DEFINITION: &str = include_str!("../../../docs/integrators/tool-definition.md");
const CLI_REFERENCE: &str = include_str!("../../../docs/cli/nurt.md");

fn nurt(args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(BIN)
        .args(args)
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// The definition printed for `args`, which must succeed.
fn definition(args: &[&str]) -> Value {
    let output = nurt(args);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{args:?}: {}",
        stderr(&output)
    );
    serde_json::from_str(&stdout(&output)).unwrap_or_else(|error| panic!("{args:?}: {error}"))
}

/// The text between the heading `heading` and the next heading.
fn section<'a>(document: &'a str, heading: &str) -> &'a str {
    let start = document
        .find(&format!("\n## {heading}\n"))
        .unwrap_or_else(|| panic!("no section `{heading}`"))
        + 1;
    let rest = &document[start..];
    rest[3..].find("\n## ").map_or(rest, |end| &rest[..end + 3])
}

/// The lines of the code blocks of a section.
fn code_lines(section: &str) -> Vec<&str> {
    let mut inside = false;
    let mut lines = Vec::new();
    for line in section.lines() {
        if line.starts_with("```") {
            inside = !inside;
        } else if inside {
            lines.push(line);
        }
    }
    lines
}

/// The entries of the object written as `{a, b: {c}, d: "x"}`, split at the
/// commas outside nested braces; nested objects are returned as the entry
/// `key` with their own entries after a `.`.
fn documented_keys(object: &str) -> BTreeSet<String> {
    let inner = object
        .trim()
        .strip_prefix('{')
        .and_then(|text| text.strip_suffix('}'))
        .unwrap_or_else(|| panic!("`{object}` is not an object"));
    let mut keys = BTreeSet::new();
    let (mut depth, mut entry) = (0, String::new());
    let mut entries = Vec::new();
    for c in inner.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                entries.push(std::mem::take(&mut entry));
                continue;
            }
            _ => {}
        }
        entry.push(c);
    }
    entries.push(entry);
    for entry in entries {
        let entry = entry.trim();
        let (key, value) = entry
            .split_once(':')
            .map_or((entry, ""), |(key, value)| (key.trim(), value.trim()));
        keys.insert(key.to_owned());
        if value.starts_with('{') {
            keys.extend(
                documented_keys(value)
                    .into_iter()
                    .map(|nested| format!("{key}.{nested}")),
            );
        }
    }
    keys
}

/// The keys of a JSON object in the form of [`documented_keys`].
fn actual_keys(value: &Value) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for (key, member) in value.as_object().unwrap() {
        keys.insert(key.clone());
        if key == "function" {
            keys.extend(
                member
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(|nested| format!("{key}.{nested}")),
            );
        }
    }
    keys
}

/// The rows of the table of formats: the name of the format and the text of
/// the column `Object`, without the code marks.
fn documented_formats() -> Vec<(String, String)> {
    section(TOOL_DEFINITION, "Result")
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let format = cells.get(1)?.trim_matches('`');
            let cell = cells.get(3)?;
            let object = match cell.find("`{") {
                Some(start) => cell[start + 1..].split('`').next()?.to_owned(),
                None if cell.starts_with("as `") => cell.replace('`', ""),
                None => return None,
            };
            (!format.contains(' ')).then(|| (format.to_owned(), object))
        })
        .collect()
}

#[test]
fn every_documented_format_prints_the_documented_object() {
    let formats = documented_formats();
    let names: Vec<&str> = formats.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "anthropic",
            "openai",
            "openai-strict",
            "openai-responses",
            "openai-responses-strict",
            "gemini",
            "mcp",
            "json-schema"
        ]
    );
    for (name, object) in &formats {
        let value = definition(&["schema", "tool", "--format", name]);
        let documented = if object.starts_with('{') {
            documented_keys(object)
        } else {
            let base = if name.starts_with("openai-responses") {
                "openai-responses"
            } else {
                "openai"
            };
            let (_, base_object) = formats.iter().find(|(known, _)| known == base).unwrap();
            documented_keys(base_object)
        };
        assert_eq!(actual_keys(&value), documented, "format {name}: {object}");
        let tool = value.get("function").unwrap_or(&value);
        assert_eq!(tool["name"], "nurt", "format {name}");
        let strict = name.ends_with("-strict");
        if name.starts_with("openai") {
            assert_eq!(tool["strict"], strict, "format {name}");
        }
    }
}

#[test]
fn the_documented_syntax_lists_the_options_the_program_accepts() {
    let syntax = code_lines(section(TOOL_DEFINITION, "Syntax"));
    let [line] = syntax.as_slice() else {
        panic!("one syntax line expected: {syntax:?}");
    };
    let options: Vec<&str> = line
        .split_whitespace()
        .filter_map(|word| word.trim_start_matches('[').strip_prefix("--"))
        .collect();
    assert_eq!(options, ["format", "language", "groups"]);
    assert!(
        CLI_REFERENCE.contains(line),
        "docs/cli/nurt.md lacks `{line}`"
    );
    for option in options {
        let flag = format!("--{option}");
        let output = nurt(&["schema", "tool", &flag, "x\u{1}"]);
        assert_eq!(output.status.code(), Some(1), "{flag}");
        assert!(stderr(&output).contains("E_UNKNOWN_VALUE"), "{flag}");
    }
    let output = nurt(&["schema", "tool", "--unknown", "x"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("unknown option `--unknown`"));
    let output = nurt(&["schema", "tool", "--format"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn the_documented_examples_run_and_print_a_definition() {
    let examples: Vec<&str> = code_lines(section(TOOL_DEFINITION, "Examples"));
    assert_eq!(examples.len(), 3, "{examples:?}");
    for example in examples {
        let args: Vec<&str> = example.split_whitespace().skip(1).collect();
        assert_eq!(&args[..2], ["schema", "tool"], "{example}");
        let value = definition(&args);
        let tool = value.get("function").unwrap_or(&value);
        assert_eq!(tool["name"], "nurt", "{example}");
        assert!(
            tool["description"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "{example}"
        );
    }
}

#[test]
fn defaults_are_documented_json_schema_english_and_the_core_group() {
    let by_default = definition(&["schema", "tool", "--format", "json-schema"]);
    assert_eq!(
        definition(&["schema", "tool", "--language", "en", "--groups", "core"]),
        by_default
    );
    assert_eq!(definition(&["schema", "tool", "--groups=core"]), by_default);
    assert!(
        by_default["catalog_version"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(
        by_default["input_schema"]["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(
        by_default["input_schema"]["properties"]["language"]["default"],
        "en"
    );
}

#[test]
fn the_language_and_the_groups_change_the_description() {
    let description = |args: &[&str]| {
        let mut all = vec!["schema", "tool", "--format", "anthropic"];
        all.extend_from_slice(args);
        definition(&all)["description"].as_str().unwrap().to_owned()
    };
    let core = description(&[]);
    let polish = description(&["--language", "pl"]);
    let git = description(&["--groups", "core,git"]);
    let all = description(&["--groups", "all"]);
    assert_ne!(core, polish);
    assert!(git.starts_with(&core) && git.len() > core.len());
    assert!(all.starts_with(&core) && all.len() > git.len());
    assert!(!core.trim_start().starts_with('{'), "{core}");
    let polish_schema = definition(&["schema", "tool", "--language", "pl"]);
    assert_eq!(
        polish_schema["input_schema"]["properties"]["language"]["default"],
        "pl"
    );
}

#[test]
fn the_output_is_deterministic() {
    let args = ["schema", "tool", "--format", "gemini", "--groups", "all"];
    assert_eq!(stdout(&nurt(&args)), stdout(&nurt(&args)));
}

#[test]
fn an_unknown_value_names_the_accepted_ones() {
    for (args, accepted) in [
        (["schema", "tool", "--format", "claude"], "anthropic"),
        (["schema", "tool", "--language", "de"], "pl"),
        (["schema", "tool", "--groups", "nosuch"], "git"),
    ] {
        let output = nurt(&args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let error = stderr(&output);
        assert!(error.contains("E_UNKNOWN_VALUE"), "{args:?}: {error}");
        assert!(error.contains(accepted), "{args:?}: {error}");
    }
}

#[test]
fn without_options_the_published_schema_is_printed() {
    let output = nurt(&["schema", "tool"]);
    assert_eq!(output.status.code(), Some(0));
    let published: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(published["title"], "nurt-tool/1");
    assert!(
        CLI_REFERENCE.contains("nurt schema tool` without options prints the published schema")
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The page `docs/commands/run.md` agrees with the catalog and has the
//! sections and examples of a command page.

use std::path::PathBuf;

/// Smallest number of examples on a command page.
const MIN_EXAMPLES: usize = 3;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn page() -> String {
    std::fs::read_to_string(repository().join("docs/commands/run.md")).unwrap()
}

/// Summary and parameter names of `run` in `catalog/core.toml`.
fn catalog_run() -> (String, Vec<String>) {
    let catalog = std::fs::read_to_string(repository().join("catalog/core.toml")).unwrap();
    let mut lines = catalog.lines().map(str::trim);
    let mut in_command = false;
    let mut summary = String::new();
    let mut params = Vec::new();
    let mut in_param = false;
    for line in lines.by_ref() {
        if line == "[[command]]" {
            if in_command {
                break;
            }
            in_param = false;
            continue;
        }
        if line == "[[command.param]]" {
            in_param = true;
            continue;
        }
        if !in_command && !in_param && line == "name = \"run\"" {
            in_command = true;
            continue;
        }
        if !in_command {
            continue;
        }
        let value = |prefix: &str| {
            line.strip_prefix(prefix)
                .map(|rest| rest.trim_matches('"').to_owned())
        };
        if in_param {
            if let Some(name) = value("name = ") {
                params.push(name);
            }
        } else if let Some(text) = value("summary = ") {
            summary = text;
        }
    }
    (summary, params)
}

#[test]
fn front_matter_agrees_with_the_catalog() {
    let page = page();
    let (summary, _) = catalog_run();
    let front = page
        .strip_prefix("---\n")
        .and_then(|rest| rest.split("\n---\n").next())
        .unwrap();
    assert!(front.lines().any(|line| line == "command: run"));
    assert!(front.lines().any(|line| line == "group: core"));
    assert_eq!(
        front
            .lines()
            .find_map(|line| line.strip_prefix("summary: ")),
        Some(summary.as_str())
    );
}

#[test]
fn every_parameter_of_the_catalog_is_described_and_no_other() {
    let page = page();
    let (_, params) = catalog_run();
    assert!(params.len() > 20);
    let table: Vec<String> = page
        .split("## Parameters")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|line| line.split('`').next())
        .map(str::to_owned)
        .collect();
    assert_eq!(table, params);
}

#[test]
fn sections_follow_the_order_of_a_command_page() {
    let page = page();
    let headings: Vec<&str> = page
        .lines()
        .filter(|line| line.starts_with("## "))
        .collect();
    assert_eq!(
        headings,
        vec![
            "## Synopsis",
            "## Description",
            "## Parameters",
            "## Input and output",
            "## Effect",
            "## Notes",
            "## Examples",
            "## Errors",
            "## Related",
        ]
    );
}

#[test]
fn the_page_has_examples_of_run() {
    let page = page();
    let examples: Vec<&str> = page
        .split("```nurt example\n")
        .skip(1)
        .filter_map(|block| block.split("\n```").next())
        .collect();
    assert!(examples.len() >= MIN_EXAMPLES);
    for example in examples {
        assert!(example.contains("run ["), "example without run: {example}");
        assert!(example.contains("cwd: \"/home/user"), "example without cwd: {example}");
    }
}

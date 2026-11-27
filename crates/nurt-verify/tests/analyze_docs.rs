// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-verify/tests/analyze_docs.rs
// Responsibility:   The command pages of the analysis commands agree with the catalog and have the sections and examples of a command page.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The command pages of the analysis commands agree with the catalog and have
//! the sections and examples of a command page.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a failed precondition of a test is reported by a panic"
)]

use std::path::PathBuf;

/// Smallest number of examples on a command page.
const EXAMPLES_MIN: usize = 3;

/// Parameters that every command accepts, given as request options.
const COMMON_PARAMETERS: [&str; 1] = ["preview"];

const COMMANDS: [&str; 4] = [
    "analyze.check",
    "analyze.show",
    "analyze.format",
    "analyze.tools",
];

fn repository() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn page(command: &str) -> String {
    std::fs::read_to_string(repository().join(format!("docs/commands/{command}.md"))).unwrap()
}

/// Summary and parameter names of a command in the verification catalog.
fn catalog_entry(command: &str) -> (String, Vec<String>) {
    let catalog = std::fs::read_to_string(repository().join("catalog/verify.toml")).unwrap();
    let wanted = format!("name = \"{command}\"");
    let mut in_command = false;
    let mut in_param = false;
    let mut summary = String::new();
    let mut params = Vec::new();
    for line in catalog.lines().map(str::trim) {
        match line {
            "[[command]]" => {
                if in_command {
                    break;
                }
                in_param = false;
            }
            "[[command.param]]" => in_param = true,
            _ if !in_command && !in_param && line == wanted => in_command = true,
            _ if in_command => {
                let value = |prefix: &str| {
                    line.strip_prefix(prefix)
                        .map(|rest| rest.trim_matches('"').to_owned())
                };
                if in_param {
                    params.extend(value("name = "));
                } else if let Some(text) = value("summary = ") {
                    summary = text;
                }
            }
            _ => {}
        }
    }
    (summary, params)
}

#[test]
fn front_matter_agrees_with_the_catalog() {
    for command in COMMANDS {
        let page = page(command);
        let (summary, _) = catalog_entry(command);
        let front = page
            .strip_prefix("---\n")
            .and_then(|rest| rest.split("\n---\n").next())
            .unwrap();
        assert!(
            front
                .lines()
                .any(|line| line == format!("command: {command}")),
            "{command}: wrong command"
        );
        assert_eq!(
            front
                .lines()
                .find_map(|line| line.strip_prefix("summary: ")),
            Some(summary.as_str()),
            "{command}: summary differs from the catalog"
        );
    }
}

#[test]
fn every_parameter_of_the_catalog_is_described_and_no_other() {
    for command in COMMANDS {
        let page = page(command);
        let (_, params) = catalog_entry(command);
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
        assert_eq!(table, params, "{command}: parameter table differs");
    }
}

#[test]
fn sections_follow_the_order_of_a_command_page() {
    for command in COMMANDS {
        let page = page(command);
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
                "## Examples",
                "## Errors",
                "## Related",
            ],
            "{command}: sections"
        );
    }
}

#[test]
fn every_page_has_examples_that_call_its_command() {
    for command in COMMANDS {
        let page = page(command);
        let examples: Vec<&str> = page
            .split("```nurt example\n")
            .skip(1)
            .filter_map(|block| block.split("\n```").next())
            .collect();
        assert!(examples.len() >= EXAMPLES_MIN, "{command}: examples");
        for example in examples {
            assert!(
                example.split_whitespace().next() == Some(command),
                "{command}: example calls another command: {example}"
            );
        }
    }
}

#[test]
fn example_parameters_exist_in_the_catalog() {
    for command in COMMANDS {
        let page = page(command);
        let (_, params) = catalog_entry(command);
        for block in page.split("```nurt example\n").skip(1) {
            let example = block.split("\n```").next().unwrap();
            for word in example.split([' ', ',']) {
                if let Some(name) = word.strip_suffix(':').filter(|name| !name.is_empty()) {
                    assert!(
                        COMMON_PARAMETERS.contains(&name)
                            || params.iter().any(|param| param == name),
                        "{command}: unknown parameter {name}"
                    );
                }
            }
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-verify/tests/test_docs.rs
// Responsibility:   The pages of `test.run`, `test.failed`, `test.show` and `test.list` agree.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The pages of `test.run`, `test.failed`, `test.show` and `test.list` agree
//! with the catalog and have the sections and examples of a command page.

use std::path::PathBuf;

/// Smallest number of examples on a command page.
const MIN_EXAMPLES: usize = 3;

const COMMANDS: [&str; 4] = ["test.run", "test.failed", "test.show", "test.list"];

const SECTIONS: [&str; 7] = [
    "## Synopsis",
    "## Description",
    "## Parameters",
    "## Input and output",
    "## Effect",
    "## Examples",
    "## Errors",
];

fn repository() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("..")
    .join("..")
}

fn page(command: &str) -> String {
    std::fs::read_to_string(repository().join(format!("docs/commands/{command}.md")))
        .unwrap_or_default()
}

/// Summary and parameter names of a command in `catalog/verify.toml`.
fn catalog(command: &str) -> (String, Vec<String>) {
    let text =
        std::fs::read_to_string(repository().join("catalog/verify.toml")).unwrap_or_default();
    let mut summary = String::new();
    let mut params = Vec::new();
    let mut selected = false;
    let mut in_param = false;
    let mut expect_name = false;
    for line in text.lines().map(str::trim) {
        if line == "[[command]]" {
            selected = false;
            in_param = false;
            expect_name = true;
            continue;
        }
        if line == "[[command.param]]" {
            in_param = true;
            continue;
        }
        let value = |prefix: &str| {
            line.strip_prefix(prefix)
                .map(|rest| rest.trim_matches('"').to_owned())
        };
        if expect_name {
            expect_name = false;
            selected = value("name = ").as_deref() == Some(command);
            continue;
        }
        if !selected {
            continue;
        }
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

fn examples(page: &str) -> Vec<String> {
    page.split("```nurt example\n")
        .skip(1)
        .filter_map(|block| block.split("\n```").next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn front_matter_of_every_page_names_the_command_and_the_catalog_summary() {
    for command in COMMANDS {
        let page = page(command);
        let (summary, _) = catalog(command);
        assert!(!summary.is_empty(), "{command}: not in the catalog");
        let front = page
            .strip_prefix("---\n")
            .and_then(|rest| rest.split("\n---\n").next())
            .unwrap_or_default();
        assert!(
            front
                .lines()
                .any(|line| line == format!("command: {command}")),
            "{command}"
        );
        assert!(
            front.lines().any(|line| line == "group: verify"),
            "{command}"
        );
        assert_eq!(
            front
                .lines()
                .find_map(|line| line.strip_prefix("summary: ")),
            Some(summary.as_str()),
            "{command}"
        );
    }
}

#[test]
fn every_parameter_of_the_catalog_is_described_and_no_other() {
    for command in COMMANDS {
        let page = page(command);
        let (_, params) = catalog(command);
        let table: Vec<String> = page
            .split("## Parameters")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .filter_map(|line| line.split('`').next())
            .map(str::to_owned)
            .collect();
        assert!(!params.is_empty(), "{command}");
        assert_eq!(table, params, "{command}");
    }
}

#[test]
fn sections_follow_the_order_of_a_command_page() {
    for command in COMMANDS {
        let page = page(command);
        let mut last = 0;
        for section in SECTIONS {
            let position = page
                .find(&format!("\n{section}\n"))
                .unwrap_or_else(|| panic!("{command}: missing {section}"));
            assert!(position > last, "{command}: {section} out of order");
            last = position;
        }
    }
}

#[test]
fn examples_call_the_command_with_parameters_of_the_catalog() {
    for command in COMMANDS {
        let page = page(command);
        let (_, params) = catalog(command);
        let found = examples(&page);
        assert!(found.len() >= MIN_EXAMPLES, "{command}: too few examples");
        for example in found {
            let call = example.split("|>").next().unwrap_or_default();
            assert!(call.starts_with(command), "{command}: {example}");
            for word in call.split_whitespace() {
                if let Some(name) = word.strip_suffix(':') {
                    assert!(
                        params.iter().any(|param| param == name),
                        "{command}: unknown parameter {name} in {example}"
                    );
                }
            }
        }
    }
}

#[test]
fn every_page_lists_the_errors_it_names_in_a_table() {
    for command in COMMANDS {
        let page = page(command);
        let errors = page
            .split("\n## Errors\n")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .unwrap_or_default();
        assert!(
            errors.contains("| Code | Meaning | Typical fix |"),
            "{command}"
        );
        assert!(errors.contains("| `E_"), "{command}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-exec/tests/stages_docs.rs
// Responsibility:   Pages of the record stages in `docs/commands`: structure and examples.
// Dependencies:     serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Pages of the record stages in `docs/commands`: structure and examples.
//!
//! Every `nurt example` block of a page is the `program` of a conformance
//! case of the command; the case holds the input of the documented stage as
//! produced by the stages before it, and the test runs the stage on it and
//! compares the result with the expected file of the case.

// Tests fail through unwrap, expect and panic by design.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "fixtures/stages/harness.rs"]
mod harness;

use std::fs;
use std::path::{Path, PathBuf};

use nurt_exec::stages::COMMANDS;
use serde_json::Value as Json;

/// Smallest number of examples of one page.
const MIN_EXAMPLES: usize = 3;

/// Opening line of an example block.
const EXAMPLE_FENCE: &str = "```nurt example";

/// Section headings of a page, in order.
const SECTIONS: [&str; 8] = [
    "## Synopsis",
    "## Description",
    "## Parameters",
    "## Input and output",
    "## Effect",
    "## Examples",
    "## Errors",
    "## Related",
];

fn root() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn page(command: &str) -> String {
    let path = root().join("docs/commands").join(format!("{command}.md"));
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn examples(page: &str) -> Vec<String> {
    let mut examples = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in page.lines() {
        match current.as_mut() {
            None if line.trim_end() == EXAMPLE_FENCE => current = Some(Vec::new()),
            None => {}
            Some(lines) if line.trim_end() == "```" => {
                examples.push(lines.join("\n"));
                current = None;
            }
            Some(lines) => lines.push(line),
        }
    }
    examples
}

fn cases(command: &str) -> Vec<(PathBuf, Json, Json)> {
    let dir = root().join("conformance/cases/stages").join(command);
    let mut cases: Vec<(PathBuf, Json, Json)> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .map(|path| {
            let read = |name: &str| -> Json {
                serde_json::from_str(&fs::read_to_string(path.join(name)).unwrap()).unwrap()
            };
            let input = read("case.in.json");
            let output = read("case.out.json");
            (path, input, output)
        })
        .collect();
    cases.sort_by(|left, right| left.0.cmp(&right.0));
    cases
}

#[test]
fn pages_follow_the_reference_structure() {
    for command in COMMANDS {
        let page = page(command);
        let mut lines = page
            .lines()
            .skip_while(|line| line.starts_with("<!--") || line.trim().is_empty());
        assert_eq!(lines.next(), Some("---"), "front matter of `{command}`");
        assert_eq!(
            lines.next(),
            Some(format!("command: {command}").as_str()),
            "front matter of `{command}`"
        );
        assert!(
            page.contains(&format!("\n# {command}\n")),
            "title of `{command}`"
        );
        let positions: Vec<usize> = SECTIONS
            .iter()
            .map(|section| {
                page.find(&format!("\n{section}\n"))
                    .unwrap_or_else(|| panic!("`{command}` lacks {section}"))
            })
            .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "sections of `{command}` are out of order"
        );
    }
}

#[test]
fn every_example_runs_as_documented() {
    for command in COMMANDS {
        let examples = examples(&page(command));
        assert!(
            examples.len() >= MIN_EXAMPLES,
            "`{command}` has {} examples",
            examples.len()
        );
        let cases = cases(command);
        for example in examples {
            let (path, input, expected) = cases
                .iter()
                .find(|(_, input, _)| input["program"].as_str() == Some(example.as_str()))
                .unwrap_or_else(|| panic!("no conformance case of `{command}` runs `{example}`"));
            let actual = harness::run_case(input);
            assert_eq!(&actual, expected, "{}", path.display());
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

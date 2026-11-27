// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-cli/tests/subcommand_table.rs
// Responsibility:   The table of subcommands, the dispatcher, the usage text and the.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The table of subcommands, the dispatcher, the usage text and the
//! documentation list the same subcommands.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "assertions of a test file"
)]

use nurt_cli::args::SUBCOMMANDS;
use nurt_cli::integrations::{COMPONENTS, NOT_CONNECTED};
use nurt_cli::text;

const TABLE: &str = include_str!("../../../tables/cli-subcommands.toml");
const REFERENCE: &str = include_str!("../../../docs/cli/nurt.md");
const EXIT_CODES: &str = include_str!("../../../docs/cli/exit-codes.md");
const MANIFEST: &str = include_str!("../Cargo.toml");

fn table() -> Vec<toml::Table> {
    let document: toml::Table = toml::from_str(TABLE).unwrap();
    document["subcommand"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.as_table().unwrap().clone())
        .collect()
}

#[test]
fn table_lists_the_subcommands_of_the_dispatcher_in_order() {
    let names: Vec<String> = table()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, SUBCOMMANDS);
}

#[test]
fn every_entry_has_a_description_and_arguments() {
    for entry in table() {
        let name = entry["name"].as_str().unwrap();
        let description = entry["description"].as_str().unwrap();
        assert!(description.ends_with('.'), "{name}");
        assert!(entry["arguments"].is_array(), "{name}");
    }
}

#[test]
fn reference_documents_every_subcommand_with_an_example() {
    for name in SUBCOMMANDS {
        let heading = format!("## `{name}`");
        let section = REFERENCE
            .split("\n## ")
            .find(|section| format!("## {section}").starts_with(&heading))
            .unwrap_or_else(|| panic!("no section for `{name}`"));
        assert!(section.contains("nurt "), "no example for `{name}`");
        assert!(section.contains("Example"), "no example for `{name}`");
    }
}

#[test]
fn usage_text_names_every_subcommand() {
    let usage = text::usage();
    for name in SUBCOMMANDS {
        assert!(usage.contains(name), "{name}");
    }
}

#[test]
fn subcommand_usage_repeats_the_description_and_arguments_of_the_table() {
    for entry in table() {
        let name = entry["name"].as_str().unwrap();
        let usage = text::subcommand_usage(name).unwrap_or_else(|| panic!("no usage of `{name}`"));
        assert!(
            usage.starts_with(entry["description"].as_str().unwrap()),
            "{name}"
        );
        let arguments: Vec<&str> = entry["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|argument| argument.as_str().unwrap())
            .collect();
        let synopsis = [format!("nurt {name}")]
            .into_iter()
            .chain(arguments.iter().map(|argument| (*argument).to_owned()))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            usage.contains(&format!("Usage: {synopsis}\n")),
            "{name}: {usage}"
        );
    }
    assert_eq!(text::subcommand_usage("not-a-subcommand"), None);
}

#[test]
fn unconnected_subcommands_are_subcommands() {
    for name in NOT_CONNECTED {
        assert!(SUBCOMMANDS.contains(name), "{name}");
    }
}

#[test]
fn component_features_are_optional_and_on_by_default() {
    let manifest: toml::Table = toml::from_str(MANIFEST).unwrap();
    let features = manifest["features"].as_table().unwrap();
    let default: Vec<&str> = features["default"]
        .as_array()
        .unwrap()
        .iter()
        .map(|feature| feature.as_str().unwrap())
        .collect();
    let components: Vec<&str> = COMPONENTS
        .iter()
        .filter_map(|component| component.feature)
        .collect();
    assert_eq!(default, components);
    let dependencies = manifest["dependencies"].as_table().unwrap();
    for component in COMPONENTS {
        assert!(SUBCOMMANDS.contains(&component.subcommand));
        let Some(feature) = component.feature else {
            continue;
        };
        let linked = features[feature].as_array().unwrap();
        let crate_name = linked[0].as_str().unwrap().strip_prefix("dep:").unwrap();
        assert_eq!(
            dependencies[crate_name]["optional"].as_bool(),
            Some(true),
            "{crate_name}"
        );
    }
}

#[test]
fn required_dependencies_are_crates_of_the_core_and_the_extensions() {
    let manifest: toml::Table = toml::from_str(MANIFEST).unwrap();
    let dependencies = manifest["dependencies"].as_table().unwrap();
    let required: Vec<&str> = dependencies
        .iter()
        .filter(|(name, spec)| {
            name.starts_with("nurt-")
                && spec.get("optional").and_then(toml::Value::as_bool) != Some(true)
        })
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        required,
        ["nurt-contracts", "nurt-core", "nurt-ext", "nurt-tooldef"]
    );
}

#[test]
fn exit_code_reference_lists_every_code() {
    for code in [
        nurt_cli::exit::SUCCESS,
        nurt_cli::exit::FAILURE,
        nurt_cli::exit::USAGE,
        nurt_cli::exit::UNAVAILABLE,
        nurt_cli::exit::TIMEOUT,
        nurt_cli::exit::INTERNAL,
        nurt_cli::exit::CANCELLED,
    ] {
        assert!(EXIT_CODES.contains(&format!("| {code} |")), "{code}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

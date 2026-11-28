// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-help/tests/describe_layers.rs
// Responsibility:   Tests of the generated description layers.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the generated description layers.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use nurt_contracts::envelope::Language;
use nurt_contracts::error::ErrorCode;
use nurt_help::describe::{
    self, CORE_TOKENS_EN, CORE_TOKENS_PL, Description, Layer, Length, TARGET_LENGTHS,
};
use nurt_help::tokens::TokenCounter;

fn layer(layer: Layer, name: &str, language: Language, length: Length) -> Description {
    describe::layer(&common::catalog(), layer, name, language, length).unwrap()
}

fn core(language: Language, length: Length) -> Description {
    layer(Layer::Core, "", language, length)
}

fn within_percent(actual: u32, target: u32, percent: u32) -> bool {
    actual.abs_diff(target) * 100 <= target * percent
}

#[test]
fn english_core_layer_has_the_reference_length() {
    let description = core(Language::En, Length::Default);
    assert!(description.estimate);
    assert!(
        within_percent(description.tokens, CORE_TOKENS_EN, 3),
        "{} tokens",
        description.tokens
    );
}

#[test]
fn polish_core_layer_is_longer_and_within_the_expected_range() {
    let description = core(Language::Pl, Length::Default);
    assert!(
        within_percent(description.tokens, CORE_TOKENS_PL, 3),
        "{} tokens",
        description.tokens
    );
    assert!((2560..=2720).contains(&description.tokens));
    assert!(description.text.contains("ZASADY"));
    assert!(description.text.contains("PRZYKŁADY"));
}

#[test]
fn core_layers_reach_each_target_length_in_both_languages() {
    for language in [Language::En, Language::Pl] {
        for target in TARGET_LENGTHS {
            let description = core(language, Length::Tokens(target));
            assert!(
                within_percent(description.tokens, target, 5),
                "{language:?} {target}: {} tokens",
                description.tokens
            );
        }
    }
}

#[test]
fn generation_is_deterministic() {
    for length in [Length::Default, Length::Tokens(1000), Length::Tokens(4000)] {
        assert_eq!(core(Language::En, length), core(Language::En, length));
    }
    let first = layer(Layer::Group, "git", Language::En, Length::Default);
    assert_eq!(
        first,
        layer(Layer::Group, "git", Language::En, Length::Default)
    );
}

#[test]
fn longer_layers_contain_the_shorter_ones() {
    let short = core(Language::En, Length::Tokens(1000)).text;
    let long = core(Language::En, Length::Tokens(3000)).text;
    for line in short.lines().filter(|line| line.starts_with("1. ")) {
        assert!(long.contains(line));
    }
    assert!(long.len() > short.len());
}

#[test]
fn every_core_command_is_listed_at_every_length() {
    let catalog = common::catalog();
    for length in [Length::Tokens(1000), Length::Default, Length::Tokens(4000)] {
        let text = core(Language::En, length).text;
        for command in catalog.core_commands() {
            let listed = text.lines().any(|line| {
                line == command.name || line.starts_with(&format!("{} ", command.name))
            });
            assert!(listed, "{} is missing at {length:?}", command.name);
        }
    }
}

#[test]
fn core_layer_states_the_rule_on_stages_after_the_effect() {
    let text = core(Language::En, Length::Default).text;
    assert!(
        text.contains(
            "after the effect by assert, filter, select, first, last, skip, count and sort"
        )
    );
    assert!(text.contains("only verify rolls back"));
}

#[test]
fn core_layer_mentions_partial_entries_and_selected_paths_without_a_window() {
    let text = core(Language::En, Length::Default).text.to_lowercase();
    assert!(text.contains("history state: partial"));
    assert!(text.contains("undo on_conflict: partial"));
    assert!(text.contains("undo paths:"));
    assert!(!text.contains("undo window"));
    assert!(!text.contains("within the last"));
}

#[test]
fn core_layer_presents_undo_as_the_repair_of_an_error() {
    let text = core(Language::En, Length::Default).text;
    assert!(text.contains("A mistake visible in the result can be repaired with undo"));
    assert!(text.contains("does not replace checking"));
}

#[test]
fn core_layer_has_no_brand_slogan_and_no_references_to_the_design() {
    for language in [Language::En, Language::Pl] {
        let text = core(language, Length::Tokens(4000)).text;
        assert!(!text.contains("command console"));
    }
}

#[test]
fn git_group_is_described_as_built_in() {
    let description = layer(Layer::Group, "git", Language::En, Length::Tokens(4000));
    let text = description.text;
    assert!(text.starts_with("GROUP git\n"));
    assert!(text.contains("All operations are built in"));
    assert!(!text.contains("external git program"));
    assert!(!text.contains("Through the external"));
    assert!(text.contains("git.status"));
    assert!(text.contains("git.push"));
}

#[test]
fn every_group_has_a_layer_of_moderate_length() {
    let catalog = common::catalog();
    let mut total = 0;
    for file in catalog.groups().filter(|file| file.group != "core") {
        let description = layer(Layer::Group, &file.group, Language::En, Length::Default);
        assert!(
            description
                .text
                .starts_with(&format!("GROUP {}\n", file.group))
        );
        for command in &file.commands {
            assert!(description.text.contains(&command.name), "{}", command.name);
        }
        assert!(
            (20..=400).contains(&description.tokens),
            "{}: {}",
            file.group,
            description.tokens
        );
        total += description.tokens;
    }
    assert!((1000..=5000).contains(&total), "{total}");
}

#[test]
fn group_layer_can_be_fitted_to_a_target() {
    let description = layer(Layer::Group, "git", Language::En, Length::Tokens(600));
    assert!(
        within_percent(description.tokens, 600, 5),
        "{}",
        description.tokens
    );
}

#[test]
fn polish_group_layer_translates_the_note() {
    let text = layer(Layer::Group, "git", Language::Pl, Length::Tokens(4000)).text;
    assert!(text.contains("Wszystkie operacje są wbudowane"));
}

#[test]
fn unknown_group_lists_the_nearest_groups() {
    let error = describe::layer(
        &common::catalog(),
        Layer::Group,
        "documnts",
        Language::En,
        Length::Default,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownName);
    assert!(
        error
            .expected
            .as_ref()
            .unwrap()
            .to_string()
            .contains("documents")
    );
}

#[test]
fn guide_layer_is_the_text_of_the_guide() {
    let description = layer(
        Layer::Guide,
        "powershell-migration",
        Language::En,
        Length::Default,
    );
    assert!(description.text.starts_with("powershell-migration — "));
    assert!(description.text.contains("Where-Object"));
    let error = describe::layer(
        &common::catalog(),
        Layer::Guide,
        "nothing",
        Language::En,
        Length::Default,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownName);
}

#[test]
fn reference_layer_describes_one_command_in_markdown() {
    let description = layer(Layer::Reference, "list", Language::En, Length::Default);
    assert!(description.text.starts_with("## list\n"));
    assert!(description.text.contains("| `path` | `path` | required |"));
    assert!(
        description
            .text
            .contains("| `type` | `enum(file\\|directory\\|link\\|any)` | `any` |")
    );
}

#[test]
fn reference_layer_without_a_name_covers_the_whole_catalog() {
    let catalog = common::catalog();
    let description = describe::layer(
        &catalog,
        Layer::Reference,
        "",
        Language::En,
        Length::Default,
    )
    .unwrap();
    for command in catalog.commands() {
        assert!(
            description.text.contains(&format!("## {}\n", command.name)),
            "{}",
            command.name
        );
    }
}

struct WordCounter;

impl TokenCounter for WordCounter {
    fn count(&self, text: &str) -> u32 {
        u32::try_from(text.split_whitespace().count()).unwrap()
    }

    fn is_estimate(&self) -> bool {
        false
    }

    fn tokenizer(&self) -> &str {
        "words"
    }
}

#[test]
fn another_token_counter_replaces_the_estimate() {
    let description = describe::layer_with_counter(
        &WordCounter,
        &common::catalog(),
        Layer::Core,
        "",
        Language::En,
        Length::Tokens(1500),
    )
    .unwrap();
    assert!(!description.estimate);
    assert!(
        within_percent(description.tokens, 1500, 5),
        "{}",
        description.tokens
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

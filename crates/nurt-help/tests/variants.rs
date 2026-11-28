// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-help/tests/variants.rs
// Responsibility:   Examples of the help and the description layers in the syntax variants.
// Dependencies:     nurt_contracts, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Examples of the help and the description layers in the syntax variants.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use nurt_contracts::envelope::Language;
use nurt_help::describe::{self, Layer, Length, TARGET_LENGTHS};
use nurt_help::help::{self, HelpFormat, HelpQuery};
use nurt_help::variant::{Variant, convert_fenced, convert_program};
use nurt_syntax::parse::{ParseOptions, parse};
use nurt_syntax::print;

/// Asserts that `converted`, written in `variant`, is the program `original`
/// of variant A. Variant B denotes the same tree; variant C also names
/// parameters, so for it the text must read back and print unchanged.
fn assert_same_program(original: &str, converted: &str, variant: Variant) {
    let options = ParseOptions::default();
    let tree = parse(original, Variant::A, &options).unwrap();
    let reparsed =
        parse(converted, variant, &options).unwrap_or_else(|error| panic!("{converted}: {error}"));
    if variant == Variant::B {
        assert_eq!(
            print::print(&reparsed, Variant::A).trim_end(),
            print::print(&tree, Variant::A).trim_end(),
            "{original}"
        );
    }
    assert_eq!(
        print::print(&reparsed, variant).trim_end(),
        converted,
        "{original}"
    );
}

fn block_after(text: &str, start: &str) -> String {
    text.lines()
        .skip_while(|line| !line.starts_with(start))
        .skip(1)
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a line opens a construct (`steps {`, `when … {`, `params {`): the
/// conversion of variants covers pipelines, and the constructs have their own
/// tests of the language.
fn opens_construct(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or_default();
    line.trim_end().ends_with('{')
        && matches!(
            first,
            "steps" | "when" | "each" | "while" | "repeat" | "define" | "params"
        )
}

fn guide_programs(text: &str) -> Vec<String> {
    let mut programs = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in text.lines() {
        match current.take() {
            Some(program) if line.starts_with("```") => {
                if !program.iter().any(|line| opens_construct(line)) {
                    programs.push(program.join("\n"));
                }
            }
            Some(mut program) => {
                program.push(line);
                current = Some(program);
            }
            None if line.starts_with("```nurt") => current = Some(Vec::new()),
            None => {}
        }
    }
    programs
}

#[test]
fn every_group_example_denotes_the_same_program_in_each_variant() {
    let catalog = common::catalog();
    let mut count = 0;
    for file in catalog.groups().filter(|file| file.group != "core") {
        for program in describe::group_examples(&file.group) {
            for variant in [Variant::B, Variant::C] {
                let converted = convert_program(&catalog, program, variant).unwrap();
                assert_same_program(program, &converted, variant);
                count += 1;
            }
        }
    }
    assert!(count >= 40);
}

#[test]
fn core_examples_denote_the_same_program_in_each_variant() {
    let catalog = common::catalog();
    let text = describe::layer(
        &catalog,
        Layer::Core,
        "",
        Language::En,
        Length::Tokens(4000),
    )
    .unwrap()
    .text;
    let examples = block_after(&text, "EXAMPLES");
    for variant in [Variant::B, Variant::C] {
        let converted = convert_program(&catalog, &examples, variant).unwrap();
        assert_same_program(&examples, &converted, variant);
    }
}

#[test]
fn page_and_guide_examples_denote_the_same_program_in_each_variant() {
    let catalog = common::catalog();
    for page in catalog.docs().pages() {
        for example in &page.examples {
            for variant in [Variant::B, Variant::C] {
                let converted = convert_program(&catalog, &example.program, variant).unwrap();
                assert_same_program(&example.program, &converted, variant);
            }
        }
    }
    for (file, text) in common::markdown_files("docs/guides") {
        for program in guide_programs(&text) {
            for variant in [Variant::B, Variant::C] {
                let converted = convert_program(&catalog, &program, variant)
                    .unwrap_or_else(|error| panic!("{file}: {error}"));
                assert_same_program(&program, &converted, variant);
            }
        }
    }
}

#[test]
fn core_layers_reach_each_target_length_in_every_variant() {
    let catalog = common::catalog();
    for variant in [Variant::B, Variant::C] {
        for target in TARGET_LENGTHS {
            let description = describe::layer_in_variant(
                &catalog,
                Layer::Core,
                "",
                Language::En,
                Length::Tokens(target),
                variant,
            )
            .unwrap();
            assert!(
                description.tokens.abs_diff(target) * 100 <= target * 5,
                "{variant:?} {target}: {} tokens",
                description.tokens
            );
        }
    }
}

#[test]
fn variant_a_layer_equals_the_default_layer() {
    let catalog = common::catalog();
    let default = describe::layer(&catalog, Layer::Core, "", Language::En, Length::Default)
        .unwrap()
        .text;
    let variant_a = describe::layer_in_variant(
        &catalog,
        Layer::Core,
        "",
        Language::En,
        Length::Default,
        Variant::A,
    )
    .unwrap()
    .text;
    assert_eq!(default, variant_a);
}

#[test]
fn core_layer_in_variant_b_shows_call_notation_and_keeps_the_prose() {
    let catalog = common::catalog();
    let text = describe::layer_in_variant(
        &catalog,
        Layer::Core,
        "",
        Language::En,
        Length::Tokens(4000),
        Variant::B,
    )
    .unwrap()
    .text;
    assert!(text.contains("EXAMPLES"));
    assert!(text.contains("first(20)"));
    assert!(text.contains("program = [let name = PIPELINE]"));
}

#[test]
fn group_layer_in_variant_c_names_positional_arguments() {
    let catalog = common::catalog();
    let text = describe::layer_in_variant(
        &catalog,
        Layer::Group,
        "git",
        Language::En,
        Length::Tokens(4000),
        Variant::C,
    )
    .unwrap()
    .text;
    assert!(text.contains("git.status repo=[\"/srv/app\"]"), "{text}");
}

#[test]
fn generation_in_a_variant_is_deterministic() {
    let catalog = common::catalog();
    let build = || {
        describe::layer_in_variant(
            &catalog,
            Layer::Core,
            "",
            Language::Pl,
            Length::Tokens(3000),
            Variant::C,
        )
        .unwrap()
        .text
    };
    assert_eq!(build(), build());
}

#[test]
fn command_help_shows_examples_in_the_requested_variant() {
    let catalog = common::catalog();
    let a = help::render(&HelpQuery::topic("evaluate"), &catalog).unwrap();
    let b = help::render(
        &HelpQuery::topic("evaluate").with_variant(Variant::B),
        &catalog,
    )
    .unwrap();
    assert_ne!(a.body, b.body);
    assert!(b.body.contains("evaluate("), "{}", b.body);
    let json = help::render(
        &HelpQuery::topic("evaluate")
            .with_variant(Variant::C)
            .with_format(HelpFormat::Json),
        &catalog,
    )
    .unwrap();
    let document: serde_json::Value = serde_json::from_str(&json.body).unwrap();
    let examples = document["examples"].as_array().unwrap();
    assert!(!examples.is_empty());
    assert!(
        examples
            .iter()
            .any(|example| example.as_str().unwrap().contains("expression="))
    );
}

#[test]
fn overview_and_guides_follow_the_variant() {
    let catalog = common::catalog();
    let overview = help::render(&HelpQuery::new().with_variant(Variant::B), &catalog).unwrap();
    assert!(
        overview
            .body
            .contains("  find(\"/home/user/app\", glob: \"**/*.log\")\n  |> remove()")
    );
    let guide = help::render(
        &HelpQuery::topic("bash-migration").with_variant(Variant::B),
        &catalog,
    )
    .unwrap();
    let converted = convert_fenced(&catalog, "```nurt\ncount\n```", Variant::B).unwrap();
    assert_eq!(converted, "```nurt\ncount()\n```");
    assert!(
        guide
            .body
            .contains("find(\"/srv/app\", glob: \"**/*.tmp\")\n|> remove()")
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Behaviour of `bind_with_correction` that the corpora do not express:
//! positions, revisions, the switch, the budget, the store of candidates and
//! the errors of binding without correction.

mod bind_support;

use std::time::Duration;

use bind_support::{catalog, expected_names, linux};
use nurt_bind::{
    BindContext, Outcome, UNCORRECTED_REV, bind, bind_with_correction, offer_candidates,
};
use nurt_contracts::config::Config;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::spec::Os;
use nurt_correct::candidates::{ManualClock, Store};
use nurt_correct::tables::load_embedded;
use nurt_syntax::parse::{ParseOptions, parse};

fn corrected_with(text: &str, config: &Config, context: &BindContext) -> Result<Outcome, Error> {
    let tables = load_embedded().unwrap();
    bind_with_correction(text, SyntaxVariant::A, catalog(), &tables, config, context)
}

fn corrected(text: &str) -> Result<Outcome, Error> {
    corrected_with(text, &Config::default(), &linux())
}

fn rules(outcome: &Outcome) -> Vec<String> {
    outcome
        .corrections
        .iter()
        .map(|correction| correction.rule.to_string())
        .collect()
}

#[test]
fn accepts_a_valid_program_without_corrections() {
    let outcome = corrected("find \"/srv/app\" glob: \"*.go\" |> first 5").unwrap();
    assert!(outcome.corrections.is_empty());
    assert_eq!(outcome.bound.corr_rev, load_embedded().unwrap().corr_rev());
    assert!(outcome.canonical.starts_with("find \"/srv/app\""));
}

#[test]
fn positions_a_value_correction_at_the_written_value() {
    let outcome = corrected("find \"/srv/app\" |> sort size order: decs").unwrap();
    assert_eq!(rules(&outcome), ["V01"]);
    let correction = &outcome.corrections[0];
    assert_eq!(correction.id, "C1");
    let at = correction.at.unwrap();
    assert_eq!((at.line, at.column, at.length), (1, 37, Some(4)));
    assert_eq!(
        (correction.from.as_str(), correction.to.as_str()),
        ("decs", "desc")
    );
}

#[test]
fn numbers_lexical_and_binding_corrections_by_position() {
    let outcome = corrected("Find \"/srv/app\" |> sotr size order: decs").unwrap();
    assert_eq!(rules(&outcome), ["L03", "L01", "V01"]);
    let ids: Vec<&str> = outcome
        .corrections
        .iter()
        .map(|correction| correction.id.as_str())
        .collect();
    assert_eq!(ids, ["C1", "C2", "C3"]);
}

#[test]
fn binds_as_written_when_correction_is_disabled() {
    let mut config = Config::default();
    config.correction.enabled = false;
    let error = corrected_with("fnid \"/srv/app\"", &config, &linux()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownCommand);
    let outcome = corrected_with("find \"/srv/app\"", &config, &linux()).unwrap();
    assert_eq!(outcome.bound.corr_rev, UNCORRECTED_REV);
}

#[test]
fn rejects_a_program_over_the_budget_with_its_corrected_form() {
    let mut config = Config::default();
    config.correction.max_read = 1;
    let error = corrected_with(
        "find \"/srv/app\" hiden: false |> sort size order: decs",
        &config,
        &linux(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Ambiguous);
    assert_eq!(error.candidates.len(), 1);
    assert!(error.candidates[0].program.contains("hidden: false"));
    assert!(error.candidates[0].program.contains("order: desc"));
}

#[test]
fn offers_the_candidates_of_an_ambiguous_change_for_acceptance() {
    let error = corrected("run [\"make\"] cwd: \"/srv/app\" max_memory: \"512M\"").unwrap_err();
    assert_eq!(error.code(), ErrorCode::Ambiguous);
    assert_eq!(error.candidates.len(), 2);
    let clock = ManualClock::new();
    let mut store = Store::new(Duration::from_secs(600), &clock);
    let offered = offer_candidates(error, &mut store);
    let reference = offered.candidates[0].reference.clone().unwrap();
    let accepted = store.accept(&reference).unwrap();
    assert_eq!(accepted, offered.candidates[0].program);
    assert!(accepted.contains("max_memory: 0.5gib"), "{accepted}");
}

#[test]
fn asks_before_reading_a_regular_expression_as_a_path_in_a_change() {
    let error = corrected("find /etc/ |> remove").unwrap_err();
    assert_eq!(error.code(), ErrorCode::Ambiguous);
    let outcome = corrected("list /etc/").unwrap();
    assert_eq!(outcome.canonical, "list \"/etc\"");
}

#[test]
fn reports_a_changing_command_two_edits_away_as_unknown() {
    let error = corrected("decomprse \"/srv/app/a.gz\"").unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownCommand);
    let expected = error.expected.clone().unwrap();
    let names: Vec<&str> = expected
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|name| name.as_str())
        .collect();
    assert!(names.contains(&"decompress"), "{names:?}");
    assert!(names.len() <= 8);
}

#[test]
fn takes_base_from_the_configuration() {
    let mut config = Config::default();
    config.scope.base = Some("/srv/app".to_owned());
    let outcome = corrected_with("find \"./src\"", &config, &linux()).unwrap();
    assert_eq!(rules(&outcome), ["L23"]);
    assert_eq!(outcome.canonical, "find \"/srv/app/src\"");
}

#[test]
fn prints_the_canonical_form_in_the_requested_variant() {
    let tables = load_embedded().unwrap();
    let outcome = bind_with_correction(
        "find path=\"/srv/app\" type=dir",
        SyntaxVariant::C,
        catalog(),
        &tables,
        &Config::default(),
        &BindContext::new(Os::Linux),
    )
    .unwrap();
    assert_eq!(rules(&outcome), ["V01"]);
    assert!(outcome.canonical.contains("type=directory"));
}

#[test]
fn moves_positional_arguments_before_named_ones() {
    let outcome = corrected("find glob: \"*.go\" \"/srv/app\"").unwrap();
    assert_eq!(rules(&outcome), ["L18"]);
    assert_eq!(outcome.canonical, "find \"/srv/app\" glob: \"*.go\"");
}

#[test]
fn splits_a_stage_at_a_missing_pipe_when_the_program_does_not_bind() {
    let outcome = corrected("find \"/srv/app\" glob: \"*.go\" search /x/").unwrap();
    assert_eq!(rules(&outcome), ["L14"]);
    assert!(
        outcome.canonical.ends_with("|> search /x/"),
        "{}",
        outcome.canonical
    );
}

#[test]
fn reports_an_unknown_field_without_correction() {
    let document = parse(
        "find \"/srv/app\" |> filter colour == \"red\"",
        SyntaxVariant::A,
        &ParseOptions::default(),
    )
    .unwrap();
    let error = bind(&document, catalog(), &linux()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownField);
    let names = expected_names(&error);
    assert!(!names.is_empty() && names.len() <= 8);
}

#[test]
fn keeps_fields_of_unknown_records_unchecked() {
    let document = parse(
        "processes |> select pid name |> filter anything > 1",
        SyntaxVariant::A,
        &ParseOptions::default(),
    )
    .unwrap();
    assert!(bind(&document, catalog(), &linux()).is_ok());
}

#[test]
fn positions_a_value_correction_in_characters_after_multi_byte_text() {
    let outcome = corrected("find \"/srv/é\" |> sort size order: decs").unwrap();
    assert_eq!(rules(&outcome), ["V01"]);
    let at = outcome.corrections[0].at.unwrap();
    assert_eq!((at.line, at.column, at.length), (1, 35, Some(4)));
    assert!(outcome.canonical.contains("/srv/é"), "{}", outcome.canonical);
    assert!(outcome.canonical.ends_with("order: desc"));
}

#[test]
fn positions_a_command_correction_in_characters_after_multi_byte_text() {
    let outcome = corrected("find \"/srv/日本語\" |> sotr size").unwrap();
    assert_eq!(rules(&outcome), ["L01"]);
    let at = outcome.corrections[0].at.unwrap();
    assert_eq!((at.line, at.column, at.length), (1, 20, Some(4)));
}

#[test]
fn keeps_multi_byte_paths_unchanged_without_corrections() {
    let outcome = corrected("find \"/srv/ünï/日本語\" glob: \"*.rs\"").unwrap();
    assert!(outcome.corrections.is_empty());
    assert!(outcome.canonical.contains("/srv/ünï/日本語"));
}

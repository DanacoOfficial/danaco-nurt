// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the help topics.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use nurt_contracts::envelope::Language;
use nurt_contracts::error::ErrorCode;
use nurt_help::help::{self, HelpFormat, HelpQuery, TopicKind};
use nurt_help::tokens::{EstimateCounter, TokenCounter};
use nurt_help::{TAGLINE, first_line};

fn text(query: &HelpQuery) -> String {
    help::render(query, &common::catalog()).unwrap().body
}

fn tokens(body: &str) -> u32 {
    EstimateCounter.count(body)
}

#[test]
fn overview_starts_with_the_product_line() {
    let body = text(&HelpQuery::new());
    let first = body.lines().next().unwrap();
    assert_eq!(first, first_line());
    assert!(first.contains(env!("CARGO_PKG_VERSION")));
    assert!(first.is_ascii());
}

#[test]
fn overview_introduces_the_product_and_lists_the_topics() {
    let body = text(&HelpQuery::new());
    assert!(body.contains("A program is a pipeline"));
    assert!(body.contains("undo reverts one"));
    assert!(body.contains("commands: list find info read"));
    assert!(body.contains("groups: documents text code"));
    assert!(body.contains("guides: administration bash-migration documents encodings"));
    assert!(body.contains("unix windows"));
    assert!(body.contains("topics: syntax, corrections"));
}

#[test]
fn answers_stay_between_the_estimated_bounds() {
    for query in [
        HelpQuery::new(),
        HelpQuery::topic("list"),
        HelpQuery::topic("replace"),
    ] {
        let count = tokens(&text(&query));
        assert!((50..=600).contains(&count), "{query:?}: {count} tokens");
    }
}

#[test]
fn command_topic_shows_signature_parameters_and_effect() {
    let body = text(&HelpQuery::topic("replace"));
    assert!(body.starts_with("replace — "));
    assert!(body.contains("\n  params: "));
    assert!(body.contains("occurrence:"));
    assert!(body.contains("\n  input: "));
    assert!(body.contains("\n  effect: "));
}

#[test]
fn command_topic_shows_the_examples_of_the_page() {
    let body = text(&HelpQuery::topic("evaluate").with_examples(2));
    assert!(body.contains("evaluate 6 * 7"));
    assert!(body.contains(r#"evaluate "Release " + "1.0""#));
    assert!(!body.contains("evaluate now() - 3d"));
    let one = text(&HelpQuery::topic("evaluate").with_examples(1));
    assert!(one.contains("evaluate 6 * 7"));
    assert!(!one.contains("Release"));
}

#[test]
fn command_topic_lists_errors_and_related_commands_of_the_page() {
    let body = text(&HelpQuery::topic("describe"));
    assert!(body.contains("errors: E_TYPE ("));
    assert!(body.contains("related: help, select, filter, summarize"));
}

#[test]
fn command_and_group_with_one_name_are_separate_topics() {
    let command = text(&HelpQuery::topic("verify"));
    assert!(command.starts_with("verify — "));
    let group = text(&HelpQuery::topic("verify").with_kind(TopicKind::Group));
    assert!(group.starts_with("GROUP verify"));
    let mut query = HelpQuery::new();
    query.group = Some("verify".to_owned());
    assert_eq!(text(&query), group);
}

#[test]
fn error_topic_gives_meaning_stage_and_fix() {
    let body = text(&HelpQuery::topic("E_ASSERT"));
    assert!(body.starts_with("E_ASSERT — an assertion does not hold"));
    assert!(body.contains("stage: 3 check · 4 plan · 5a after"));
    assert!(body.contains("\nfix: "));
}

#[test]
fn every_error_code_has_a_topic() {
    let catalog = common::catalog();
    for code in ErrorCode::ALL {
        let answer = help::render(&HelpQuery::topic(code.as_str()), &catalog).unwrap();
        assert!(answer.body.starts_with(code.as_str()), "{code}");
    }
}

#[test]
fn guide_topic_shows_the_guide() {
    let mut query = HelpQuery::new();
    query.skill = Some("bash-migration".to_owned());
    let body = text(&query);
    assert!(body.starts_with("bash-migration — "));
    assert!(body.contains("E_ONE_EFFECT"));
}

#[test]
fn syntax_topic_includes_the_rule_on_stages_after_the_effect() {
    let body = text(&HelpQuery::topic("syntax"));
    assert!(body.contains("program = [let name = PIPELINE]"));
    assert!(
        body.contains(
            "after the effect by assert, filter, select, first, last, skip, count and sort"
        )
    );
    assert!(body.contains("only verify rolls back"));
}

#[test]
fn corrections_topic_describes_classes_and_policy() {
    let body = text(&HelpQuery::topic("corrections"));
    assert!(body.contains("classes: C certain"));
    assert!(body.contains("P is never applied and gives E_AMBIGUOUS"));
}

#[test]
fn search_finds_commands_errors_and_guides() {
    let mut query = HelpQuery::new();
    query.search = Some("rename symbol".to_owned());
    let body = text(&query);
    assert!(body.starts_with("search: rename symbol"));
    assert!(body.contains("code.rename"));
    query.search = Some("assertion".to_owned());
    assert!(text(&query).contains("E_ASSERT"));
    query.search = Some("powershell".to_owned());
    assert!(text(&query).contains("powershell-migration"));
}

#[test]
fn search_without_a_match_says_so() {
    let mut query = HelpQuery::new();
    query.search = Some("zzzzqq".to_owned());
    assert!(text(&query).contains("no matches"));
}

#[test]
fn unknown_topic_lists_the_nearest_names() {
    let error = help::render(&HelpQuery::topic("remvoe"), &common::catalog()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownName);
    let expected = error.expected.clone().unwrap();
    let names = expected.as_array().unwrap();
    assert!(names.contains(&serde_json::json!("remove")));
    assert!(names.len() <= 8);
}

#[test]
fn unknown_error_code_lists_codes() {
    let error = help::render(&HelpQuery::topic("E_ASERT"), &common::catalog()).unwrap_err();
    let expected = error.expected.clone().unwrap();
    assert!(
        expected
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("E_ASSERT"))
    );
}

#[test]
fn two_subjects_conflict() {
    let mut query = HelpQuery::topic("list");
    query.group = Some("git".to_owned());
    let error = help::render(&query, &common::catalog()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::ParamConflict);
}

#[test]
fn json_format_returns_a_document() {
    let query = HelpQuery::topic("replace").with_format(HelpFormat::Json);
    let answer = help::render(&query, &common::catalog()).unwrap();
    assert_eq!(answer.format, HelpFormat::Json);
    let document: serde_json::Value = serde_json::from_str(&answer.body).unwrap();
    assert_eq!(document["command"], "replace");
    assert_eq!(document["group"], "core");
    assert!(document["params"].as_array().unwrap().len() > 5);
}

#[test]
fn json_documents_exist_for_every_kind_of_topic() {
    let catalog = common::catalog();
    for topic in ["E_STALE", "syntax", "corrections", "git", "bash-migration"] {
        let query = HelpQuery::topic(topic).with_format(HelpFormat::Json);
        let answer = help::render(&query, &catalog).unwrap();
        let document: serde_json::Value = serde_json::from_str(&answer.body).unwrap();
        assert!(document.is_object(), "{topic}");
    }
}

#[test]
fn polish_overview_translates_the_labels_only() {
    let query = HelpQuery::new().with_language(Language::Pl);
    let body = text(&query);
    assert_eq!(body.lines().next().unwrap(), first_line());
    assert!(body.contains("polecenia: list find info read"));
    assert!(body.contains("Program jest potokiem"));
}

#[test]
fn topic_kinds_match_the_values_of_the_catalog_parameter() {
    let catalog = common::catalog();
    let spec = catalog.command("help").unwrap();
    let names: Vec<&str> = TopicKind::ALL.iter().map(|kind| kind.name()).collect();
    let values: Vec<&str> = spec
        .param("kind")
        .unwrap()
        .values
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(names, values);
    for name in ["topic", "group", "skill", "search", "examples"] {
        assert!(spec.param(name).is_some(), "{name}");
    }
    assert_eq!(
        spec.param("examples")
            .unwrap()
            .default
            .as_ref()
            .unwrap()
            .as_str(),
        "3"
    );
}

#[test]
fn group_note_points_at_the_group_description() {
    let catalog = common::catalog();
    assert_eq!(
        help::group_note(&catalog, "set", &[]).as_deref(),
        Some("note: group documents is described by: help group: documents")
    );
    assert_eq!(help::group_note(&catalog, "set", &["documents"]), None);
    assert_eq!(help::group_note(&catalog, "list", &[]), None);
    assert_eq!(help::group_note(&catalog, "nothing", &[]), None);
}

#[test]
fn tagline_is_the_canonical_sentence() {
    assert_eq!(
        TAGLINE,
        "Command shell and language where every change can be undone"
    );
}

#[test]
fn commands_topic_lists_every_command_under_its_group() {
    let catalog = common::catalog();
    let body = help::render(&HelpQuery::topic("commands"), &catalog)
        .unwrap()
        .body;
    for file in catalog.groups() {
        assert!(
            body.contains(&format!("{} ({})", file.group, file.commands.len())),
            "group {}",
            file.group
        );
        for command in &file.commands {
            let line = format!("  {} — {}", command.name, command.summary);
            assert!(body.lines().any(|found| found == line), "{}", command.name);
        }
    }
}

#[test]
fn commands_topic_is_a_json_document_of_groups() {
    let catalog = common::catalog();
    let query = HelpQuery::topic("commands").with_format(HelpFormat::Json);
    let body = help::render(&query, &catalog).unwrap().body;
    let document: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(document["kind"], "commands");
    assert_eq!(document["count"], catalog.command_count());
    let groups = document["groups"].as_array().unwrap();
    assert_eq!(groups.len(), catalog.groups().count());
    assert!(groups[0]["commands"].as_array().unwrap().len() > 1);
}

#[test]
fn commands_topic_follows_the_language() {
    let query = HelpQuery::topic("commands").with_language(Language::Pl);
    let body = text(&query);
    assert!(body.starts_with("polecenia według grup"));
    assert!(text(&HelpQuery::topic("commands")).starts_with("commands by group"));
}

#[test]
fn overview_and_unknown_topics_name_the_commands_topic() {
    assert!(text(&HelpQuery::new()).contains("topics: syntax, corrections, commands"));
    let error = help::render(&HelpQuery::topic("comands"), &common::catalog()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownName);
    let expected = error.expected.clone().unwrap();
    assert_eq!(expected.as_array().unwrap()[0], "commands");
}

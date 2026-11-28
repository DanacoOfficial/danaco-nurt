// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-help/tests/help_topics.rs
// Responsibility:   Tests of the help topics.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
fn topic_kinds_are_found_by_their_names() {
    for kind in TopicKind::ALL {
        assert_eq!(TopicKind::from_name(kind.name()), Some(kind));
    }
    assert_eq!(TopicKind::from_name("nothing"), None);
    assert_eq!(TopicKind::from_name(""), None);
}

#[test]
fn the_kind_selects_the_topic_among_its_own_names() {
    for name in ["", "syntax"] {
        let syntax = text(&HelpQuery::topic(name).with_kind(TopicKind::Syntax));
        assert!(syntax.contains("program = [let name = PIPELINE]"), "{name}");
    }
    let part = text(&HelpQuery::topic("pipelines").with_kind(TopicKind::Syntax));
    assert_eq!(
        part,
        text(&HelpQuery::topic("pipelines")),
        "a part of the syntax answers as without a kind"
    );
    for name in ["", "corrections"] {
        let corrections = text(&HelpQuery::topic(name).with_kind(TopicKind::Corrections));
        assert!(corrections.contains("classes: C certain"), "{name}");
    }
    let error = text(&HelpQuery::topic("E_STALE").with_kind(TopicKind::Error));
    assert!(error.starts_with("E_STALE — "));
}

#[test]
fn the_syntax_and_corrections_kinds_reject_a_topic_of_another_name() {
    let catalog = common::catalog();
    for (kind, topic) in [
        (TopicKind::Syntax, "anything"),
        (TopicKind::Syntax, "corrections"),
        (TopicKind::Syntax, "remove"),
        (TopicKind::Corrections, "syntax"),
        (TopicKind::Corrections, "anything"),
    ] {
        let query = HelpQuery::topic(topic).with_kind(kind);
        let error = help::render(&query, &catalog).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownName, "{kind:?} {topic}");
        assert_eq!(error.found.as_deref(), Some(topic));
    }
}

#[test]
fn the_kind_restricts_the_lookup_to_that_kind() {
    let catalog = common::catalog();
    let command = HelpQuery::topic("git").with_kind(TopicKind::Command);
    assert_eq!(
        help::render(&command, &catalog).unwrap_err().code(),
        ErrorCode::UnknownName
    );
    let error = HelpQuery::topic("E_NOTHING").with_kind(TopicKind::Error);
    assert_eq!(
        help::render(&error, &catalog).unwrap_err().code(),
        ErrorCode::UnknownName
    );
    let guide = HelpQuery::topic("list").with_kind(TopicKind::Skill);
    assert_eq!(
        help::render(&guide, &catalog).unwrap_err().code(),
        ErrorCode::UnknownName
    );
}

#[test]
fn search_ignores_words_of_one_character() {
    let mut query = HelpQuery::new();
    query.search = Some("a".to_owned());
    assert!(text(&query).contains("no matches"));
    query.search = Some(String::new());
    assert!(text(&query).contains("no matches"));
}

#[test]
fn search_in_json_lists_the_results_by_kind() {
    let mut query = HelpQuery::new().with_format(HelpFormat::Json);
    query.search = Some("assertion".to_owned());
    let document: serde_json::Value = serde_json::from_str(&text(&query)).unwrap();
    assert_eq!(document["search"], "assertion");
    assert!(document["commands"].is_array());
    assert!(document["guides"].is_array());
    assert!(
        document["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "E_ASSERT")
    );
}

#[test]
fn the_number_of_examples_zero_hides_them() {
    let body = text(&HelpQuery::topic("evaluate").with_examples(0));
    assert!(!body.contains("\nexamples:"));
    assert!(!body.contains("evaluate 6 * 7"));
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

/// Topics that the sources of other crates write as the help of an error.
fn topics_in(directory: &std::path::Path, found: &mut Vec<(String, String)>) {
    const MARKS: [&str; 3] = [".help(\"", ".with_help(\"", "help: \""];
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            topics_in(&path, found);
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for line in source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
        {
            for mark in MARKS {
                let mut rest = line;
                while let Some(position) = rest.find(mark) {
                    rest = &rest[position + mark.len()..];
                    let Some(end) = rest.find('"') else { break };
                    found.push((path.display().to_string(), rest[..end].to_owned()));
                }
            }
        }
    }
}

#[test]
fn every_topic_an_error_points_at_exists() {
    let catalog = common::catalog();
    let mut found = Vec::new();
    for krate in ["nurt-bind", "nurt-correct", "nurt-syntax", "nurt-cli"] {
        let directory = common::root().join("crates").join(krate).join("src");
        topics_in(&directory, &mut found);
    }
    assert!(found.len() > 10, "{found:?}");
    for (file, topic) in found {
        let topic = topic.strip_prefix("help ").unwrap_or(&topic);
        if topic.is_empty() {
            continue;
        }
        let result = help::render(&HelpQuery::topic(topic), &catalog);
        assert!(result.is_ok(), "{file}: help \"{topic}\" fails: {result:?}");
    }
}

#[test]
fn part_topics_answer_in_both_languages() {
    for topic in ["pipelines", "let", "paths", "records", "lists"] {
        let english = text(&HelpQuery::topic(topic));
        let polish = text(&HelpQuery::topic(topic).with_language(Language::Pl));
        assert!(!english.is_empty() && !polish.is_empty(), "{topic}");
        assert_ne!(english, polish, "{topic}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

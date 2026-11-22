// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Corpus tests of YAML sequences, anchors, merge keys, documents, comments
//! and tags: identity of unchanged documents, appending to and removing from
//! every list with a check by the reference parser, the YAML Test Suite cases
//! of these constructs and a comparison of merged keys with `yq`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "helpers outside test functions fail the test on unexpected input"
)]

#[path = "u05_support/suite.rs"]
mod suite;
mod u06_support;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;
use nurt_doc::formats::yaml::anchor::{self, Anchors};
use nurt_doc::formats::yaml::comment::Comments;
use nurt_doc::formats::yaml::merge;
use nurt_doc::formats::yaml::model::{NodeId, NodeKind, YamlModel};
use nurt_doc::formats::yaml::seq;
use serde_json::Value as Json;
use u06_support::{
    apply_edits, corpus_dir, defines_used_anchor, inside_key, json_at, reachable_by_alias,
    reference, suite_case_ids, yaml_files,
};

/// Minimum number of files of the advanced corpus.
const MIN_CORPUS_FILES: usize = 250;

/// Text appended to every list by the operation test.
const APPENDED: &str = "appended item";

/// Failures listed in the message of a failed corpus test.
const LISTED_FAILURES: usize = 20;

#[derive(Default)]
struct Tally {
    operations: usize,
    failures: Vec<String>,
}

impl Tally {
    fn fail(&mut self, message: String) {
        self.failures.push(message);
    }

    fn assert_clean(&self, what: &str) {
        assert!(
            self.failures.is_empty(),
            "{what}: {} of {} operations failed:\n{}",
            self.failures.len(),
            self.operations,
            self.failures
                .iter()
                .take(LISTED_FAILURES)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

fn lists(model: &YamlModel, anchors: &Anchors) -> Vec<NodeId> {
    model
        .node_ids()
        .filter(|&id| matches!(model.node(id).kind, NodeKind::Sequence(_)))
        .filter(|&id| !reachable_by_alias(model, anchors, id) && !inside_key(model, id))
        .collect()
}

/// Appends to and removes the first item of every list of `text`, checking
/// that bytes outside the edit are kept and that the reference parser reads
/// the expected value. An item that defines an anchor used by an alias is
/// not removed, since the alias would be left undefined.
fn exercise_lists(name: &str, text: &str, tally: &mut Tally) {
    let Ok(model) = YamlModel::parse(text) else {
        tally.fail(format!("{name}: does not parse"));
        return;
    };
    let Ok(before) = reference(text) else {
        return;
    };
    let anchors = Anchors::new(&model);
    for list in lists(&model, &anchors) {
        let path = anchor::path_of(&model, list);
        let mut expected = before.clone();
        let Some(Json::Array(items)) = json_at(&mut expected, &path) else {
            continue;
        };
        items.push(Json::String(APPENDED.to_owned()));
        tally.operations += 1;
        match seq::plan_append(&model, list, &Value::Text(APPENDED.to_owned()), &path) {
            Ok(plan) => check(name, &path, "append", &model, &plan.edits, &expected, tally),
            Err(error) => tally.fail(format!("{name}: append to `{path}`: {}", error.message)),
        }
        let first = model
            .node(list)
            .as_sequence()
            .and_then(|sequence| sequence.items.first())
            .map(|item| item.node);
        let Some(first) = first.filter(|&item| !defines_used_anchor(&model, &anchors, item)) else {
            continue;
        };
        let mut expected = before.clone();
        let Some(Json::Array(items)) = json_at(&mut expected, &path) else {
            continue;
        };
        items.remove(0);
        tally.operations += 1;
        match seq::plan_remove(&model, list, 0, &path) {
            Ok(plan) => check(name, &path, "remove", &model, &plan.edits, &expected, tally),
            Err(error) => tally.fail(format!(
                "{name}: remove `{path}[0]` at byte {}: {}",
                model.node(first).span.start,
                error.message
            )),
        }
    }
}

fn check(
    name: &str,
    path: &KeyPath,
    operation: &str,
    model: &YamlModel,
    edits: &[nurt_doc::edit::ByteEdit],
    expected: &[Json],
    tally: &mut Tally,
) {
    let result = apply_edits(model, edits);
    let source = model.source();
    let first = edits.iter().map(|edit| edit.span.start).min().unwrap_or(0);
    let last = edits.iter().map(|edit| edit.span.end).max().unwrap_or(0);
    if !result.starts_with(&source[..first]) || !result.ends_with(&source[last..]) {
        tally.fail(format!(
            "{name}: {operation} `{path}` changed bytes outside the edit"
        ));
        return;
    }
    let region = &result[first..result.len() - (source.len() - last)];
    match reference(&result) {
        Ok(after) if after == expected => {}
        Ok(_) => tally.fail(format!(
            "{name}: {operation} `{path}` gives another value; edited text {region:?}"
        )),
        Err(error) => tally.fail(format!(
            "{name}: {operation} `{path}` breaks the document: {error}; edited text {region:?}"
        )),
    }
}

#[test]
fn corpus_has_the_required_size() {
    assert!(yaml_files(&corpus_dir()).len() >= MIN_CORPUS_FILES);
}

#[test]
fn corpus_documents_open_unchanged_and_within_alias_limits() {
    let mut tally = Tally::default();
    for file in yaml_files(&corpus_dir()) {
        let text = std::fs::read_to_string(&file).expect("corpus file");
        let name = file.display().to_string();
        tally.operations += 1;
        match YamlModel::parse(&text) {
            Ok(model) => {
                if model.source() != text {
                    tally.fail(format!("{name}: source differs after parsing"));
                }
                if let Err(error) = anchor::check_expansion(&model) {
                    tally.fail(format!("{name}: {}", error.message));
                }
                check_bindings(&name, &model, &mut tally);
            }
            Err(error) => tally.fail(format!("{name}: {}", error.message)),
        }
    }
    tally.assert_clean("open");
}

/// Every alias has a target, every node has a key path that parses back,
/// and comments bind without panics.
fn check_bindings(name: &str, model: &YamlModel, tally: &mut Tally) {
    let anchors = Anchors::new(model);
    let comments = Comments::new(model);
    for id in model.node_ids() {
        let node = model.node(id);
        if matches!(node.kind, NodeKind::Alias(_)) && anchors.target(id).is_none() {
            tally.fail(format!(
                "{name}: alias without anchor at byte {}",
                node.span.start
            ));
        }
        let path = anchor::path_of(model, id);
        if KeyPath::parse(&path.to_string()).ok().as_ref() != Some(&path) {
            tally.fail(format!("{name}: key path `{path}` does not parse back"));
        }
        let start = node.full_span().start;
        let _ = comments.describe(start, model.line_end(start));
    }
}

#[test]
fn every_corpus_list_takes_an_appended_item_and_loses_its_first_item() {
    let mut tally = Tally::default();
    for file in yaml_files(&corpus_dir()) {
        let text = std::fs::read_to_string(&file).expect("corpus file");
        exercise_lists(&file.display().to_string(), &text, &mut tally);
    }
    assert!(tally.operations > 0);
    tally.assert_clean("corpus lists");
}

#[test]
fn yaml_test_suite_cases_of_these_constructs_are_read_and_edited() {
    let wanted: BTreeSet<String> = suite_case_ids().into_iter().collect();
    let cases: Vec<suite::SuiteCase> = suite::load_cases()
        .into_iter()
        .filter(|case| !case.fail && wanted.contains(&case.id))
        .collect();
    assert!(!cases.is_empty());
    let mut tally = Tally::default();
    for case in &cases {
        tally.operations += 1;
        match YamlModel::parse(&case.yaml) {
            Ok(model) => {
                if model.source() != case.yaml {
                    tally.fail(format!("{}: source differs after parsing", case.id));
                }
                if let Err(error) = anchor::check_expansion(&model) {
                    tally.fail(format!("{}: {}", case.id, error.message));
                }
                check_bindings(&case.id, &model, &mut tally);
                exercise_lists(&case.id, &case.yaml, &mut tally);
            }
            Err(error) => tally.fail(format!("{}: {}", case.id, error.message)),
        }
    }
    tally.assert_clean("yaml-test-suite");
}

fn yq_available() -> bool {
    Command::new("yq")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn yq_documents(file: &Path) -> Option<Vec<Json>> {
    let output = Command::new("yq")
        .args([
            "--yaml-fix-merge-anchor-to-spec=true",
            "-o",
            "json",
            "-I0",
            ".",
        ])
        .arg(file)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[test]
fn merged_keys_match_yq_when_yq_is_installed() {
    if !yq_available() {
        return;
    }
    let mut tally = Tally::default();
    for file in yaml_files(&corpus_dir()) {
        let text = std::fs::read_to_string(&file).expect("corpus file");
        let Ok(model) = YamlModel::parse(&text) else {
            continue;
        };
        let Some(mut documents) = yq_documents(&file) else {
            continue;
        };
        let name = file.display().to_string();
        for map in model.node_ids() {
            let Some(mapping) = model.node(map).as_mapping() else {
                continue;
            };
            if !mapping.entries.iter().any(|entry| entry.merge) || inside_key(&model, map) {
                continue;
            }
            tally.operations += 1;
            let path = anchor::path_of(&model, map);
            let Some(Json::Object(object)) = json_at(&mut documents, &path) else {
                tally.fail(format!("{name}: yq has no mapping at `{path}`"));
                continue;
            };
            let mut ours: BTreeSet<String> = mapping
                .entries
                .iter()
                .filter(|entry| !entry.merge)
                .filter_map(|entry| merge::key_name(&model, entry.key))
                .collect();
            for member in merge::inherited_members(&model, map) {
                if let (Some(scalar), Some(Json::String(theirs))) = (
                    model.node(member.value).as_scalar(),
                    object.get(&member.name),
                ) && &scalar.value != theirs
                {
                    tally.fail(format!(
                        "{name}: `{path}.{}` is `{}`, yq reads `{theirs}`",
                        member.name, scalar.value
                    ));
                }
                ours.insert(member.name);
            }
            let theirs: BTreeSet<String> = object.keys().cloned().collect();
            if ours != theirs {
                tally.fail(format!(
                    "{name}: keys of `{path}` differ: {ours:?} and yq {theirs:?}"
                ));
            }
        }
    }
    tally.assert_clean("merge keys against yq");
}

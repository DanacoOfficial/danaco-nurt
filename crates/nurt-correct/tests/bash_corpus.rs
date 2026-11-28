// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-correct/tests/bash_corpus.rs
// Responsibility:   Corpus of bash habits: every case of `tests/d1/correction/bash` is translated in a live call and compared with its expected reaction.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Corpus of bash habits: every case of `tests/d1/correction/bash` is
//! translated in a live call and compared with its expected reaction.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use nurt_contracts::ast::{AstDocument, Expr, Head, Pipeline, Statement};
use nurt_correct::bash::{Mode, TranslateContext, translate_with};
use serde_json::{Value, json};

/// Least number of cases of every rule `B01` to `B40`.
const MIN_CASES_PER_RULE: usize = 3;

/// Least number of cases of the mistake corpus.
const MIN_MISTAKES: usize = 120;

/// Least number of cases of the adversarial corpus.
const MIN_ADVERSARIAL: usize = 40;

/// Number of rules of the bash table.
const RULES: u8 = 40;

/// Certainty classes from the most to the least certain.
const CLASS_ORDER: &str = "CSFP";

const REQUIRED: [&str; 7] = [
    "id",
    "input",
    "variant",
    "expect",
    "program_kind",
    "intent",
    "source",
];

type CorpusFile = (PathBuf, Vec<Value>);

fn corpus_files() -> Result<Vec<CorpusFile>, String> {
    let dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d1/correction/bash");
    let mut paths = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            paths.push(path);
        }
    }
    paths.sort();
    let mut files = Vec::new();
    for path in paths {
        let text =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let cases = text
            .lines()
            .map(|line| {
                serde_json::from_str(line).map_err(|error| format!("{}: {error}", path.display()))
            })
            .collect::<Result<Vec<Value>, String>>()?;
        files.push((path, cases));
    }
    Ok(files)
}

fn cases() -> Result<Vec<Value>, String> {
    Ok(corpus_files()?
        .into_iter()
        .flat_map(|(_, cases)| cases)
        .collect())
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| items.iter().map(|item| text(item).to_owned()).collect())
        .unwrap_or_default()
}

/// The reaction of the translation to a case, with every member that an
/// expectation can name.
fn reaction(case: &Value) -> Value {
    let lets = strings(&case["context"]["lets"]);
    let context = TranslateContext {
        base: case["context"]["base"].as_str(),
        lets: &lets,
        functions: &[],
        mode: Mode::Live,
    };
    match translate_with(text(&case["input"]), &context) {
        None => json!({"unchanged": true}),
        Some(Ok(translation)) => {
            let rules: Vec<String> = translation
                .rules
                .iter()
                .map(|rule| rule.rule.to_string())
                .collect();
            let class = translation
                .rules
                .iter()
                .map(|rule| rule.class.as_str())
                .max_by_key(|class| CLASS_ORDER.find(class))
                .unwrap_or("C");
            json!({
                "canonical": translation.text(),
                "rules": rules,
                "class": class,
                "alternatives": translation.alternative_texts(),
            })
        }
        Some(Err(error)) => {
            let data = error.data();
            let candidates: Vec<&str> = data
                .candidates
                .iter()
                .map(|candidate| candidate.program.as_str())
                .collect();
            json!({
                "error": error.code().as_str(),
                "fix": data.fix,
                "next_steps": data.next_steps,
                "candidates": candidates,
            })
        }
    }
}

/// The expectation with the members it implies and the reaction limited to
/// the members of the expectation.
fn compared(expect: &Value, actual: &Value) -> (Value, Value) {
    let mut expected = expect.as_object().cloned().unwrap_or_default();
    if expected.contains_key("canonical") && !expected.contains_key("alternatives") {
        expected.insert("alternatives".to_owned(), json!([]));
    }
    let projected = expected
        .keys()
        .map(|key| (key.clone(), actual.get(key).cloned().unwrap_or(Value::Null)))
        .collect();
    (Value::Object(expected), Value::Object(projected))
}

#[test]
fn corpus_cases_have_the_required_members_and_sorted_unique_ids() {
    let mut ids = BTreeSet::new();
    for (path, cases) in corpus_files().unwrap() {
        let file_ids: Vec<&str> = cases.iter().map(|case| text(&case["id"])).collect();
        let mut sorted = file_ids.clone();
        sorted.sort_unstable();
        assert_eq!(file_ids, sorted, "{} is not sorted by id", path.display());
        for case in &cases {
            for member in REQUIRED {
                assert!(
                    case.get(member).is_some(),
                    "{}: missing `{member}`",
                    case["id"]
                );
            }
            let id = text(&case["id"]);
            assert!(
                id.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "{id}"
            );
            assert!(ids.insert(id.to_owned()), "{id} appears twice");
            assert_eq!(case["variant"], "a", "{id}");
            assert!(
                matches!(text(&case["program_kind"]), "read" | "effect"),
                "{id}"
            );
            assert!(
                matches!(text(&case["intent"]), "unambiguous" | "ambiguous"),
                "{id}"
            );
        }
    }
}

#[test]
fn every_rule_has_cases_and_both_corpora_are_large_enough() {
    let cases = cases().unwrap();
    let mut per_rule: BTreeMap<u8, usize> = BTreeMap::new();
    for case in &cases {
        let id = text(&case["id"]);
        if let Some(number) = id.strip_prefix("bash-b").and_then(|rest| rest.get(..2)) {
            *per_rule.entry(number.parse().unwrap()).or_default() += 1;
        }
    }
    for rule in 1..=RULES {
        let count = per_rule.get(&rule).copied().unwrap_or_default();
        assert!(count >= MIN_CASES_PER_RULE, "B{rule:02} has {count} cases");
    }
    let adversarial = cases
        .iter()
        .filter(|case| case["corpus"] == "adversarial")
        .count();
    assert!(
        adversarial >= MIN_ADVERSARIAL,
        "{adversarial} adversarial cases"
    );
    assert!(
        cases.len() - adversarial >= MIN_MISTAKES,
        "{} mistake cases",
        cases.len() - adversarial
    );
}

#[test]
fn corpus_cases_match_their_expected_reaction() {
    let mut failures = Vec::new();
    for case in cases().unwrap() {
        let actual = reaction(&case);
        let (expected, projected) = compared(&case["expect"], &actual);
        if expected != projected {
            failures.push(format!(
                "{}\n  expected {expected}\n  actual   {actual}",
                text(&case["id"])
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failing cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn translated_paths_keep_the_written_segments() {
    let mut failures = Vec::new();
    for case in cases().unwrap() {
        let lets = strings(&case["context"]["lets"]);
        let base = case["context"]["base"].as_str();
        let context = TranslateContext {
            base,
            lets: &lets,
            functions: &[],
            mode: Mode::Live,
        };
        let input = text(&case["input"]);
        let Some(Ok(translation)) = translate_with(input, &context) else {
            continue;
        };
        let mut texts = Vec::new();
        document_texts(&translation.tree, &mut texts);
        for path in texts.iter().filter(|text| text.starts_with('/')) {
            let foreign = path
                .split('/')
                .filter(|segment| !segment.is_empty())
                .find(|segment| {
                    !input.contains(segment) && !base.is_some_and(|base| base.contains(segment))
                });
            if let Some(segment) = foreign {
                failures.push(format!(
                    "{}: segment `{segment}` of `{path}`",
                    text(&case["id"])
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

fn document_texts(document: &AstDocument, texts: &mut Vec<String>) {
    for statement in &document.program.statements {
        match statement {
            Statement::Let(binding) => pipeline_texts(&binding.value, texts),
            Statement::Pipeline(pipeline) => pipeline_texts(pipeline, texts),
            _ => {}
        }
    }
}

fn pipeline_texts(pipeline: &Pipeline, texts: &mut Vec<String>) {
    let head = match &pipeline.head {
        Head::Stage(stage) => Some(stage),
        _ => None,
    };
    for stage in head.into_iter().chain(&pipeline.stages) {
        for arg in &stage.args {
            expr_texts(&arg.value, texts);
        }
    }
}

fn expr_texts(expr: &Expr, texts: &mut Vec<String>) {
    match expr {
        Expr::Text { v, .. } => texts.push(v.clone()),
        Expr::List { items, .. } => items.iter().for_each(|item| expr_texts(item, texts)),
        Expr::Map { entries, .. } => entries.iter().for_each(|entry| expr_texts(&entry.v, texts)),
        Expr::Sub { p, .. } => pipeline_texts(p, texts),
        Expr::Bin { l, r, .. } => {
            expr_texts(l, texts);
            expr_texts(r, texts);
        }
        Expr::Un { x, .. } | Expr::Spread { x, .. } => expr_texts(x, texts),
        _ => {}
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

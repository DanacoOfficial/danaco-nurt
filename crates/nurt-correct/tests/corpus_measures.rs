// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Measures of the correction corpora at the front part of correction: how
//! many cases the lexical rules decide alone, how many they leave to the
//! binder, and whether any case is corrected where the expected reaction is
//! no correction, an ambiguity or another reading.
//!
//! An expected `E_AMBIGUOUS` is raised by the policy after the front part
//! has repaired the text, so a repair alone is not counted against it.
//! Cases of `bash/` and `powershell/` are decided by the translation of
//! habits and count as deferred.
//!
//! Cases whose expected result needs the binder (rules `L01`, `L02`, `L05`,
//! `L06`, `L19`, `L22` to `L24`, `T`, `V`, `B` and `P`, and errors raised by
//! binding) count as deferred. Measures that include binding belong to the
//! binder tests.

mod lexical_support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lexical_support::{Catalog, correction, repository_root, syntax};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_correct::lexical::front;
use nurt_correct::tables::load_embedded;
use nurt_syntax::parse::{ParseOptions, parse};
use nurt_syntax::print::print;
use serde_json::Value;

/// Corpus directory, relative to the repository root.
const CORPORA: &str = "tests/d1/correction";

/// Largest share of `E_AMBIGUOUS` among decided changing programs whose
/// intent is unambiguous.
const MAX_AMB: f64 = 0.05;

/// Rules decided by the front part of correction.
const FRONT_RULES: [&str; 16] = [
    "L03", "L04", "L07", "L08", "L09", "L10", "L11", "L12", "L13", "L14", "L15", "L16", "L17",
    "L18", "L20", "L21",
];

/// Reaction of the front part to one case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Pass,
    Ambiguous,
    Deferred,
    FalseCorrection,
    Miss,
}

#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    pass: usize,
    ambiguous: usize,
    deferred: usize,
    false_corrections: usize,
    misses: usize,
    amb_counted: usize,
    amb_found: usize,
}

impl Tally {
    fn total(&self) -> usize {
        self.pass + self.ambiguous + self.deferred + self.false_corrections + self.misses
    }

    fn add(&mut self, other: &Self) {
        self.pass += other.pass;
        self.ambiguous += other.ambiguous;
        self.deferred += other.deferred;
        self.false_corrections += other.false_corrections;
        self.misses += other.misses;
        self.amb_counted += other.amb_counted;
        self.amb_found += other.amb_found;
    }
}

fn corpus_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            corpus_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path);
        }
    }
}

fn variant_of(case: &Value) -> SyntaxVariant {
    match case["variant"].as_str() {
        Some("b") => SyntaxVariant::B,
        Some("c") => SyntaxVariant::C,
        _ => SyntaxVariant::A,
    }
}

/// Corpus files whose cases are decided by the translation of habits or by
/// the binder, not by the front part alone.
const OTHER_DECIDERS: [&str; 3] = ["bash/", "powershell/", "lexical/bind.jsonl"];

fn needs_binder(case: &Value) -> bool {
    let expect = &case["expect"];
    let rules = expect["rules"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let foreign_rule = rules
        .iter()
        .filter_map(Value::as_str)
        .any(|rule| !FRONT_RULES.contains(&rule));
    let has_context = case["context"].get("config").is_some();
    foreign_rule || has_context
}

struct Setup {
    tables: nurt_correct::tables::Tables,
    catalog: Catalog,
}

impl Setup {
    fn judge(&self, name: &str, case: &Value) -> Outcome {
        let input = case["input"].as_str().unwrap_or_default();
        let variant = variant_of(case);
        let expect = &case["expect"];
        if needs_binder(case) || OTHER_DECIDERS.iter().any(|prefix| name.starts_with(prefix)) {
            return Outcome::Deferred;
        }
        let result = front(
            input,
            &syntax(variant),
            &correction(),
            &self.tables,
            &self.catalog,
        );
        let expected_error = expect["error"].as_str();
        match result {
            Ok(front) => {
                let corrected = !front.corrections.is_empty();
                if expect["unchanged"] == true {
                    return if corrected {
                        Outcome::FalseCorrection
                    } else {
                        Outcome::Pass
                    };
                }
                if let Some(canonical) = expect["canonical"].as_str() {
                    let expected = parse(canonical, SyntaxVariant::A, &ParseOptions::default())
                        .map(|document| print(&document, SyntaxVariant::A))
                        .unwrap_or_else(|_| canonical.to_owned());
                    return if print(&front.document, SyntaxVariant::A) == expected {
                        Outcome::Pass
                    } else if corrected {
                        Outcome::FalseCorrection
                    } else {
                        Outcome::Miss
                    };
                }
                Outcome::Deferred
            }
            Err(error) => {
                if expected_error == Some(error.code().as_str()) {
                    if error.code() == ErrorCode::Ambiguous {
                        Outcome::Ambiguous
                    } else {
                        Outcome::Pass
                    }
                } else if expect["unchanged"] == true || expect["canonical"].is_string() {
                    Outcome::Miss
                } else {
                    Outcome::Deferred
                }
            }
        }
    }
}

fn measure() -> (BTreeMap<String, Tally>, Vec<String>) {
    let setup = Setup {
        tables: load_embedded().unwrap(),
        catalog: Catalog::load(),
    };
    let root = repository_root().join(CORPORA);
    let mut files = Vec::new();
    corpus_files(&root, &mut files);
    files.sort();
    let mut table = BTreeMap::new();
    let mut false_corrections = Vec::new();
    for file in files {
        let name = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let mut tally = Tally::default();
        for line in fs::read_to_string(&file).unwrap().lines() {
            let case: Value = serde_json::from_str(line).unwrap();
            let outcome = setup.judge(&name, &case);
            match outcome {
                Outcome::Pass => tally.pass += 1,
                Outcome::Ambiguous => tally.ambiguous += 1,
                Outcome::Deferred => tally.deferred += 1,
                Outcome::Miss => tally.misses += 1,
                Outcome::FalseCorrection => {
                    tally.false_corrections += 1;
                    false_corrections.push(format!(
                        "{name}: {}",
                        case["id"].as_str().unwrap_or_default()
                    ));
                }
            }
            if case["program_kind"] == "effect"
                && case["intent"] == "unambiguous"
                && outcome != Outcome::Deferred
            {
                tally.amb_counted += 1;
                if outcome == Outcome::Ambiguous {
                    tally.amb_found += 1;
                }
            }
        }
        table.insert(name, tally);
    }
    (table, false_corrections)
}

#[test]
fn front_part_corrects_no_case_wrongly() {
    let (table, false_corrections) = measure();
    let mut all = Tally::default();
    for (name, tally) in &table {
        eprintln!(
            "{name}: {} cases, {} pass, {} ambiguous, {} deferred, {} misses, {} false corrections",
            tally.total(),
            tally.pass,
            tally.ambiguous,
            tally.deferred,
            tally.misses,
            tally.false_corrections
        );
        all.add(tally);
    }
    eprintln!(
        "all: {} cases, {} pass, {} ambiguous, {} deferred, {} misses, {} false corrections",
        all.total(),
        all.pass,
        all.ambiguous,
        all.deferred,
        all.misses,
        all.false_corrections
    );
    assert!(all.total() > 0);
    assert!(false_corrections.is_empty(), "{false_corrections:#?}");
}

#[test]
fn front_part_keeps_ambiguity_within_the_limit() {
    let (table, _) = measure();
    let mut all = Tally::default();
    for tally in table.values() {
        all.add(tally);
    }
    let share = if all.amb_counted == 0 {
        0.0
    } else {
        all.amb_found as f64 / all.amb_counted as f64
    };
    eprintln!(
        "AMB: {}/{} ({:.1}%)",
        all.amb_found,
        all.amb_counted,
        100.0 * share
    );
    assert!(share <= MAX_AMB, "AMB {share:.3}");
}

#[test]
fn every_corpus_file_is_measured() {
    let (table, _) = measure();
    for name in [
        "adversarial/manual.jsonl",
        "adversarial/generated.jsonl",
        "mutate/generated.jsonl",
    ] {
        assert!(
            table.get(name).is_some_and(|tally| tally.total() > 0),
            "{name}"
        );
    }
}

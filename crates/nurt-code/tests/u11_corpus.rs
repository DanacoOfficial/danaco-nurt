// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-code/tests/u11_corpus.rs
// Responsibility:   Snapshot tests over the sample corpus in `tests/d5/code`.
// Dependencies:     nurt_contracts, nurt_testkit, serde_json, u11_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Snapshot tests over the sample corpus in `tests/d5/code`.
//!
//! | Directory | Cases | Snapshot |
//! |---|---|---|
//! | `symbols/<language>` | at least ten samples of every language | symbols in source order |
//! | `outline/<language>` | one nested sample of every language | outline at depth 0 and in full, qualified lookups |
//! | `local/<language>` | samples of the languages with scope rules | definitions with their uses, unresolved names, calls, syntax issues |
//!
//! The language of a sample is detected from its file name. Snapshots are
//! written with `NURT_BLESS=1`.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u11_support;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use nurt_code::local::{LocalAnalysis, OccurrenceKind, analyze};
use nurt_code::outline::{find, outline};
use nurt_code::registry::Registry;
use nurt_code::symbols::symbols_of;
use nurt_code::syntax_check::check_syntax;
use nurt_contracts::services::LanguageId;
use nurt_testkit::golden::{GoldenCase, GoldenError, GoldenRunner};
use serde_json::{Value, json};

use u11_support::{corpus_dir, position, symbol_line};

/// Smallest number of symbol samples per language.
const MIN_SYMBOL_SAMPLES: usize = 10;

/// Languages whose scope rules are covered by the local analysis corpus.
const LOCAL_LANGUAGES: &[&str] = &[
    "c",
    "cpp",
    "csharp",
    "go",
    "java",
    "javascript",
    "kotlin",
    "php",
    "python",
    "rust",
    "swift",
    "typescript",
];

/// Languages with at least three local analysis samples.
const LOCAL_LANGUAGES_IN_DEPTH: &[&str] = &["csharp", "go", "java", "python", "rust", "typescript"];

fn registry() -> &'static Registry {
    Registry::builtin()
}

fn sample(case: &GoldenCase) -> (Vec<u8>, LanguageId) {
    let bytes = fs::read(&case.input_path).unwrap();
    let language = registry()
        .detect(Some(&case.input_path), &bytes)
        .unwrap_or_else(|| panic!("no language for {}", case.input_path.display()));
    (bytes, language)
}

fn language_dirs(kind: &str) -> Vec<(String, std::path::PathBuf)> {
    let mut dirs: Vec<_> = fs::read_dir(corpus_dir(kind))
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .collect();
    dirs.sort();
    dirs
}

/// Runs `produce` over every language directory of `kind` and fails with
/// the differences of every failed language.
fn check_snapshots(kind: &str, produce: impl Fn(&GoldenCase) -> Value) {
    let mut failures = Vec::new();
    for (name, dir) in language_dirs(kind) {
        match GoldenRunner::new(&dir).run(&produce) {
            Ok(_) => {}
            Err(GoldenError::Mismatch { failures: cases }) => {
                for case in cases {
                    failures.push(format!(
                        "{kind}/{name}/{}: {}",
                        case.name,
                        case.differences.join("; ")
                    ));
                }
            }
            Err(error) => failures.push(format!("{kind}/{name}: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} snapshot(s) differ; set NURT_BLESS=1 to accept:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn symbols_snapshot(case: &GoldenCase) -> Value {
    let (bytes, language) = sample(case);
    let symbols = symbols_of(registry(), &bytes, &language).unwrap();
    json!(symbols.iter().map(symbol_line).collect::<Vec<_>>())
}

fn outline_snapshot(case: &GoldenCase) -> Value {
    let (bytes, language) = sample(case);
    let top = outline(registry(), &bytes, &language, Some(0)).unwrap();
    let full = outline(registry(), &bytes, &language, None).unwrap();
    let lookups: Vec<String> = full
        .iter()
        .filter_map(|info| {
            let container = info.container.as_ref()?;
            let qualified = format!("{container}.{}", info.name);
            let found = find(registry(), &bytes, &language, &qualified).unwrap();
            Some(format!("{qualified} -> {}", found.len()))
        })
        .collect();
    json!({
        "depth_0": top.iter().map(symbol_line).collect::<Vec<_>>(),
        "outline": full.iter().map(symbol_line).collect::<Vec<_>>(),
        "lookups": lookups,
    })
}

fn kind_text(kind: OccurrenceKind) -> &'static str {
    match kind {
        OccurrenceKind::Symbol => "symbol",
        OccurrenceKind::Local => "local",
        OccurrenceKind::Import => "import",
        OccurrenceKind::Use => "use",
    }
}

fn definitions(analysis: &LocalAnalysis<'_>) -> Vec<String> {
    analysis
        .occurrences()
        .iter()
        .filter(|occurrence| occurrence.is_definition())
        .map(|occurrence| {
            let uses: Vec<String> = analysis
                .bound_to(occurrence)
                .into_iter()
                .filter(|other| other.span != occurrence.span)
                .map(|other| position(other.span.start))
                .collect();
            format!(
                "{} {} @{} -> [{}]",
                kind_text(occurrence.kind),
                occurrence.name,
                position(occurrence.span.start),
                uses.join(", ")
            )
        })
        .collect()
}

fn unresolved(analysis: &LocalAnalysis<'_>) -> Vec<String> {
    let names: BTreeSet<String> = analysis
        .occurrences()
        .iter()
        .filter(|occurrence| {
            occurrence.kind == OccurrenceKind::Use
                && occurrence.binding.is_none()
                && !occurrence.member
        })
        .map(|occurrence| occurrence.name.clone())
        .collect();
    names.into_iter().collect()
}

fn calls(analysis: &LocalAnalysis<'_>) -> Vec<String> {
    analysis
        .calls()
        .iter()
        .map(|call| {
            let caller = call
                .caller
                .and_then(|index| analysis.symbols().get(index))
                .map_or("-", |symbol| symbol.name.as_str());
            format!("{caller} -> {} @{}", call.callee, position(call.span.start))
        })
        .collect()
}

fn local_snapshot(case: &GoldenCase) -> Value {
    let (bytes, language) = sample(case);
    let analysis = analyze(registry(), &bytes, &language).unwrap();
    let issues: Vec<String> = check_syntax(registry(), &bytes, &language)
        .unwrap()
        .iter()
        .map(|issue| format!("{} @{}", issue.message, position(issue.span.start)))
        .collect();
    json!({
        "definitions": definitions(&analysis),
        "unresolved": unresolved(&analysis),
        "calls": calls(&analysis),
        "syntax": issues,
    })
}

#[test]
fn every_language_has_ten_symbol_samples() {
    let root = corpus_dir("symbols");
    for spec in registry().specs() {
        let dir = root.join(spec.id.as_str());
        let cases = GoldenRunner::new(&dir).bless(false).cases().unwrap();
        assert!(
            cases.len() >= MIN_SYMBOL_SAMPLES,
            "{} has {} symbol samples",
            spec.id,
            cases.len()
        );
        for case in &cases {
            let (_, language) = sample(case);
            assert_eq!(language, spec.id, "{}", case.input_path.display());
        }
    }
}

#[test]
fn every_language_has_an_outline_sample() {
    let root = corpus_dir("outline");
    for spec in registry().specs() {
        let cases = GoldenRunner::new(root.join(spec.id.as_str()))
            .bless(false)
            .cases()
            .unwrap();
        assert!(!cases.is_empty(), "{} has no outline sample", spec.id);
    }
}

#[test]
fn local_corpus_covers_the_languages_with_scope_rules() {
    let root = corpus_dir("local");
    for id in LOCAL_LANGUAGES {
        let cases = GoldenRunner::new(root.join(id))
            .bless(false)
            .cases()
            .unwrap();
        let minimum = if LOCAL_LANGUAGES_IN_DEPTH.contains(id) {
            3
        } else {
            1
        };
        assert!(cases.len() >= minimum, "{id} has {} samples", cases.len());
    }
}

#[test]
fn samples_of_programming_languages_define_symbols() {
    for (name, dir) in language_dirs("symbols") {
        let spec = registry().spec(&u11_support::language(&name)).unwrap();
        if !spec.programming {
            continue;
        }
        for case in GoldenRunner::new(&dir).bless(false).cases().unwrap() {
            let (bytes, language) = sample(&case);
            let symbols = symbols_of(registry(), &bytes, &language).unwrap();
            assert!(!symbols.is_empty(), "{name}/{} defines nothing", case.name);
        }
    }
}

#[test]
fn symbol_samples_parse_without_syntax_errors() {
    for (name, dir) in language_dirs("symbols") {
        for case in GoldenRunner::new(&dir).bless(false).cases().unwrap() {
            let (bytes, language) = sample(&case);
            let issues = check_syntax(registry(), &bytes, &language).unwrap();
            assert!(
                issues.is_empty(),
                "{name}/{}: {:?}",
                case.name,
                issues
                    .iter()
                    .map(|issue| format!("{} @{}", issue.message, position(issue.span.start)))
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn symbols_match_snapshots() {
    check_snapshots("symbols", symbols_snapshot);
}

#[test]
fn outlines_match_snapshots() {
    check_snapshots("outline", outline_snapshot);
}

#[test]
fn local_analysis_matches_snapshots() {
    check_snapshots("local", local_snapshot);
}

#[test]
fn symbol_extraction_is_deterministic() {
    let dir = corpus_dir("symbols").join("rust");
    for case in GoldenRunner::new(&dir).bless(false).cases().unwrap() {
        let first = symbols_snapshot(&case);
        let second = symbols_snapshot(&case);
        assert_eq!(first, second, "{}", Path::new(&case.name).display());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

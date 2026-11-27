// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-bind/tests/corpus_support/mod.rs
// Responsibility:   Runner of the correction corpora through the full pipeline: reading with lexical correction, binding with correction, the invariants and the policy.
// Dependencies:     nurt_contracts, nurt_correct, nurt_syntax, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Runner of the correction corpora through the full pipeline: reading with
//! lexical correction, binding with correction, the invariants and the
//! policy. The format of the corpora is described in
//! `tests/d1/correction/format.md`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use nurt_bind::{BindContext, Catalog, Outcome, bind, bind_with_correction};
use nurt_contracts::config::Config;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::spec::Os;
use nurt_correct::policy::{Class, InvariantInput, check_invariants, is_effecting};
use nurt_correct::tables::{Tables, load_embedded};
use nurt_syntax::hash::ast_hash;
use nurt_syntax::parse::{ParseOptions, parse};
use nurt_syntax::print::try_print;
use serde_json::Value;

/// Root of the workspace.
pub(crate) fn workspace() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("..")
    .join("..")
}

/// The reference catalog, read once.
pub(crate) fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| Catalog::load_dir(&workspace().join("catalog")).unwrap())
}

/// The embedded correction tables, read once.
pub(crate) fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| load_embedded().unwrap())
}

/// One case of a corpus.
pub(crate) struct Case {
    pub(crate) file: String,
    pub(crate) id: String,
    pub(crate) input: String,
    pub(crate) variant: SyntaxVariant,
    pub(crate) context: Value,
    pub(crate) expect: Value,
    pub(crate) effect: bool,
    pub(crate) unambiguous: bool,
}

/// Measures of a corpus run.
#[derive(Default)]
pub(crate) struct Measures {
    pub(crate) cases: usize,
    pub(crate) passed: usize,
    pub(crate) failures: Vec<String>,
    pub(crate) false_corrections: Vec<String>,
    pub(crate) violations: Vec<String>,
    pub(crate) effect_unambiguous: usize,
    pub(crate) ambiguous_effects: usize,
    pub(crate) incorrect_inputs: usize,
    pub(crate) fixed: usize,
    pub(crate) unchanged: usize,
    pub(crate) kept: usize,
}

impl Measures {
    /// Share of `E_AMBIGUOUS` among unambiguous programs that change the
    /// world.
    pub(crate) fn ambiguity_share(&self) -> f64 {
        ratio(self.ambiguous_effects, self.effect_unambiguous)
    }

    /// Share of incorrect inputs accepted as expected.
    pub(crate) fn fixed_share(&self) -> f64 {
        ratio(self.fixed, self.incorrect_inputs)
    }

    /// One line with every measure.
    pub(crate) fn summary(&self) -> String {
        format!(
            "cases {}, passed {}, FCR {}, violations {}, AMB {}/{} ({:.1}%), FC {}/{} ({:.1}%), FV {}/{}",
            self.cases,
            self.passed,
            self.false_corrections.len(),
            self.violations.len(),
            self.ambiguous_effects,
            self.effect_unambiguous,
            self.ambiguity_share() * 100.0,
            self.fixed,
            self.incorrect_inputs,
            self.fixed_share() * 100.0,
            self.kept,
            self.unchanged
        )
    }

    /// The summary followed by the listed problems, at most `limit` of each
    /// kind.
    pub(crate) fn report(&self, limit: usize) -> String {
        let mut text = self.summary();
        for (title, items) in [
            ("false corrections", &self.false_corrections),
            ("violations", &self.violations),
            ("failures", &self.failures),
        ] {
            if items.is_empty() {
                continue;
            }
            let _ = write!(text, "\n{title}:");
            for item in items.iter().take(limit) {
                let _ = write!(text, "\n  {item}");
            }
        }
        text
    }
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        f64::from(u32::try_from(part).unwrap()) / f64::from(u32::try_from(whole).unwrap())
    }
}

/// Reads every `*.jsonl` file under a directory, in path order.
pub(crate) fn read_cases(dir: &Path) -> Vec<Case> {
    let mut files = Vec::new();
    collect(dir, &mut files);
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let name = file
            .strip_prefix(workspace())
            .unwrap_or(&file)
            .display()
            .to_string();
        for line in fs::read_to_string(&file).unwrap().lines() {
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("{name}: {error}: {line}"));
            cases.push(case(&name, &value));
        }
    }
    cases
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, files);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            files.push(path);
        }
    }
}

fn case(file: &str, value: &Value) -> Case {
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
    Case {
        file: file.to_owned(),
        id: text("id"),
        input: text("input"),
        variant: variant(value["variant"].as_str().unwrap_or("a")),
        context: value["context"].clone(),
        expect: value["expect"].clone(),
        effect: value["program_kind"] == "effect",
        unambiguous: value["intent"] == "unambiguous",
    }
}

fn variant(name: &str) -> SyntaxVariant {
    match name {
        "b" => SyntaxVariant::B,
        "c" => SyntaxVariant::C,
        _ => SyntaxVariant::A,
    }
}

/// Configuration of a case: the defaults with the overrides of
/// `context.config`.
fn config(case: &Case) -> Config {
    let mut config = Config::default();
    if let Some(overrides) = case.context["config"].as_object() {
        for (key, value) in overrides {
            let number = || u32::try_from(value.as_u64().unwrap_or_default()).unwrap_or_default();
            match key.as_str() {
                "correction.enabled" => config.correction.enabled = value.as_bool().unwrap_or(true),
                "correction.max_read" => config.correction.max_read = number(),
                "correction.max_effect" => config.correction.max_effect = number(),
                "correction.require_first_char" => {
                    config.correction.require_first_char = value.as_bool().unwrap_or(true);
                }
                "syntax.tolerant" => config.syntax.tolerant = value.as_bool().unwrap_or(true),
                _ => {}
            }
        }
    }
    config
}

/// Whether a program is written with a drive letter (`C:\x`, `c:/x`) or a
/// registry path, which only a Windows host accepts. A case without
/// `context.os` that shows such a path runs on a Windows host.
fn windows_notation(input: &str) -> bool {
    if input.contains("registry:/") {
        return true;
    }
    let chars: Vec<char> = input.chars().collect();
    chars.windows(3).enumerate().any(|(index, window)| {
        let alone = index == 0 || !chars[index - 1].is_alphanumeric();
        alone
            && window[0].is_ascii_alphabetic()
            && window[1] == ':'
            && matches!(window[2], '\\' | '/')
    })
}

/// Binding context of a case.
pub(crate) fn context(case: &Case) -> BindContext {
    let os = match case.context["os"].as_str() {
        Some("windows") => Os::Windows,
        Some("macos") => Os::Macos,
        Some(_) => Os::Linux,
        None if windows_notation(&case.input) => Os::Windows,
        None => Os::Linux,
    };
    let mut context = BindContext::new(os);
    if let Some(base) = case.context["base"].as_str() {
        context = context.with_base(base);
    }
    for name in case.context["lets"].as_array().into_iter().flatten() {
        context = context.with_let(name.as_str().unwrap_or_default());
    }
    context
}

/// Canonical text of a program in variant A, or `None` when it does not
/// parse.
fn canonical_a(text: &str) -> Option<String> {
    let document = parse(text, SyntaxVariant::A, &ParseOptions::default()).ok()?;
    try_print(&document, SyntaxVariant::A)
        .ok()
        .map(|printed| printed.trim_end().to_owned())
}

/// Program texts of an expectation compared after printing.
fn programs(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|text| canonical_a(text).unwrap_or_else(|| text.to_owned()))
        .collect()
}

fn class_rank(class: Class) -> u8 {
    match class {
        Class::Certain => 0,
        Class::Structural => 1,
        Class::Fuzzy => 2,
        _ => 3,
    }
}

fn class_name(rank: u8) -> &'static str {
    ["C", "S", "F", "P"][usize::from(rank.min(3))]
}

/// Runs every case and collects the measures.
pub(crate) fn run(cases: &[Case]) -> Measures {
    let mut measures = Measures::default();
    for case in cases {
        let result = bind_with_correction(
            &case.input,
            case.variant,
            catalog(),
            tables(),
            &config(case),
            &context(case),
        );
        judge(case, &result, &mut measures);
    }
    measures
}

fn judge(case: &Case, result: &Result<Outcome, Error>, measures: &mut Measures) {
    measures.cases += 1;
    let at = format!("{} {}", case.file, case.id);
    if case.effect && case.unambiguous {
        measures.effect_unambiguous += 1;
        if matches!(result, Err(error) if error.code() == ErrorCode::Ambiguous) {
            measures.ambiguous_effects += 1;
        }
    }
    match result {
        Err(error) if error.code() == ErrorCode::InternalCorrection => {
            measures.violations.push(format!(
                "{at}: {}",
                error.problem.clone().unwrap_or_default()
            ));
        }
        Ok(outcome) => {
            if let Err(violation) = invariants(case, outcome) {
                measures.violations.push(format!("{at}: {violation}"));
            }
        }
        Err(_) => {}
    }
    let verdict = if case.expect.get("canonical").is_some() {
        measures.incorrect_inputs += 1;
        expect_canonical(case, result, measures)
    } else if case.expect["unchanged"] == true {
        measures.unchanged += 1;
        expect_unchanged(case, result, measures)
    } else {
        expect_error(case, result, measures)
    };
    match verdict {
        Ok(()) => measures.passed += 1,
        Err(problem) => measures.failures.push(format!("{at}: {problem}")),
    }
}

fn invariants(case: &Case, outcome: &Outcome) -> Result<(), String> {
    let context = context(case);
    let effecting = |name: &str| catalog().command(name).is_some_and(is_effecting);
    check_invariants(&InvariantInput {
        before: None,
        after: &outcome.bound,
        corrections: &outcome.corrections,
        base: context.base.as_deref(),
        tables: tables(),
        effecting: &effecting,
    })
    .map_err(|violation| violation.to_string())
}

fn expect_canonical(
    case: &Case,
    result: &Result<Outcome, Error>,
    measures: &mut Measures,
) -> Result<(), String> {
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => return Err(format!("{} {}", error.code(), error.message)),
    };
    let expected_text = case.expect["canonical"].as_str().unwrap_or_default();
    let expected = canonical_a(expected_text).unwrap_or_else(|| expected_text.to_owned());
    let mine = try_print(&outcome.surface, SyntaxVariant::A)
        .map(|text| text.trim_end().to_owned())
        .unwrap_or_default();
    let same_tree = same_bound_tree(case, outcome, &expected);
    if mine != expected || !same_tree {
        if case.unambiguous {
            measures
                .false_corrections
                .push(format!("{} {}: {mine:?}", case.file, case.id));
        }
        return Err(format!("canonical {mine:?}, expected {expected:?}"));
    }
    measures.fixed += 1;
    let rules: Vec<String> = outcome
        .corrections
        .iter()
        .map(|correction| correction.rule.to_string())
        .collect();
    let expected_rules: Vec<String> = programs_raw(&case.expect["rules"]);
    if rules != expected_rules {
        return Err(format!("rules {rules:?}, expected {expected_rules:?}"));
    }
    let class = outcome
        .corrections
        .iter()
        .map(|correction| class_rank(correction.class))
        .max()
        .map(class_name);
    if let Some(expected_class) = case.expect["class"].as_str()
        && class != Some(expected_class)
    {
        return Err(format!("class {class:?}, expected {expected_class}"));
    }
    Ok(())
}

fn programs_raw(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Whether the bound tree equals the bound tree of the expected program
/// read without correction; both trees carry the revision of the tables,
/// which is part of the hash.
fn same_bound_tree(case: &Case, outcome: &Outcome, expected: &str) -> bool {
    let Ok(document) = parse(expected, SyntaxVariant::A, &ParseOptions::default()) else {
        return false;
    };
    let context = context(case).with_corr_rev(outcome.bound.corr_rev.clone());
    match bind(&document, catalog(), &context) {
        Ok(reference) => match (ast_hash(&reference), ast_hash(&outcome.bound)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        },
        Err(_) => false,
    }
}

fn expect_unchanged(
    case: &Case,
    result: &Result<Outcome, Error>,
    measures: &mut Measures,
) -> Result<(), String> {
    match result {
        Ok(outcome) if outcome.corrections.is_empty() => {
            measures.kept += 1;
            Ok(())
        }
        Ok(outcome) => {
            if case.unambiguous {
                measures
                    .false_corrections
                    .push(format!("{} {}: {}", case.file, case.id, outcome.canonical));
            }
            Err(format!("corrected to {:?}", outcome.canonical))
        }
        Err(error) => Err(format!("{} {}", error.code(), error.message)),
    }
}

fn expect_error(
    case: &Case,
    result: &Result<Outcome, Error>,
    measures: &mut Measures,
) -> Result<(), String> {
    let code = case.expect["error"].as_str().unwrap_or_default();
    let error = match result {
        Err(error) => error,
        Ok(outcome) => {
            if case.unambiguous {
                measures
                    .false_corrections
                    .push(format!("{} {}: {}", case.file, case.id, outcome.canonical));
            }
            return Err(format!(
                "accepted as {:?}, expected {code}",
                outcome.canonical
            ));
        }
    };
    if error.code().to_string() != code {
        return Err(format!(
            "{} {}, expected {code}",
            error.code(),
            error.message
        ));
    }
    let members = [
        (
            "candidates",
            error
                .candidates
                .iter()
                .map(|candidate| candidate.program.clone())
                .collect::<Vec<_>>(),
        ),
        ("fix", error.fix.clone()),
        ("next_steps", error.next_steps.clone()),
    ];
    for (member, found) in members {
        if case.expect.get(member).is_none() {
            continue;
        }
        let expected = programs(&case.expect[member]);
        let found: Vec<String> = found
            .iter()
            .map(|text| canonical_a(text).unwrap_or_else(|| text.clone()))
            .collect();
        if found != expected {
            return Err(format!("{member} {found:?}, expected {expected:?}"));
        }
    }
    Ok(())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

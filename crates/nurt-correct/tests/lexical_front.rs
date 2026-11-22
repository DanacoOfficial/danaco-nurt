// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The front part of correction on the lexical corpus: expected canonical
//! text, rules and class, no correction of programs that need none, the
//! correction invariants that a surface tree shows, idempotence, properties
//! and a deterministic mutation run.

mod lexical_support;

use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use lexical_support::{Case, Catalog, correction, load_cases, syntax};
use nurt_contracts::ast::{AstDocument, Expr, Head, Pipeline, Stage, Statement};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::CorrectionClass;
use nurt_correct::lexical::front::MAX_ALTERNATIVES;
use nurt_correct::lexical::{CatalogView, Front, Rule, front};
use nurt_correct::policy::SELECTING_STAGES;
use nurt_correct::tables::{Tables, load_embedded};
use nurt_syntax::parse::{ParseOptions, parse};
use nurt_syntax::print::{print, try_print};
use proptest::prelude::*;

/// Corpus files whose cases the front part decides alone; the cases of
/// `bind.jsonl` need the binder.
const FRONT_FILES: [&str; 2] = ["tokens.jsonl", "repairs.jsonl"];

/// Rules applied by the front part.
const FRONT_RULES: [&str; 16] = [
    "L03", "L04", "L07", "L08", "L09", "L10", "L11", "L12", "L13", "L14", "L15", "L16", "L17",
    "L18", "L20", "L21",
];

/// Least number of mistake cases decided by the front part.
const MIN_FRONT_CASES: usize = 150;

/// Least share of decided cases, in percent, among all mistake cases.
const MIN_FIXED_PERCENT: usize = 40;

/// Number of mutated inputs of the long mutation run.
const MUTATIONS: usize = 100_000;

/// Number of mutated inputs of the mutation run of every test run.
const QUICK_MUTATIONS: usize = 3_000;

/// Time limit of reading one input in the mutation run and the property on
/// any text; a debug build reads a typical input in well under a
/// millisecond.
const INPUT_TIMEOUT: Duration = Duration::from_secs(2);

/// Every how many mutated inputs the result is read again.
const IDEMPOTENCE_STRIDE: usize = 10;

/// Cases of each property in every test run; the variable `PROPTEST_CASES`
/// does not change it.
const PROPERTY_CASES: u32 = 1_000;

/// Time limit of one property case, in milliseconds; a longer case fails.
const PROPERTY_TIMEOUT_MS: u32 = 5_000;

/// Time limit of the search for a smaller failing case, in milliseconds.
const PROPERTY_SHRINK_MS: u32 = 60_000;

/// Characters inserted by the mutation run.
const INSERTED: [char; 22] = [
    '"', '\'', '|', '>', '(', ')', '[', ']', '{', '}', ',', ':', '=', ' ', '\n', '\u{a0}', '-',
    '/', '\\', '#', '\u{201e}', '\u{201d}',
];

/// Multiplier and increment of the linear congruential generator.
const LCG: (u64, u64) = (6_364_136_223_846_793_005, 1_442_695_040_888_963_407);

struct Setup {
    tables: Tables,
    catalog: Catalog,
}

impl Setup {
    fn new() -> Self {
        Self {
            tables: load_embedded().expect("embedded tables"),
            catalog: Catalog::load(),
        }
    }

    fn run(
        &self,
        text: &str,
        variant: SyntaxVariant,
    ) -> Result<Front, nurt_contracts::error::Error> {
        front(
            text,
            &syntax(variant),
            &correction(),
            &self.tables,
            &self.catalog,
        )
    }
}

fn canonical_of(text: &str) -> Option<String> {
    parse(text, SyntaxVariant::A, &ParseOptions::default())
        .ok()
        .map(|document| print(&document, SyntaxVariant::A))
}

fn rule_ids(result: &Front) -> Vec<String> {
    result
        .corrections
        .iter()
        .map(|correction| correction.rule.to_string())
        .collect()
}

fn least_certain(result: &Front) -> Option<CorrectionClass> {
    let rank = |class: &CorrectionClass| {
        ["C", "S", "F", "P"]
            .iter()
            .position(|code| *code == class.as_str())
    };
    result
        .corrections
        .iter()
        .map(|correction| correction.class)
        .max_by_key(|class| rank(class))
}

fn is_front_case(case: &Case) -> bool {
    let rules = case.rules();
    FRONT_FILES.contains(&case.file.as_str())
        && !case.adversarial
        && !case.has_context()
        && !rules.is_empty()
        && rules
            .iter()
            .all(|rule| FRONT_RULES.contains(&rule.as_str()))
}

/// Problems of a result against a case that expects a canonical text.
fn canonical_problems(case: &Case, result: &Front) -> Option<String> {
    let expected = canonical_of(case.canonical()?)?;
    let printed = print(&result.document, SyntaxVariant::A);
    let rules = rule_ids(result);
    let class = least_certain(result).map(|class| class.as_str().to_owned());
    let expected_class = case.expect().get("class").and_then(|value| value.as_str());
    let matches =
        printed == expected && rules == case.rules() && class.as_deref() == expected_class;
    (!matches).then(|| format!("{printed:?} {rules:?} {class:?}"))
}

#[test]
fn front_gives_the_expected_canonical_text_rules_and_class() {
    let setup = Setup::new();
    let cases = load_cases();
    let mistakes = cases.iter().filter(|case| !case.adversarial).count();
    let mut decided = 0;
    let mut failures = Vec::new();
    for case in cases.iter().filter(|case| is_front_case(case)) {
        decided += 1;
        match setup.run(case.text("input"), case.variant()) {
            Ok(result) => {
                if let Some(problem) = canonical_problems(case, &result) {
                    failures.push(format!("{}: {problem}", case.location));
                }
            }
            Err(error) => failures.push(format!("{}: {}", case.location, error.message)),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(decided >= MIN_FRONT_CASES, "{decided} cases decided");
    assert!(
        decided * 100 >= mistakes * MIN_FIXED_PERCENT,
        "{decided} of {mistakes} mistake cases fixed by the front part"
    );
}

#[test]
fn cases_without_correction_get_none() {
    let setup = Setup::new();
    let mut failures = Vec::new();
    for case in load_cases().iter().filter(|case| case.unchanged()) {
        match setup.run(case.text("input"), case.variant()) {
            Ok(result) if result.corrections.is_empty() => {}
            Ok(result) => failures.push(format!("{}: {:?}", case.location, rule_ids(&result))),
            Err(error) => failures.push(format!("{}: {}", case.location, error.message)),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn every_case_keeps_the_invariants_visible_in_the_tree() {
    let setup = Setup::new();
    let mut failures = Vec::new();
    for case in load_cases() {
        let input = case.text("input");
        let Ok(result) = setup.run(input, case.variant()) else {
            continue;
        };
        if let Some(problem) = invariant_problem(input, &result, &setup.catalog) {
            failures.push(format!("{}: {problem}", case.location));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn front_makes_no_correction_on_its_own_result() {
    let setup = Setup::new();
    let mut failures = Vec::new();
    for case in load_cases() {
        let variant = case.variant();
        let Ok(result) = setup.run(case.text("input"), variant) else {
            continue;
        };
        let Ok(printed) = try_print(&result.document, variant) else {
            continue;
        };
        match setup.run(&printed, variant) {
            Ok(again) if again.corrections.is_empty() => {}
            Ok(again) => failures.push(format!("{}: {:?}", case.location, rule_ids(&again))),
            Err(error) => failures.push(format!("{}: {}", case.location, error.message)),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn corrections_are_numbered_in_order_of_position() {
    let setup = Setup::new();
    for case in load_cases() {
        let Ok(result) = setup.run(case.text("input"), case.variant()) else {
            continue;
        };
        let positions: Vec<_> = result
            .corrections
            .iter()
            .map(|correction| correction.at.map(|at| (at.line, at.column)))
            .collect();
        let mut sorted = positions.clone();
        sorted.sort();
        assert_eq!(positions, sorted, "{}", case.location);
        for (index, correction) in result.corrections.iter().enumerate() {
            assert_eq!(
                correction.id,
                format!("C{}", index + 1),
                "{}",
                case.location
            );
        }
    }
}

#[test]
fn every_rule_records_the_class_of_the_rule_table() {
    let setup = Setup::new();
    for case in load_cases() {
        let Ok(result) = setup.run(case.text("input"), case.variant()) else {
            continue;
        };
        for correction in &result.corrections {
            let rule = Rule::ALL
                .iter()
                .find(|rule| rule.id() == correction.rule.to_string());
            if let Some(rule) = rule {
                assert_eq!(rule.class(), correction.class, "{}", case.location);
            }
        }
    }
}

#[test]
fn mutated_corpus_inputs_never_panic_and_stay_stable() {
    run_mutations(QUICK_MUTATIONS);
}

#[test]
#[ignore = "long run of the mutation test; run in a release build"]
fn a_hundred_thousand_mutated_inputs_never_panic_and_stay_stable() {
    run_mutations(MUTATIONS);
}

/// Messages of the worker thread of a mutation run.
enum Progress {
    /// The worker starts to read this input in this variant.
    Reading(String, SyntaxVariant),
    /// The worker has finished with these failures.
    Done(Vec<String>),
}

/// Runs the mutation test on a worker thread that announces every input
/// before reading it; an input read longer than [`INPUT_TIMEOUT`] fails the
/// test with the input and the variant.
fn run_mutations(count: usize) {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let failures = mutation_failures(count, &sender);
        let _ = sender.send(Progress::Done(failures));
    });
    let mut current = None;
    loop {
        match receiver.recv_timeout(INPUT_TIMEOUT) {
            Ok(Progress::Reading(input, variant)) => current = Some((input, variant)),
            Ok(Progress::Done(failures)) => {
                assert!(
                    failures.is_empty(),
                    "{:#?}",
                    &failures[..failures.len().min(20)]
                );
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                panic!("input read longer than {INPUT_TIMEOUT:?}: {current:?}")
            }
            Err(RecvTimeoutError::Disconnected) => panic!("worker panicked after {current:?}"),
        }
    }
}

fn mutation_failures(count: usize, progress: &mpsc::Sender<Progress>) -> Vec<String> {
    let setup = Setup::new();
    let seeds: Vec<String> = load_cases()
        .iter()
        .map(|case| case.text("input").to_owned())
        .collect();
    let variants = [SyntaxVariant::A, SyntaxVariant::B, SyntaxVariant::C];
    let mut state = 0x5eed_u64;
    let mut failures = Vec::new();
    let announce = |input: &str, variant: SyntaxVariant| {
        let _ = progress.send(Progress::Reading(input.to_owned(), variant));
    };
    for round in 0..count {
        let seed = &seeds[round % seeds.len()];
        let mutated = mutate(seed, &mut state);
        let variant = variants[round % variants.len()];
        announce(&mutated, variant);
        let Ok(result) = setup.run(&mutated, variant) else {
            continue;
        };
        let lines = u32::try_from(mutated.split('\n').count()).unwrap_or(u32::MAX);
        let outside = result.corrections.iter().any(|correction| {
            correction
                .at
                .is_some_and(|at| at.line == 0 || at.line > lines)
        });
        if outside {
            failures.push(format!("position outside the text: {mutated:?}"));
        }
        if let Some(problem) = invariant_problem(&mutated, &result, &setup.catalog) {
            failures.push(format!("{mutated:?}: {problem}"));
        }
        if round % IDEMPOTENCE_STRIDE == 0
            && let Ok(printed) = try_print(&result.document, variant)
        {
            announce(&printed, variant);
            let again = setup.run(&printed, variant);
            if !again.is_ok_and(|again| again.corrections.is_empty()) {
                failures.push(format!("not idempotent: {mutated:?}"));
            }
        }
    }
    failures
}

/// Tables and catalog shared by the threads of the property on any text.
static SHARED: LazyLock<(Tables, Catalog)> =
    LazyLock::new(|| (load_embedded().expect("embedded tables"), Catalog::load()));

/// Number of alternatives of `front` on `text`, `None` for an error; reads
/// on a worker thread and fails the test with the input when the reading
/// takes longer than [`INPUT_TIMEOUT`] or panics.
fn alternatives_in_time(text: &str) -> Option<usize> {
    let (sender, receiver) = mpsc::channel();
    let input = text.to_owned();
    thread::spawn(move || {
        let (tables, catalog) = &*SHARED;
        let result = front(
            &input,
            &syntax(SyntaxVariant::A),
            &correction(),
            tables,
            catalog,
        );
        let _ = sender.send(result.ok().map(|result| result.alternatives.len()));
    });
    match receiver.recv_timeout(INPUT_TIMEOUT) {
        Ok(alternatives) => alternatives,
        Err(RecvTimeoutError::Timeout) => {
            panic!("input read longer than {INPUT_TIMEOUT:?}: {text:?}")
        }
        Err(RecvTimeoutError::Disconnected) => panic!("front panicked on {text:?}"),
    }
}

proptest! {
    // A failing case already names its input; shrinking would start more
    // readings that do not end.
    #![proptest_config(ProptestConfig {
        cases: PROPERTY_CASES,
        max_shrink_iters: 0,
        ..ProptestConfig::default()
    })]

    #[test]
    fn front_accepts_any_text_without_panic(text in "\\PC{0,48}") {
        if let Some(alternatives) = alternatives_in_time(&text) {
            prop_assert!(alternatives <= MAX_ALTERNATIVES);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: PROPERTY_CASES,
        timeout: PROPERTY_TIMEOUT_MS,
        max_shrink_time: PROPERTY_SHRINK_MS,
        ..ProptestConfig::default()
    })]

    #[test]
    fn valid_programs_get_no_correction_and_the_strict_tree(program in valid_program()) {
        let (tables, catalog) = &*SHARED;
        let strict = parse(&program, SyntaxVariant::A, &ParseOptions::default());
        prop_assume!(strict.is_ok());
        let result = front(&program, &syntax(SyntaxVariant::A), &correction(), tables, catalog)
            .expect("valid program");
        prop_assert!(result.corrections.is_empty());
        prop_assert_eq!(Some(result.document), strict.ok());
    }
}

/// Programs of variant A built from stages that parse.
fn valid_program() -> impl Strategy<Value = String> {
    let heads = prop::sample::select(vec![
        "find \"/srv/app\"",
        "list \"/var/log\" hidden: true",
        "search /Pending/i \"/srv\"",
        "find \"/a\" glob: \"*.go\" depth: 2",
        "read \"/etc/hosts\"",
    ]);
    let stages = prop::collection::vec(
        prop::sample::select(vec![
            "count",
            "first 5",
            "filter size > 1mb",
            "filter name == \"a -> b\"",
            "sort name",
            "filter modified > ago(24h) and not hidden",
        ]),
        0..4,
    );
    (heads, stages, any::<bool>()).prop_map(|(head, stages, lines)| {
        let separator = if lines { "\n|> " } else { " |> " };
        std::iter::once(head)
            .chain(stages)
            .collect::<Vec<_>>()
            .join(separator)
    })
}

/// One random edit of `text`: deletion, duplication, swap, insertion or a
/// change of letter case.
fn mutate(text: &str, state: &mut u64) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    let position = next(state, chars.len() + 1);
    match next(state, 5) {
        0 if position < chars.len() => {
            chars.remove(position);
        }
        1 if position < chars.len() => {
            let c = chars[position];
            chars.insert(position, c);
        }
        2 if position + 1 < chars.len() => chars.swap(position, position + 1),
        3 if position < chars.len() => {
            let c = chars[position];
            chars[position] = if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            };
        }
        _ => chars.insert(position, INSERTED[next(state, INSERTED.len())]),
    }
    chars.into_iter().collect()
}

fn next(state: &mut u64, bound: usize) -> usize {
    *state = state.wrapping_mul(LCG.0).wrapping_add(LCG.1);
    let high = usize::try_from(*state >> 33).unwrap_or(0);
    if bound == 0 { 0 } else { high % bound }
}

/// Facts of a tree that correction keeps.
#[derive(Default)]
struct Facts {
    texts: Vec<String>,
    patterns: Vec<String>,
    integers: Vec<i64>,
    stages: Vec<String>,
}

fn collect(document: &AstDocument) -> Facts {
    let mut facts = Facts::default();
    for statement in &document.program.statements {
        match statement {
            Statement::Let(binding) => pipeline_facts(&binding.value, &mut facts),
            Statement::Pipeline(pipeline) => pipeline_facts(pipeline, &mut facts),
            _ => {}
        }
    }
    facts
}

fn pipeline_facts(pipeline: &Pipeline, facts: &mut Facts) {
    if let Head::Stage(stage) = &pipeline.head {
        stage_facts(stage, facts);
    }
    for stage in &pipeline.stages {
        stage_facts(stage, facts);
    }
}

fn stage_facts(stage: &Stage, facts: &mut Facts) {
    facts.stages.push(stage.name.clone());
    for arg in &stage.args {
        expr_facts(&arg.value, facts);
    }
}

fn expr_facts(expr: &Expr, facts: &mut Facts) {
    match expr {
        Expr::Text { v, .. } => facts.texts.push(v.clone()),
        Expr::Regex { v, .. } => facts.patterns.push(v.clone()),
        Expr::Int { v, .. } => facts.integers.push(*v),
        Expr::List { items, .. } => items.iter().for_each(|item| expr_facts(item, facts)),
        Expr::Map { entries, .. } => entries.iter().for_each(|entry| expr_facts(&entry.v, facts)),
        Expr::Call { args, .. } => args.iter().for_each(|arg| expr_facts(arg, facts)),
        Expr::Un { x, .. } | Expr::Spread { x, .. } => expr_facts(x, facts),
        Expr::Bin { l, r, .. } => {
            expr_facts(l, facts);
            expr_facts(r, facts);
        }
        Expr::Sub { p, .. } => pipeline_facts(p, facts),
        _ => {}
    }
}

/// Words of a text in lower case, with `-` read as `_`.
fn word_counts(text: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    let lowered = text.to_ascii_lowercase().replace('-', "_");
    for word in lowered.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')) {
        if !word.is_empty() {
            *counts.entry(word.to_owned()).or_insert(0) += 1;
        }
    }
    counts
}

/// The invariants that the surface tree of the front part shows: literal
/// bytes, integers, the stages that select targets and the changing stages.
fn invariant_problem(input: &str, result: &Front, catalog: &Catalog) -> Option<String> {
    if !result.assumptions.is_empty()
        || result
            .corrections
            .iter()
            .any(|c| c.rule.to_string().starts_with(['B', 'P']))
    {
        return None;
    }
    let facts = collect(&result.document);
    let escaped_text =
        |value: &str| value.contains(['"', '\\']) || value.chars().any(char::is_control);
    let escaped_pattern = |value: &str| value.contains(['/', '\\']);
    let lost = facts
        .texts
        .iter()
        .filter(|text| !escaped_text(text))
        .chain(
            facts
                .patterns
                .iter()
                .filter(|pattern| !escaped_pattern(pattern)),
        )
        .find(|literal| !input.contains(literal.as_str()));
    if let Some(literal) = lost {
        return Some(format!("literal {literal:?} is not in the input"));
    }
    if let Some(number) = facts
        .integers
        .iter()
        .find(|number| !input.contains(&number.to_string()))
    {
        return Some(format!("number {number} is not in the input"));
    }
    let words = word_counts(&result.normalized);
    let mut stages: BTreeMap<&str, usize> = BTreeMap::new();
    for name in &facts.stages {
        *stages.entry(name.as_str()).or_insert(0) += 1;
    }
    for (name, count) in stages {
        let guarded = SELECTING_STAGES.contains(&name) || catalog.is_mutating(name);
        let written = words.get(name).copied().unwrap_or(0);
        if guarded && written < count {
            return Some(format!(
                "stage {name} appears {count} times but is written {written} times"
            ));
        }
    }
    None
}

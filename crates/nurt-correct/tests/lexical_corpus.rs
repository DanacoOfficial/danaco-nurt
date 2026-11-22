// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lexical correction corpus: the format of every case and the result of the
//! character normalization and token rules on the cases they decide alone.
//!
//! Cases whose rules need the binder or the repair hooks of the parser are
//! checked for their format here and for their result by the tests of the
//! whole correction pipeline.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::CorrectionClass;
use nurt_correct::lexical::tokens::{TokenContext, rewrite};
use nurt_correct::lexical::{ArgumentShape, CatalogView, Rule};
use nurt_correct::normalize::normalize;
use nurt_correct::record::Correction;
use nurt_correct::tables::load_embedded;
use serde_json::Value;
use toml_edit::{Document, Item};

/// Rules that run on the program text before parsing.
const TOKEN_RULES: [&str; 7] = ["L03", "L04", "L07", "L13", "L16", "L17", "L21"];

/// Least number of cases of the mistake corpus.
const MIN_MISTAKE_CASES: usize = 250;

/// Least number of cases of the adversarial corpus.
const MIN_ADVERSARIAL_CASES: usize = 80;

/// Least number of mistake cases of each rule.
const MIN_CASES_PER_RULE: usize = 10;

/// Least number of mistake cases decided by the token rules alone.
const MIN_TOKEN_CASES: usize = 80;

/// Members every case has.
const REQUIRED: [&str; 7] = [
    "id",
    "input",
    "variant",
    "expect",
    "program_kind",
    "intent",
    "source",
];

struct Catalog {
    names: Vec<&'static str>,
    mutating: BTreeSet<String>,
}

impl Catalog {
    fn load() -> Self {
        let directory = repository_root().join("catalog");
        let mut names = Vec::new();
        let mut mutating = BTreeSet::new();
        let mut files: Vec<PathBuf> = fs::read_dir(&directory)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        for file in files
            .iter()
            .filter(|file| file.extension().is_some_and(|ext| ext == "toml"))
        {
            let text = fs::read_to_string(file).unwrap_or_default();
            let Ok(document) = Document::parse(text) else {
                continue;
            };
            let Some(commands) = document.get("command").and_then(Item::as_array_of_tables) else {
                continue;
            };
            for command in commands {
                let Some(name) = command.get("name").and_then(Item::as_str) else {
                    continue;
                };
                if command.get("effect").and_then(Item::as_str) != Some("read") {
                    mutating.insert(name.to_owned());
                }
                names.push(&*Box::leak(name.to_owned().into_boxed_str()));
            }
        }
        Self { names, mutating }
    }
}

impl CatalogView for Catalog {
    fn is_command(&self, name: &str) -> bool {
        self.names.contains(&name)
    }

    fn command_names(&self) -> &[&str] {
        &self.names
    }

    fn is_mutating(&self, name: &str) -> bool {
        self.mutating.contains(name)
    }

    fn can_take_argument(&self, _: &str, _: usize, _: ArgumentShape) -> bool {
        false
    }
}

struct Case {
    location: String,
    adversarial: bool,
    value: Value,
}

impl Case {
    fn text(&self, member: &str) -> &str {
        self.value
            .get(member)
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    fn expect(&self) -> &Value {
        self.value.get("expect").unwrap_or(&Value::Null)
    }

    fn rules(&self) -> Vec<String> {
        self.expect()
            .get("rules")
            .and_then(Value::as_array)
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn variant(&self) -> SyntaxVariant {
        match self.text("variant") {
            "b" => SyntaxVariant::B,
            "c" => SyntaxVariant::C,
            _ => SyntaxVariant::A,
        }
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corpus_files(directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        if path.is_dir() {
            corpus_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
}

fn load_cases() -> Vec<Case> {
    let root = repository_root().join("tests/d1/correction/lexical");
    let mut files = Vec::new();
    corpus_files(&root, &mut files);
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let in_adversarial = file
            .components()
            .any(|part| part.as_os_str() == "adversarial");
        let text = fs::read_to_string(&file).unwrap_or_default();
        for (index, line) in text.lines().enumerate() {
            let location = format!("{}:{}", file.display(), index + 1);
            let value: Value = serde_json::from_str(line).unwrap_or(Value::Null);
            let marked = value.get("corpus").and_then(Value::as_str) == Some("adversarial");
            cases.push(Case {
                location,
                adversarial: in_adversarial || marked,
                value,
            });
        }
    }
    cases
}

fn class_rank(class: &str) -> usize {
    ["C", "S", "F", "P"]
        .iter()
        .position(|code| *code == class)
        .unwrap_or(usize::MAX)
}

fn lexical_class(rule: &str) -> Option<CorrectionClass> {
    Rule::ALL
        .iter()
        .find(|candidate| candidate.id() == rule)
        .map(|candidate| candidate.class())
}

/// Applies normalization and the token rules; corrections in text order.
fn apply_token_rules(
    input: &str,
    variant: SyntaxVariant,
    catalog: &Catalog,
    habits: &BTreeSet<String>,
) -> (String, Vec<Correction>) {
    let is_habit = |word: &str| habits.contains(word);
    let context = TokenContext {
        variant,
        catalog,
        is_habit: &is_habit,
    };
    let normalized = normalize(input);
    let rewritten = rewrite(&normalized.text, &context);
    let mut corrections = normalized.corrections;
    corrections.extend(rewritten.corrections);
    corrections.sort_by_key(|correction| correction.at.map(|at| (at.line, at.column)));
    (rewritten.text, corrections)
}

fn habit_words() -> BTreeSet<String> {
    load_embedded()
        .map(|tables| tables.habits().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn format_problems(case: &Case, seen: &mut BTreeSet<String>) -> Vec<String> {
    let mut problems = Vec::new();
    if !case.value.is_object() {
        return vec!["not a JSON object".to_owned()];
    }
    for member in REQUIRED {
        if case.value.get(member).is_none() {
            problems.push(format!("missing `{member}`"));
        }
    }
    let id = case.text("id");
    let id_valid = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !id_valid {
        problems.push(format!("invalid id `{id}`"));
    }
    if !seen.insert(id.to_owned()) {
        problems.push(format!("duplicate id `{id}`"));
    }
    let allowed = [
        ("variant", &["a", "b", "c"][..]),
        ("program_kind", &["read", "effect"][..]),
        ("intent", &["unambiguous", "ambiguous"][..]),
        ("source", &["manual", "generated", "example", "model"][..]),
    ];
    for (member, values) in allowed {
        if !values.contains(&case.text(member)) {
            problems.push(format!("invalid `{member}`"));
        }
    }
    problems.extend(expect_problems(case));
    problems
}

fn expect_problems(case: &Case) -> Vec<String> {
    let expect = case.expect();
    let forms = ["canonical", "unchanged", "error"]
        .iter()
        .filter(|form| expect.get(**form).is_some())
        .count();
    if forms != 1 {
        return vec!["`expect` needs exactly one of canonical, unchanged, error".to_owned()];
    }
    if expect.get("canonical").is_none() {
        return Vec::new();
    }
    let rules = case.rules();
    let mut problems = Vec::new();
    if rules.is_empty() {
        problems.push("a canonical form lists its rules".to_owned());
    }
    if rules
        .iter()
        .any(|rule| !nurt_contracts::report::Correction::is_rule_id(rule))
    {
        problems.push("invalid rule identifier".to_owned());
    }
    let class = expect
        .get("class")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let classes: Option<Vec<CorrectionClass>> =
        rules.iter().map(|rule| lexical_class(rule)).collect();
    if let Some(classes) = classes {
        let least = classes
            .iter()
            .map(|class| class_rank(class.as_str()))
            .max()
            .unwrap_or(0);
        if class_rank(class) != least {
            problems.push(format!(
                "class `{class}` is not the least certain class of the rules"
            ));
        }
    }
    problems
}

#[test]
fn every_case_follows_the_corpus_format() {
    let cases = load_cases();
    let mut seen = BTreeSet::new();
    let problems: Vec<String> = cases
        .iter()
        .flat_map(|case| {
            format_problems(case, &mut seen)
                .into_iter()
                .map(|problem| format!("{}: {problem}", case.location))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn corpus_covers_every_lexical_rule() {
    let cases = load_cases();
    let mistakes = cases.iter().filter(|case| !case.adversarial).count();
    let adversarial = cases.iter().filter(|case| case.adversarial).count();
    assert!(mistakes >= MIN_MISTAKE_CASES, "{mistakes} mistake cases");
    assert!(
        adversarial >= MIN_ADVERSARIAL_CASES,
        "{adversarial} adversarial cases"
    );
    let mut per_rule: BTreeMap<String, usize> = BTreeMap::new();
    for case in cases.iter().filter(|case| !case.adversarial) {
        let mut ids = case.rules();
        if ids.is_empty() {
            ids.push(
                case.text("id")
                    .split('-')
                    .nth(1)
                    .unwrap_or_default()
                    .to_uppercase(),
            );
        }
        for rule in ids {
            *per_rule.entry(rule).or_default() += 1;
        }
    }
    let missing: Vec<&str> = Rule::ALL
        .iter()
        .map(|rule| rule.id())
        .filter(|id| per_rule.get(*id).copied().unwrap_or(0) < MIN_CASES_PER_RULE)
        .collect();
    assert!(missing.is_empty(), "rules with too few cases: {missing:?}");
}

#[test]
fn token_rule_cases_give_the_expected_canonical_text() {
    let catalog = Catalog::load();
    let habits = habit_words();
    let mut checked = 0;
    let mut failures = Vec::new();
    for case in load_cases() {
        let rules = case.rules();
        let decided = case.variant() == SyntaxVariant::A
            && case.value.get("context").is_none()
            && !rules.is_empty()
            && rules
                .iter()
                .all(|rule| TOKEN_RULES.contains(&rule.as_str()));
        let Some(canonical) = case.expect().get("canonical").and_then(Value::as_str) else {
            continue;
        };
        if !decided {
            continue;
        }
        checked += 1;
        let (text, corrections) =
            apply_token_rules(case.text("input"), case.variant(), &catalog, &habits);
        let applied: Vec<String> = corrections
            .iter()
            .map(|correction| correction.rule.to_string())
            .collect();
        if text != canonical || applied != rules {
            failures.push(format!("{}: {text:?} {applied:?}", case.location));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert!(checked >= MIN_TOKEN_CASES, "{checked} cases checked");
}

#[test]
fn cases_without_correction_get_none_from_the_token_rules() {
    let catalog = Catalog::load();
    let habits = habit_words();
    let mut failures = Vec::new();
    for case in load_cases() {
        let unchanged = case.expect().get("unchanged").and_then(Value::as_bool) == Some(true);
        if !unchanged {
            continue;
        }
        let input = case.text("input");
        let (text, corrections) = apply_token_rules(input, case.variant(), &catalog, &habits);
        if text != input || !corrections.is_empty() {
            failures.push(format!("{}: {text:?}", case.location));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn token_rules_make_no_further_correction_on_their_result() {
    let catalog = Catalog::load();
    let habits = habit_words();
    let mut failures = Vec::new();
    for case in load_cases() {
        let (first, _) = apply_token_rules(case.text("input"), case.variant(), &catalog, &habits);
        let (second, corrections) = apply_token_rules(&first, case.variant(), &catalog, &habits);
        if second != first || !corrections.is_empty() {
            failures.push(format!("{}: {first:?} -> {second:?}", case.location));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn literal_and_comment_bytes_survive_the_token_rules() {
    let catalog = Catalog::load();
    let habits = habit_words();
    let mut failures = Vec::new();
    for case in load_cases() {
        let input = case.text("input");
        let (text, _) = apply_token_rules(input, case.variant(), &catalog, &habits);
        for literal in double_quoted_literals(input) {
            if !text.contains(&literal) {
                failures.push(format!("{}: lost {literal:?}", case.location));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Closed text literals `"…"` of a program, with their quotes.
fn double_quoted_literals(text: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if c != '"'
            || text
                .get(..start)
                .is_some_and(|before| before.ends_with('r') || before.ends_with('#'))
        {
            continue;
        }
        let mut escaped = false;
        for (index, inner) in chars.by_ref() {
            match inner {
                '\\' if !escaped => escaped = true,
                '"' if !escaped => {
                    literals.push(text.get(start..=index).unwrap_or_default().to_owned());
                    break;
                }
                '\n' => break,
                _ => escaped = false,
            }
        }
    }
    literals
}

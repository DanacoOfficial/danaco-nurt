// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-correct/tests/powershell_corpus.rs
// Responsibility:   The PowerShell correction corpus and the consistency of the PowerShell translation table with the command catalog.
// Dependencies:     nurt_contracts, nurt_syntax, serde_json, toml_edit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The PowerShell correction corpus and the consistency of the PowerShell
//! translation table with the command catalog.
//!
//! Every case of the corpus directory `tests/d1/correction/powershell` of
//! the workspace is translated in a live call and compared with its expected
//! reaction. All differences of one run are reported together.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use nurt_contracts::error::Error;
use nurt_contracts::report::CorrectionClass;
use nurt_correct::powershell::{
    PowerShellTable, TranslateContext, Translation, translate_with, translate_with_table,
};
use serde_json::Value;
use toml_edit::DocumentMut;

/// Result of a helper that reads test data.
type Loaded<T> = Result<T, Box<dyn std::error::Error>>;

/// Rules of the PowerShell translation table.
const RULES: std::ops::RangeInclusive<u8> = 1..=42;

/// Smallest number of cases of every rule.
const MIN_CASES_PER_RULE: usize = 3;

/// Smallest number of adversarial cases.
const MIN_ADVERSARIAL_CASES: usize = 40;

/// Largest number of candidates, fixes, next steps and alternatives.
const MAX_LISTED: usize = 3;

/// Members every case has.
const REQUIRED_MEMBERS: [&str; 7] = [
    "id",
    "input",
    "variant",
    "expect",
    "program_kind",
    "intent",
    "source",
];

const TABLE_TEXT: &str = include_str!("../tables/powershell.toml");
const HABITS_TEXT: &str = include_str!("../tables/habits_powershell.toml");

fn workspace() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
}

fn corpus_files() -> Loaded<Vec<PathBuf>> {
    let root = workspace().join("tests/d1/correction/powershell");
    let mut files = Vec::new();
    for directory in [root.clone(), root.join("adversarial")] {
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

struct Case {
    file: String,
    id: String,
    input: String,
    variant: String,
    program_kind: String,
    intent: String,
    corpus: Option<String>,
    adversarial_file: bool,
    base: Option<String>,
    lets: Vec<String>,
    expect: Value,
}

fn text_member(case: &Value, member: &str, place: &str) -> Loaded<String> {
    case.get(member)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{place}: `{member}` is missing or not a string").into())
}

fn read_cases(path: &Path) -> Loaded<Vec<Case>> {
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let adversarial_file = path
        .parent()
        .is_some_and(|parent| parent.ends_with("adversarial"));
    let text = std::fs::read_to_string(path)?;
    let mut cases = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let place = format!("{file}:{}", index + 1);
        let case: Value =
            serde_json::from_str(line).map_err(|error| format!("{place}: {error}"))?;
        if let Some(member) = REQUIRED_MEMBERS
            .iter()
            .find(|member| case.get(**member).is_none())
        {
            return Err(format!("{place}: `{member}` is missing").into());
        }
        let context = case.get("context");
        let base = context
            .and_then(|context| context["base"].as_str())
            .map(str::to_owned);
        let lets = context
            .and_then(|context| context["lets"].as_array())
            .map(|lets| {
                lets.iter()
                    .filter_map(|name| name.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        cases.push(Case {
            file: file.clone(),
            id: text_member(&case, "id", &place)?,
            input: text_member(&case, "input", &place)?,
            variant: text_member(&case, "variant", &place)?,
            program_kind: text_member(&case, "program_kind", &place)?,
            intent: text_member(&case, "intent", &place)?,
            corpus: case["corpus"].as_str().map(str::to_owned),
            adversarial_file,
            base,
            lets,
            expect: case["expect"].clone(),
        });
    }
    Ok(cases)
}

fn all_cases() -> Loaded<Vec<Case>> {
    let mut cases = Vec::new();
    for path in corpus_files()? {
        cases.extend(read_cases(&path)?);
    }
    Ok(cases)
}

fn class_code(class: CorrectionClass) -> (u8, &'static str) {
    match class {
        CorrectionClass::Certain => (0, "C"),
        CorrectionClass::Structural => (1, "S"),
        CorrectionClass::Fuzzy => (2, "F"),
        _ => (3, "P"),
    }
}

fn least_certain(translation: &Translation) -> Option<&'static str> {
    translation
        .rules
        .iter()
        .map(|rule| class_code(rule.class))
        .max()
        .map(|(_, code)| code)
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn describe(result: &Option<Result<Translation, Error>>) -> String {
    match result {
        None => "no translation".to_owned(),
        Some(Ok(translation)) => {
            let rules: Vec<&str> = translation
                .rules
                .iter()
                .map(|rule| rule.rule.as_str())
                .collect();
            format!(
                "canonical {:?} rules {rules:?} class {}",
                translation.text(),
                least_certain(translation).unwrap_or("-")
            )
        }
        Some(Err(error)) => {
            let candidates: Vec<&str> = error
                .candidates
                .iter()
                .map(|candidate| candidate.program.as_str())
                .collect();
            format!(
                "error {} {:?} fix {:?} candidates {candidates:?} next_steps {:?}",
                error.code.as_str(),
                error.message,
                error.fix,
                error.next_steps
            )
        }
    }
}

fn matches_expectation(expect: &Value, result: &Option<Result<Translation, Error>>) -> bool {
    if expect.get("unchanged").is_some() {
        return result.is_none();
    }
    if let Some(canonical) = expect["canonical"].as_str() {
        let Some(Ok(translation)) = result else {
            return false;
        };
        let rules: Vec<String> = translation
            .rules
            .iter()
            .map(|rule| rule.rule.clone())
            .collect();
        let alternatives: Vec<String> = translation
            .alternatives
            .iter()
            .map(|document| {
                nurt_syntax::print::print(document, nurt_syntax::parse::Variant::A)
                    .trim_end()
                    .to_owned()
            })
            .collect();
        return translation.text() == canonical
            && rules == strings(&expect["rules"])
            && least_certain(translation) == expect["class"].as_str()
            && (expect.get("alternatives").is_none()
                || alternatives == strings(&expect["alternatives"]));
    }
    let Some(code) = expect["error"].as_str() else {
        return false;
    };
    let Some(Err(error)) = result else {
        return false;
    };
    let candidates: Vec<String> = error
        .candidates
        .iter()
        .map(|candidate| candidate.program.clone())
        .collect();
    error.code.as_str() == code
        && (expect.get("fix").is_none() || error.fix == strings(&expect["fix"]))
        && (expect.get("candidates").is_none() || candidates == strings(&expect["candidates"]))
        && (expect.get("next_steps").is_none()
            || error.next_steps == strings(&expect["next_steps"]))
}

#[test]
fn corpus_files_follow_the_format() {
    let mut previous: BTreeMap<String, String> = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for case in all_cases().unwrap() {
        let last = previous.entry(case.file.clone()).or_default();
        assert!(
            case.id > *last,
            "{}: `{}` is not sorted after `{last}`",
            case.file,
            case.id
        );
        last.clone_from(&case.id);
        assert!(ids.insert(case.id.clone()), "`{}` appears twice", case.id);
        assert_eq!(case.variant, "a", "{}", case.id);
        assert!(
            matches!(case.program_kind.as_str(), "read" | "effect"),
            "{}",
            case.id
        );
        assert!(
            matches!(case.intent.as_str(), "unambiguous" | "ambiguous"),
            "{}",
            case.id
        );
        if case.adversarial_file {
            assert_eq!(case.corpus.as_deref(), Some("adversarial"), "{}", case.id);
        }
        for list in ["fix", "candidates", "next_steps", "alternatives"] {
            assert!(
                strings(&case.expect[list]).len() <= MAX_LISTED,
                "{}: too many {list}",
                case.id
            );
        }
    }
    assert!(!ids.is_empty(), "the corpus is empty");
}

#[test]
fn corpus_cases_get_their_expected_reaction() {
    let cases = all_cases().unwrap();
    let mut failures = Vec::new();
    for case in &cases {
        let context = TranslateContext {
            base: case.base.as_deref(),
            lets: &case.lets,
            ..TranslateContext::default()
        };
        let result = translate_with(&case.input, &context);
        if !matches_expectation(&case.expect, &result) {
            failures.push(format!(
                "{} {}: input {:?}\n  expected {}\n  actual   {}",
                case.file,
                case.id,
                case.input,
                case.expect,
                describe(&result)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn every_rule_has_cases() {
    let mut per_rule: BTreeMap<u8, usize> = BTreeMap::new();
    for case in all_cases().unwrap() {
        let number = case
            .id
            .strip_prefix("powershell-p")
            .and_then(|rest| rest.get(..2))
            .and_then(|digits| digits.parse::<u8>().ok());
        if let Some(number) = number {
            *per_rule.entry(number).or_default() += 1;
        }
    }
    let missing: Vec<String> = RULES
        .filter(|rule| per_rule.get(rule).copied().unwrap_or_default() < MIN_CASES_PER_RULE)
        .map(|rule| format!("P{rule:02}"))
        .collect();
    assert!(missing.is_empty(), "rules with too few cases: {missing:?}");
}

#[test]
fn adversarial_corpus_has_enough_cases() {
    let adversarial = all_cases()
        .unwrap()
        .iter()
        .filter(|case| case.corpus.as_deref() == Some("adversarial"))
        .count();
    assert!(
        adversarial >= MIN_ADVERSARIAL_CASES,
        "{adversarial} adversarial cases, at least {MIN_ADVERSARIAL_CASES} required"
    );
}

/// Command names and parameter names of the catalog.
fn catalog() -> Loaded<BTreeMap<String, BTreeSet<String>>> {
    let root = workspace().join("catalog");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let path = entry?.path();
        let is_catalog = path.extension().is_some_and(|ext| ext == "toml")
            && path.file_name().is_some_and(|name| name != "_index.toml");
        if is_catalog {
            files.push(path);
        }
    }
    files.sort();
    let mut commands = BTreeMap::new();
    for path in files {
        let document: DocumentMut = std::fs::read_to_string(&path)?.parse()?;
        let Some(entries) = document
            .get("command")
            .and_then(|item| item.as_array_of_tables())
        else {
            continue;
        };
        for command in entries {
            let Some(name) = command.get("name").and_then(|item| item.as_str()) else {
                continue;
            };
            let params: BTreeSet<String> = command
                .get("param")
                .and_then(|item| item.as_array_of_tables())
                .map(|params| {
                    params
                        .iter()
                        .filter_map(|param| param.get("name")?.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            commands.insert(name.to_owned(), params);
        }
    }
    Ok(commands)
}

#[test]
fn table_names_exist_in_the_catalog() {
    let table = PowerShellTable::embedded().unwrap();
    let catalog = catalog().unwrap();
    let unknown_commands: Vec<String> = table
        .command_names()
        .into_iter()
        .filter(|name| !catalog.contains_key(name))
        .collect();
    assert!(
        unknown_commands.is_empty(),
        "commands missing from the catalog: {unknown_commands:?}"
    );
    let unknown_params: Vec<String> = table
        .argument_names()
        .into_iter()
        .filter(|(command, param)| {
            catalog
                .get(command)
                .is_some_and(|params| !params.contains(param))
        })
        .map(|(command, param)| format!("{command} {param}:"))
        .collect();
    assert!(
        unknown_params.is_empty(),
        "parameters missing from the catalog: {unknown_params:?}"
    );
}

#[test]
fn property_mapping_uses_lower_case_field_names() {
    let table = PowerShellTable::embedded().unwrap();
    let fields = table.field_names();
    assert!(
        fields
            .iter()
            .any(|(record, field)| record == "entry" && field == "size")
    );
    assert!(
        fields
            .iter()
            .any(|(record, field)| record == "entry" && field == "modified")
    );
    for (record, field) in fields {
        assert!(
            field.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{record}.{field} is not a field name"
        );
    }
}

#[test]
fn translation_follows_the_table_data() {
    let original = "[[entry.variant]]\nrule = \"P08\"\nclass = \"C\"\nstages = [\"remove\"]\n";
    assert!(TABLE_TEXT.contains(original));
    let changed = TABLE_TEXT.replace(
        original,
        "[[entry.variant]]\nrule = \"P08\"\nclass = \"C\"\nstages = [\"trash\"]\n",
    );
    let table = PowerShellTable::parse(&changed, HABITS_TEXT).unwrap();
    let translation =
        translate_with_table("Remove-Item /srv/x", &TranslateContext::default(), &table)
            .unwrap()
            .unwrap();
    assert_eq!(translation.text(), "trash \"/srv/x\"");
    let without_alias = HABITS_TEXT.replace("\"gci\", ", "");
    assert!(PowerShellTable::parse(TABLE_TEXT, &without_alias).is_ok());
    let broken = TABLE_TEXT.replace("stages = [\"remove\"]", "stages = \"remove\"");
    assert!(PowerShellTable::parse(&broken, HABITS_TEXT).is_err());
}

/// Variants of a corpus input that reach the error and option paths of the
/// translation.
fn variants(input: &str) -> Vec<String> {
    let mut out = vec![
        format!("{input} -Bogus"),
        format!("{input} -Bogus 1"),
        format!("{input} -WhatIf -Confirm:$false"),
        format!("{input} -ErrorAction SilentlyContinue"),
        format!("{input} -ErrorAction Stop"),
        format!("{input} -Force"),
        format!("{input} -Path"),
        format!("{input} > out.txt"),
        format!("{input} >> out.txt"),
        format!("{input} | Out-File out.txt"),
        format!("{input} | Out-File out.txt -Append"),
        format!("{input} | ConvertTo-Json"),
        format!("{input} | Select-Object -First 3"),
        format!("{input} | Sort-Object Length -Descending"),
        format!("{input} | Measure-Object"),
        format!("{input} | Where-Object Length -gt 1"),
        format!("{input} | 5"),
        format!("{input}; {input}"),
        format!("{input} |"),
        format!("$x = {input}"),
        format!("$x = {input}; $x"),
        format!("$env:A = {input}"),
        format!("$a.b = {input}"),
        format!("({input})"),
        format!("{input} {{ }}"),
        format!("{input} 'a' \"b\" $c $env:D"),
        format!("{input} -Path C:\\a -LiteralPath C:\\b"),
        format!("& {input}"),
        format!("{input} 1KB 2.5 -3"),
    ];
    if let Some((head, _)) = input.rsplit_once(' ') {
        out.push(head.to_owned());
    }
    out
}

#[test]
fn mutated_corpus_inputs_give_a_reaction_without_panicking() {
    let cases = all_cases().unwrap();
    let mut translated = 0_usize;
    for case in &cases {
        for text in variants(&case.input) {
            for base in [None, Some("C:/work")] {
                let context = TranslateContext {
                    base,
                    ..TranslateContext::default()
                };
                if let Some(Ok(translation)) = translate_with(&text, &context) {
                    assert!(!translation.text().is_empty(), "empty program for {text:?}");
                    translated += 1;
                }
            }
        }
    }
    assert!(translated > 0, "no variant was translated");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

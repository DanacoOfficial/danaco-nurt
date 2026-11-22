// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Checks of the journal coverage table `tables/journal-coverage.toml`: the
//! table is well formed, and every command and action of the catalog that
//! changes data or runs a program has a row.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::journal::PreserveMode;
use nurt_contracts::kinds::{Category, EffectKind, Reversibility};
use nurt_contracts::report::Irreversible;
use serde::Deserialize;

const FORMAT: &str = "nurt-journal-coverage/1";
const CATALOG_FORMAT: &str = "nurt-catalog/1";
const GROUPS: &[&str] = &[
    "core",
    "documents",
    "text",
    "code",
    "notebook",
    "git",
    "archive",
    "system",
    "admin",
    "package",
    "network",
    "schedule",
    "journal",
    "program",
    "verify",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageFile {
    format: String,
    generic_irreversible: Vec<Irreversible>,
    row: Vec<Row>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Coverage {
    Full,
    Partial,
    #[serde(rename = "none")]
    Uncovered,
    NoEffect,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    command: String,
    #[serde(default)]
    action: Option<String>,
    group: String,
    changes: String,
    preserve: Vec<PreserveMode>,
    coverage: Coverage,
    #[serde(default)]
    irreversible: Vec<Irreversible>,
    #[serde(default)]
    exception: String,
    #[serde(default)]
    note: String,
}

impl Row {
    fn label(&self) -> String {
        match &self.action {
            Some(action) => format!("{} {}", self.command, action),
            None => self.command.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    format: String,
    group: String,
    #[serde(default)]
    command: Vec<CatalogCommand>,
}

#[derive(Debug, Deserialize)]
struct CatalogCommand {
    name: String,
    category: Category,
    reversibility: Reversibility,
    #[serde(default)]
    effects: Vec<CatalogEffect>,
}

#[derive(Debug, Deserialize)]
struct CatalogEffect {
    action: String,
    effect: EffectKind,
    reversibility: Reversibility,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_coverage() -> CoverageFile {
    let path = repository_root().join("tables/journal-coverage.toml");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Catalog files present in the repository, keyed by group.
fn load_catalog() -> BTreeMap<String, CatalogFile> {
    let dir = repository_root().join("catalog");
    let mut files = BTreeMap::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return files;
    };
    let mut paths: Vec<PathBuf> = entries.map(|entry| entry.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        let is_toml = path.extension().is_some_and(|ext| ext == "toml");
        let is_index = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with('_'));
        if !is_toml || is_index {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let file: CatalogFile =
            toml::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        if file.format != CATALOG_FORMAT {
            continue;
        }
        files.insert(file.group.clone(), file);
    }
    files
}

fn is_sentence(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == text
        && trimmed.ends_with('.')
        && trimmed
            .chars()
            .next()
            .is_some_and(|first| !first.is_lowercase())
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
}

fn check_row(row: &Row) -> Vec<String> {
    let mut problems = check_row_text(row);
    problems.extend(check_row_coverage(row));
    problems
}

fn check_row_text(row: &Row) -> Vec<String> {
    let label = row.label();
    let mut problems = Vec::new();
    if !is_identifier(&row.command) {
        problems.push(format!("{label}: command is not an identifier"));
    }
    if row
        .action
        .as_deref()
        .is_some_and(|action| !is_identifier(action))
    {
        problems.push(format!("{label}: action is not an identifier"));
    }
    if !GROUPS.contains(&row.group.as_str()) {
        problems.push(format!("{label}: unknown group `{}`", row.group));
    }
    if !is_sentence(&row.changes) {
        problems.push(format!("{label}: `changes` must be a sentence"));
    }
    if !row.note.is_empty() && !is_sentence(&row.note) {
        problems.push(format!("{label}: `note` must be a sentence"));
    }
    if !row.exception.is_empty() && !is_sentence(&row.exception) {
        problems.push(format!("{label}: `exception` must be a sentence"));
    }
    problems
}

fn check_row_coverage(row: &Row) -> Vec<String> {
    let label = row.label();
    let mut problems = Vec::new();
    let modes: BTreeSet<&str> = row.preserve.iter().map(|mode| mode.as_str()).collect();
    if modes.len() != row.preserve.len() {
        problems.push(format!("{label}: duplicate preserve modes"));
    }
    let reasons: BTreeSet<&str> = row
        .irreversible
        .iter()
        .map(|reason| reason.as_str())
        .collect();
    if reasons.len() != row.irreversible.len() {
        problems.push(format!("{label}: duplicate irreversible reasons"));
    }
    for rule in coverage_rule_violations(row) {
        problems.push(format!("{label}: {rule}"));
    }
    problems
}

/// Rules of the coverage values, as pairs of a condition that must hold and
/// the message reported when it does not.
fn coverage_rule_violations(row: &Row) -> Vec<&'static str> {
    let keeps_data = row.preserve.iter().any(|mode| mode.is_preserved());
    let keeps_nothing = row.preserve.iter().any(|mode| !mode.is_preserved());
    let no_modes = row.preserve.is_empty();
    let has_reasons = !row.irreversible.is_empty();
    let has_exception = !row.exception.is_empty();
    let rules = match row.coverage {
        Coverage::Full => vec![
            (
                keeps_data && !keeps_nothing,
                "full coverage needs modes that keep data only",
            ),
            (
                !has_reasons && !has_exception,
                "full coverage has no reasons or exception",
            ),
        ],
        Coverage::Partial => vec![
            (keeps_data, "partial coverage needs a mode that keeps data"),
            (has_exception, "partial coverage needs an exception"),
        ],
        Coverage::Uncovered => vec![
            (
                keeps_nothing,
                "no coverage needs the mode `none` or `external`",
            ),
            (has_exception, "no coverage needs an exception"),
        ],
        Coverage::NoEffect => vec![(
            no_modes && !has_reasons,
            "no effect has no modes and no reasons",
        )],
    };
    rules
        .into_iter()
        .filter(|(holds, _)| !holds)
        .map(|(_, message)| message)
        .collect()
}

fn fail_on(problems: &[String]) {
    assert!(
        problems.is_empty(),
        "journal coverage problems:\n{}",
        problems.join("\n")
    );
}

#[test]
fn coverage_table_is_well_formed() {
    let table = load_coverage();
    assert_eq!(table.format, FORMAT);
    assert!(!table.generic_irreversible.is_empty());
    let generic: BTreeSet<&str> = table
        .generic_irreversible
        .iter()
        .map(|reason| reason.as_str())
        .collect();
    assert_eq!(
        generic.len(),
        table.generic_irreversible.len(),
        "duplicate generic reasons"
    );
    assert!(!table.row.is_empty());

    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for row in &table.row {
        if !seen.insert((row.command.clone(), row.action.clone())) {
            problems.push(format!("{}: duplicate row", row.label()));
        }
        problems.extend(check_row(row));
        let groups: BTreeSet<&str> = table
            .row
            .iter()
            .filter(|other| other.command == row.command)
            .map(|other| other.group.as_str())
            .collect();
        if groups.len() != 1 {
            problems.push(format!(
                "{}: rows of one command name several groups",
                row.label()
            ));
        }
    }
    fail_on(&problems);
}

/// Whether a catalog command must have rows: it changes data, runs a
/// program, or is a journal command with an effect.
fn needs_rows(command: &CatalogCommand) -> bool {
    match command.category {
        Category::Change | Category::Run => true,
        Category::Journal => command.reversibility != Reversibility::Read,
        _ => false,
    }
}

#[test]
fn every_journaled_catalog_command_and_action_has_a_row() {
    let table = load_coverage();
    let catalog = load_catalog();
    let mut problems = Vec::new();
    for file in catalog.values() {
        for command in &file.command {
            let rows: Vec<&Row> = table
                .row
                .iter()
                .filter(|row| row.command == command.name)
                .collect();
            let whole = rows.iter().find(|row| row.action.is_none());
            if needs_rows(command) && rows.is_empty() {
                problems.push(format!("{}: no row for command", command.name));
                continue;
            }
            for row in &rows {
                if row.group != file.group {
                    problems.push(format!(
                        "{}: row group `{}` differs from catalog group `{}`",
                        row.label(),
                        row.group,
                        file.group
                    ));
                }
            }
            if let Some(row) = whole {
                if command.reversibility == Reversibility::Irreversible
                    && row.coverage == Coverage::Full
                {
                    problems.push(format!(
                        "{}: irreversible in catalog, full in table",
                        row.label()
                    ));
                }
                if command.reversibility == Reversibility::Reversible
                    && row.coverage == Coverage::Uncovered
                {
                    problems.push(format!(
                        "{}: reversible in catalog, none in table",
                        row.label()
                    ));
                }
            }
            for effect in &command.effects {
                if effect.effect == EffectKind::Read {
                    continue;
                }
                let exact = rows
                    .iter()
                    .find(|row| row.action.as_deref() == Some(effect.action.as_str()));
                let Some(row) = exact.or(whole) else {
                    problems.push(format!(
                        "{} {}: no row for action",
                        command.name, effect.action
                    ));
                    continue;
                };
                if exact.is_none() {
                    continue;
                }
                if effect.reversibility == Reversibility::Irreversible
                    && row.coverage == Coverage::Full
                {
                    problems.push(format!(
                        "{}: irreversible in catalog, full in table",
                        row.label()
                    ));
                }
                if effect.reversibility == Reversibility::Reversible
                    && row.coverage == Coverage::Uncovered
                {
                    problems.push(format!(
                        "{}: reversible in catalog, none in table",
                        row.label()
                    ));
                }
            }
        }
    }
    fail_on(&problems);
}

#[test]
fn every_row_names_a_catalog_command() {
    let table = load_coverage();
    let catalog = load_catalog();
    let mut problems = Vec::new();
    for row in &table.row {
        let Some(file) = catalog.get(&row.group) else {
            continue;
        };
        if !file
            .command
            .iter()
            .any(|command| command.name == row.command)
        {
            problems.push(format!(
                "{}: not a command of the catalog group `{}`",
                row.label(),
                row.group
            ));
        }
    }
    fail_on(&problems);
}

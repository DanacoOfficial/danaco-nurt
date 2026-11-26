// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The cheat sheets "Nurt for bash users" and "Nurt for PowerShell users"
//! (`docs/cheatsheets/*.toml`) against the correction pipeline that
//! `nurt <program>` runs: every habit is read, corrected and bound with
//! `bind_with_correction` on the `host` of the sheet and with the call
//! parameter `base` of the sheet, and the result must be the one the entry
//! declares.
//! An entry with the outcome `corrected` must run as its `canonical` text;
//! an entry with the outcome `error` must fail with a ready replacement,
//! a candidate or a next step equal to its `canonical` text; an entry with
//! the outcome `rejected` must fail with its `code` and no program is
//! compared. Two programs are the same when they bind, without correction, to
//! the same tree, so that `to: "/b"` and a positional `"/b"` are one program.

mod bind_support;

use std::fmt::Write as _;
use std::fs;

use bind_support::{catalog, workspace};
use nurt_bind::{BindContext, bind, bind_with_correction};
use nurt_contracts::config::Config;
use nurt_contracts::error::Error;
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::spec::Os;
use nurt_correct::tables::load_embedded;
use nurt_syntax::hash::ast_hash;
use nurt_syntax::parse::{ParseOptions, parse};
use nurt_syntax::print::try_print;

/// Operating system of the paths a sheet is written with, from its member
/// `host`: the bash sheet uses `/home/user`, the PowerShell sheet
/// `C:\Users\user`.
fn host_of(table: &toml::Table) -> Os {
    match table["host"].as_str().unwrap() {
        "linux" => Os::Linux,
        "windows" => Os::Windows,
        other => panic!("unknown host {other}"),
    }
}

/// What the pipeline did with a habit.
#[derive(Debug, PartialEq, Eq)]
enum Actual {
    /// The habit runs as this canonical text.
    Corrected(String),
    /// The habit fails with this code and these ready programs.
    Failed { code: String, programs: Vec<String> },
}

fn programs_of(error: &Error) -> Vec<String> {
    let data = error.data();
    data.fix
        .iter()
        .cloned()
        .chain(
            data.candidates
                .iter()
                .map(|candidate| candidate.program.clone()),
        )
        .chain(data.next_steps.iter().cloned())
        .collect()
}

fn context(os: Os, base: &str) -> BindContext {
    BindContext::new(os)
        .with_base(base)
        .with_corr_rev(load_embedded().unwrap().corr_rev())
}

fn run(habit: &str, os: Os, base: &str) -> Actual {
    let tables = load_embedded().unwrap();
    let result = bind_with_correction(
        habit,
        SyntaxVariant::A,
        catalog(),
        &tables,
        &Config::default(),
        &context(os, base),
    );
    match result {
        Ok(outcome) => Actual::Corrected(outcome.canonical),
        Err(error) => Actual::Failed {
            code: error.code().to_string(),
            programs: programs_of(&error),
        },
    }
}

/// Program text in the canonical notation of the printer; a text that does
/// not read is returned as it is.
fn normal(text: &str) -> String {
    parse(text, SyntaxVariant::A, &ParseOptions::default())
        .ok()
        .and_then(|document| try_print(&document, SyntaxVariant::A).ok())
        .map_or_else(|| text.to_owned(), |printed| printed.trim_end().to_owned())
}

/// Whether two program texts are one program: they bind without correction
/// to the same tree, or, when one does not bind, print the same.
fn same_program(left: &str, right: &str, context: &BindContext) -> bool {
    let tree = |text: &str| {
        let document = parse(text, SyntaxVariant::A, &ParseOptions::default()).ok()?;
        ast_hash(&bind(&document, catalog(), context).ok()?).ok()
    };
    match (tree(left), tree(right)) {
        (Some(left), Some(right)) => left == right,
        _ => normal(left) == normal(right),
    }
}

fn read(shell: &str) -> toml::Table {
    let path = workspace().join(format!("docs/cheatsheets/{shell}.toml"));
    fs::read_to_string(&path).unwrap().parse().unwrap()
}

/// Problems of the entries of one sheet, one line each.
fn problems(shell: &str) -> String {
    let table = read(shell);
    let (os, base) = (host_of(&table), table["base"].as_str().unwrap());
    let context = context(os, base);
    let mut report = String::new();
    for entry in table["entry"].as_array().unwrap() {
        let field = |name: &str| entry[name].as_str().unwrap();
        let (id, habit) = (field("id"), field("habit"));
        let canonical = field("canonical");
        let actual = run(habit, os, base);
        let agrees = match (field("outcome"), &actual) {
            ("corrected", Actual::Corrected(text)) => same_program(text, canonical, &context),
            ("error", Actual::Failed { programs, .. }) => programs
                .iter()
                .any(|program| same_program(program, canonical, &context)),
            ("rejected", Actual::Failed { code, .. }) => {
                entry.get("code").and_then(toml::Value::as_str) == Some(code.as_str())
            }
            _ => false,
        };
        if !agrees {
            let _ = writeln!(
                report,
                "{id} [{}] `{habit}` expected `{}`, got {actual:?}",
                field("outcome"),
                field("canonical")
            );
        }
    }
    report
}

#[test]
fn every_bash_entry_has_the_declared_result() {
    let report = problems("bash");
    assert!(report.is_empty(), "\n{report}");
}

#[test]
fn every_powershell_entry_has_the_declared_result() {
    let report = problems("powershell");
    assert!(report.is_empty(), "\n{report}");
}

#[test]
fn the_entries_of_the_sheets_are_not_empty() {
    for shell in ["bash", "powershell"] {
        assert!(
            read(shell)["entry"].as_array().unwrap().len() >= 30,
            "{shell}"
        );
    }
}

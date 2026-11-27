// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/coverage/consistency.rs
// Responsibility:   Checks of the coverage table against the catalog and against the coverage.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Checks of the coverage table against the catalog and against the coverage
//! cases; they need no journal implementation and always run.

use std::collections::{BTreeMap, BTreeSet};

use nurt_contracts::kinds::Reversibility;

use crate::cases::{CASES, Rows};
use crate::catalog::{self, Command};
use crate::table::{self, Coverage, CoverageTable, FORMAT};

/// Commands of the catalog.
const COMMANDS: usize = 177;
/// Commands with at least one row that changes data.
const CHANGING_COMMANDS: usize = 101;
/// Commands whose rows only read.
const READ_ONLY_COMMANDS: usize = 76;
/// Rows of the table. A source row that lists several catalog actions is
/// split into one row per action (`http.request` with the methods `post`,
/// `put`, `patch`, `delete` and `options`).
const ROWS: usize = 265;
/// Rows per coverage value.
const ROWS_FULL: usize = 120;
const ROWS_PARTIAL: usize = 21;
const ROWS_NONE: usize = 37;
const ROWS_NO_EFFECT: usize = 87;
/// Distinct row sets exercised by the cases: one per source row of the
/// table, and one per parameter common to every command.
const SOURCE_ROWS: usize = 261;
const COMMON_PARAMETERS: usize = 5;
/// Coverage cases, one per test function.
const CASE_COUNT: usize = 307;

fn rows_by_command(table: &CoverageTable) -> BTreeMap<&str, Vec<&table::Row>> {
    let mut by_command: BTreeMap<&str, Vec<&table::Row>> = BTreeMap::new();
    for row in &table.row {
        by_command
            .entry(row.command.as_str())
            .or_default()
            .push(row);
    }
    by_command
}

#[test]
fn coverage_table_counts_match_the_catalog() {
    let table = table::load();
    assert_eq!(table.format, FORMAT);
    assert_eq!(table.row.len(), ROWS, "rows of the table");

    let mut per_value: BTreeMap<Coverage, usize> = BTreeMap::new();
    for row in &table.row {
        *per_value.entry(row.coverage).or_default() += 1;
    }
    let expected = BTreeMap::from([
        (Coverage::Full, ROWS_FULL),
        (Coverage::Partial, ROWS_PARTIAL),
        (Coverage::Uncovered, ROWS_NONE),
        (Coverage::NoEffect, ROWS_NO_EFFECT),
    ]);
    assert_eq!(per_value, expected, "rows per coverage value");

    let by_command = rows_by_command(&table);
    assert_eq!(by_command.len(), COMMANDS, "commands of the table");
    let changing = by_command
        .values()
        .filter(|rows| rows.iter().any(|row| row.coverage != Coverage::NoEffect))
        .count();
    assert_eq!(changing, CHANGING_COMMANDS, "commands that change data");
    assert_eq!(
        by_command.len() - changing,
        READ_ONLY_COMMANDS,
        "commands that only read"
    );
    assert_eq!(catalog::load().len(), COMMANDS, "commands of the catalog");

    let mut keys = BTreeSet::new();
    for row in &table.row {
        assert!(keys.insert(row.key()), "{}: duplicate row", row.label());
    }
}

#[test]
fn every_row_states_its_reasons_and_exception() {
    let table = table::load();
    let generic = &table.generic_irreversible;
    for row in &table.row {
        let label = row.label();
        for reason in &row.irreversible {
            assert!(
                !generic.contains(reason),
                "{label}: generic reason {} listed in the row",
                reason.as_str()
            );
        }
        match row.coverage {
            Coverage::Full | Coverage::NoEffect => {
                assert!(row.irreversible.is_empty(), "{label}: unexpected reasons");
                assert!(row.exception.is_empty(), "{label}: unexpected exception");
            }
            Coverage::Partial | Coverage::Uncovered => {
                assert!(!row.irreversible.is_empty(), "{label}: no reason");
                assert!(!row.exception.is_empty(), "{label}: no exception");
            }
        }
        assert!(!row.changes.is_empty(), "{label}: no description");
    }
}

fn catalog_by_name(commands: &[Command]) -> BTreeMap<&str, &Command> {
    commands
        .iter()
        .map(|command| (command.name.as_str(), command))
        .collect()
}

#[test]
fn every_catalog_command_and_changing_action_has_a_row() {
    let table = table::load();
    let commands = catalog::load();
    let by_command = rows_by_command(&table);
    let mut problems = Vec::new();
    for command in &commands {
        let Some(rows) = by_command.get(command.name.as_str()) else {
            problems.push(format!("{}: no row", command.name));
            continue;
        };
        if rows.iter().any(|row| row.group != command.group) {
            problems.push(format!("{}: group differs from the catalog", command.name));
        }
        let has_default = rows.iter().any(|row| row.action.is_none());
        for action in command.changing_actions() {
            let exact = rows.iter().any(|row| row.action.as_deref() == Some(action));
            if !exact && !has_default {
                problems.push(format!("{} {action}: no row", command.name));
            }
        }
        let read_only = rows.iter().all(|row| row.coverage == Coverage::NoEffect);
        let changes_data = command.reversibility != Reversibility::Read
            || command.changing_actions().next().is_some();
        if read_only && changes_data {
            problems.push(format!(
                "{}: changes data in the catalog, no effect in the table",
                command.name
            ));
        }
        if !read_only && !changes_data && !command.is_journaled() {
            problems.push(format!(
                "{}: only reads in the catalog, changes data in the table",
                command.name
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "catalog without rows:\n{}",
        problems.join("\n")
    );
}

#[test]
fn every_row_names_a_catalog_command_of_its_group() {
    let table = table::load();
    let commands = catalog::load();
    let catalog = catalog_by_name(&commands);
    let problems: Vec<String> = table
        .row
        .iter()
        .filter(|row| {
            catalog
                .get(row.command.as_str())
                .is_none_or(|command| command.group != row.group)
        })
        .map(table::Row::label)
        .collect();
    assert!(
        problems.is_empty(),
        "rows without a catalog command:\n{}",
        problems.join("\n")
    );
}

#[test]
fn every_row_and_case_resolve_to_each_other() {
    let table = table::load();
    assert_eq!(CASES.len(), CASE_COUNT, "coverage cases");

    let names: BTreeSet<&str> = CASES.iter().map(|case| case.name).collect();
    assert_eq!(names.len(), CASES.len(), "case names are unique");

    let mut exercised = BTreeSet::new();
    let mut row_sets = BTreeSet::new();
    let mut common = BTreeSet::new();
    for case in CASES {
        match case.rows {
            Rows::Table(keys) => {
                for &(command, action) in keys {
                    let row = table
                        .find(command, action)
                        .unwrap_or_else(|| panic!("{}: no row {command} {action:?}", case.name));
                    exercised.insert(row.key());
                }
                row_sets.insert(keys);
            }
            Rows::Common(parameter) => {
                common.insert(parameter);
            }
        }
    }
    let unexercised: Vec<String> = table
        .row
        .iter()
        .filter(|row| !exercised.contains(&row.key()))
        .map(table::Row::label)
        .collect();
    assert!(
        unexercised.is_empty(),
        "rows without a case:\n{}",
        unexercised.join("\n")
    );
    assert_eq!(row_sets.len(), SOURCE_ROWS, "source rows exercised");
    assert_eq!(
        common.len(),
        COMMON_PARAMETERS,
        "common parameters exercised"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-migrate/tests/migrate/binding.rs
// Responsibility:   Binding of migrated programs to the command catalog.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Binding of migrated programs to the command catalog.
//!
//! Every statement of the table scripts and of the common command set that is
//! translated must bind to the catalog and keep its meaning; a statement that
//! does not bind is reported as an error of the translation with its code and
//! never counts as translated. The statements that do not bind yet are listed
//! in the expected files next to the data; they point at gaps of the habit
//! tables, not at the migration.

use std::fs;
use std::path::Path;

use nurt_contracts::spec::Os;
use nurt_migrate::{
    Catalog, MigrateOptions, Migration, Reason, ScriptLanguage, Status, TableTranslator,
    migrate_with,
};

use crate::golden::{assert_matches_file, data_directory};

/// Directory of the command catalog, relative to the crate.
const CATALOG_DIRECTORY: &str = "../../catalog";

pub(crate) fn translator(host_os: Os) -> TableTranslator {
    let dir = Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join(CATALOG_DIRECTORY);
    let catalog = match Catalog::load_dir(&dir) {
        Ok(catalog) => catalog,
        Err(error) => panic!("cannot load the catalog: {error}"),
    };
    match TableTranslator::new().with_catalog(catalog, host_os) {
        Ok(translator) => translator,
        Err(error) => panic!("cannot enable binding: {error:?}"),
    }
}

pub(crate) fn migrate_bound(
    source: &str,
    language: ScriptLanguage,
    translator: &TableTranslator,
) -> Migration {
    let options = MigrateOptions::new().with_script("bound.sh");
    match migrate_with(source, language, &options, translator) {
        Ok(migration) => migration,
        Err(error) => panic!("migration failed: {error:?}"),
    }
}

fn lines(path: &Path) -> Vec<String> {
    match fs::read_to_string(path) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(error) => panic!("cannot read {}: {error}", path.display()),
    }
}

/// Number of commands of the common command set.
const COMMON_COMMANDS: usize = 500;

/// Statements that were translated but do not bind, with their errors.
fn binding_failures(migration: &Migration) -> Vec<String> {
    migration
        .report
        .items
        .iter()
        .filter(|item| item.reason == Some(Reason::Translation))
        .map(|item| {
            format!(
                "{} -> {} {}",
                item.source,
                item.code.map(|code| code.to_string()).unwrap_or_default(),
                item.problem.as_deref().unwrap_or_default()
            )
        })
        .collect()
}

#[test]
fn translated_commands_bind_or_report_an_error() {
    let bash = translator(Os::Linux);
    let powershell = translator(Os::Windows);
    let directory = data_directory().join("common-500");
    let mut failures = Vec::new();
    let mut translated = 0_usize;
    for (input, language, translator) in [
        ("bash.txt", ScriptLanguage::Bash, &bash),
        ("powershell.txt", ScriptLanguage::PowerShell, &powershell),
    ] {
        for command in lines(&directory.join(input)) {
            let migration = migrate_bound(&format!("{command}\n"), language, translator);
            let items = &migration.report.items;
            if !items.is_empty() && items.iter().all(|item| item.status.is_translated()) {
                translated += 1;
            }
            for item in items {
                if item.status == Status::Error {
                    assert!(
                        item.code.is_some() && item.problem.is_some(),
                        "error without a code or a problem: {command}"
                    );
                }
            }
            failures.extend(binding_failures(&migration));
        }
    }
    let text = format!(
        "translated and bound without error: {translated} of {COMMON_COMMANDS}\ntranslations that do not bind: {}\n{}\n",
        failures.len(),
        failures.join("\n")
    );
    assert_matches_file(&directory.join("binding.expected"), &text);
}

#[test]
fn translated_table_scripts_bind() {
    let bash = translator(Os::Linux);
    let powershell = translator(Os::Windows);
    let mut failures = Vec::new();
    for (directory, extension, language, translator) in [
        ("bash", "sh", ScriptLanguage::Bash, &bash),
        ("powershell", "ps1", ScriptLanguage::PowerShell, &powershell),
    ] {
        let Ok(entries) = fs::read_dir(data_directory().join(directory)) else {
            panic!("cannot list the {directory} scripts");
        };
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|value| value == extension))
            .collect();
        paths.sort();
        for path in paths {
            let source = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => panic!("cannot read {}: {error}", path.display()),
            };
            let migration = migrate_bound(&source, language, translator);
            failures.extend(binding_failures(&migration));
        }
    }
    let text = format!(
        "translations that do not bind: {}\n{}\n",
        failures.len(),
        failures.join("\n")
    );
    assert_matches_file(&data_directory().join("binding-scripts.expected"), &text);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl

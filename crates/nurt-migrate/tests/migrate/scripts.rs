// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test scripts of the bash and PowerShell habit table entries.
//!
//! Each entry `B01` to `B40` and `P01` to `P42` has at least one script named
//! after the entry in lower case (`b01-remove.sh`, `p02-find.ps1`).

use std::fs;
use std::path::PathBuf;

use nurt_migrate::{MigrateOptions, Reason, ScriptLanguage, migrate_with};

use crate::golden::data_directory;
use crate::mock::MockTranslator;

/// Number of entries of the bash habit table.
const BASH_ENTRIES: u32 = 40;

/// Number of entries of the PowerShell habit table.
const POWERSHELL_ENTRIES: u32 = 42;

struct Suite {
    directory: &'static str,
    extension: &'static str,
    prefix: char,
    entries: u32,
    language: ScriptLanguage,
}

const SUITES: [Suite; 2] = [
    Suite {
        directory: "bash",
        extension: "sh",
        prefix: 'b',
        entries: BASH_ENTRIES,
        language: ScriptLanguage::Bash,
    },
    Suite {
        directory: "powershell",
        extension: "ps1",
        prefix: 'p',
        entries: POWERSHELL_ENTRIES,
        language: ScriptLanguage::PowerShell,
    },
];

fn scripts(suite: &Suite) -> Vec<PathBuf> {
    let directory = data_directory().join(suite.directory);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => panic!("cannot list {}: {error}", directory.display()),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == suite.extension)
        })
        .collect();
    paths.sort();
    paths
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[test]
fn every_table_entry_has_a_script() {
    for suite in &SUITES {
        let names: Vec<String> = scripts(suite).iter().map(|path| file_name(path)).collect();
        for entry in 1..=suite.entries {
            let prefix = format!("{}{entry:02}-", suite.prefix);
            assert!(
                names.iter().any(|name| name.starts_with(&prefix)),
                "no script for entry {prefix} in {}",
                suite.directory
            );
        }
    }
}

#[test]
fn table_scripts_have_no_syntax_errors() {
    for suite in &SUITES {
        for path in scripts(suite) {
            let name = file_name(&path);
            let source = match fs::read_to_string(&path) {
                Ok(source) => source,
                Err(error) => panic!("cannot read {name}: {error}"),
            };
            let translator = MockTranslator::new();
            let options = MigrateOptions::new().with_script(name.as_str());
            let migration = match migrate_with(&source, suite.language, &options, &translator) {
                Ok(migration) => migration,
                Err(error) => panic!("migration of {name} failed: {error:?}"),
            };
            assert!(
                !migration.report.items.is_empty(),
                "{name} has no statement"
            );
            let invalid: Vec<String> = migration
                .report
                .items
                .iter()
                .filter(|item| item.reason == Some(Reason::Syntax))
                .map(|item| format!("{}:{} {}", item.line, item.column, item.source))
                .collect();
            assert!(invalid.is_empty(), "{name} has syntax errors: {invalid:?}");
        }
    }
}

#[test]
fn table_scripts_cover_both_languages() {
    let counts: Vec<usize> = SUITES.iter().map(|suite| scripts(suite).len()).collect();
    let total: usize = counts.iter().sum();
    let entries: u32 = SUITES.iter().map(|suite| suite.entries).sum();
    assert!(
        total >= usize::try_from(entries).unwrap_or(usize::MAX),
        "{counts:?}"
    );
}

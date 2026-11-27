// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Migration with the habit tables of the correction crate.
//!
//! Every script of the table entries is compared byte for byte with the
//! expected program file and JSON report next to it; the common command set
//! is compared with its expected results and must reach the translation
//! measure. With `NURT_REGENERATE_MIGRATE=1` the tests write the expected
//! files instead of comparing them.

use std::fs;
use std::path::{Path, PathBuf};

use nurt_correct::bash::BashTable;
use nurt_correct::powershell::PowerShellTable;
use nurt_migrate::{
    MigrateOptions, Migration, Reason, ScriptLanguage, Status, TableTranslator, migrate,
    migrate_with,
};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

use crate::golden::{assert_matches_file, data_directory};

/// Smallest share of common commands translated without an error, in
/// percent.
const TRANSLATION_MEASURE_PERCENT: usize = 87;

/// Number of commands of the common command set.
const COMMON_COMMANDS: usize = 500;

/// Directory of the correction tables, relative to the crate.
const TABLE_DIRECTORY: &str = "../nurt-correct/tables";

/// Script directories of the table entries.
const SUITES: [(&str, &str, ScriptLanguage); 2] = [
    ("bash", "sh", ScriptLanguage::Bash),
    ("powershell", "ps1", ScriptLanguage::PowerShell),
];

/// Files of the common command set with their expected results.
const COMMON_SETS: [(&str, &str, ScriptLanguage); 2] = [
    ("bash.txt", "bash.expected", ScriptLanguage::Bash),
    (
        "powershell.txt",
        "powershell.expected",
        ScriptLanguage::PowerShell,
    ),
];

fn scripts(directory: &str, extension: &str) -> Vec<PathBuf> {
    let directory = data_directory().join(directory);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => panic!("cannot list {}: {error}", directory.display()),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|value| value == extension))
        .collect();
    paths.sort();
    paths
}

fn read(path: &Path) -> String {
    match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => panic!("cannot read {}: {error}", path.display()),
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn run(source: &str, language: ScriptLanguage, script: &str) -> Migration {
    let options = MigrateOptions::new().with_script(script);
    match migrate(source, language, &options) {
        Ok(migration) => migration,
        Err(error) => panic!("migration of {script} failed: {error:?}"),
    }
}

fn run_with(source: &str, language: ScriptLanguage, translator: &TableTranslator) -> Migration {
    let options = MigrateOptions::new().with_script("table.sh");
    match migrate_with(source, language, &options, translator) {
        Ok(migration) => migration,
        Err(error) => panic!("migration failed: {error:?}"),
    }
}

fn table(file: &str) -> String {
    read(
        &Path::new(
            std::env::var("CARGO_MANIFEST_DIR")
                .as_deref()
                .unwrap_or(env!("CARGO_MANIFEST_DIR")),
        )
        .join(TABLE_DIRECTORY)
        .join(file),
    )
}

#[test]
fn table_scripts_match_expected_output() {
    for (directory, extension, language) in SUITES {
        for path in scripts(directory, extension) {
            let migration = run(&read(&path), language, &name(&path));
            assert_matches_file(&path.with_extension("nurt"), &migration.nurt);
            assert_matches_file(&path.with_extension("json"), &migration.report.to_json());
        }
    }
}

#[test]
fn translated_statements_of_table_scripts_parse() {
    let mut failures = Vec::new();
    for (directory, extension, language) in SUITES {
        for path in scripts(directory, extension) {
            let migration = run(&read(&path), language, &name(&path));
            failures.extend(
                migration
                    .report
                    .items
                    .iter()
                    .filter(|item| item.reason == Some(Reason::Translation))
                    .map(|item| format!("{}:{} {}", name(&path), item.line, item.source)),
            );
        }
    }
    assert!(
        failures.is_empty(),
        "translations that do not parse:\n{}",
        failures.join("\n")
    );
}

#[test]
fn table_scripts_translate_statements_of_both_tables() {
    for (directory, extension, language) in SUITES {
        let translated = scripts(directory, extension)
            .iter()
            .map(|path| run(&read(path), language, &name(path)))
            .flat_map(|migration| migration.report.items)
            .filter(|item| item.status.is_translated())
            .count();
        assert!(translated > 0, "no statement of {directory} is translated");
    }
}

#[test]
fn migration_with_tables_is_deterministic() {
    for (directory, extension, language) in SUITES {
        for path in scripts(directory, extension) {
            let source = read(&path);
            let first = run(&source, language, &name(&path));
            let second = run(&source, language, &name(&path));
            assert_eq!(first, second, "{}", name(&path));
        }
    }
}

/// One line of the expected results: status, command and program or error.
fn common_line(command: &str, migration: &Migration) -> (bool, String) {
    let items = &migration.report.items;
    let translated = !items.is_empty() && items.iter().all(|item| item.status.is_translated());
    let result: Vec<String> = items
        .iter()
        .map(|item| match (&item.output, item.code) {
            (Some(output), None) if item.status.is_translated() => {
                output.lines().collect::<Vec<_>>().join(" ")
            }
            (_, Some(code)) => format!("{code}: {}", item.problem.as_deref().unwrap_or_default()),
            _ => String::new(),
        })
        .collect();
    let status = if translated { "translated" } else { "failed" };
    (
        translated,
        format!("{status}\t{command}\t{}", result.join(" ; ")),
    )
}

/// Migrates the common command set, compares the results with the expected
/// files and returns the numbers of translated and of all commands.
fn common_commands() -> (usize, usize) {
    let directory = data_directory().join("common-500");
    let mut total = 0_usize;
    let mut translated = 0_usize;
    for (input, expected, language) in COMMON_SETS {
        let commands = read(&directory.join(input));
        let mut lines = Vec::new();
        let mut set_translated = 0_usize;
        let mut set_total = 0_usize;
        for command in commands.lines() {
            let migration = run(&format!("{command}\n"), language, input);
            let (ok, line) = common_line(command, &migration);
            set_translated += usize::from(ok);
            set_total += 1;
            lines.push(line);
        }
        let mut text = format!("translated without error: {set_translated} of {set_total}\n");
        for line in lines {
            text.push_str(&line);
            text.push('\n');
        }
        assert_matches_file(&directory.join(expected), &text);
        total += set_total;
        translated += set_translated;
    }
    (translated, total)
}

#[test]
fn common_commands_match_expected_results() {
    let (_, total) = common_commands();
    assert_eq!(total, COMMON_COMMANDS);
}

#[test]
#[ignore = "acceptance measure of the habit tables; run with --ignored"]
fn common_commands_meet_translation_measure() {
    let (translated, total) = common_commands();
    assert!(
        translated * 100 >= total * TRANSLATION_MEASURE_PERCENT,
        "{translated} of {total} common commands translated without error"
    );
}

#[test]
fn bash_table_entry_change_changes_migration() {
    let original = table("bash.toml");
    let changed = original.replacen(
        "template = \"remove $all\"",
        "template = \"delete $all\"",
        1,
    );
    assert_ne!(changed, original, "the bash table has no entry to change");
    let Ok(custom) = BashTable::parse(&changed, &table("habits_bash.toml")) else {
        panic!("the changed bash table is invalid");
    };
    let script = "rm -rf /srv/app/build\n";
    let before = run(script, ScriptLanguage::Bash, "table.sh");
    let after = run_with(
        script,
        ScriptLanguage::Bash,
        &TableTranslator::new().with_bash_table(custom),
    );
    assert_eq!(
        before.report.items[0].output.as_deref(),
        Some("remove \"/srv/app/build\"")
    );
    assert_eq!(
        after.report.items[0].output.as_deref(),
        Some("delete \"/srv/app/build\"")
    );
}

#[test]
fn powershell_table_entry_change_changes_migration() {
    let original = table("powershell.toml");
    let changed = original.replacen("\"Remove-Item\", \"ri\", ", "\"Remove-Item\", ", 1);
    assert_ne!(
        changed, original,
        "the PowerShell table has no entry to change"
    );
    let Ok(custom) = PowerShellTable::parse(&changed, &table("habits_powershell.toml")) else {
        panic!("the changed PowerShell table is invalid");
    };
    let script = "ri C:\\proj\\obj -Recurse -Force\n";
    let before = run(script, ScriptLanguage::PowerShell, "table.ps1");
    let after = run_with(
        script,
        ScriptLanguage::PowerShell,
        &TableTranslator::new().with_powershell_table(custom),
    );
    assert!(before.report.items[0].status.is_translated());
    assert_eq!(after.report.items[0].status, Status::Impossible);
}

#[test]
fn assignment_name_is_known_to_later_statement() {
    let script = "$files = Get-ChildItem C:\\proj -Recurse -Filter *.cs\n$files | Remove-Item\n";
    let migration = run(script, ScriptLanguage::PowerShell, "assign.ps1");
    let items = &migration.report.items;
    assert!(
        items[0]
            .output
            .as_deref()
            .is_some_and(|output| output.starts_with("let files = find ")),
        "{items:?}"
    );
    assert!(
        items[1]
            .problem
            .as_deref()
            .is_none_or(|problem| !problem.contains("unknown_variable")),
        "{items:?}"
    );
}

#[test]
fn rejection_names_problem_of_habit_table() {
    let migration = run("sudo -i\n", ScriptLanguage::Bash, "sudo.sh");
    let item = &migration.report.items[0];
    assert_eq!(item.status, Status::Impossible);
    assert!(
        item.problem
            .as_deref()
            .is_some_and(|problem| problem.ends_with("(elevated_session)")),
        "{item:?}"
    );
}

#[test]
fn sudo_translates_with_the_call_parameter_on_the_stage() {
    let migration = run(
        "sudo rm -rf /var/cache/app\nsudo -u deploy touch /srv/www/.maintenance\n",
        ScriptLanguage::Bash,
        "sudo.sh",
    );
    let program = &migration.nurt;
    assert!(
        program.contains("remove \"/var/cache/app\" privilege: elevated"),
        "{program}"
    );
    assert!(program.contains("as_user: \"deploy\""), "{program}");
    assert!(
        migration
            .report
            .items
            .iter()
            .all(|item| item.status == Status::Translated)
    );
}

#[test]
fn program_with_a_long_script_path_parses() {
    let script = format!("/srv/{}deploy.sh", "release-2026-11-26/".repeat(8));
    for (source, language) in [
        (
            "rm -rf /srv/app/build\nfor f in *; do echo; done\n",
            ScriptLanguage::Bash,
        ),
        (
            "Remove-Item C:\\build -Recurse\nforeach ($f in $x) { $f }\n",
            ScriptLanguage::PowerShell,
        ),
    ] {
        let migration = run(source, language, &script);
        assert!(migration.report.fallback.is_some(), "{language}");
        assert!(
            parse(&migration.nurt, Variant::A, &ParseOptions::default()).is_ok(),
            "{}",
            migration.nurt
        );
    }
}

#[test]
fn untranslated_statement_proposes_running_script_unchanged() {
    let migration = run(
        "for f in /srv/*.log; do gzip \"$f\"; done\n",
        ScriptLanguage::Bash,
        "/srv/zip.sh",
    );
    assert_eq!(
        migration.report.fallback.as_deref(),
        Some("run [\"bash\", \"/srv/zip.sh\"]")
    );
}

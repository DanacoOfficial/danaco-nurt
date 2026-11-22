// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Mixed scripts compared byte for byte with the expected program file and
//! reports.
//!
//! With the environment variable `NURT_REGENERATE_MIGRATE=1` the tests write
//! the expected files instead of comparing them.

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::error::ErrorCode;
use nurt_migrate::{MigrateOptions, ScriptLanguage, migrate};

use crate::mock::MockTranslator;

/// Environment variable that switches the tests to writing expected files.
const REGENERATE_VARIABLE: &str = "NURT_REGENERATE_MIGRATE";

/// Directory of the migration test data, relative to the crate.
const DATA_DIRECTORY: &str = "../../tests/d1/migrate";

pub(crate) fn data_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(DATA_DIRECTORY)
}

fn regenerate() -> bool {
    std::env::var(REGENERATE_VARIABLE).is_ok_and(|value| value == "1")
}

/// Compares `actual` with the content of `path`, or writes it when the
/// expected files are regenerated.
pub(crate) fn assert_matches_file(path: &Path, actual: &str) {
    if regenerate() {
        if let Err(error) = fs::write(path, actual) {
            panic!("cannot write {}: {error}", path.display());
        }
        return;
    }
    match fs::read_to_string(path) {
        Ok(expected) => assert!(
            expected == actual,
            "{} differs from the migration result:\n{actual}",
            path.display()
        ),
        Err(error) => panic!(
            "cannot read {}: {error}; set {REGENERATE_VARIABLE}=1 to create it",
            path.display()
        ),
    }
}

/// Migrates `mock/<name>.<extension>` and compares the program file, the
/// JSON report and the text report with `mock/<name>.{nurt,json,txt}`.
fn check_script(
    name: &str,
    extension: &str,
    language: ScriptLanguage,
    translator: &MockTranslator,
) {
    let directory = data_directory().join("mock");
    let script = format!("{name}.{extension}");
    let source = match fs::read_to_string(directory.join(&script)) {
        Ok(source) => source,
        Err(error) => panic!("cannot read {script}: {error}"),
    };
    let options = MigrateOptions::new().with_script(script.as_str());
    let migration = match migrate(&source, language, &options, translator) {
        Ok(migration) => migration,
        Err(error) => panic!("migration of {script} failed: {error:?}"),
    };
    assert_matches_file(&directory.join(format!("{name}.nurt")), &migration.nurt);
    assert_matches_file(
        &directory.join(format!("{name}.json")),
        &migration.report.to_json(),
    );
    assert_matches_file(
        &directory.join(format!("{name}.txt")),
        &migration.report.to_text(),
    );
}

#[test]
fn bash_mixed_script_matches_expected_output() {
    let mock = MockTranslator::new()
        .program("rm -rf /srv/app/build", "remove \"/srv/app/build\"", &["B01", "L21"])
        .program(
            "cd /srv/app && go test ./...",
            "run [\"go\", \"test\", \"./...\"] cwd: \"/srv/app\"",
            &["B20"],
        )
        .program(
            "SOURCES=$(find /srv/app/src -name '*.go')",
            "find \"/srv/app/src\" glob: \"**/*.go\"",
            &["B26:P", "B14"],
        )
        .assume(
            "SOURCES=$(find /srv/app/src -name '*.go')",
            "the command output is a list of paths",
        )
        .binds("SOURCES=$(find /srv/app/src -name '*.go')", "sources")
        .program(
            "grep -rn \"oldName(\" $SOURCES",
            "search \"oldName(\" ...@sources.path",
            &["B15:P", "B25"],
        )
        .assume(
            "grep -rn \"oldName(\" $SOURCES",
            "the pattern is a basic regular expression without special characters",
        )
        .requires("grep -rn \"oldName(\" $SOURCES", "SOURCES")
        .program("mkdir -p /srv/app/out", "create \"/srv/app/out\" type: directory", &["B05"])
        .program(
            "cp -r /srv/app/static /srv/app/out",
            "copy \"/srv/app/static\" to: \"/srv/app/out\"",
            &["B04"],
        )
        .fail(
            "ls /srv/app/out || echo \"missing output\"",
            ErrorCode::Uncorrectable,
            "no alternative branch; check the condition with assert and run the alternative as a separate step",
            &["list \"/srv/app/out\""],
        )
        .program(
            "cat > /srv/app/out/VERSION <<EOF\n1.4.2\nEOF",
            "write \"/srv/app/out/VERSION\" \"1.4.2\\n\" mode: overwrite",
            &["B39", "B19"],
        )
        .fail(
            "sudo systemctl restart app",
            ErrorCode::Uncorrectable,
            "no privilege elevation; run the console with the required privileges",
            &["services name: \"app\" |> restart"],
        )
        .fail(
            "cd /srv/app",
            ErrorCode::Uncorrectable,
            "the program has no current directory; relative paths resolve against the invocation parameter base",
            &[],
        )
        .program(
            "tar -czf /srv/backup/app.tgz /srv/app/out &",
            "find \"/srv/app/out\" |> archive.create \"/srv/backup/app.tgz\" background: true",
            &["B34", "B29"],
        )
        .unbound("background: true");
    check_script("bash-mixed", "sh", ScriptLanguage::Bash, &mock);
}

#[test]
fn powershell_mixed_script_matches_expected_output() {
    let mock = MockTranslator::new()
        .program(
            "$files = Get-ChildItem -Path C:\\proj -Recurse -Filter *.go",
            "find \"C:/proj\" glob: \"**/*.go\"",
            &["P32", "P02", "L19"],
        )
        .binds("$files = Get-ChildItem -Path C:\\proj -Recurse -Filter *.go", "files")
        .program(
            "$files | Where-Object { $_.Length -gt 1MB } | Remove-Item",
            "@files |> filter size > 1mb |> remove",
            &["P15:P", "P08"],
        )
        .assume(
            "$files | Where-Object { $_.Length -gt 1MB } | Remove-Item",
            "the property Length is the file size",
        )
        .requires("$files | Where-Object { $_.Length -gt 1MB } | Remove-Item", "files")
        .fail(
            "Set-Location C:\\proj",
            ErrorCode::Uncorrectable,
            "the program has no current location; relative paths resolve against the invocation parameter base",
            &[],
        )
        .fail(
            "Remove-Item .\\build -Recurse -Force",
            ErrorCode::PathRelative,
            "relative path without the invocation parameter base",
            &[],
        )
        .program("Write-Host \"done\"", "evaluate \"done\"", &["P36"])
        .program(
            "Copy-Item -Path C:\\proj\\a.txt -Destination C:\\backup",
            "copy \"C:/proj/a.txt\" to: \"C:/backup\"",
            &["P06", "L19"],
        )
        .program("Write-Output \"copied\"", "evaluate \"copied\"", &["P36"])
        .fail(
            "Start-Process -FilePath git -ArgumentList \"status --short\" -Wait",
            ErrorCode::Ambiguous,
            "the argument list is one text; the division into arguments is not determined",
            &[],
        )
        .candidates(
            "Start-Process -FilePath git -ArgumentList \"status --short\" -Wait",
            &[
                "run [\"git\", \"status\", \"--short\"] wait: forever",
                "run [\"git\", \"status --short\"] wait: forever",
            ],
        );
    check_script("powershell-mixed", "ps1", ScriptLanguage::PowerShell, &mock);
}

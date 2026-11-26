// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of the script language.

use nurt_contracts::error::ErrorCode;
use nurt_migrate::{MigrateOptions, ScriptLanguage, Status, migrate};

#[test]
fn detects_language_from_extension() {
    let cases = [
        ("deploy.sh", ScriptLanguage::Bash),
        ("deploy.bash", ScriptLanguage::Bash),
        ("C:/scripts/Setup.PS1", ScriptLanguage::PowerShell),
        ("setup.ps1", ScriptLanguage::PowerShell),
    ];
    for (path, expected) in cases {
        assert_eq!(
            ScriptLanguage::detect(path, "").ok(),
            Some(expected),
            "{path}"
        );
    }
}

#[test]
fn extension_takes_precedence_over_shebang() {
    let language = ScriptLanguage::detect("setup.ps1", "#!/bin/bash\n").ok();
    assert_eq!(language, Some(ScriptLanguage::PowerShell));
}

#[test]
fn detects_language_from_shebang() {
    let cases = [
        ("#!/bin/sh\n", ScriptLanguage::Bash),
        ("#!/bin/bash -e\n", ScriptLanguage::Bash),
        ("#!/usr/bin/env bash\n", ScriptLanguage::Bash),
        ("#!/usr/bin/env -S dash -e\n", ScriptLanguage::Bash),
        ("\u{feff}#!/usr/bin/pwsh\n", ScriptLanguage::PowerShell),
        (
            "#!/usr/bin/env pwsh -NoProfile\n",
            ScriptLanguage::PowerShell,
        ),
        (
            "#!C:\\Windows\\powershell.exe\n",
            ScriptLanguage::PowerShell,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(
            ScriptLanguage::detect("tool", source).ok(),
            Some(expected),
            "{source}"
        );
    }
}

#[test]
fn rejects_program_file_as_input() {
    let error = ScriptLanguage::detect("/srv/clean.nurt", "remove \"/srv/a\"\n").err();
    let error = error.map(|error| (error.code(), error.data().message.clone()));
    assert_eq!(
        error,
        Some((
            ErrorCode::Arg,
            "the file is already a program; migrate a .sh or .ps1 script".to_owned()
        ))
    );
}

#[test]
fn rejects_script_of_unknown_language() {
    for (path, source) in [
        ("tool", "rm x\n"),
        ("tool.py", "#!/usr/bin/env python3\n"),
        ("tool", ""),
    ] {
        let code = ScriptLanguage::detect(path, source)
            .err()
            .map(|error| error.code());
        assert_eq!(code, Some(ErrorCode::Arg), "{path}");
    }
}

#[test]
fn names_language_and_interpreter() {
    assert_eq!(ScriptLanguage::Bash.as_str(), "bash");
    assert_eq!(ScriptLanguage::PowerShell.to_string(), "powershell");
    assert_eq!(ScriptLanguage::Bash.interpreter(), "bash");
    assert_eq!(ScriptLanguage::PowerShell.interpreter(), "pwsh");
}

#[test]
fn habit_table_follows_the_language_of_the_script_and_not_the_shape_of_the_text() {
    // The table is chosen by the language given for the script (from its name
    // or shebang), so a statement is never read with the table of the other shell.
    let source = "rm -rf /srv/app/build\n";
    let options = MigrateOptions::new().with_script("table.sh");
    let status = |language| {
        migrate(source, language, &options)
            .map(|migration| migration.report.items.first().map(|item| item.status))
    };
    let bash = status(ScriptLanguage::Bash);
    let powershell = status(ScriptLanguage::PowerShell);
    assert!(
        matches!(bash, Ok(Some(Status::Translated | Status::Assumption))),
        "{bash:?}"
    );
    assert!(
        matches!(powershell, Ok(Some(Status::Impossible | Status::Error))),
        "{powershell:?}"
    );
}

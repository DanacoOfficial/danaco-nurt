// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Statement handling of the migration.

use std::time::{Duration, Instant};

use nurt_contracts::error::ErrorCode;
use nurt_migrate::{
    Item, MigrateOptions, Migration, REPORT_FORMAT, Reason, ScriptLanguage, Status, migrate,
};

use crate::mock::MockTranslator;

const BASH: ScriptLanguage = ScriptLanguage::Bash;
const POWERSHELL: ScriptLanguage = ScriptLanguage::PowerShell;

/// Upper bound for the migration of a script of 1 000 lines.
const LARGE_SCRIPT_LIMIT: Duration = Duration::from_secs(1);

/// Number of lines of the large script.
const LARGE_SCRIPT_LINES: usize = 1_000;

/// Statements on each line of the large script.
const PARTS_PER_LINE: usize = 3;

fn run(source: &str, language: ScriptLanguage, translator: &MockTranslator) -> Migration {
    let options = MigrateOptions::new().with_script("/srv/s.sh");
    match migrate(source, language, &options, translator) {
        Ok(migration) => migration,
        Err(error) => panic!("migration failed: {error:?}"),
    }
}

fn statuses(migration: &Migration) -> Vec<(u32, u32, Status)> {
    migration
        .report
        .items
        .iter()
        .map(|item| (item.line, item.column, item.status))
        .collect()
}

fn reasons(migration: &Migration) -> Vec<Option<Reason>> {
    migration
        .report
        .items
        .iter()
        .map(|item| item.reason)
        .collect()
}

fn only_item(migration: &Migration) -> &Item {
    match migration.report.items.as_slice() {
        [item] => item,
        items => panic!("expected one item, found {}", items.len()),
    }
}

#[test]
fn translates_simple_statement_with_rule_comment() {
    let mock = MockTranslator::new().program(
        "rm -rf /srv/app/build",
        "remove \"/srv/app/build\"",
        &["B01", "L21"],
    );
    let migration = run("rm -rf /srv/app/build\n", BASH, &mock);
    let item = only_item(&migration);
    assert_eq!(item.status, Status::Translated);
    assert_eq!(item.rules, ["B01", "L21"]);
    assert_eq!(item.output.as_deref(), Some("remove \"/srv/app/build\""));
    assert!(
        migration
            .nurt
            .ends_with("\n# translated: B01, L21\nremove \"/srv/app/build\"\n")
    );
    assert_eq!(migration.report.fallback, None);
}

#[test]
fn probable_rule_or_assumption_yields_assumption_status() {
    let mock = MockTranslator::new()
        .program("grep -rn x /srv/a", "search \"x\" \"/srv/a\"", &["B15:P"])
        .program("cat /srv/a", "read \"/srv/a\"", &["B10"])
        .assume("cat /srv/a", "the file is text");
    let migration = run("grep -rn x /srv/a\ncat /srv/a\n", BASH, &mock);
    assert_eq!(
        statuses(&migration),
        [(1, 1, Status::Assumption), (2, 1, Status::Assumption)]
    );
    assert!(
        migration
            .nurt
            .contains("# translated: B10\n# assumption: the file is text\nread \"/srv/a\"\n")
    );
}

#[test]
fn repeated_rule_is_listed_once() {
    let mock =
        MockTranslator::new().program("ls /a /b", "list \"/a\" \"/b\"", &["B13", "L21", "L21"]);
    let migration = run("ls /a /b\n", BASH, &mock);
    assert_eq!(only_item(&migration).rules, ["B13", "L21"]);
}

#[test]
fn whole_list_is_translated_when_the_translator_accepts_it() {
    let statement = "cd /srv/app && go test ./...";
    let mock = MockTranslator::new().program(
        statement,
        "run [\"go\", \"test\", \"./...\"] cwd: \"/srv/app\"",
        &["B20"],
    );
    let migration = run("cd /srv/app && go test ./...\n", BASH, &mock);
    assert_eq!(mock.requests(), [statement]);
    assert_eq!(only_item(&migration).status, Status::Translated);
    assert_eq!(migration.report.base, None);
}

#[test]
fn and_list_with_two_effects_becomes_consecutive_statements() {
    let mock = MockTranslator::new()
        .program(
            "mkdir -p /srv/out",
            "create \"/srv/out\" type: directory",
            &["B05"],
        )
        .program(
            "cp /srv/a /srv/out",
            "copy \"/srv/a\" to: \"/srv/out\"",
            &["B04"],
        );
    let migration = run("mkdir -p /srv/out && cp /srv/a /srv/out\n", BASH, &mock);
    assert_eq!(
        mock.requests(),
        [
            "mkdir -p /srv/out && cp /srv/a /srv/out",
            "mkdir -p /srv/out",
            "cp /srv/a /srv/out"
        ]
    );
    assert_eq!(
        statuses(&migration),
        [(1, 1, Status::Translated), (1, 22, Status::Translated)]
    );
    assert!(migration.nurt.contains(
        "\n# translated: B05\ncreate \"/srv/out\" type: directory\n# translated: B04\ncopy \"/srv/a\" to: \"/srv/out\"\n"
    ));
}

#[test]
fn statement_after_semicolon_carries_stop_on_error_assumption() {
    let mock = MockTranslator::new()
        .program("rm /srv/a", "remove \"/srv/a\"", &["B01"])
        .program("rm /srv/b", "remove \"/srv/b\"", &["B01"]);
    let migration = run("rm /srv/a; rm /srv/b\n", BASH, &mock);
    assert_eq!(
        statuses(&migration),
        [(1, 1, Status::Translated), (1, 12, Status::Assumption)]
    );
    let assumptions = &migration.report.items[1].assumptions;
    assert_eq!(assumptions.len(), 1);
    assert!(assumptions[0].starts_with("the script stops on the first error"));
}

#[test]
fn semicolon_joins_only_statements_on_one_line() {
    let mock = MockTranslator::new();
    run("rm /srv/a;\nrm /srv/b\n", BASH, &mock);
    assert_eq!(mock.requests(), ["rm /srv/a", "rm /srv/b"]);
}

#[test]
fn or_list_is_not_split_and_reports_or_list() {
    let statement = "ls /srv/a || echo missing";
    let mock = MockTranslator::new().fail(
        statement,
        ErrorCode::Uncorrectable,
        "no alternative branch",
        &["list \"/srv/a\""],
    );
    let migration = run("ls /srv/a || echo missing\n", BASH, &mock);
    assert_eq!(mock.requests(), [statement]);
    let item = only_item(&migration);
    assert_eq!(item.status, Status::Impossible);
    assert_eq!(item.reason, Some(Reason::OrList));
    assert_eq!(item.fix, ["list \"/srv/a\""]);
    assert_eq!(
        migration.report.fallback.as_deref(),
        Some("run [\"bash\", \"/srv/s.sh\"]")
    );
}

#[test]
fn rejected_list_other_than_one_effect_is_reported_whole() {
    let statement = "sudo rm /etc/x && echo done";
    let mock = MockTranslator::new().fail(
        statement,
        ErrorCode::Uncorrectable,
        "no privilege elevation",
        &[],
    );
    let migration = run("sudo rm /etc/x && echo done\n", BASH, &mock);
    let item = only_item(&migration);
    assert_eq!(
        (item.status, item.reason),
        (Status::Impossible, Some(Reason::Rule))
    );
    assert_eq!(item.problem.as_deref(), Some("no privilege elevation"));
}

#[test]
fn bound_variable_is_passed_to_later_statements() {
    let mock = MockTranslator::new()
        .program(
            "FILES=$(find /srv -name '*.log')",
            "find \"/srv\" glob: \"**/*.log\"",
            &["B26:P", "B14"],
        )
        .binds("FILES=$(find /srv -name '*.log')", "files")
        .program("rm $FILES", "@files |> remove", &["B25", "B01"])
        .requires("rm $FILES", "FILES");
    let migration = run("FILES=$(find /srv -name '*.log')\nrm $FILES\n", BASH, &mock);
    assert_eq!(
        statuses(&migration),
        [(1, 1, Status::Assumption), (2, 1, Status::Translated)]
    );
    assert!(
        migration
            .nurt
            .contains("let files = find \"/srv\" glob: \"**/*.log\"\n")
    );
}

#[test]
fn powershell_assignment_binds_variable() {
    let mock = MockTranslator::new()
        .program(
            "$logs = Get-ChildItem C:\\logs",
            "find \"C:/logs\"",
            &["P32", "P01", "L19"],
        )
        .binds("$logs = Get-ChildItem C:\\logs", "logs")
        .program("Remove-Item $logs", "@logs |> remove", &["P08"])
        .requires("Remove-Item $logs", "logs");
    let migration = run(
        "$logs = Get-ChildItem C:\\logs\nRemove-Item $logs\n",
        POWERSHELL,
        &mock,
    );
    assert_eq!(
        statuses(&migration),
        [(1, 1, Status::Translated), (2, 1, Status::Translated)]
    );
}

#[test]
fn variable_of_rejected_assignment_stays_unbound() {
    let mock = MockTranslator::new()
        .program("rm $FILES", "@files |> remove", &["B25", "B01"])
        .requires("rm $FILES", "FILES");
    let migration = run("FILES=$(ls /srv)\nrm $FILES\n", BASH, &mock);
    assert_eq!(
        statuses(&migration),
        [(1, 1, Status::Impossible), (2, 1, Status::Impossible)]
    );
}

#[test]
fn translation_that_does_not_bind_is_an_error_with_output() {
    let mock = MockTranslator::new()
        .program(
            "tar -czf /b/a.tgz /a &",
            "archive.create \"/b/a.tgz\" background: true",
            &["B34", "B29"],
        )
        .unbound("background: true");
    let migration = run("tar -czf /b/a.tgz /a &\n", BASH, &mock);
    let item = only_item(&migration);
    assert_eq!(item.status, Status::Error);
    assert_eq!(item.reason, Some(Reason::Translation));
    assert_eq!(item.code, Some(ErrorCode::UnknownParam));
    assert_eq!(
        item.output.as_deref(),
        Some("archive.create \"/b/a.tgz\" background: true")
    );
    assert!(
        migration
            .nurt
            .contains("# untranslated: E_UNKNOWN_PARAM: tar -czf /b/a.tgz /a &\n")
    );
}

#[test]
fn background_operator_belongs_to_its_statement() {
    let mock = MockTranslator::new();
    run("sleep 10 & rm /srv/a\n", BASH, &mock);
    assert_eq!(mock.requests(), ["sleep 10 &", "rm /srv/a"]);
}

#[test]
fn syntax_error_affects_only_its_statement() {
    let mock = MockTranslator::new().program("rm /srv/a", "remove \"/srv/a\"", &["B01"]);
    let migration = run("rm /srv/a\nrm \"/srv/b\n", BASH, &mock);
    assert_eq!(
        statuses(&migration),
        [(1, 1, Status::Translated), (2, 1, Status::Error)]
    );
    let item = &migration.report.items[1];
    assert_eq!(item.code, Some(ErrorCode::Syntax));
    assert_eq!(item.reason, Some(Reason::Syntax));
    assert_eq!(mock.requests(), ["rm /srv/a"]);
}

#[test]
fn bash_constructs_are_impossible_without_translation() {
    let source = "\
for f in /a/*; do rm \"$f\"; done
while read l; do echo \"$l\"; done < /a/list
until false; do sleep 1; done
if [ -f /a ]; then rm /a; fi
case $1 in x) echo x;; esac
clean() { rm -rf /tmp/x; }
( cd /a && make )
{ echo a; echo b; } > /a/out
eval \"$CMD\"
source /etc/profile
. /etc/profile
";
    let mock = MockTranslator::new();
    let migration = run(source, BASH, &mock);
    assert!(mock.requests().is_empty());
    assert_eq!(
        reasons(&migration),
        [
            Some(Reason::Loop),
            Some(Reason::Loop),
            Some(Reason::Loop),
            Some(Reason::Condition),
            Some(Reason::Condition),
            Some(Reason::Function),
            Some(Reason::Block),
            Some(Reason::Block),
            Some(Reason::Eval),
            Some(Reason::Source),
            Some(Reason::Source),
        ]
    );
    assert!(migration.report.items.iter().all(|item| {
        item.status == Status::Impossible && item.code == Some(ErrorCode::Uncorrectable)
    }));
}

#[test]
fn loop_inside_pipeline_or_substitution_is_a_loop() {
    let mock = MockTranslator::new();
    let migration = run(
        "ls /a | while read f; do rm \"$f\"; done\nX=$(for i in 1 2; do echo $i; done)\n",
        BASH,
        &mock,
    );
    assert_eq!(
        reasons(&migration),
        [Some(Reason::Loop), Some(Reason::Loop)]
    );
}

#[test]
fn powershell_constructs_are_impossible_without_translation() {
    let source = "\
foreach ($f in $files) { Remove-Item $f }
for ($i = 0; $i -lt 3; $i++) { Start-Sleep 1 }
while ($true) { break }
do { Start-Sleep 1 } while ($false)
if (Test-Path C:\\a) { Remove-Item C:\\a }
switch ($x) { 1 { 'one' } }
function Clear-Build { Remove-Item C:\\build -Recurse }
try { Remove-Item C:\\a } catch { Write-Host 'failed' }
Invoke-Expression 'Get-Date'
iex $command
. C:\\scripts\\env.ps1
exit 1
";
    let mock = MockTranslator::new();
    let migration = run(source, POWERSHELL, &mock);
    assert!(mock.requests().is_empty());
    assert_eq!(
        reasons(&migration),
        [
            Some(Reason::Loop),
            Some(Reason::Loop),
            Some(Reason::Loop),
            Some(Reason::Loop),
            Some(Reason::Condition),
            Some(Reason::Condition),
            Some(Reason::Function),
            Some(Reason::Block),
            Some(Reason::Eval),
            Some(Reason::Eval),
            Some(Reason::Source),
            Some(Reason::Control),
        ]
    );
}

#[test]
fn script_parameters_are_impossible() {
    let mock = MockTranslator::new();
    let migration = run(
        "param([string]$Path)\nRemove-Item $Path\n",
        POWERSHELL,
        &mock,
    );
    assert_eq!(reasons(&migration).first(), Some(&Some(Reason::Parameters)));
}

#[test]
fn cmdlet_script_block_is_submitted_for_translation() {
    let statement = "Get-ChildItem C:\\a | Where-Object { $_.Length -gt 1MB }";
    let mock = MockTranslator::new().program(
        statement,
        "find \"C:/a\" |> filter size > 1mb",
        &["P01", "P15", "L19"],
    );
    let migration = run(&format!("{statement}\n"), POWERSHELL, &mock);
    assert_eq!(only_item(&migration).status, Status::Translated);
}

#[test]
fn powershell_lists_follow_bash_lists() {
    let mock = MockTranslator::new()
        .program("Remove-Item C:\\a", "remove \"C:/a\"", &["P08", "L19"])
        .program("Write-Host done", "evaluate \"done\"", &["P36"]);
    let migration = run(
        "Remove-Item C:\\a; Write-Host done\nRemove-Item C:\\a && Write-Host done\nRemove-Item C:\\a || Write-Host done\n",
        POWERSHELL,
        &mock,
    );
    assert_eq!(
        statuses(&migration),
        [
            (1, 1, Status::Translated),
            (1, 19, Status::Assumption),
            (2, 1, Status::Translated),
            (2, 21, Status::Translated),
            (3, 1, Status::Impossible),
        ]
    );
    assert_eq!(migration.report.items[4].reason, Some(Reason::OrList));
}

#[test]
fn change_of_directory_proposes_base_without_setting_it() {
    let mock = MockTranslator::new()
        .fail(
            "cd /srv/app",
            ErrorCode::Uncorrectable,
            "the program has no current directory",
            &[],
        )
        .fail(
            "rm build/x",
            ErrorCode::PathRelative,
            "relative path without base",
            &[],
        );
    let migration = run("cd /srv/app\nrm build/x\n", BASH, &mock);
    assert_eq!(
        reasons(&migration),
        [
            Some(Reason::CurrentDirectory),
            Some(Reason::CurrentDirectory)
        ]
    );
    assert_eq!(migration.report.base.as_deref(), Some("/srv/app"));
    assert!(
        migration
            .nurt
            .contains("# Relative paths require the invocation parameter base: \"/srv/app\"\n")
    );
    assert_eq!(mock.requests(), ["cd /srv/app", "rm build/x"]);
}

#[test]
fn base_is_proposed_only_for_one_absolute_directory() {
    let cases = [
        ("cd /srv/a\ncd '/srv/a'\n", BASH, Some("/srv/a")),
        ("cd /srv/a\ncd /srv/b\n", BASH, None),
        ("cd /srv/a\ncd build\n", BASH, None),
        ("cd build\n", BASH, None),
        ("cd \"$HOME\"\n", BASH, None),
        (
            "Set-Location -Path 'C:\\proj'\n",
            POWERSHELL,
            Some("C:\\proj"),
        ),
        ("sl C:/proj\n", POWERSHELL, Some("C:/proj")),
        (
            "Push-Location \\\\server\\share\n",
            POWERSHELL,
            Some("\\\\server\\share"),
        ),
        ("cd $env:TEMP\n", POWERSHELL, None),
    ];
    for (source, language, expected) in cases {
        let migration = run(source, language, &MockTranslator::new());
        assert_eq!(migration.report.base.as_deref(), expected, "{source}");
    }
}

#[test]
fn rejected_here_document_reports_here_document() {
    let mock = MockTranslator::new();
    let migration = run("mail -s report admin <<EOF\nall done\nEOF\n", BASH, &mock);
    assert_eq!(
        mock.requests(),
        ["mail -s report admin <<EOF\nall done\nEOF"]
    );
    assert_eq!(reasons(&migration), [Some(Reason::HereDocument)]);
    assert!(migration.nurt.contains(
        "# untranslated: E_UNCORRECTABLE: mail -s report admin <<EOF\n# all done\n# EOF\n"
    ));
}

#[test]
fn candidates_of_ambiguous_translation_become_fixes() {
    let statement = "Start-Process git -ArgumentList \"status --short\"";
    let mock = MockTranslator::new()
        .fail(
            statement,
            ErrorCode::Ambiguous,
            "the argument list is one text",
            &[],
        )
        .candidates(
            statement,
            &[
                "run [\"git\", \"status\", \"--short\"]",
                "run [\"git\", \"status --short\"]",
            ],
        );
    let migration = run(&format!("{statement}\n"), POWERSHELL, &mock);
    let item = only_item(&migration);
    assert_eq!(item.code, Some(ErrorCode::Ambiguous));
    assert_eq!(item.fix.len(), 2);
    assert!(migration.nurt.contains("#   fix: run [\"git\", \"status\", \"--short\"]\n#   fix: run [\"git\", \"status --short\"]\n"));
}

#[test]
fn comments_and_blank_lines_are_carried_over() {
    let source = "#!/bin/bash\n# Remove old files.\n\n\nrm /srv/a # first\n\n# Then the second.\nrm /srv/b\n";
    let mock = MockTranslator::new()
        .program("rm /srv/a", "remove \"/srv/a\"", &["B01"])
        .program("rm /srv/b", "remove \"/srv/b\"", &["B01"]);
    let migration = run(source, BASH, &mock);
    let body = migration.nurt.split_once("\n\n").map(|(_, body)| body);
    assert_eq!(
        body,
        Some(
            "# Remove old files.\n\n# translated: B01\nremove \"/srv/a\"\n# first\n\n# Then the second.\n# translated: B01\nremove \"/srv/b\"\n"
        )
    );
    assert!(!migration.nurt.contains("#!/bin/bash"));
}

#[test]
fn powershell_block_comment_becomes_line_comments() {
    let source = "<#\n  Cleans the build.\n\n  Runs nightly.\n#>\nRemove-Item C:\\build\n";
    let migration = run(source, POWERSHELL, &MockTranslator::new());
    assert!(
        migration
            .nurt
            .contains("\n# Cleans the build.\n#\n# Runs nightly.\n")
    );
}

#[test]
fn columns_count_unicode_scalar_values() {
    let mock = MockTranslator::new();
    let migration = run("echo \"zażółć\"; rm /srv/a\n", BASH, &mock);
    assert_eq!(
        migration.report.items.get(1).map(|item| item.column),
        Some(16)
    );
}

#[test]
fn windows_line_endings_and_byte_order_mark_are_accepted() {
    let mock = MockTranslator::new();
    let migration = run("\u{feff}rm /srv/a\r\nrm /srv/b\r\n", BASH, &mock);
    assert_eq!(mock.requests(), ["rm /srv/a", "rm /srv/b"]);
    assert_eq!(statuses(&migration).len(), 2);
}

#[test]
fn fallback_names_interpreter_and_script() {
    let options = MigrateOptions::new();
    let mock = MockTranslator::new();
    let bash = migrate("sudo ls\n", BASH, &options, &mock)
        .ok()
        .map(|m| m.report.fallback);
    let powershell = migrate("Get-Foo\n", POWERSHELL, &options, &mock)
        .ok()
        .map(|m| m.report.fallback);
    assert_eq!(bash, Some(Some("run [\"bash\", \"script.sh\"]".to_owned())));
    assert_eq!(
        powershell,
        Some(Some("run [\"pwsh\", \"script.ps1\"]".to_owned()))
    );
}

#[test]
fn header_states_counts_and_fallback() {
    let mock = MockTranslator::new().program("rm /srv/a", "remove \"/srv/a\"", &["B01"]);
    let migration = run("rm /srv/a\nsudo rm /srv/b\n", BASH, &mock);
    let header: Vec<&str> = migration.nurt.lines().take(4).collect();
    assert_eq!(
        header,
        [
            "# Source: /srv/s.sh (bash)",
            "# Statements run in order; the first error stops the script.",
            "# Statements: 2; translated: 1; with assumptions: 0; impossible: 1; errors: 0.",
            "# Untranslated statements run unchanged with: run [\"bash\", \"/srv/s.sh\"]",
        ]
    );
}

#[test]
fn empty_script_yields_header_only() {
    let migration = run("", BASH, &MockTranslator::new());
    assert!(migration.report.items.is_empty());
    assert_eq!(migration.nurt.lines().count(), 3);
}

#[test]
fn migration_is_deterministic() {
    let source = "rm /srv/a; rm /srv/b\nfor f in *; do echo; done\ncd /srv\n";
    let first = run(source, BASH, &MockTranslator::new());
    let second = run(source, BASH, &MockTranslator::new());
    assert_eq!(first, second);
}

#[test]
fn json_report_has_format_summary_and_items() {
    let mock = MockTranslator::new().program("rm /srv/a", "remove \"/srv/a\"", &["B01"]);
    let migration = run("rm /srv/a\nfor f in *; do echo; done\n", BASH, &mock);
    let json: serde_json::Value = match serde_json::from_str(&migration.report.to_json()) {
        Ok(json) => json,
        Err(error) => panic!("invalid JSON: {error}"),
    };
    assert_eq!(json["format"], REPORT_FORMAT);
    assert_eq!(json["language"], "bash");
    assert_eq!(
        json["summary"],
        serde_json::json!({"statements": 2, "translated": 1, "assumption": 0, "impossible": 1, "error": 0})
    );
    assert_eq!(json["items"][0]["output"], "remove \"/srv/a\"");
    assert_eq!(json["items"][1]["reason"], "loop");
    assert_eq!(json["items"][1]["code"], "E_UNCORRECTABLE");
    assert!(json["items"][0].get("code").is_none());
    assert_eq!(json["fallback"], "run [\"bash\", \"/srv/s.sh\"]");
    assert!(json.get("base").is_none());
}

#[test]
fn text_report_lists_items_and_proposals() {
    let mock = MockTranslator::new().fail(
        "cd /srv/app",
        ErrorCode::Uncorrectable,
        "the program has no current directory",
        &[],
    );
    let migration = run("cd /srv/app\n", BASH, &mock);
    assert_eq!(
        migration.report.to_text(),
        "script /srv/s.sh (bash)\n\
         statements 1: translated 0, assumption 0, impossible 1, error 0\n\
         1:1 impossible: E_UNCORRECTABLE (current_directory)\n  \
         source: cd /srv/app\n  \
         problem: the program has no current directory\n\
         fallback: run [\"bash\", \"/srv/s.sh\"]\n\
         base: \"/srv/app\"\n"
    );
}

#[test]
#[ignore = "timing test; run with --ignored"]
fn large_script_is_migrated_within_one_second() {
    let mut source = String::new();
    for index in 0..LARGE_SCRIPT_LINES {
        source.push_str(&format!(
            "rm -rf /srv/app/build/{index} && cp /srv/a /srv/b; ls /srv | grep x\n"
        ));
    }
    let mock = MockTranslator::new();
    let started = Instant::now();
    let migration = run(&source, BASH, &mock);
    assert!(
        started.elapsed() < LARGE_SCRIPT_LIMIT,
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(
        migration.report.items.len(),
        LARGE_SCRIPT_LINES * PARTS_PER_LINE
    );
}

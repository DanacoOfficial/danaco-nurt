// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The program of a migration holds the comments of the script and the text
//! the migration itself writes, and nothing else.
//!
//! A migrated script once ended with a license header that the script did not
//! have; the header came from the file of the test input, not from the
//! migration. The inputs are therefore literals of this file, which no tool
//! that writes files can add a header to.

use nurt_contracts::spec::Os;
use nurt_migrate::{Migration, Reason, ScriptLanguage, Status};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

use crate::binding::{migrate_bound, translator};

/// Beginnings of the comment lines that the migration writes itself.
const GENERATED_COMMENTS: &[&str] = &[
    "# Source: ",
    "# Statements",
    "# Untranslated statements run unchanged with:",
    "# Relative paths require",
    "# translated: ",
    "# assumption: ",
    "# untranslated: ",
    "#   ",
    "# ]",
];

/// Words of the license header of the files of the repository, which no
/// script of this file contains.
const FOREIGN_WORDS: &[&str] = &[
    "Copyright",
    "Danaco",
    "license terms",
    "All rights reserved",
];

const WITHOUT_COMMENTS: &str = "set -euo pipefail\n\
for f in /var/log/app/*.log; do\n\
  gzip -c \"$f\" | wc -c > \"/tmp/size.txt\"\n\
done\n\
cat /etc/hostname | sort | uniq > /tmp/hosts.txt\n\
find /var/log/app -name '*.old' | wc -l\n";

const WITH_HEADER_COMMENTS: &str = "#!/usr/bin/env bash\n\
#\n\
# Rotates the log files of the application.\n\
# Runs every night.\n\
#\n\
# Author: somebody\n\
set -euo pipefail\n\
\n\
# Size of every log.\n\
for f in /var/log/app/*.log; do\n\
  gzip -c \"$f\" | wc -c > \"/tmp/size.txt\"\n\
done\n\
# Distinct hosts.\n\
cat /etc/hostname | sort | uniq > /tmp/hosts.txt\n\
find /var/log/app -name '*.old' | wc -l\n";

fn migrate(source: &str) -> Migration {
    migrate_bound(source, ScriptLanguage::Bash, &translator(Os::Linux))
}

/// Comment lines of the script, without the interpreter line.
fn comments_of(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter(|line| line.starts_with('#') && !line.starts_with("#!"))
        .collect()
}

fn is_generated(line: &str) -> bool {
    GENERATED_COMMENTS
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

/// Whether the comment line repeats a line of the script, as the lines after
/// the first one of an untranslated statement do.
fn echoes_the_script(line: &str, source: &str) -> bool {
    let text = line.trim_start_matches('#').trim();
    !text.is_empty() && source.lines().any(|script_line| script_line.trim() == text)
}

/// Asserts that the program binds and holds no text that the script lacks.
fn assert_only_the_script_and_the_migration(source: &str, migration: &Migration) {
    let program = &migration.nurt;
    for word in FOREIGN_WORDS {
        assert!(!program.contains(word), "`{word}` in\n{program}");
    }
    let script_comments = comments_of(source);
    for line in program.lines() {
        if line.is_empty() || is_generated(line) || !line.starts_with('#') {
            continue;
        }
        assert!(
            script_comments.contains(&line) || echoes_the_script(line, source),
            "`{line}` is neither a comment of the script nor a line of the migration:\n{program}"
        );
    }
    for comment in script_comments {
        assert!(
            program.lines().any(|line| line == comment),
            "the comment `{comment}` is lost:\n{program}"
        );
    }
    assert!(
        parse(program, Variant::A, &ParseOptions::default()).is_ok(),
        "the program does not parse:\n{program}"
    );
    let failures: Vec<&str> = migration
        .report
        .items
        .iter()
        .filter(|item| item.reason == Some(Reason::Translation) || item.status == Status::Error)
        .map(|item| item.source.as_str())
        .collect();
    assert!(failures.is_empty(), "{failures:?} do not bind:\n{program}");
    assert!(
        migration
            .report
            .items
            .iter()
            .any(|item| matches!(item.status, Status::Translated | Status::Assumption)),
        "nothing was translated:\n{program}"
    );
}

#[test]
fn a_script_without_comments_is_migrated_without_foreign_text() {
    let migration = migrate(WITHOUT_COMMENTS);
    assert_only_the_script_and_the_migration(WITHOUT_COMMENTS, &migration);
    let foreign: Vec<&str> = migration
        .nurt
        .lines()
        .filter(|line| {
            line.starts_with('#')
                && !is_generated(line)
                && !echoes_the_script(line, WITHOUT_COMMENTS)
        })
        .collect();
    assert!(foreign.is_empty(), "{foreign:?} in\n{}", migration.nurt);
}

#[test]
fn header_comments_of_several_lines_are_carried_over_and_the_program_binds() {
    let migration = migrate(WITH_HEADER_COMMENTS);
    assert_only_the_script_and_the_migration(WITH_HEADER_COMMENTS, &migration);
}

#[test]
fn every_line_of_the_header_precedes_the_first_statement() {
    let migration = migrate(WITH_HEADER_COMMENTS);
    let program = &migration.nurt;
    let first_statement = program
        .lines()
        .position(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or_else(|| panic!("no statement in\n{program}"));
    let last_header_line = program
        .lines()
        .position(|line| line == "# Author: somebody")
        .unwrap_or_else(|| panic!("the comment is lost in\n{program}"));
    assert!(last_header_line < first_statement, "{program}");
}

#[test]
fn strict_mode_lines_are_an_assumption_not_an_impossible_statement() {
    for strict in [
        "set -e",
        "set -u",
        "set -o pipefail",
        "set -eu",
        "set -euo pipefail",
    ] {
        let source = format!("{strict}\nmkdir -p /srv/out\n");
        let migration = migrate(&source);
        assert_only_the_script_and_the_migration(&source, &migration);
        let item = &migration.report.items[0];
        assert_eq!(item.source, strict);
        assert_eq!(item.status, Status::Assumption, "{strict}");
        assert!(
            item.output.as_deref().is_some_and(str::is_empty),
            "{strict}"
        );
        assert!(
            migration
                .report
                .items
                .iter()
                .all(|item| item.status.is_translated()),
            "{strict}: {:?}",
            migration.report.items
        );
        assert!(
            !migration.nurt.contains("working directory"),
            "{}",
            migration.nurt
        );
    }
}

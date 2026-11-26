// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Differential test of the division of scripts into statements: the
//! tree-sitter grammars against the lexers of the bash and PowerShell habit
//! tables.
//!
//! Both readers must find the same statements and the same parts of lists
//! joined with `&&`, `||` and `;` in every test script and every command of
//! the common command set. Statements inside constructs without an
//! equivalent (loops, conditions, definitions) are not compared, because the
//! lexers do not know the keywords of these constructs.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_correct::bash::{self, Operator, PosixTokenKind};
use nurt_correct::powershell::lex::{self as powershell, TokenKind};

use super::{Segment, SegmentKind, segment};
use crate::language::ScriptLanguage;

/// Directory of the migration test data, relative to the crate.
const DATA_DIRECTORY: &str = "../../tests/d1/migrate";

/// Directories of scripts with their language and extension.
const SUITES: [(&str, &str, ScriptLanguage); 4] = [
    ("bash", "sh", ScriptLanguage::Bash),
    ("powershell", "ps1", ScriptLanguage::PowerShell),
    ("mock", "sh", ScriptLanguage::Bash),
    ("mock", "ps1", ScriptLanguage::PowerShell),
];

/// Files of the common command set: one script of one line per command.
const COMMON_SETS: [(&str, ScriptLanguage); 2] = [
    ("common-500/bash.txt", ScriptLanguage::Bash),
    ("common-500/powershell.txt", ScriptLanguage::PowerShell),
];

type Start = (u32, u32);

fn data_directory() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join(DATA_DIRECTORY)
}

/// Starts of the statements and list parts found by the grammar.
fn grammar_starts(segments: &[Segment]) -> BTreeSet<Start> {
    segments
        .iter()
        .filter_map(|segment| match &segment.kind {
            SegmentKind::Statement(statement) => Some(statement),
            _ => None,
        })
        .flat_map(|statement| statement.parts.iter())
        .map(|part| (part.start.line, part.start.column))
        .collect()
}

/// Whether a position lies in a statement found by the grammar.
fn in_statement(segments: &[Segment], (line, column): Start) -> bool {
    segments.iter().any(|segment| {
        matches!(segment.kind, SegmentKind::Statement(_))
            && (segment.start.line, segment.start.column) <= (line, column)
            && line <= segment.end_line
    })
}

/// Starts of statements and list parts found by the POSIX lexer.
fn bash_starts(source: &str) -> Option<Vec<Start>> {
    let tokens = bash::lex(source).ok()?;
    let mut starts = Vec::new();
    let mut expect_start = true;
    for token in &tokens {
        match &token.kind {
            PosixTokenKind::Newline
            | PosixTokenKind::Operator(
                Operator::And
                | Operator::Or
                | Operator::Semicolon
                | Operator::Background
                | Operator::CaseEnd,
            ) => expect_start = true,
            PosixTokenKind::Comment(_) => {}
            _ if expect_start => {
                starts.push((token.span.line, token.span.column));
                expect_start = false;
            }
            _ => {}
        }
    }
    Some(starts)
}

/// Starts of statements and list parts found by the PowerShell lexer
/// outside braces, parentheses and sub-expressions.
fn powershell_starts(source: &str) -> Option<Vec<Start>> {
    let tokens = powershell::lex(source).ok()?;
    let mut starts = Vec::new();
    let mut expect_start = true;
    let mut depth = 0_usize;
    for token in &tokens {
        let separator = match &token.kind {
            TokenKind::Newline | TokenKind::Semicolon => depth == 0,
            TokenKind::Operator(operator) => depth == 0 && matches!(operator.as_str(), "&&" | "||"),
            _ => false,
        };
        if separator {
            expect_start = true;
            continue;
        }
        if expect_start && depth == 0 {
            starts.push((token.span.line, token.span.column));
            expect_start = false;
        }
        match &token.kind {
            TokenKind::OpenBrace
            | TokenKind::OpenParen
            | TokenKind::SubExprOpen
            | TokenKind::ArrayOpen
            | TokenKind::HashOpen => depth += 1,
            TokenKind::CloseBrace | TokenKind::CloseParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Some(starts)
}

fn lexer_starts(source: &str, language: ScriptLanguage) -> Option<Vec<Start>> {
    match language {
        ScriptLanguage::Bash => bash_starts(source),
        ScriptLanguage::PowerShell => powershell_starts(source),
    }
}

/// Starts found by the lexer in each statement segment read on its own, for
/// scripts that the lexer cannot read as a whole.
fn segment_lexer_starts(
    source: &str,
    segments: &[Segment],
    language: ScriptLanguage,
) -> Vec<Start> {
    let mut starts = Vec::new();
    for segment in segments {
        if !matches!(segment.kind, SegmentKind::Statement(_)) {
            continue;
        }
        let Some(relative) = lexer_starts(segment.text(source), language) else {
            continue;
        };
        starts.extend(relative.into_iter().map(|(line, column)| {
            let column = if line == 1 {
                segment.start.column + column - 1
            } else {
                column
            };
            (segment.start.line + line - 1, column)
        }));
    }
    starts
}

/// Differences between the two divisions of one script, as readable lines.
fn differences(name: &str, source: &str, language: ScriptLanguage) -> Vec<String> {
    let segments = segment(source, language).unwrap();
    let grammar = grammar_starts(&segments);
    let lexer: BTreeSet<Start> = lexer_starts(source, language)
        .unwrap_or_else(|| segment_lexer_starts(source, &segments, language))
        .into_iter()
        .filter(|start| in_statement(&segments, *start))
        .collect();
    let only_grammar = grammar
        .difference(&lexer)
        .map(|(line, column)| format!("{name}:{line}:{column} found only by the grammar"));
    let only_lexer = lexer
        .difference(&grammar)
        .map(|(line, column)| format!("{name}:{line}:{column} found only by the lexer"));
    only_grammar.chain(only_lexer).collect()
}

fn scripts(directory: &str, extension: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(data_directory().join(directory))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|value| value == extension))
        .collect();
    paths.sort();
    paths
}

#[test]
fn grammar_and_lexer_divide_test_scripts_alike() {
    let mut found = Vec::new();
    let mut compared = 0_usize;
    for (directory, extension, language) in SUITES {
        for path in scripts(directory, extension) {
            let source = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            found.extend(differences(&name, &source, language));
            compared += 1;
        }
    }
    assert!(compared >= 82, "only {compared} scripts compared");
    assert!(found.is_empty(), "divisions differ:\n{}", found.join("\n"));
}

#[test]
fn grammar_and_lexer_divide_common_commands_alike() {
    let mut found = Vec::new();
    for (file, language) in COMMON_SETS {
        let text = fs::read_to_string(data_directory().join(file)).unwrap();
        for (index, command) in text.lines().enumerate() {
            let name = format!("{file} command {}", index + 1);
            found.extend(differences(&name, &format!("{command}\n"), language));
        }
    }
    assert!(found.is_empty(), "divisions differ:\n{}", found.join("\n"));
}

#[test]
fn lexer_starts_skip_nested_script_blocks() {
    let starts =
        powershell_starts("Get-Process | Where-Object { $_.CPU -gt 1; $_.Id }\nGet-Date\n");
    assert_eq!(starts, Some(vec![(1, 1), (2, 1)]));
}

#[test]
fn lexer_starts_split_lists() {
    let starts = bash_starts("mkdir /a && cp /b /a; ls /a || echo x\n");
    assert_eq!(starts, Some(vec![(1, 1), (1, 13), (1, 23), (1, 32)]));
}

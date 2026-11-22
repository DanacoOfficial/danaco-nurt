// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Syntax errors of the call notation (variant B) and their ready fixes.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::error::{At, Error, ErrorCode, ErrorData, Stage};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

/// Largest number of ready fixes of one error.
const MAX_FIXES: usize = 3;

fn error(text: &str) -> ErrorData {
    let error = parse(text, Variant::B, &ParseOptions::default())
        .err()
        .unwrap_or_else(|| panic!("{text:?} parses"));
    Error::from(error).into_data()
}

/// Checks the common members of a syntax error and that every fix parses.
fn assert_syntax_error(text: &str, at: At, fixes: &[&str]) -> ErrorData {
    let data = error(text);
    assert_eq!(data.code, ErrorCode::Syntax, "{text:?}: {data:?}");
    assert_eq!(
        data.at.and_then(|at| at.stage),
        Some(Stage::Parse),
        "{text:?}"
    );
    assert_eq!(
        data.at,
        Some(at.with_stage(Stage::Parse)),
        "{text:?}: {data:?}"
    );
    assert_eq!(data.fix, fixes, "{text:?}");
    assert_eq!(data.help.as_deref(), Some("help syntax"), "{text:?}");
    assert!(data.found.is_some(), "{text:?}: found");
    assert!(data.problem.is_some(), "{text:?}: problem");
    assert!(data.expected.is_some(), "{text:?}: expected");
    assert_message_style(&data.message);
    for fix in &data.fix {
        assert!(
            parse(fix, Variant::B, &ParseOptions::default()).is_ok(),
            "{text:?}: fix {fix:?} does not parse"
        );
    }
    data
}

fn assert_message_style(message: &str) {
    assert!(!message.is_empty());
    assert!(!message.ends_with('.'), "{message:?} ends with a period");
    assert!(
        !message.starts_with(|first: char| first.is_uppercase()),
        "{message:?} starts with a capital letter"
    );
}

#[test]
fn missing_parentheses_after_command_offers_empty_call() {
    assert_syntax_error(
        "find(\"/a\") |> remove",
        At::new(1, 15, 6),
        &["find(\"/a\") |> remove()"],
    );
    assert_syntax_error("remove", At::new(1, 1, 6), &["remove()"]);
    assert_syntax_error(
        "find(\"/a\")\n|> remove\n|> count()",
        At::new(2, 4, 6),
        &["find(\"/a\")\n|> remove()\n|> count()"],
    );
    assert_syntax_error(
        "verify(git.status() |> count)",
        At::new(1, 24, 5),
        &["verify(git.status() |> count())"],
    );
}

#[test]
fn missing_parentheses_before_arguments_has_no_fix() {
    let data = error("find \"/a\" glob: \"*.go\"");
    assert_eq!(data.code, ErrorCode::Syntax);
    assert_eq!(data.at, Some(At::new(1, 1, 4).with_stage(Stage::Parse)));
    assert!(data.fix.is_empty(), "{:?}", data.fix);
}

#[test]
fn missing_comma_between_arguments_offers_comma() {
    assert_syntax_error(
        "find(\"/a\" glob: \"*.go\")",
        At::new(1, 11, 4),
        &["find(\"/a\", glob: \"*.go\")"],
    );
    assert_syntax_error(
        "sort(size order: desc)",
        At::new(1, 11, 5),
        &["sort(size, order: desc)"],
    );
    assert_syntax_error(
        "write(\"/a\"\"b\")",
        At::new(1, 11, 3),
        &["write(\"/a\", \"b\")"],
    );
}

#[test]
fn equals_sign_after_parameter_name_offers_colon() {
    assert_syntax_error(
        "find(\"/a\", glob=\"*.go\")",
        At::new(1, 16, 1),
        &["find(\"/a\", glob: \"*.go\")"],
    );
    assert_syntax_error(
        "find(\"/a\", glob = \"*.go\")",
        At::new(1, 17, 1),
        &["find(\"/a\", glob: \"*.go\")"],
    );
    assert_syntax_error(
        "move(into=\"/b\")",
        At::new(1, 10, 1),
        &["move(into: \"/b\")"],
    );
}

#[test]
fn space_before_colon_of_named_argument_offers_attached_colon() {
    assert_syntax_error(
        "find(\"/a\", glob : \"*.go\")",
        At::new(1, 17, 1),
        &["find(\"/a\", glob: \"*.go\")"],
    );
}

#[test]
fn unclosed_parenthesis_before_pipe_offers_closing_parenthesis() {
    assert_syntax_error(
        "find(\"/a\" |> count()",
        At::new(1, 11, 2),
        &["find(\"/a\") |> count()"],
    );
    assert_syntax_error(
        "find(\"/a\")\n|> filter(size > 1mb\n|> count()",
        At::new(3, 1, 2),
        &["find(\"/a\")\n|> filter(size > 1mb)\n|> count()"],
    );
    assert_syntax_error(
        "find(\"/a\", glob: \"*\" |> count()",
        At::new(1, 22, 2),
        &["find(\"/a\", glob: \"*\") |> count()"],
    );
}

#[test]
fn unclosed_parenthesis_at_end_offers_closing_parenthesis() {
    assert_syntax_error("find(\"/a\"", At::new(1, 10, 0), &["find(\"/a\")"]);
    assert_syntax_error(
        "verify(run([\"go\"]) |> count()",
        At::new(1, 30, 0),
        &["verify(run([\"go\"]) |> count())"],
    );
}

#[test]
fn extra_closing_parenthesis_is_reported() {
    let data = error("count())");
    assert_eq!(data.code, ErrorCode::Syntax);
    assert_eq!(data.at, Some(At::new(1, 8, 1).with_stage(Stage::Parse)));
}

#[test]
fn two_statements_on_one_line_offer_line_break() {
    let data = error("history() journal.space()");
    assert_eq!(data.code, ErrorCode::Syntax);
    assert_eq!(data.at, Some(At::new(1, 11, 7).with_stage(Stage::Parse)));
    assert_eq!(data.fix, ["history()\njournal.space()"]);
    assert_message_style(&data.message);
}

#[test]
fn keyword_in_command_position_is_reported() {
    let data = error("find(\"/a\") |> not()");
    assert_eq!(data.code, ErrorCode::Syntax);
    assert_eq!(data.at, Some(At::new(1, 15, 3).with_stage(Stage::Parse)));
    assert_message_style(&data.message);
}

#[test]
fn reference_after_pipe_is_reported() {
    let data = error("find(\"/a\") |> @x");
    assert_eq!(data.code, ErrorCode::Syntax);
    let at = data.at.unwrap();
    assert_eq!((at.line, at.column), (Some(1), Some(15)));
}

#[test]
fn every_error_has_no_more_than_three_fixes() {
    for text in [
        "remove",
        "find(\"/a\" glob: \"*\")",
        "find(glob=\"*\")",
        "find(\"/a\" |> count()",
        "find(\"/a\"",
        "count())",
        "history() count()",
        "f(,)",
        "f(a:)",
        "f(a: 1 b: 2 c: 3)",
    ] {
        let data = error(text);
        assert!(data.fix.len() <= MAX_FIXES, "{text:?}: {:?}", data.fix);
        assert_message_style(&data.message);
    }
}

#[test]
fn lexical_errors_keep_their_codes() {
    assert_eq!(error("write(\"/a\", \"\\q\")").code, ErrorCode::Escape);
    assert_eq!(error("write(\"/a\", \"open)").code, ErrorCode::Lexical);
}

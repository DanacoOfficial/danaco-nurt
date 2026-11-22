// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Errors of the variant A parser: code, position, problem and ready
//! corrections.

mod parse_a_support;

use nurt_contracts::error::{Error, ErrorCode, Stage};
use nurt_syntax::error::SyntaxError;
use parse_a_support::parse_a;

fn error_of(text: &str) -> SyntaxError {
    match parse_a(text) {
        Ok(document) => panic!("{text:?} parsed into {document:?}"),
        Err(error) => error,
    }
}

/// Asserts the code `E_SYNTAX`, the problem, the line and the column.
fn assert_syntax(text: &str, problem: &str, line: u32, column: u32) -> SyntaxError {
    let error = error_of(text);
    assert_eq!(error.code, ErrorCode::Syntax, "{text:?}: {error}");
    assert_eq!(error.problem.as_deref(), Some(problem), "{text:?}: {error}");
    let at = error.at.expect("position");
    assert_eq!((at.line, at.column), (line, column), "{text:?}: {error}");
    assert_eq!(error.help.as_deref(), Some("help syntax"));
    assert!(!error.message.ends_with('.'), "{}", error.message);
    assert!(
        error.message.starts_with(|c: char| !c.is_ascii_uppercase()),
        "{}",
        error.message
    );
    error
}

#[test]
fn rejects_a_program_without_statements() {
    for text in ["", "\n\n", "# only a comment\n"] {
        let error = error_of(text);
        assert_eq!(error.problem.as_deref(), Some("empty_program"));
        assert_eq!(error.message, "a program has at least one statement");
    }
}

#[test]
fn rejects_two_statements_on_one_line_with_a_line_break_fix() {
    let error = assert_syntax("find \"/a\" let x = count", "statement_on_same_line", 1, 11);
    assert_eq!(error.fix, ["find \"/a\"\nlet x = count"]);
    let error = assert_syntax("@x @y", "statement_on_same_line", 1, 4);
    assert_eq!(error.fix, ["@x\n@y"]);
}

#[test]
fn rejects_a_reference_after_the_pipeline_operator() {
    assert_syntax("find \"/a\" |> @x", "reference_after_pipe", 1, 14);
}

#[test]
fn rejects_keywords_where_a_command_or_a_name_must_stand() {
    assert_syntax("in \"/a\"", "expected_statement", 1, 1);
    let error = assert_syntax("find \"/a\" |> in", "expected_command", 1, 14);
    assert_eq!(
        error.message,
        "`in` is a keyword and cannot be a command name"
    );
    let error = assert_syntax("let in = count", "expected_name", 1, 5);
    assert!(error.message.starts_with("`in` is a keyword"));
    assert_syntax("\"abc\"", "expected_statement", 1, 1);
    assert_syntax("git. status", "unexpected_token", 1, 6);
    assert_syntax("git.in", "unexpected_token", 1, 5);
}

#[test]
fn rejects_a_command_name_longer_than_129_characters() {
    let member = "a".repeat(64);
    assert!(parse_a(&format!("{member}.{member}")).is_ok());
    assert_syntax(&format!("{member}.{member}.b"), "name_too_long", 1, 1);
}

#[test]
fn rejects_chained_comparisons_with_an_and_fix() {
    let error = assert_syntax("filter a < b < c", "chained_comparison", 1, 14);
    assert_eq!(error.fix, ["filter a < b and b < c"]);
}

#[test]
fn rejects_double_negations_with_parenthesized_fixes() {
    let error = assert_syntax("filter not not x", "double_not", 1, 12);
    assert_eq!(error.fix, ["filter not (not x)"]);
    let error = assert_syntax("filter a * - -x", "double_minus", 1, 14);
    assert_eq!(error.fix, ["filter a * -(-x)"]);
}

#[test]
fn rejects_not_as_a_comparison_operand() {
    let error = assert_syntax("filter a == not b", "not_in_comparison", 1, 13);
    assert_eq!(error.fix, ["filter a == (not b)"]);
}

#[test]
fn rejects_a_unary_plus() {
    let error = assert_syntax("first +1", "unary_plus", 1, 7);
    assert_eq!(error.fix, ["first 1"]);
    assert_syntax("filter a * +1", "unary_plus", 1, 12);
}

#[test]
fn rejects_named_argument_forms_of_other_notations() {
    let error = assert_syntax("find \"/a\" glob : \"x\"", "space_before_colon", 1, 16);
    assert_eq!(error.fix, ["find \"/a\" glob: \"x\""]);
    let error = assert_syntax("find \"/a\", glob: \"x\"", "comma_between_arguments", 1, 10);
    assert_eq!(error.fix, ["find \"/a\" glob: \"x\""]);
    let error = assert_syntax("find \"/a\" glob=\"x\"", "equals_in_named_argument", 1, 15);
    assert_eq!(error.fix, ["find \"/a\" glob: \"x\""]);
}

#[test]
fn rejects_a_named_argument_without_a_value() {
    let error = assert_syntax("find \"/a\" glob:", "missing_value", 1, 16);
    assert_eq!(error.expected, ["value"]);
    assert_syntax("find \"/a\" glob:\n|> count", "missing_value", 2, 1);
    assert_syntax("find glob: depth: 1", "missing_value", 1, 12);
}

#[test]
fn rejects_a_spread_outside_a_list_with_a_list_fix() {
    let error = assert_syntax("info ...@x", "spread_outside_list", 1, 6);
    assert_eq!(error.fix, ["info [...@x]"]);
    assert_syntax("info {a: ...@x}", "spread_outside_list", 1, 10);
}

#[test]
fn rejects_the_second_occurrence_of_a_map_key() {
    assert_syntax("x {a: 1, a: 2}", "duplicate_key", 1, 10);
    assert_syntax("x {a: 1, \"a\": 2}", "duplicate_key", 1, 10);
}

#[test]
fn rejects_range_bounds_that_are_not_integers() {
    assert_syntax("lines \"/a\" 1.5..", "range_bound", 1, 12);
    assert_syntax("lines \"/a\" -1..5", "range_bound", 1, 12);
    assert_syntax("lines \"/a\" 1..2.5", "range_bound", 1, 15);
    assert_syntax("lines \"/a\" x..5", "range_bound", 1, 12);
    assert_syntax("lines \"/a\" 1..2..3", "unexpected_token", 1, 16);
}

#[test]
fn rejects_calls_on_fields_and_references() {
    assert_syntax("filter a.b(x)", "call_on_field", 1, 10);
    assert_syntax("filter @a(x)", "call_on_field", 1, 9);
    assert_syntax("filter @a.b(x)", "call_on_field", 1, 11);
}

#[test]
fn rejects_keywords_as_values() {
    assert_syntax("filter a + not b", "keyword_as_value", 1, 12);
    assert_syntax("filter and", "unexpected_token", 1, 8);
}

#[test]
fn rejects_malformed_calls_lists_and_parentheses() {
    let error = assert_syntax("x f(a,)", "trailing_comma", 1, 6);
    assert_eq!(error.fix, ["x f(a)"]);
    let error = assert_syntax("x [1 2]", "unexpected_token", 1, 6);
    assert_eq!(error.expected, ["`,`", "`]`"]);
    assert_syntax("x {a 1}", "unexpected_token", 1, 6);
    assert_syntax("x {1: 2}", "unexpected_token", 1, 4);
    assert_syntax("x (count", "unexpected_token", 1, 9);
    assert_syntax("x ()", "unexpected_token", 1, 4);
    assert_syntax("count )", "unexpected_token", 1, 7);
    assert_syntax("x (1 + 2 |> count)", "expected_command", 1, 4);
    assert_syntax("x @ y", "unexpected_token", 1, 5);
    assert_syntax("x a. b", "unexpected_token", 1, 6);
}

#[test]
fn reports_found_fragment_and_expected_tokens() {
    let error = error_of("count )");
    assert_eq!(error.found.as_deref(), Some(")"));
    assert_eq!(error.expected, ["argument", "`|>`", "end of line"]);
    assert_eq!(
        error.message,
        "expected argument, `|>` or end of line, found `)`"
    );
    let error = error_of("x (count");
    assert_eq!(error.found.as_deref(), Some("end of input"));
}

#[test]
fn passes_lexical_errors_through() {
    let error = error_of("find '/a'");
    assert_eq!(error.code, ErrorCode::Lexical);
    let error = error_of("find \"\\d\"");
    assert_eq!(error.code, ErrorCode::Escape);
}

#[test]
fn converts_into_a_language_error_of_the_parse_stage() {
    let error = Error::from(error_of("filter a < b < c"));
    assert_eq!(error.code, ErrorCode::Syntax);
    assert_eq!(error.stage, Some(Stage::Parse));
    assert_eq!(error.fix, ["filter a < b and b < c"]);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/parse_a_errors.rs
// Responsibility:   Errors of the variant A parser: code, position, problem and ready.
// Dependencies:     nurt_contracts, parse_a_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Errors of the variant A parser: code, position, problem and ready
//! corrections.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

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
    assert_eq!(
        (at.line, at.column),
        (Some(line), Some(column)),
        "{text:?}: {error}"
    );
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
    assert_syntax("x (1 + 2 |> count)", "expected_command", 1, 4);
    assert_syntax("x @ y", "unexpected_token", 1, 5);
    assert_syntax("x a. b", "unexpected_token", 1, 6);
}

#[test]
fn rejects_an_unmatched_closing_bracket_with_a_removal_fix() {
    let error = assert_syntax("count )", "unmatched_bracket", 1, 7);
    assert_eq!(error.fix, ["count"]);
    assert_eq!(error.message, "`)` closes no open bracket; remove it");
    let error = assert_syntax("find \"/a\" ] |> count", "unmatched_bracket", 1, 11);
    assert_eq!(error.fix, ["find \"/a\" |> count"]);
    let error = assert_syntax("count }x", "unmatched_bracket", 1, 7);
    assert_eq!(error.fix, ["count x"]);
}

#[test]
fn reports_found_fragment_and_expected_tokens() {
    let error = error_of("count :");
    assert_eq!(error.found.as_deref(), Some(":"));
    assert_eq!(error.expected, ["argument", "`|>`", "end of line"]);
    assert_eq!(
        error.message,
        "expected argument, `|>` or end of line, found `:`"
    );
    let error = error_of("x (count");
    assert_eq!(error.found.as_deref(), Some("end of input"));
}

/// The statement, pipeline and expression grammar is shared by the three
/// variants; its errors name the notation of the variant parsed.
mod shared_grammar {
    use nurt_contracts::error::ErrorCode;
    use nurt_syntax::error::SyntaxError;
    use nurt_syntax::parse::{ParseOptions, Variant, parse};

    fn error_in(variant: Variant, text: &str) -> SyntaxError {
        match parse(text, variant, &ParseOptions::default()) {
            Ok(document) => panic!("{text:?} parsed into {document:?}"),
            Err(error) => error,
        }
    }

    fn assert_error(
        variant: Variant,
        text: &str,
        problem: &str,
        column: u32,
        fix: &[&str],
    ) -> SyntaxError {
        let error = error_in(variant, text);
        assert_eq!(error.code, ErrorCode::Syntax, "{text:?}: {error}");
        assert_eq!(error.problem.as_deref(), Some(problem), "{text:?}: {error}");
        let at = error.at.expect("position");
        assert_eq!((at.line, at.column), (Some(1), Some(column)), "{text:?}");
        assert_eq!(error.fix, fix, "{text:?}");
        assert!(!error.message.ends_with('.'), "{}", error.message);
        error
    }

    fn parses(variant: Variant, text: &str) -> bool {
        parse(text, variant, &ParseOptions::default()).is_ok()
    }

    #[test]
    fn variant_b_rejects_a_bare_command_before_the_pipeline_operator() {
        let error = assert_error(
            Variant::B,
            "verify((count |> first(1)))",
            "missing_parentheses",
            9,
            &["verify((count() |> first(1)))"],
        );
        assert_eq!(error.at.and_then(|at| at.length), Some(5));
        assert_eq!(error.expected, ["`(`"]);
        assert!(parses(Variant::B, "verify((count() |> first(1)))"));
        assert_error(
            Variant::B,
            "verify((git.status |> count()))",
            "missing_parentheses",
            9,
            &["verify((git.status() |> count()))"],
        );
        assert!(parses(Variant::B, "verify((@x |> count()))"));
    }

    #[test]
    fn variants_a_and_c_accept_a_bare_command_before_the_pipeline_operator() {
        assert!(parses(Variant::A, "verify (count |> first 1)"));
        assert!(parses(Variant::C, "verify check=(count |> first n=1)"));
    }

    #[test]
    fn variant_b_reports_a_comma_after_a_stage_in_its_own_notation() {
        let error = assert_error(Variant::B, "count(),", "comma_after_stage", 8, &["count()"]);
        assert!(!error.message.contains("spaces"), "{}", error.message);
        assert_eq!(error.expected, ["`|>`", "end of line"]);
        assert_error(
            Variant::B,
            "find(\"/a\") , |> count()",
            "comma_after_stage",
            12,
            &["find(\"/a\") |> count()"],
        );
        assert_error(
            Variant::B,
            "find(\"/a\"), glob: \"*.go\")",
            "comma_after_stage",
            11,
            &[],
        );
    }

    #[test]
    fn variant_a_names_spaces_as_the_argument_separator() {
        let error = assert_error(
            Variant::A,
            "find \"/a\", glob: \"x\"",
            "comma_between_arguments",
            10,
            &["find \"/a\" glob: \"x\""],
        );
        assert!(error.message.contains("spaces"), "{}", error.message);
    }

    #[test]
    fn equals_after_a_stage_is_a_named_argument_only_in_variant_a() {
        assert_error(
            Variant::A,
            "find \"/a\" glob=\"x\"",
            "equals_in_named_argument",
            15,
            &["find \"/a\" glob: \"x\""],
        );
        let error = assert_error(Variant::B, "count()=1", "unexpected_token", 8, &[]);
        assert_eq!(error.expected, ["`|>`", "end of line"]);
    }

    #[test]
    fn every_variant_removes_an_unmatched_closing_parenthesis() {
        assert_error(Variant::B, "count())", "unmatched_bracket", 8, &["count()"]);
        assert_error(Variant::A, "count )", "unmatched_bracket", 7, &["count"]);
        assert_error(
            Variant::C,
            "find path=\"/a\")",
            "unmatched_bracket",
            15,
            &["find path=\"/a\""],
        );
        assert!(parses(Variant::B, "count()"));
    }

    #[test]
    fn variant_b_closes_the_arguments_when_a_value_is_missing_before_the_pipeline_operator() {
        let error = assert_error(
            Variant::B,
            "find(\"/a\", |> count()",
            "unclosed_arguments",
            12,
            &["find(\"/a\") |> count()"],
        );
        assert_eq!(error.expected, ["value", "`)`"]);
        assert!(parses(Variant::B, "find(\"/a\") |> count()"));
        assert_error(
            Variant::B,
            "find( |> count()",
            "unclosed_arguments",
            7,
            &["find() |> count()"],
        );
        assert!(parses(Variant::B, "find() |> count()"));
        assert_error(
            Variant::B,
            "let x = find(\"/a\", |> count()",
            "unclosed_arguments",
            20,
            &["let x = find(\"/a\") |> count()"],
        );
    }

    #[test]
    fn a_missing_value_without_a_certain_closing_has_no_fix() {
        assert_error(
            Variant::B,
            "find(glob: |> count()",
            "unexpected_token",
            12,
            &[],
        );
        assert_error(
            Variant::B,
            "verify(run([\"go\"], |> count())",
            "unexpected_token",
            20,
            &[],
        );
        assert_error(Variant::B, "f(,)", "unexpected_token", 3, &[]);
        assert_error(
            Variant::A,
            "find \"/a\" glob: |> count",
            "missing_value",
            17,
            &[],
        );
    }
}

#[test]
fn locates_an_invalid_escape_by_scalar_value_columns_and_lines() {
    let error = error_of("find \"żółw\\q\"");
    assert_eq!(error.code, ErrorCode::Escape);
    let at = error.at.expect("position");
    assert_eq!(
        (at.line, at.column, at.length),
        (Some(1), Some(11), Some(2))
    );
    let error = error_of("find \"ab\ncd\\q\"");
    assert_eq!(error.code, ErrorCode::Escape);
    let at = error.at.expect("position");
    assert_eq!((at.line, at.column, at.length), (Some(2), Some(3), Some(2)));
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
    assert_eq!(error.stage(), Some(Stage::Parse));
    assert_eq!(error.fix, ["filter a < b and b < c"]);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
